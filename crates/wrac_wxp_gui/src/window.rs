use std::ffi::c_void;
use std::num::{NonZeroIsize, NonZeroU32};
use std::ptr::NonNull;

use raw_window_handle::{
    AppKitWindowHandle, HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle,
    WindowHandle, XcbWindowHandle,
};
use wrac_clap_adapter::{HostWindow, PluginError, PluginResult};

/// Wrapper that exposes the host parent window as a `raw-window-handle`.
/// Platform branching and handle lifetime concerns are absorbed here once and
/// kept out of product code.
#[derive(Debug)]
pub struct ParentWindowHandle {
    raw: RawWindowHandle,
}

impl TryFrom<HostWindow> for ParentWindowHandle {
    type Error = PluginError;

    fn try_from(window: HostWindow) -> Result<Self, Self::Error> {
        StoredParentWindow::from_host_window(window).to_parent_window_handle()
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum StoredParentWindow {
    Cocoa { ns_view: usize },
    Win32 { hwnd: isize },
    X11 { window: u64 },
}

impl StoredParentWindow {
    pub(crate) fn from_host_window(window: HostWindow) -> Self {
        match window {
            HostWindow::Cocoa { ns_view } => Self::Cocoa {
                ns_view: ns_view.get(),
            },
            HostWindow::Win32 { hwnd } => Self::Win32 { hwnd: hwnd.get() },
            HostWindow::X11 { window } => Self::X11 {
                window: window.get(),
            },
        }
    }

    #[cfg(windows)]
    pub(crate) fn win32_hwnd(self) -> Option<*mut c_void> {
        // Keep platform-specific parent inspection here so GUI policy code does not need
        // to duplicate raw-window-handle matching or make assumptions about other backends.
        match self {
            Self::Win32 { hwnd } => Some(hwnd as *mut c_void),
            _ => None,
        }
    }

    pub(crate) fn to_parent_window_handle(self) -> PluginResult<ParentWindowHandle> {
        match self {
            Self::Cocoa { ns_view } => {
                let ns_view =
                    NonNull::new(ns_view as *mut c_void).ok_or(PluginError::InvalidState)?;
                Ok(ParentWindowHandle {
                    raw: RawWindowHandle::AppKit(AppKitWindowHandle::new(ns_view)),
                })
            }
            Self::Win32 { hwnd } => {
                let hwnd = NonZeroIsize::new(hwnd).ok_or(PluginError::InvalidState)?;
                Ok(ParentWindowHandle {
                    raw: RawWindowHandle::Win32(Win32WindowHandle::new(hwnd)),
                })
            }
            Self::X11 { window } => {
                let window = u32::try_from(window)
                    .ok()
                    .and_then(NonZeroU32::new)
                    .ok_or(PluginError::InvalidState)?;
                Ok(ParentWindowHandle {
                    raw: RawWindowHandle::Xcb(XcbWindowHandle::new(window)),
                })
            }
        }
    }
}

impl HasWindowHandle for ParentWindowHandle {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        // Safety: `ParentWindowHandle` is constructed from the handle passed in CLAP
        // `set_parent()`. The underlying lifetime is governed by the host's parent window
        // contract; this wrapper is used only for WebView creation and subsequent resize.
        Ok(unsafe { WindowHandle::borrow_raw(self.raw) })
    }
}
