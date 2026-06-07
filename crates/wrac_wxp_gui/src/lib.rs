//! Shared building blocks for using wxp WebViews in WRAC plugin GUIs.
//!
//! Code that needs to understand the CLAP ABI stays in `wrac_clap_adapter`. This crate
//! owns only the toolkit boundary shared by product GUI runtimes: window-handle conversion,
//! GUI thread affinity, and WebView DPI/bounds management.

mod commands;
mod controller;
mod dpi;
mod pointer;
mod resize_drag;
mod runtime;
mod session;
mod window;

pub use commands::{register_native_cursor_bridge_commands, register_resize_commands};
pub use controller::{GuiSizeLimits, WxpGuiController, WxpGuiResizeHandle};
pub use dpi::HostGuiSizeUnit;
pub use resize_drag::WxpNativeResizeDrag;
pub use runtime::{WxpGuiFactory, WxpGuiRuntime};
pub use session::{WxpFrontendSource, WxpWebViewConfig, WxpWebViewSession};
pub use window::ParentWindowHandle;
