use std::ffi::c_void;
use std::num::{NonZeroIsize, NonZeroU32};
use std::ptr::NonNull;

use raw_window_handle::{
    AppKitWindowHandle, HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle,
    WindowHandle, XcbWindowHandle,
};
use wrac_clap_adapter::{ClapWindow, PluginError, PluginResult};

/// host の親 window を `raw-window-handle` として公開する wrapper。
/// platform 分岐と handle lifetime をここで一度だけ吸収し、製品へ漏らさない。
#[derive(Debug)]
pub struct ParentWindowHandle {
    raw: RawWindowHandle,
}

impl TryFrom<ClapWindow> for ParentWindowHandle {
    type Error = PluginError;

    fn try_from(window: ClapWindow) -> Result<Self, Self::Error> {
        StoredParentWindow::from_clap_window(window).to_parent_window_handle()
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum StoredParentWindow {
    Cocoa { ns_view: usize },
    Win32 { hwnd: isize },
    X11 { window: u64 },
}

impl StoredParentWindow {
    pub(crate) fn from_clap_window(window: ClapWindow) -> Self {
        match window {
            ClapWindow::Cocoa { ns_view } => Self::Cocoa {
                ns_view: ns_view.as_ptr() as usize,
            },
            ClapWindow::Win32 { hwnd } => Self::Win32 { hwnd: hwnd.get() },
            ClapWindow::X11 { window } => Self::X11 {
                window: window.get(),
            },
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
        // 安全性: `ParentWindowHandle` は CLAP `set_parent()` で渡された handle から作る。
        // 実体の lifetime は host の parent window 契約に従い、この wrapper は WebView
        // 作成と後続 resize のためだけに使う。
        Ok(unsafe { WindowHandle::borrow_raw(self.raw) })
    }
}
