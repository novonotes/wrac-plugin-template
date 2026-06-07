use std::cell::RefCell;

use wxp::dpi::LogicalSize;

use crate::pointer::global_pointer_position;

#[derive(Debug, Clone, Copy)]
struct NativeResizeDrag {
    drag_id: u64,
    start_mouse_x: f64,
    start_mouse_y: f64,
    start_width: f64,
    start_height: f64,
}

/// Native pointer correction for host-owned editor resizing.
///
/// The shared resize command bridge owns the command names and JSON schemas. This type
/// only absorbs the case where WebView-relative coordinates move during resize, so the
/// bridge can pass and return frontend-facing logical sizes.
#[derive(Debug, Default)]
pub struct WxpNativeResizeDrag {
    current: RefCell<Option<NativeResizeDrag>>,
}

impl WxpNativeResizeDrag {
    pub fn begin(&self, drag_id: u64, size: LogicalSize<f64>) -> bool {
        let Some(pointer) = global_pointer_position() else {
            return false;
        };

        // Some hosts move the child WebView during resize. Computing each delta from the
        // starting desktop cursor avoids accumulating WebView-relative coordinate error.
        *self.current.borrow_mut() = Some(NativeResizeDrag {
            drag_id,
            start_mouse_x: pointer.x,
            start_mouse_y: pointer.y,
            start_width: size.width,
            start_height: size.height,
        });
        true
    }

    pub fn end(&self, drag_id: u64) {
        let mut current = self.current.borrow_mut();
        // Do not let a stale end event clear a newer drag that has already started.
        if current.as_ref().is_some_and(|drag| drag.drag_id == drag_id) {
            *current = None;
        }
    }

    pub fn resolve_size(
        &self,
        drag_id: Option<u64>,
        fallback: LogicalSize<f64>,
    ) -> LogicalSize<f64> {
        let Some(drag_id) = drag_id else {
            return fallback;
        };
        let current = self.current.borrow();
        let Some(drag) = current.as_ref().filter(|drag| drag.drag_id == drag_id) else {
            return fallback;
        };
        let Some(pointer) = global_pointer_position() else {
            return fallback;
        };

        // Return frontend-facing logical pixels. `WxpGuiResizeHandle::request_resize`
        // owns the only logical-to-physical conversion at the host boundary.
        LogicalSize::new(
            drag.start_width + (pointer.x - drag.start_mouse_x),
            drag.start_height + (pointer.y - drag.start_mouse_y),
        )
    }
}
