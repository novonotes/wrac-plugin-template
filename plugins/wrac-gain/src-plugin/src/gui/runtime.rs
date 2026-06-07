use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use novonotes_run_loop::RunLoopLocal;
use run_loop_timer::Timer;
use wrac_clap_adapter::{
    GuiConfig, GuiSize, HostContext, HostGuiResizeRequester, HostParamsEditNotifier,
    PluginDescriptor, PluginError, PluginResult,
};
use wrac_wxp_gui::{
    GuiSizeLimits, ParentWindowHandle, WxpFrontendSource, WxpGuiResizeHandle, WxpGuiRuntime,
    WxpWebViewConfig, WxpWebViewSession,
};
use wxp::WxpCommandHandler;

use crate::commands::{CommandRegistrationDependencies, register_commands};
use crate::gui::GuiStateNotifier;
use crate::plugin::notify_gui_parameters;
use crate::state::{ProjectStateStore, SharedState};

// GUI window size bounds (pixels). The host opens at the default; resize is clamped to min..=max.
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

// Embed the frontend zip only for release builds; debug builds use the Vite dev server.
#[cfg(not(debug_assertions))]
const FRONTEND_ZIP: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wrac_gain_plugin_gui.zip"));

#[derive(Clone)]
pub(super) struct GuiRuntimeDependencies {
    pub(super) descriptor: PluginDescriptor,
    pub(super) project_state: Arc<ProjectStateStore>,
    pub(super) shared: Arc<SharedState>,
    pub(super) gui_notifier: Arc<GuiStateNotifier>,
    pub(super) host_parameter_edit_notifier: Arc<dyn HostParamsEditNotifier>,
    pub(super) host_gui_resize_requester: Arc<dyn HostGuiResizeRequester>,
    pub(super) resize_handle: WxpGuiResizeHandle,
    pub(super) host_context: HostContext,
}

/// Runtime for one GUI window. Created each time the host opens the GUI; dropped when closed.
pub(crate) struct WracGainGuiRuntime {
    gui_notifier: Arc<GuiStateNotifier>,
    // WebView ownership and DPI/bounds live in the shared component; product runtime
    // remains responsible for choosing its state sync strategy.
    webview: WxpWebViewSession,
    // Host callbacks can be delayed by wrapper behavior, so this product chooses periodic
    // sync on the GUI run loop.
    gui_update_timer: Timer,
}

impl WracGainGuiRuntime {
    /// Factory called from the closure in `plugin.rs` when the host requests the GUI to open.
    /// Creates a WebView attached to the parent window and returns it.
    pub(super) fn create(
        run_loop: &RunLoopLocal,
        dependencies: GuiRuntimeDependencies,
        configuration: GuiConfig,
        initial_size: GuiSize,
        parent: ParentWindowHandle,
    ) -> PluginResult<Self> {
        // This sample supports only embedded mode (attached to the parent).
        // Implement floating window support separately if needed.
        if configuration.is_floating {
            log::warn!("rejecting floating GUI configuration");
            return Err(PluginError::Message("unsupported GUI configuration"));
        }
        log::debug!(
            "creating GUI runtime: width={}, height={}, configuration={configuration:?}",
            initial_size.width,
            initial_size.height
        );

        let command_handler = Rc::new(WxpCommandHandler::new());
        let host_size_unit = dependencies.resize_handle.host_size_unit();
        register_commands(
            command_handler.clone(),
            CommandRegistrationDependencies {
                project_state: dependencies.project_state.clone(),
                shared: dependencies.shared.clone(),
                gui_notifier: dependencies.gui_notifier.clone(),
                descriptor: dependencies.descriptor,
                host_parameter_edit_notifier: dependencies.host_parameter_edit_notifier,
                host_gui_resize_requester: dependencies.host_gui_resize_requester,
                gui_resize_handle: dependencies.resize_handle,
                host_context: dependencies.host_context,
            },
        );

        let webview = WxpWebViewSession::create(
            WxpWebViewConfig {
                plugin_id: dependencies.descriptor.id,
                initial_size,
                limits: GuiSizeLimits {
                    min: MIN_GUI_SIZE,
                    max: MAX_GUI_SIZE,
                },
                host_size_unit,
                parent,
                frontend: frontend_source(),
                devtools: cfg!(debug_assertions),
            },
            command_handler,
        )?;

        // Push the current value to the GUI at ~30 Hz (33 ms). Reading shared state on
        // every tick is simpler than maintaining a dirty flag. CLAP's `request_callback()`
        // depends on the host's dispatch implementation when going through a wrapper and
        // can leave the GUI with stale values, so a timer on the GUI runtime's own run
        // loop is used instead.
        let gui_update_timer = Timer::new(Duration::from_millis(33), {
            let shared = dependencies.shared.clone();
            let gui_notifier = dependencies.gui_notifier.clone();
            move || {
                notify_gui_parameters(&shared, |parameter_id, value| {
                    gui_notifier.notify_parameter(parameter_id, value);
                });
            }
        });
        gui_update_timer.start(run_loop);

        log::debug!("creating GUI runtime: completed");
        Ok(Self {
            gui_notifier: dependencies.gui_notifier,
            webview,
            gui_update_timer,
        })
    }
}

// Trait implementation for resize, scale, and size operations called by the host.
impl WxpGuiRuntime for WracGainGuiRuntime {
    /// Called when the host reports a display scale factor (e.g. HiDPI).
    fn set_scale(&mut self, scale: f64) -> PluginResult<()> {
        self.webview.set_scale(scale)
    }

    /// Called when the host changes the window size. Clamps to the valid range before applying.
    fn set_size(&mut self, size: GuiSize) -> PluginResult<()> {
        self.webview.set_size(size)
    }

    fn show(&mut self, run_loop: &RunLoopLocal) -> PluginResult<()> {
        log::debug!("showing GUI runtime");
        self.webview.show()?;
        self.gui_update_timer.start(run_loop);
        log::debug!("showing GUI runtime completed");
        Ok(())
    }

    fn hide(&mut self) -> PluginResult<()> {
        log::debug!("hiding GUI runtime");
        self.gui_update_timer.stop();
        self.webview.hide()?;
        log::debug!("hiding GUI runtime completed");
        Ok(())
    }
}

fn frontend_source() -> WxpFrontendSource {
    #[cfg(debug_assertions)]
    {
        WxpFrontendSource::Url {
            url: "http://127.0.0.1:5173/",
        }
    }

    #[cfg(not(debug_assertions))]
    {
        WxpFrontendSource::Zip {
            scheme: "wxp-plugin",
            url: "wxp-plugin://localhost/",
            bytes: FRONTEND_ZIP,
        }
    }
}

impl Drop for WracGainGuiRuntime {
    fn drop(&mut self) {
        log::debug!("dropping GUI runtime");
        self.gui_update_timer.stop();
        log::debug!("dropping GUI runtime: timer stopped");
        self.gui_notifier.clear_subscriptions();
        log::debug!("dropping GUI runtime: subscriptions cleared");
        let _ = self.gui_update_timer.is_running();
    }
}
