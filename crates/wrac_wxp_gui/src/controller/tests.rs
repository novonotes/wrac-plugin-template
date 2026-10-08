use super::resize::GuiResizePolicy;
use super::*;
use wrac_host_context::DetectedHost;
use wxp::dpi::LogicalSize;

#[test]
fn clamps_logical_resize_request_in_physical_pixels() {
    let layout = HostGuiLayout::new(
        GuiSize {
            width: 600,
            height: 400,
        },
        GuiSizeLimits {
            min: GuiSize {
                width: 300,
                height: 200,
            },
            max: GuiSize {
                width: 900,
                height: 600,
            },
        },
        GuiResizePolicy::RESIZABLE,
    );

    let clamped = layout.clamp_logical_size(LogicalSize::new(700.0, 100.0), 1.5);

    assert_eq!(clamped.width, 600.0);
    assert_eq!(clamped.height, 200.0 / 1.5);
}

#[test]
fn selects_logical_host_size_for_macos_wrappers() {
    let formats = [PluginFormat::Vst3, PluginFormat::Au, PluginFormat::Aax];

    for plugin_format in formats {
        let context = HostContext {
            host: DetectedHost {
                family: HostFamily::Unknown,
                display_name: "test".to_string(),
                process_name: "test".to_string(),
                process_path: String::new(),
                version: None,
            },
            plugin_format,
            system: wrac_host_context::SystemContext::detect(),
        };

        let expected = if cfg!(target_os = "macos") {
            HostGuiSizeUnit::LogicalPoints
        } else {
            HostGuiSizeUnit::PhysicalPixels
        };
        assert_eq!(host_gui_size_unit_for_context(&context), expected);
    }
}
