//! WRAC Gain 固有の WebView GUI runtime。
//!
//! GUI 本体は HTML/CSS/TypeScript で書かれており (`src-gui/` 以下)、
//! これを embed した WebView を host window に貼り付けるのがこの module の
//! 役目。WebView との通信は [`wxp`] crate の command/channel 機構を使い、
//! frontend から `set_parameter_value` などの command を invoke できる。
//!
//! 役割分担:
//! - `wrac_wxp_gui`: host UI thread の所有、callback dispatch、parent window
//!   の raw handle 変換などの厄介な部分を引き受ける
//! - この module    : WebView の内容 (URL / 埋め込み zip)、register する
//!   command、resize/scale の挙動など、製品ごとに変わる部分だけを書く

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
use crate::state::SharedState;

// GUI window のサイズ範囲 (pixel)。host は initial size でウインドウを開き、
// ユーザーがリサイズしたときは min..=max の範囲にクランプされる。
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

// release build 時のみ、`build.rs` が作った frontend zip を埋め込む。
// debug build では Vite dev server (`http://127.0.0.1:5173/`) を見るので不要。
#[cfg(not(debug_assertions))]
const FRONTEND_ZIP: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wrac_gain_plugin_gui.zip"));

pub(crate) struct GuiIntegration {
    pub(crate) controller: Arc<WxpGuiController>,
    pub(crate) notifier: Arc<GuiStateNotifier>,
}

#[derive(Clone)]
struct GuiRuntimeDependencies {
    shared: Arc<SharedState>,
    gui_notifier: Arc<GuiStateNotifier>,
    host_parameter_edit_notifier: Arc<dyn HostParameterEditNotifier>,
    host_gui_resize_requester: Arc<dyn HostGuiResizeRequester>,
    resize_handle: WxpGuiResizeHandle,
}

/// plugin core から使う GUI extension 一式を作る。
///
/// `plugin.rs` 側には GUI window のサイズ制約や WebView runtime の詳細を置かず、
/// host-facing な core 実装から GUI 固有の組み立てを切り離す。
pub(crate) fn create_gui_integration(
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

/// WebView 側へ GUI state を push するための通知口。
///
/// [`Channel`] と UI run loop の扱いは GUI runtime 固有なので、共有 state ではなく
/// GUI module に閉じ込める。通知タイミング自体は呼び出し元が決める。
pub(crate) struct GuiStateNotifier {
    next_subscription_id: AtomicU64,
    subscriptions: Mutex<HashMap<GuiSubscriptionId, GuiSubscription>>,
}

/// WebView 側 subscriber 1 つぶんの登録情報。
///
/// `kind` で「何の stream を購読しているか」、`channel` で「どこに送るか」を分けて持つ。
/// こうしておけば、同じ GUI が parameter / meter / analyzer などを個別に購読・解除でき、
/// 古い cleanup が新しい購読を巻き込んで消してしまう事故も起きない。
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

/// 購読の種類。現状は parameter 変化通知のみ。
///
/// meter や analyzer などの stream を足すときは、ここに variant を追加し、
/// `notify_*` 側でその variant を持つ subscription にだけ配信すればよい。
/// Channel そのものを増やすのではなく、種別で振り分ける形にしておく狙い。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuiSubscriptionKind {
    Parameters,
}

impl GuiStateNotifier {
    fn new() -> Self {
        Self {
            next_subscription_id: AtomicU64::new(1),
            subscriptions: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn subscribe_parameters(&self, channel: Channel) -> GuiSubscriptionId {
        // id は Rust 側で採番する。wxp の Channel id とは独立させることで、
        // transport (Channel) と購読 lifecycle を別々に管理できる。
        let id = GuiSubscriptionId(self.next_subscription_id.fetch_add(1, Ordering::Relaxed));
        self.subscriptions.lock().insert(
            id,
            GuiSubscription {
                kind: GuiSubscriptionKind::Parameters,
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
        // lock を握ったまま送信しない。送り先の処理が再び notifier を触りに来ても
        // deadlock しないように、配信対象を先に clone してから lock を離す。
        let subscriptions: Vec<_> = self
            .subscriptions
            .lock()
            .values()
            .filter(|subscription| subscription.kind == GuiSubscriptionKind::Parameters)
            .cloned()
            .collect();
        if subscriptions.is_empty() {
            // GUI が開いていなければ送り先がないので何もしない。
            return;
        }

        let payload = parameter_payload(parameter_id, value);
        for subscription in subscriptions {
            let payload = payload.clone();
            // WebView channel は GUI runtime と同じ UI thread 上で扱う必要がある。
            // host / audio thread から直接 send すると native UI の thread affinity を
            // 破るので、いったん run loop に戻してから channel に渡す。
            subscription.sender.send(move || {
                let _ = subscription.channel.send(payload);
            });
        }
    }
}

/// WebView へ送る JSON payload。GUI (TypeScript 側) はこの形を期待している。
///
/// 新しい parameter を追加しても payload の形は変えず、`parameterId` と `text` の中身だけを
/// 増やす。UI 側は parameter id ごとに表示先を選べばよい。
pub(crate) fn parameter_payload(parameter_id: u32, value: f32) -> serde_json::Value {
    json!({
        "type": "parameter-value",
        "parameterId": parameter_id,
        "value": value,
        "text": parameter_value_text(parameter_id, value as f64).unwrap_or_else(|_| value.to_string()),
    })
}

/// GUI window 1 つに対応する runtime。host が GUI を開くたびに 1 つ作られ、
/// 閉じるときに drop される。
pub(crate) struct WracGainGuiRuntime {
    // WebView 側 subscription への通知口。
    gui_notifier: Arc<GuiStateNotifier>,
    // `WxpWebView` は native WebView の寿命を所有する !Send + !Sync token。host からの操作は
    // runtime が載っている GUI thread で受け、必要なときだけ dispatch handle に変換して post する。
    // Option にしてあるのは Drop の順序を制御するため。
    web_view: Option<WxpWebView>,
    // wxp の WebContext。WebView より長く生かしておく必要があるので保持する。
    wxp_context: Option<WebContext>,
    // frontend からの command を受け取って Rust 側関数を呼ぶ dispatcher。
    command_handler: Rc<WxpCommandHandler>,
    // shared state の現在値を定期的に GUI に反映するための timer。
    gui_update_timer: Timer,
    // 現在の論理サイズ。
    gui_size: LogicalSize<f64>,
    // DPI スケール (1.0, 1.5, 2.0 など) を考慮した bounds 変換に使う。
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

        // debug build では Vite dev server を見るので、frontend を変更しても
        // native plugin の再 build が不要になり開発体験が良くなる。
        // release build では DAW 環境で外部 dev server に依存できないので、
        // `build.rs` で固めた zip を WebView に直接 serve させる。
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

        // 33ms ≒ 30Hz で現在値を GUI に流す。サンプルでは値が変わったかの dirty
        // flag を持たず、GUI runtime が shared state を読む形にしておく方が構造を
        // 追いやすい。
        //
        // 補足: CLAP には `request_callback()` で main thread に処理を戻す API も
        // あるが、clap-wrapper 経由で VST3/AU/AAX に流すと host ごとの dispatch
        // 実装に依存してしまう。host の癖で GUI だけ古い値を出し続ける問題を防ぐ
        // ため、GUI runtime 自身の run loop 上で timer を回して定期的に回収する。
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
    match ProjectDirs::from("com", "your-company", "wrac-gain") {
        Some(dirs) => dirs.data_dir().join("webview").join(plugin_dir),
        None => std::env::temp_dir()
            .join("wrac-gain-plugin")
            .join("webview")
            .join(plugin_dir),
    }
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

// host が GUI を閉じると runtime が drop される。
// drop 順を field 宣言順に任せず、明示的に切断 → WebView 破棄 → context 破棄の
// 順で進めることで、callback が解放後の object を触る事故を防ぐ。
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
