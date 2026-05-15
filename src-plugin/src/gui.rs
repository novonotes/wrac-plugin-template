//! この plugin 固有の WebView GUI runtime。
//!
//! GUI 本体は `src-gui/` の HTML/CSS/TypeScript。この module はそれを embed した
//! WebView を host window に貼り付け、[`wxp`] の command/channel で frontend と
//! 通信する。
//!
//! 役割分担:
//! - `wrac_wxp_gui`: host UI thread の所有、callback dispatch、parent handle 変換
//!   といった format 共通の厄介事
//! - この module    : WebView の中身・登録 command・resize/scale など製品固有部分

use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use directories::ProjectDirs;
use novonotes_run_loop::{RunLoop, RunLoopSender};
use parking_lot::Mutex;
use run_loop_timer::Timer;
use serde_json::json;
use wrac_clap_adapter::{
    GuiConfiguration, GuiSize, HostGuiResizeRequester, HostParameterEditNotifier, PluginError,
    PluginResult,
};
use wrac_wxp_gui::{
    DpiConverter, GuiSizeLimits, ParentWindowHandle, WxpGuiController, WxpGuiResizeHandle,
    WxpGuiRuntime, gui_size_to_logical,
};
use wxp::{
    Channel, WebContext, WxpCommandHandler, WxpWebView, WxpWebViewBuilder, dpi::LogicalSize,
};

use crate::commands::register_commands;
use crate::plugin::{PARAM_GAIN_ID, PLUGIN_ID, parameter_value_text};
use crate::state::{EditorPage, ProjectStateStore, SharedState};

// GUI window のサイズ範囲 (pixel)。host は default で開き、resize は min..=max に clamp。
const DEFAULT_GUI_SIZE: GuiSize = GuiSize {
    width: 320,
    height: 380,
};
const MIN_GUI_SIZE: GuiSize = GuiSize {
    width: 320,
    height: 380,
};
const MAX_GUI_SIZE: GuiSize = GuiSize {
    width: 720,
    height: 720,
};

// resize 時にクランプする論理ピクセルの上下限。
const MIN_LOGICAL_GUI_SIZE: LogicalSize<f64> = LogicalSize::new(320.0, 340.0);
const MAX_LOGICAL_GUI_SIZE: LogicalSize<f64> = LogicalSize::new(720.0, 720.0);

// release のみ frontend zip を埋め込む。debug は Vite dev server を見るので不要。
#[cfg(not(debug_assertions))]
const FRONTEND_ZIP: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wrac_gain_plugin_gui.zip"));

pub(crate) struct GuiIntegration {
    pub(crate) controller: Arc<WxpGuiController>,
    pub(crate) notifier: Arc<GuiStateNotifier>,
}

#[derive(Clone)]
struct GuiRuntimeDependencies {
    project_state: Arc<ProjectStateStore>,
    shared: Arc<SharedState>,
    gui_notifier: Arc<GuiStateNotifier>,
    host_parameter_edit_notifier: Arc<dyn HostParameterEditNotifier>,
    host_gui_resize_requester: Arc<dyn HostGuiResizeRequester>,
    resize_handle: WxpGuiResizeHandle,
}

/// plugin core が使う GUI extension 一式を組み立てる。
/// GUI 固有の詳細を `plugin.rs` から切り離すための入口。
pub(crate) fn create_gui_integration(
    project_state: Arc<ProjectStateStore>,
    shared: Arc<SharedState>,
    host_parameter_edit_notifier: Arc<dyn HostParameterEditNotifier>,
    host_gui_resize_requester: Arc<dyn HostGuiResizeRequester>,
) -> GuiIntegration {
    let notifier = Arc::new(GuiStateNotifier::new());
    let resize_handle = WxpGuiResizeHandle::new(
        DEFAULT_GUI_SIZE,
        GuiSizeLimits {
            min: MIN_GUI_SIZE,
            max: MAX_GUI_SIZE,
        },
    );
    let runtime_dependencies = GuiRuntimeDependencies {
        project_state,
        shared,
        gui_notifier: notifier.clone(),
        host_parameter_edit_notifier,
        host_gui_resize_requester,
        resize_handle: resize_handle.clone(),
    };
    let controller = Arc::new(WxpGuiController::new_with_resize_handle(
        move |configuration, initial_size, parent| {
            WracGainGuiRuntime::create(
                runtime_dependencies.clone(),
                configuration,
                initial_size,
                parent,
            )
            .map(|runtime| Box::new(runtime) as Box<dyn WxpGuiRuntime>)
        },
        resize_handle,
    ));

    GuiIntegration {
        controller,
        notifier,
    }
}

/// WebView 側へ GUI state を push する通知口。通知タイミングは呼び出し元が決める。
pub(crate) struct GuiStateNotifier {
    next_subscription_id: AtomicU64,
    subscriptions: Mutex<HashMap<GuiSubscriptionId, GuiSubscription>>,
}

/// WebView 側 subscriber 1 つぶんの登録情報。
///
/// `kind` (何の stream か) と `channel` (送り先) を分けて持つことで、parameter /
/// meter / analyzer などを個別に購読・解除でき、古い cleanup が別の購読を
/// 巻き込んで消す事故も防げる。
#[derive(Clone)]
struct GuiSubscription {
    kind: GuiSubscriptionKind,
    // 通知を UI thread に戻すための run loop sender。
    sender: RunLoopSender,
    // WebView 側 JS の subscriber に値を送る channel。
    channel: Channel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct GuiSubscriptionId(u64);

impl GuiSubscriptionId {
    pub(crate) fn get(self) -> u64 {
        self.0
    }

    pub(crate) fn from_raw(value: u64) -> Self {
        Self(value)
    }
}

/// 購読の種類。meter や analyzer の stream を足すときは variant を追加し、
/// `notify_*` でその variant の subscription にだけ配信する
/// (Channel を増やさず種別で振り分ける設計)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuiSubscriptionKind {
    Parameters,
    EditorPage,
}

impl GuiStateNotifier {
    fn new() -> Self {
        Self {
            next_subscription_id: AtomicU64::new(1),
            subscriptions: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn subscribe_parameters(&self, channel: Channel) -> GuiSubscriptionId {
        self.subscribe(GuiSubscriptionKind::Parameters, channel)
    }

    pub(crate) fn subscribe_editor_page(&self, channel: Channel) -> GuiSubscriptionId {
        self.subscribe(GuiSubscriptionKind::EditorPage, channel)
    }

    fn subscribe(&self, kind: GuiSubscriptionKind, channel: Channel) -> GuiSubscriptionId {
        // id は wxp の Channel id とは独立に採番。transport と購読 lifecycle を
        // 別々に管理するため。
        let id = GuiSubscriptionId(self.next_subscription_id.fetch_add(1, Ordering::Relaxed));
        self.subscriptions.lock().insert(
            id,
            GuiSubscription {
                kind,
                sender: RunLoop::sender(),
                channel,
            },
        );
        id
    }

    pub(crate) fn unsubscribe(&self, id: GuiSubscriptionId) {
        self.subscriptions.lock().remove(&id);
    }

    pub(crate) fn clear_subscriptions(&self) {
        self.subscriptions.lock().clear();
    }

    pub(crate) fn notify_parameter(&self, parameter_id: u32, value: f32) {
        self.notify(
            GuiSubscriptionKind::Parameters,
            parameter_payload(parameter_id, value),
        );
    }

    pub(crate) fn notify_editor_page(&self, editor_page: EditorPage) {
        self.notify(
            GuiSubscriptionKind::EditorPage,
            editor_page_payload(editor_page),
        );
    }

    fn notify(&self, kind: GuiSubscriptionKind, payload: serde_json::Value) {
        // 送り先が notifier を再入しても deadlock しないよう、配信対象を
        // clone してから lock を離す。
        let subscriptions: Vec<_> = self
            .subscriptions
            .lock()
            .values()
            .filter(|subscription| subscription.kind == kind)
            .cloned()
            .collect();
        if subscriptions.is_empty() {
            // GUI が開いていなければ送り先がないので何もしない。
            return;
        }

        for subscription in subscriptions {
            let payload = payload.clone();
            // WebView channel は GUI runtime と同じ UI thread でしか触れない。
            // host/audio thread から直接送ると thread affinity を破るので、
            // 必ず run loop に戻してから channel に渡す。
            subscription.sender.send(move || {
                let _ = subscription.channel.send(payload);
            });
        }
    }
}

/// WebView へ送る JSON payload。TypeScript 側はこの形を期待する。
/// 新しい parameter でも payload の形は変えず `parameterId` で振り分ける。
pub(crate) fn parameter_payload(parameter_id: u32, value: f32) -> serde_json::Value {
    json!({
        "type": "parameter-value",
        "parameterId": parameter_id,
        "value": value,
        "text": parameter_value_text(parameter_id, value as f64).unwrap_or_else(|_| value.to_string()),
    })
}

pub(crate) fn editor_page_payload(editor_page: EditorPage) -> serde_json::Value {
    json!({
        "type": "editor-page",
        "page": editor_page.as_str(),
    })
}

/// GUI window 1 つ分の runtime。host が GUI を開くたびに作られ、閉じると drop。
pub(crate) struct WracGainGuiRuntime {
    gui_notifier: Arc<GuiStateNotifier>,
    // native WebView の寿命を持つ !Send + !Sync token。Drop 順を制御するため Option。
    web_view: Option<WxpWebView>,
    // WebView より長く生かす必要があるので保持する (Drop 順は下の Drop 実装参照)。
    wxp_context: Option<WebContext>,
    command_handler: Rc<WxpCommandHandler>,
    // shared state の現在値を定期的に GUI へ反映する timer。
    gui_update_timer: Timer,
    gui_size: LogicalSize<f64>,
    // DPI スケールを考慮した bounds 変換に使う。
    dpi_converter: DpiConverter,
}

impl WracGainGuiRuntime {
    /// host が「GUI を開いて」と要求してきたタイミングで `plugin.rs` の closure
    /// から呼ばれる factory。parent window に貼り付ける WebView を作って返す。
    fn create(
        dependencies: GuiRuntimeDependencies,
        configuration: GuiConfiguration,
        initial_size: GuiSize,
        parent: ParentWindowHandle,
    ) -> PluginResult<Self> {
        // このサンプルは embedded (parent に貼り付けるタイプ) しか対応していない。
        // floating window が必要な場合は別途実装する。
        if configuration.is_floating {
            log::warn!("rejecting floating GUI configuration");
            return Err(PluginError::Message("unsupported GUI configuration"));
        }
        log::debug!(
            "creating GUI runtime: width={}, height={}, configuration={configuration:?}",
            initial_size.width,
            initial_size.height
        );

        // WebView から呼べる parameter command を登録する。
        log::debug!("creating GUI runtime: creating command handler");
        let command_handler = Rc::new(WxpCommandHandler::new());
        log::debug!("creating GUI runtime: registering commands");
        register_commands(
            command_handler.clone(),
            dependencies.project_state.clone(),
            dependencies.shared.clone(),
            dependencies.gui_notifier.clone(),
            dependencies.host_parameter_edit_notifier,
            dependencies.host_gui_resize_requester,
            dependencies.resize_handle,
        );
        log::debug!("creating GUI runtime: commands registered");

        // WebView2 は同じ user data folder を別の Environment options で共有すると作成に
        // 失敗し得るため、OS 標準のアプリデータ配下に plugin ID 単位で分離する。
        let data_dir = webview_data_dir(PLUGIN_ID);
        std::fs::create_dir_all(&data_dir)
            .map_err(|_| PluginError::Message("failed to create GUI data directory"))?;
        log::debug!("using GUI data directory: {}", data_dir.display());

        log::debug!("creating GUI runtime: creating WebContext");
        let mut wxp_context = WebContext::new(data_dir);
        // 初期 scale は 1.0 とし、後で host から `set_scale` で書き換えられる。
        let dpi_converter = DpiConverter::new(1.0);
        let gui_size = gui_size_to_logical(initial_size);
        let bounds = dpi_converter.create_webview_bounds(gui_size);
        log::debug!(
            "creating GUI runtime: computed logical size: width={}, height={}",
            gui_size.width,
            gui_size.height
        );

        // debug は Vite dev server を見る (frontend 変更で native の再 build 不要)。
        // release は dev server に依存できないので build.rs が固めた zip を serve する。
        #[cfg(debug_assertions)]
        let builder = {
            let url = "http://127.0.0.1:5173/";
            log::debug!("creating GUI runtime: configuring debug WebView builder: url={url}");
            WxpWebViewBuilder::new(&mut wxp_context)
                .with_command_handler(command_handler.clone())
                .with_devtools(cfg!(debug_assertions))
                .with_visible(true)
                .with_bounds(bounds)
                .with_url(url)
        };

        #[cfg(not(debug_assertions))]
        let builder = {
            let url = "wxp-plugin://localhost/";
            log::debug!("creating GUI runtime: configuring release WebView builder: url={url}");
            WxpWebViewBuilder::new(&mut wxp_context)
                .with_command_handler(command_handler.clone())
                .with_devtools(cfg!(debug_assertions))
                .with_visible(true)
                .with_bounds(bounds)
                // 埋め込み zip を `wxp-plugin://` scheme で配信する。
                .with_serve_zip("wxp-plugin", FRONTEND_ZIP)
                .map_err(|_| PluginError::Message("failed to serve GUI assets"))?
                .with_url(url)
        };

        // parent window 上に子として WebView を作る。これで host UI に埋め込まれる。
        log::debug!("creating GUI runtime: build_as_child start");
        let web_view = builder
            .build_as_child(&parent)
            .map_err(|_| PluginError::Message("failed to build webview"))?;
        log::debug!("creating GUI runtime: build_as_child completed");

        // 33ms ≒ 30Hz で現在値を GUI に流す。dirty flag を持たず毎回 shared state
        // を読む方が構造が単純。CLAP の `request_callback()` は wrapper 経由だと
        // host の dispatch 実装に依存し GUI だけ値が古くなることがあるので、
        // GUI runtime 自身の run loop の timer で定期回収する。
        let gui_update_timer = Timer::new(Duration::from_millis(33), {
            let shared = dependencies.shared.clone();
            let gui_notifier = dependencies.gui_notifier.clone();
            move || {
                gui_notifier.notify_parameter(PARAM_GAIN_ID, shared.gain());
            }
        });
        log::debug!("creating GUI runtime: starting GUI update timer");
        gui_update_timer.start();
        log::debug!("creating GUI runtime: GUI update timer started");

        log::debug!("creating GUI runtime: completed");
        Ok(Self {
            gui_notifier: dependencies.gui_notifier,
            web_view: Some(web_view),
            wxp_context: Some(wxp_context),
            command_handler,
            gui_update_timer,
            gui_size,
            dpi_converter,
        })
    }
}

// host から呼ばれる resize / scale / size 取得などの操作を実装する trait。
impl WxpGuiRuntime for WracGainGuiRuntime {
    /// host が表示倍率 (HiDPI 等) を伝えてきたときに呼ばれる。
    fn set_scale(&mut self, scale: f64) -> PluginResult<()> {
        log::debug!("setting GUI scale: scale={scale}");
        self.dpi_converter.set_scale(scale);
        Ok(())
    }

    /// host が window サイズを変えたときに呼ばれる。範囲を clamp してから WebView に反映する。
    fn set_size(&mut self, size: GuiSize) -> PluginResult<()> {
        let requested = LogicalSize::new(size.width as f64, size.height as f64);
        self.gui_size = LogicalSize::new(
            requested
                .width
                .clamp(MIN_LOGICAL_GUI_SIZE.width, MAX_LOGICAL_GUI_SIZE.width),
            requested
                .height
                .clamp(MIN_LOGICAL_GUI_SIZE.height, MAX_LOGICAL_GUI_SIZE.height),
        );
        log::debug!(
            "setting GUI size: requested_width={}, requested_height={}, applied_width={}, applied_height={}",
            size.width,
            size.height,
            self.gui_size.width,
            self.gui_size.height
        );

        if let Some(web_view) = &self.web_view {
            // wxp は native WebView の直接操作を owner から分離している。ここは GUI thread 上だが、
            // stale-close checks と post/enqueue semantics を同じ経路に揃えるため dispatch 経由にする。
            web_view
                .dispatch()
                .post_set_bounds(self.dpi_converter.create_webview_bounds(self.gui_size))
                .map_err(|_| PluginError::Message("failed to resize webview"))?;
        }
        Ok(())
    }

    fn show(&mut self) -> PluginResult<()> {
        log::debug!("showing GUI runtime");
        if let Some(web_view) = &self.web_view {
            // show/hide は host lifecycle と競合しやすいので、owner を直接触らず wxp 側の
            // close-aware dispatch path に寄せる。
            web_view
                .dispatch()
                .post_set_visible(true)
                .map_err(|_| PluginError::Message("failed to show webview"))?;
        }
        self.gui_update_timer.start();
        log::debug!("showing GUI runtime completed");
        Ok(())
    }

    fn hide(&mut self) -> PluginResult<()> {
        log::debug!("hiding GUI runtime");
        self.gui_update_timer.stop();
        if let Some(web_view) = &self.web_view {
            // hide は destroy 直前に呼ばれることがある。dispatch は WebView が閉じていれば
            // WebViewClosed を返し、native object の寿命を延ばさない。
            web_view
                .dispatch()
                .post_set_visible(false)
                .map_err(|_| PluginError::Message("failed to hide webview"))?;
        }
        log::debug!("hiding GUI runtime completed");
        Ok(())
    }
}

fn webview_data_dir(plugin_id: &str) -> PathBuf {
    let plugin_dir = sanitize_plugin_data_dir(plugin_id);
    // WebView user-data も plugin_id 由来にする。ここだけ template 名を持つと、
    // rename 後の plugin が旧 plugin と cookie/cache/storage を共有してしまう。
    match project_dirs_from_plugin_id(plugin_id) {
        Some(dirs) => dirs.data_dir().join("webview").join(plugin_dir),
        None => std::env::temp_dir()
            .join(plugin_dir)
            .join("webview")
            .join("data"),
    }
}

fn project_dirs_from_plugin_id(plugin_id: &str) -> Option<ProjectDirs> {
    let mut parts = plugin_id.split('.');
    let qualifier = parts.next()?;
    let organization = parts.next()?;
    let application = parts.collect::<Vec<_>>().join("-");
    if application.is_empty() {
        return None;
    }
    ProjectDirs::from(qualifier, organization, &application)
}

fn sanitize_plugin_data_dir(plugin_id: &str) -> String {
    plugin_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

// drop 順を field 宣言順に任せず、切断 → WebView 破棄 → context 破棄の順に
// 明示する。callback が解放済み object を触る事故を防ぐため。
impl Drop for WracGainGuiRuntime {
    fn drop(&mut self) {
        log::debug!("dropping GUI runtime");
        // timer callback は run loop と GUI subscription に依存する。native WebView を
        // 落とす前に止めて、破棄途中の GUI state を tick が見る余地をなくす。
        self.gui_update_timer.stop();
        log::debug!("dropping GUI runtime: timer stopped");
        // GUI が消えるので、shared state からも channel を外しておく。
        self.gui_notifier.clear_subscriptions();
        log::debug!("dropping GUI runtime: subscriptions cleared");
        // WebView → WebContext の順で drop。逆だと wry が context 不在で panic することがある。
        self.web_view = None;
        log::debug!("dropping GUI runtime: webview dropped");
        self.wxp_context = None;
        log::debug!("dropping GUI runtime: web context dropped");
        // `command_handler` と `gui_update_timer` は field drop に任せる。
        // 下記 2 行は「ここまで生かしたい」ことを明示するためのダミー read。
        let _ = Rc::strong_count(&self.command_handler);
        let _ = self.gui_update_timer.is_running();
    }
}
