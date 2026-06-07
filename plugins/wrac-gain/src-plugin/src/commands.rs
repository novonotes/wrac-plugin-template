//! Registration of commands callable from the WebView frontend.
//!
//! From Rust's perspective this module is the contract with the TypeScript UI.
//! When renaming commands or changing payload shapes, update the `invoke(...)` calls
//! and subscriptions in `src-gui` at the same time.

use std::rc::Rc;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::json;
use wrac_clap_adapter::{
    HostContext, HostFamily, HostGuiResizeRequester, HostParamsEditNotifier, PluginDescriptor,
};
use wrac_wxp_gui::{
    WxpGuiResizeHandle, register_native_cursor_bridge_commands, register_resize_commands,
};
use wxp::{Channel, WxpCommandHandler};

use crate::gui::{GuiStateNotifier, GuiSubscriptionId, editor_page_payload, parameter_payload};
use crate::plugin::{parameter_default_value, parameter_host_value, parameter_text_value};
use crate::state::{EditorPage, ProjectStateStore, SharedState};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum FrontendLogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrontendLogEntry {
    level: FrontendLogLevel,
    message: String,
    #[serde(default)]
    data: Option<serde_json::Value>,
}

pub(crate) struct CommandRegistrationDependencies {
    pub(crate) project_state: Arc<ProjectStateStore>,
    pub(crate) shared: Arc<SharedState>,
    pub(crate) gui_notifier: Arc<GuiStateNotifier>,
    pub(crate) descriptor: PluginDescriptor,
    pub(crate) host_parameter_edit_notifier: Arc<dyn HostParamsEditNotifier>,
    pub(crate) host_gui_resize_requester: Arc<dyn HostGuiResizeRequester>,
    pub(crate) gui_resize_handle: WxpGuiResizeHandle,
    pub(crate) host_context: HostContext,
}

/// Registers commands callable from the WebView frontend with the [`WxpCommandHandler`].
///
/// The frontend (TypeScript in `src-gui`) invokes these commands using calls like
/// `invoke("set_parameter_value", { parameterId, value })`.
pub(crate) fn register_commands(
    command_handler: Rc<WxpCommandHandler>,
    dependencies: CommandRegistrationDependencies,
) {
    let CommandRegistrationDependencies {
        project_state,
        shared,
        gui_notifier,
        descriptor,
        host_parameter_edit_notifier,
        host_gui_resize_requester,
        gui_resize_handle,
        host_context,
    } = dependencies;

    // Metadata comes from the descriptor selected by the host, not from Vite build-time
    // constants. That keeps the GUI correct if one binary exposes multiple products.
    command_handler.register_sync("get_plugin_metadata", move |_| {
        Ok::<_, String>(json!({
            "pluginId": descriptor.id,
            "pluginName": descriptor.name,
            "companyName": descriptor.vendor,
            "version": descriptor.version,
        }))
    });

    // The WebView console is often invisible inside a DAW. Bridge frontend logs to the
    // plugin's logger so GUI initialisation progress is visible in native log output.
    command_handler.register_sync("write_to_log", move |ctx| {
        let entry = ctx
            .arg::<FrontendLogEntry>("entry")
            .map_err(|e| e.to_string())?;
        write_frontend_log(entry);
        Ok::<_, String>(json!({ "ok": true }))
    });

    let frontend_runtime_context = frontend_runtime_context(&host_context);
    command_handler.register_sync("get_frontend_runtime_context", move |_| {
        Ok::<_, String>(frontend_runtime_context.clone())
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

    // Window-management commands are shared wxp GUI plumbing, not WRAC Gain product state.
    // Register them here so the product command list stays the single Rust/TS rendezvous point.
    register_resize_commands(
        &command_handler,
        host_gui_resize_requester,
        gui_resize_handle,
    );
    register_native_cursor_bridge_commands(&command_handler);
}

fn write_frontend_log(entry: FrontendLogEntry) {
    let message = match entry.data {
        Some(data) => format!("{} data={data}", entry.message),
        None => entry.message,
    };

    match entry.level {
        FrontendLogLevel::Debug => log::debug!(target: "frontend", "{message}"),
        FrontendLogLevel::Info => log::info!(target: "frontend", "{message}"),
        FrontendLogLevel::Warn => log::warn!(target: "frontend", "{message}"),
        FrontendLogLevel::Error => log::error!(target: "frontend", "{message}"),
    }
}

fn frontend_runtime_context(host_context: &HostContext) -> serde_json::Value {
    json!({
        "os": std::env::consts::OS,
        "pluginFormat": host_context.plugin_format.as_str(),
        "hostFamily": host_family_id(host_context.host.family),
        "hostName": host_context.host.display_name,
        "processName": host_context.host.process_name,
    })
}

fn host_family_id(family: HostFamily) -> &'static str {
    match family {
        HostFamily::AbletonLive => "ableton-live",
        HostFamily::AdobeAudition => "adobe-audition",
        HostFamily::AdobePremiere => "adobe-premiere",
        HostFamily::AppleAuLab => "apple-au-lab",
        HostFamily::AppleAuval => "apple-auval",
        HostFamily::AppleFinalCut => "apple-final-cut",
        HostFamily::AppleGarageBand => "apple-garage-band",
        HostFamily::AppleInfoHelper => "apple-info-helper",
        HostFamily::AppleLogic => "apple-logic",
        HostFamily::AppleMainStage => "apple-mainstage",
        HostFamily::Ardour => "ardour",
        HostFamily::BitwigStudio => "bitwig-studio",
        HostFamily::CakewalkByBandlab => "cakewalk-by-bandlab",
        HostFamily::CakewalkSonar => "cakewalk-sonar",
        HostFamily::DaVinciResolve => "davinci-resolve",
        HostFamily::DigitalPerformer => "digital-performer",
        HostFamily::FlStudio => "fl-studio",
        HostFamily::JuceAudioPluginHost => "juce-audio-plugin-host",
        HostFamily::Luna => "luna",
        HostFamily::MagixSamplitude => "magix-samplitude",
        HostFamily::MagixSequoia => "magix-sequoia",
        HostFamily::MuseReceptor => "muse-receptor",
        HostFamily::NiMaschine => "ni-maschine",
        HostFamily::Pluginval => "pluginval",
        HostFamily::ProTools => "pro-tools",
        HostFamily::Pyramix => "pyramix",
        HostFamily::Reason => "reason",
        HostFamily::Renoise => "renoise",
        HostFamily::Reaper => "reaper",
        HostFamily::Sadie => "sadie",
        HostFamily::SteinbergCubase => "steinberg-cubase",
        HostFamily::SteinbergCubaseBridged => "steinberg-cubase-bridged",
        HostFamily::SteinbergNuendo => "steinberg-nuendo",
        HostFamily::SteinbergTestHost => "steinberg-test-host",
        HostFamily::SteinbergWavelab => "steinberg-wavelab",
        HostFamily::StudioOne => "studio-one",
        HostFamily::Tracktion => "tracktion",
        HostFamily::TracktionWaveform => "tracktion-waveform",
        HostFamily::VbVstScanner => "vb-vst-scanner",
        HostFamily::ViennaEnsemblePro => "vienna-ensemble-pro",
        HostFamily::WaveBurner => "waveburner",
        HostFamily::Unknown => "unknown",
    }
}
