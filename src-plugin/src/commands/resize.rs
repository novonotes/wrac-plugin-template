use std::cell::RefCell;
#[cfg(target_os = "macos")]
use std::ffi::c_void;
use std::rc::Rc;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::json;
use wrac_clap_adapter::HostGuiResizeRequester;
use wrac_wxp_gui::WxpGuiResizeHandle;
use wxp::WxpCommandHandler;

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

#[derive(Debug, Clone, Copy)]
struct NativeResizeDrag {
    drag_id: u64,
    start_mouse_x: f64,
    start_mouse_y: f64,
    start_width: f64,
    start_height: f64,
}

#[derive(Debug, Clone, Copy)]
struct GlobalMouseLocation {
    x: f64,
    y: f64,
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventCreate(source: *const c_void) -> *mut c_void;
    fn CGEventGetLocation(event: *mut c_void) -> CGPoint;
}

#[cfg(target_os = "macos")]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const c_void);
}

fn global_mouse_location() -> Option<GlobalMouseLocation> {
    #[cfg(target_os = "macos")]
    {
        // resize grip は WebView 内にあるが、リサイズ対象の editor window は host が
        // 所有する。Logic などはドラッグ中にその WebView を動かすため、WebView 座標は
        // 物理マウスではなく動く子 view に対して飛んでしまう。OS カーソルを直接読めば、
        // WebView と host の layout 更新の外にある安定した座標系 (デスクトップ) を使える。
        let event = unsafe { CGEventCreate(std::ptr::null()) };
        if event.is_null() {
            return None;
        }
        let location = unsafe { CGEventGetLocation(event) };
        unsafe { CFRelease(event.cast()) };
        Some(GlobalMouseLocation {
            x: location.x,
            y: location.y,
        })
    }

    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

pub(super) fn register_resize_commands(
    command_handler: &Rc<WxpCommandHandler>,
    host_gui_resize_requester: Arc<dyn HostGuiResizeRequester>,
    gui_resize_handle: WxpGuiResizeHandle,
) {
    // resize ドラッグを 2 責務に分ける。ジェスチャの寿命は JS が持つ (pointer
    // capture/release は browser の概念)。macOS の座標は Rust が持つ (browser の
    // 座標系こそ host が動かしている面だから)。drag id がブラウザ側トリガと
    // この native snapshot を結び付け、毎回の resize 要求を元のデスクトップ
    // カーソル位置から再計算できる (WebView 内座標の誤差を累積しない)。
    let native_resize_drag = Rc::new(RefCell::new(None::<NativeResizeDrag>));

    {
        let native_resize_drag = native_resize_drag.clone();
        command_handler.register_sync("begin_gui_resize_drag", move |ctx| {
            let request = ctx
                .arg::<BeginGuiResizeDragRequest>("request")
                .map_err(|e| e.to_string())?;
            let Some(mouse) = global_mouse_location() else {
                return Ok::<_, String>(json!({ "ok": false }));
            };

            *native_resize_drag.borrow_mut() = Some(NativeResizeDrag {
                drag_id: request.drag_id,
                start_mouse_x: mouse.x,
                start_mouse_y: mouse.y,
                start_width: request.width,
                start_height: request.height,
            });
            Ok::<_, String>(json!({ "ok": true }))
        });
    }

    {
        let native_resize_drag = native_resize_drag.clone();
        command_handler.register_sync("end_gui_resize_drag", move |ctx| {
            let request = ctx
                .arg::<EndGuiResizeDragRequest>("request")
                .map_err(|e| e.to_string())?;
            let mut drag = native_resize_drag.borrow_mut();
            if drag
                .as_ref()
                .is_some_and(|drag| drag.drag_id == request.drag_id)
            {
                *drag = None;
            }
            Ok::<_, String>(json!({ "ok": true }))
        });
    }

    {
        let native_resize_drag = native_resize_drag.clone();
        command_handler.register_sync("request_gui_resize", move |ctx| {
            let request = ctx
                .arg::<RequestGuiResizeRequest>("request")
                .map_err(|e| e.to_string())?;

            let native_request = request.drag_id.and_then(|drag_id| {
                let drag = native_resize_drag.borrow();
                let drag = drag.as_ref().filter(|drag| drag.drag_id == drag_id)?;
                let mouse = global_mouse_location()?;
                Some((
                    drag.start_width + (mouse.x - drag.start_mouse_x),
                    drag.start_height + (mouse.y - drag.start_mouse_y),
                ))
            });

            let (width, height) = native_request.unwrap_or((request.width, request.height));
            let size = gui_resize_handle
                .request_resize(
                    wxp::dpi::LogicalSize::new(width, height),
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
}
