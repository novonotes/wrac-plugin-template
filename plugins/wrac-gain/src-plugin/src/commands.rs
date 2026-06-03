//! Registration of commands callable from the WebView frontend.
//!
//! From Rust's perspective this module is the contract with the TypeScript UI.
//! When renaming commands or changing payload shapes, update the `invoke(...)` calls
//! and subscriptions in `src-gui` at the same time.

use std::rc::Rc;
use std::sync::Arc;

use serde_json::json;
use wrac_clap_adapter::{HostGuiResizeRequester, HostParamsEditNotifier};
use wrac_wxp_gui::WxpGuiResizeHandle;
use wxp::{Channel, WxpCommandHandler};

use crate::gui::{GuiStateNotifier, GuiSubscriptionId, editor_page_payload, parameter_payload};
use crate::plugin::{parameter_default_value, parameter_host_value, parameter_text_value};
use crate::state::{EditorPage, ProjectStateStore, SharedState};

mod resize;

use resize::register_resize_commands;

/// Registers commands callable from the WebView frontend with the [`WxpCommandHandler`].
///
/// The frontend (TypeScript in `src-gui`) invokes these commands using calls like
/// `invoke("set_parameter_value", { parameterId, value })`.
pub(crate) fn register_commands(
    command_handler: Rc<WxpCommandHandler>,
    project_state: Arc<ProjectStateStore>,
    shared: Arc<SharedState>,
    gui_notifier: Arc<GuiStateNotifier>,
    host_parameter_edit_notifier: Arc<dyn HostParamsEditNotifier>,
    host_gui_resize_requester: Arc<dyn HostGuiResizeRequester>,
    gui_resize_handle: WxpGuiResizeHandle,
) {
    // The WebView console is often invisible inside a DAW. Bridge frontend logs to the
    // plugin's logger so GUI initialisation progress is visible in native log output.
    command_handler.register_sync("write_to_log", move |ctx| {
        let message = ctx.arg::<String>("message").map_err(|e| e.to_string())?;
        log::debug!("frontend: {message}");
        Ok::<_, String>(json!({ "ok": true }))
    });

    // Editor page is project state unrelated to audio. It lives in a separate store from
    // the SharedState read by the audio thread and is merged with the parameter snapshot
    // at save time.
    {
        let project_state = project_state.clone();
        command_handler.register_sync("get_editor_page", move |_| {
            Ok::<_, String>(editor_page_payload(project_state.editor_page()))
        });
    }

    {
        let project_state = project_state.clone();
        let gui_notifier = gui_notifier.clone();
        command_handler.register_sync("set_editor_page", move |ctx| {
            let page = ctx.arg::<String>("page").map_err(|e| e.to_string())?;
            let editor_page =
                EditorPage::from_str(&page).ok_or_else(|| "invalid editor page".to_string())?;
            project_state.set_editor_page(editor_page);
            gui_notifier.notify_editor_page(editor_page);
            Ok::<_, String>(editor_page_payload(editor_page))
        });
    }

    // Returns the current parameter value. Used for initial display when the GUI launches.
    {
        let shared = shared.clone();
        command_handler.register_sync("get_parameter_state", move |ctx| {
            let parameter_id = ctx.arg::<u32>("parameterId").map_err(|e| e.to_string())?;
            let value = shared
                .parameter_value(parameter_id)
                .ok_or_else(|| "invalid parameter id".to_string())?;
            Ok::<_, String>(parameter_payload(parameter_id, value))
        });
    }

    // Converts a display string back to a plain value via the Rust parameter parser and applies it.
    {
        let shared = shared.clone();
        let gui_notifier = gui_notifier.clone();
        let host_parameter_edit_notifier = host_parameter_edit_notifier.clone();
        command_handler.register_sync("set_parameter_text", move |ctx| {
            let parameter_id = ctx.arg::<u32>("parameterId").map_err(|e| e.to_string())?;
            let text = ctx.arg::<String>("text").map_err(|e| e.to_string())?;
            let value = parameter_text_value(parameter_id, &text).map_err(|e| e.to_string())?;
            host_parameter_edit_notifier.begin_edit(parameter_id);
            let applied = shared
                .set_parameter_value(parameter_id, value)
                .ok_or_else(|| "invalid parameter id".to_string())?;
            gui_notifier.notify_parameter(parameter_id, applied);
            host_parameter_edit_notifier.update_edit(
                parameter_id,
                parameter_host_value(parameter_id, applied)
                    .map_err(|_| "invalid parameter id".to_string())?,
            );
            host_parameter_edit_notifier.end_edit(parameter_id);
            Ok::<_, String>(parameter_payload(parameter_id, applied))
        });
    }

    // Allows the frontend to signal a reset without knowing the default value itself.
    {
        let shared = shared.clone();
        let gui_notifier = gui_notifier.clone();
        let host_parameter_edit_notifier = host_parameter_edit_notifier.clone();
        command_handler.register_sync("reset_parameter_to_default", move |ctx| {
            let parameter_id = ctx.arg::<u32>("parameterId").map_err(|e| e.to_string())?;
            let value = parameter_default_value(parameter_id).map_err(|e| e.to_string())?;
            host_parameter_edit_notifier.begin_edit(parameter_id);
            let applied = shared
                .set_parameter_value(parameter_id, value)
                .ok_or_else(|| "invalid parameter id".to_string())?;
            gui_notifier.notify_parameter(parameter_id, applied);
            host_parameter_edit_notifier.update_edit(
                parameter_id,
                parameter_host_value(parameter_id, applied)
                    .map_err(|_| "invalid parameter id".to_string())?,
            );
            host_parameter_edit_notifier.end_edit(parameter_id);
            Ok::<_, String>(parameter_payload(parameter_id, applied))
        });
    }

    // Called when the user first touches a control. Signals the start of an undo unit to the host.
    {
        let host_parameter_edit_notifier = host_parameter_edit_notifier.clone();
        command_handler.register_sync("begin_parameter_gesture", move |ctx| {
            let parameter_id = ctx.arg::<u32>("parameterId").map_err(|e| e.to_string())?;
            host_parameter_edit_notifier.begin_edit(parameter_id);
            Ok::<_, String>(json!({ "ok": true }))
        });
    }

    // Called while the control is moving. Applies the value and notifies the host.
    {
        let shared = shared.clone();
        let gui_notifier = gui_notifier.clone();
        let host_parameter_edit_notifier = host_parameter_edit_notifier.clone();
        command_handler.register_sync("set_parameter_value", move |ctx| {
            let parameter_id = ctx.arg::<u32>("parameterId").map_err(|e| e.to_string())?;
            let value = ctx.arg::<f64>("value").map_err(|e| e.to_string())?;
            let applied = shared
                .set_parameter_value(parameter_id, value)
                .ok_or_else(|| "invalid parameter id".to_string())?;
            gui_notifier.notify_parameter(parameter_id, applied);
            host_parameter_edit_notifier.update_edit(
                parameter_id,
                parameter_host_value(parameter_id, applied)
                    .map_err(|_| "invalid parameter id".to_string())?,
            );
            Ok::<_, String>(parameter_payload(parameter_id, applied))
        });
    }

    // Called when the user releases the control. Signals the end of the undo unit to the host.
    {
        let host_parameter_edit_notifier = host_parameter_edit_notifier.clone();
        command_handler.register_sync("end_parameter_gesture", move |ctx| {
            let parameter_id = ctx.arg::<u32>("parameterId").map_err(|e| e.to_string())?;
            host_parameter_edit_notifier.end_edit(parameter_id);
            Ok::<_, String>(json!({ "ok": true }))
        });
    }

    // Starts a subscription that receives parameter changes.
    // `channel` is a callback channel created on the JS side; the plugin pushes value
    // changes into it. The returned `subscriptionId` identifies the subscription so the
    // JS side can unsubscribe precisely at cleanup, without cancelling subscriptions it
    // didn't create.
    {
        let gui_notifier = gui_notifier.clone();
        command_handler.register_sync("subscribe_parameters", move |ctx| {
            let channel = ctx.arg::<Channel>("channel").map_err(|e| e.to_string())?;
            let subscription_id = gui_notifier.subscribe_parameters(channel);
            Ok::<_, String>(json!({
                "ok": true,
                "subscriptionId": subscription_id.get(),
            }))
        });
    }

    {
        let gui_notifier = gui_notifier.clone();
        command_handler.register_sync("subscribe_editor_page", move |ctx| {
            let channel = ctx.arg::<Channel>("channel").map_err(|e| e.to_string())?;
            let subscription_id = gui_notifier.subscribe_editor_page(channel);
            Ok::<_, String>(json!({
                "ok": true,
                "subscriptionId": subscription_id.get(),
            }))
        });
    }

    // Cancels a subscription. If the given ID is not registered this is a no-op.
    // Using an explicit ID prevents a delayed, stale cleanup from accidentally cancelling
    // a subscription that was created later.
    {
        let gui_notifier = gui_notifier.clone();
        command_handler.register_sync("unsubscribe_gui_subscription", move |ctx| {
            let subscription_id = ctx
                .arg::<u64>("subscriptionId")
                .map_err(|e| e.to_string())?;
            gui_notifier.unsubscribe(GuiSubscriptionId::from_raw(subscription_id));
            Ok::<_, String>(json!({ "ok": true }))
        });
    }

    command_handler.register_sync("focus_host_window", move |ctx| {
        ctx.webview()
            .post_focus_parent()
            .map_err(|e| format!("focus_parent failed: {e}"))?;
        Ok::<_, String>(json!({ "ok": true }))
    });

    register_resize_commands(
        &command_handler,
        host_gui_resize_requester,
        gui_resize_handle,
    );
}
