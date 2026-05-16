use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use directories::ProjectDirs;
use run_loop_timer::Timer;
use wrac_clap_adapter::{
    GuiConfiguration, GuiSize, HostGuiResizeRequester, HostParameterEditNotifier, PluginError,
    PluginResult,
};
use wrac_wxp_gui::{
    DpiConverter, ParentWindowHandle, WxpGuiResizeHandle, WxpGuiRuntime, gui_size_to_logical,
};
use wxp::{WebContext, WxpCommandHandler, WxpWebView, WxpWebViewBuilder, dpi::LogicalSize};

use crate::commands::register_commands;
use crate::gui::GuiStateNotifier;
use crate::plugin::{PARAM_GAIN_ID, PLUGIN_ID};
use crate::state::{ProjectStateStore, SharedState};

// GUI window のサイズ範囲 (pixel)。host は default で開き、resize は min..=max に clamp。
pub(super) const DEFAULT_GUI_SIZE: GuiSize = GuiSize {
    width: 320,
    height: 380,
};
pub(super) const MIN_GUI_SIZE: GuiSize = GuiSize {
    width: 320,
    height: 380,
};
pub(super) const MAX_GUI_SIZE: GuiSize = GuiSize {
    width: 720,
    height: 720,
};

// resize 時にクランプする論理ピクセルの上下限。
const MIN_LOGICAL_GUI_SIZE: LogicalSize<f64> = LogicalSize::new(320.0, 340.0);
const MAX_LOGICAL_GUI_SIZE: LogicalSize<f64> = LogicalSize::new(720.0, 720.0);

// release のみ frontend zip を埋め込む。debug は Vite dev server を見るので不要。
#[cfg(not(debug_assertions))]
const FRONTEND_ZIP: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wrac_gain_plugin_gui.zip"));

#[derive(Clone)]
pub(super) struct GuiRuntimeDependencies {
    pub(super) project_state: Arc<ProjectStateStore>,
    pub(super) shared: Arc<SharedState>,
    pub(super) gui_notifier: Arc<GuiStateNotifier>,
    pub(super) host_parameter_edit_notifier: Arc<dyn HostParameterEditNotifier>,
    pub(super) host_gui_resize_requester: Arc<dyn HostGuiResizeRequester>,
    pub(super) resize_handle: WxpGuiResizeHandle,
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
    pub(super) fn create(
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
