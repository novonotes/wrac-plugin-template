//! WebView frontend から呼べる command の登録。
//!
//! Rust 側から見ると、ここが TypeScript UI との約束事になる。command 名や payload
//! の形を変えるときは `src-gui` 側の `invoke(...)` / subscription と一緒に変更する。

use std::cell::RefCell;
#[cfg(target_os = "macos")]
use std::ffi::c_void;
use std::rc::Rc;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::json;
use wrac_clap_adapter::{HostGuiResizeRequester, HostParameterEditNotifier};
use wrac_wxp_gui::WxpGuiResizeHandle;
use wxp::{Channel, WxpCommandHandler};

use crate::gui::{GuiStateNotifier, GuiSubscriptionId, editor_page_payload, parameter_payload};
use crate::plugin::{parameter_default_value, parameter_host_value, parameter_text_value};
use crate::state::{EditorPage, ProjectStateStore, SharedState};

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
        // The resize grip lives inside the WebView, but the host owns the native
        // editor window being resized. Hosts such as Logic can move/relayout that
        // WebView during the same drag, which makes WebView pointer coordinates jump
        // relative to the changing child view instead of tracking the physical mouse.
        // Reading the OS cursor here gives the resize code one stable coordinate
        // space: the desktop, outside both the WebView and the host's layout updates.
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

/// WebView (フロントエンド) から呼べる command を [`WxpCommandHandler`] に登録する。
///
/// フロントエンド側 (`src-gui` の TypeScript) は `invoke("set_parameter_value",
/// { parameterId, value })` のような形でこれらの command を呼び出す。
pub(crate) fn register_commands(
    command_handler: Rc<WxpCommandHandler>,
    project_state: Arc<ProjectStateStore>,
    shared: Arc<SharedState>,
    gui_notifier: Arc<GuiStateNotifier>,
    host_parameter_edit_notifier: Arc<dyn HostParameterEditNotifier>,
    host_gui_resize_requester: Arc<dyn HostGuiResizeRequester>,
    gui_resize_handle: WxpGuiResizeHandle,
) {
    // GUI 初期化のどこまで到達したかを native log から追えるようにする。
    // WebView console は DAW 環境で見えないことが多いため、frontend 起点の診断ログを
    // plugin 側 logger に橋渡しする command を用意しておく。
    command_handler.register_sync("write_to_log", move |ctx| {
        let message = ctx.arg::<String>("message").map_err(|e| e.to_string())?;
        log::info!("frontend: {message}");
        Ok::<_, String>(json!({ "ok": true }))
    });

    // Split the resize drag into two responsibilities. JS owns the gesture lifetime
    // because pointer capture/release is a browser concept, but Rust owns the resize
    // coordinates on macOS because the browser's coordinate space is the surface the
    // host is actively moving. The drag id ties those browser triggers to this native
    // snapshot so every resize request can be recomputed from the original desktop
    // cursor position instead of accumulating WebView-local pointer noise.
    let native_resize_drag = Rc::new(RefCell::new(None::<NativeResizeDrag>));

    // editor page は音に関係しない project state。audio thread が読む SharedState とは
    // 別の store に置き、保存時に parameter snapshot と合成する。
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

    // subscription を解除する。指定の id が登録されていなければ no-op。
    // id 指定にすることで、遅れて届いた古い cleanup が、後から始まった別の購読を
    // 誤って解除してしまう事故を防げる。
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
