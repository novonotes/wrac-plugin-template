use std::rc::Rc;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::json;
use wrac_clap_adapter::HostGuiResizeRequester;
use wxp::{WxpCommandHandler, dpi::LogicalSize};

use crate::{controller::WxpGuiResizeHandle, resize_drag::WxpNativeResizeDrag};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequestGuiResizeRequest {
    width: f64,
    height: f64,
    drag_id: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BeginGuiResizeDragRequest {
    drag_id: u64,
    width: f64,
    height: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EndGuiResizeDragRequest {
    drag_id: u64,
}

/// Registers the resize commands used by the shared WRAC frontend resize bridge.
///
/// Template plugins should not define their own copies of these command names. The host resize
/// path has DAW-specific threading and re-entry behavior, so keeping this command contract with
/// `WxpGuiResizeHandle` makes future fixes apply to all wxp-based plugin GUIs.
pub fn register_resize_commands(
    command_handler: &Rc<WxpCommandHandler>,
    host_gui_resize_requester: Arc<dyn HostGuiResizeRequester>,
    gui_resize_handle: WxpGuiResizeHandle,
) {
    let native_resize_drag = Rc::new(WxpNativeResizeDrag::default());

    {
        let native_resize_drag = native_resize_drag.clone();
        command_handler.register_sync("begin_gui_resize_drag", move |ctx| {
            let request = ctx
                .arg::<BeginGuiResizeDragRequest>("request")
                .map_err(|e| e.to_string())?;
            let ok = native_resize_drag.begin(
                request.drag_id,
                LogicalSize::new(request.width, request.height),
            );
            Ok::<_, String>(json!({ "ok": ok }))
        });
    }

    {
        let native_resize_drag = native_resize_drag.clone();
        command_handler.register_sync("end_gui_resize_drag", move |ctx| {
            let request = ctx
                .arg::<EndGuiResizeDragRequest>("request")
                .map_err(|e| e.to_string())?;
            native_resize_drag.end(request.drag_id);
            Ok::<_, String>(json!({ "ok": true }))
        });
    }

    {
        let native_resize_drag = native_resize_drag.clone();
        command_handler.register_sync("request_gui_resize", move |ctx| {
            let request = ctx
                .arg::<RequestGuiResizeRequest>("request")
                .map_err(|e| e.to_string())?;

            let requested = native_resize_drag.resolve_size(
                request.drag_id,
                LogicalSize::new(request.width, request.height),
            );
            let size = gui_resize_handle
                .request_resize(requested, ctx.webview(), host_gui_resize_requester.as_ref())
                .map_err(|e| e.to_string())?;
            Ok::<_, String>(json!({
                "ok": true,
                "width": size.width,
                "height": size.height,
            }))
        });
    }
}
