use wrac_clap_adapter::{GuiApi, GuiConfig};

pub(super) fn default_gui_api() -> GuiApi {
    if cfg!(target_os = "macos") {
        GuiApi::Cocoa
    } else if cfg!(target_os = "windows") {
        GuiApi::Win32
    } else {
        GuiApi::X11
    }
}

pub(super) fn default_gui_configuration() -> GuiConfig {
    GuiConfig {
        api: default_gui_api(),
        is_floating: false,
    }
}
