use crate::dpi::HostGuiSizeUnit;
use wrac_clap_adapter::interface::{GuiApi, GuiConfig};

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

pub(super) fn default_host_gui_size_unit() -> HostGuiSizeUnit {
    // Cocoa uses logical points at both the native CLAP and wrapper GUI boundaries.
    // Win32 and X11 exchange physical pixels with the host.
    if cfg!(target_os = "macos") {
        HostGuiSizeUnit::LogicalPoints
    } else {
        HostGuiSizeUnit::PhysicalPixels
    }
}
