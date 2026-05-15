//! WebView frontend から呼べる command の登録。
//!
//! Rust 側から見ると、ここが TypeScript UI との約束事になる。command 名や payload
//! の形を変えるときは `src-gui` 側の `invoke(...)` / subscription と一緒に変更する。

use std::rc::Rc;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::json;
use wrac_clap_adapter::{HostGuiResizeRequester, HostParameterEditNotifier};
use wrac_wxp_gui::WxpGuiResizeHandle;
use wxp::{Channel, WxpCommandHandler};

use crate::gui::{GuiStateNotifier, GuiSubscriptionId, parameter_payload};
use crate::plugin::{parameter_default_value, parameter_host_value, parameter_text_value};
use crate::state::SharedState;

#[derive(Debug, Deserialize)]
struct RequestGuiResizeRequest {
    width: f64,
    height: f64,
}

/// WebView (フロントエンド) から呼べる command を [`WxpCommandHandler`] に登録する。
///
/// フロントエンド側 (`src-gui` の TypeScript) は `invoke("set_parameter_value",
/// { parameterId, value })` のような形でこれらの command を呼び出す。
pub(crate) fn register_commands(
    command_handler: Rc<WxpCommandHandler>,
    shared: Arc<SharedState>,
    gui_notifier: Arc<GuiStateNotifier>,
    host_parameter_edit_notifier: Arc<dyn HostParameterEditNotifier>,
    host_gui_resize_requester: Arc<dyn HostGuiResizeRequester>,
    gui_resize_handle: WxpGuiResizeHandle,
) {
    // 現在の parameter 値を取得する。GUI 起動直後の初期表示などに使う。
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

    // 表示文字列を Rust 側の parameter parser で plain value に戻して反映する。
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

    // frontend が default 値を持たずに reset intent だけを送れるようにする。
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

    // control に触れ始めたタイミング。host に「これから undo 単位」と伝える。
    {
        let host_parameter_edit_notifier = host_parameter_edit_notifier.clone();
        command_handler.register_sync("begin_parameter_gesture", move |ctx| {
            let parameter_id = ctx.arg::<u32>("parameterId").map_err(|e| e.to_string())?;
            host_parameter_edit_notifier.begin_edit(parameter_id);
            Ok::<_, String>(json!({ "ok": true }))
        });
    }

    // control が動いたタイミング。値を反映して host にも通知する。
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

    // control から指を離したタイミング。undo 単位の終了を host に伝える。
    {
        let host_parameter_edit_notifier = host_parameter_edit_notifier.clone();
        command_handler.register_sync("end_parameter_gesture", move |ctx| {
            let parameter_id = ctx.arg::<u32>("parameterId").map_err(|e| e.to_string())?;
            host_parameter_edit_notifier.end_edit(parameter_id);
            Ok::<_, String>(json!({ "ok": true }))
        });
    }

    // parameter の変化を受け取る subscription を開始する。
    // `channel` は JS 側で作った callback channel で、plugin はここに値の変化を push する。
    // 戻り値の `subscriptionId` は購読の識別子。JS は cleanup 時にこの id を返すことで、
    // 自分が始めた購読だけを確実に解除できる。
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

    // subscription を解除する。指定の id が登録されていなければ no-op。
    // id 指定にすることで、遅れて届いた古い cleanup が、後から始まった別の購読を
    // 誤って解除してしまう事故を防げる。
    {
        let gui_notifier = gui_notifier.clone();
        command_handler.register_sync("unsubscribe_parameters", move |ctx| {
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

    command_handler.register_sync("request_gui_resize", move |ctx| {
        let request = ctx
            .arg::<RequestGuiResizeRequest>("request")
            .map_err(|e| e.to_string())?;
        let size = gui_resize_handle
            .request_resize(
                wxp::dpi::LogicalSize::new(request.width, request.height),
                ctx.webview(),
                host_gui_resize_requester.as_ref(),
            )
            .map_err(|e| e.to_string())?;
        Ok::<_, String>(json!({
            "ok": true,
            "width": size.width,
            "height": size.height,
        }))
    });
}
