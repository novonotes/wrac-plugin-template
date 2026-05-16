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

use std::sync::Arc;

mod notifier;
mod runtime;

pub(crate) use notifier::{
    GuiStateNotifier, GuiSubscriptionId, editor_page_payload, parameter_payload,
};

use runtime::{
    DEFAULT_GUI_SIZE, GuiRuntimeDependencies, MAX_GUI_SIZE, MIN_GUI_SIZE, WracGainGuiRuntime,
};
use wrac_clap_adapter::{HostGuiResizeRequester, HostParameterEditNotifier};
use wrac_wxp_gui::{GuiSizeLimits, WxpGuiController, WxpGuiResizeHandle, WxpGuiRuntime};

use crate::state::{ProjectStateStore, SharedState};

pub(crate) struct GuiIntegration {
    pub(crate) controller: Arc<WxpGuiController>,
    pub(crate) notifier: Arc<GuiStateNotifier>,
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
