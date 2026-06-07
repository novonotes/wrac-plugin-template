//! Commands shared by WRAC WebView frontends.
//!
//! These commands are not product behavior like parameters or project state. They are the
//! frontend-to-native contract for wxp-hosted plugin windows, so keeping them here avoids
//! every template-derived plugin having to copy the same command names and payload shapes.

mod cursor;
mod resize;

pub use cursor::register_native_cursor_bridge_commands;
pub use resize::register_resize_commands;
