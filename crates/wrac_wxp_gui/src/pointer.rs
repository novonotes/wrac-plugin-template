#[cfg(target_os = "macos")]
use std::ffi::c_void;

#[derive(Debug, Clone, Copy)]
pub(crate) struct GlobalPointerPosition {
    pub(crate) x: f64,
    pub(crate) y: f64,
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

pub(crate) fn global_pointer_position() -> Option<GlobalPointerPosition> {
    #[cfg(target_os = "macos")]
    {
        // WebView coordinates can move during a host-owned editor resize, so read the
        // desktop cursor to keep drag math independent of child-view relayout.
        let event = unsafe { CGEventCreate(std::ptr::null()) };
        if event.is_null() {
            return None;
        }
        let location = unsafe { CGEventGetLocation(event) };
        unsafe { CFRelease(event.cast()) };
        Some(GlobalPointerPosition {
            x: location.x,
            y: location.y,
        })
    }

    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}
