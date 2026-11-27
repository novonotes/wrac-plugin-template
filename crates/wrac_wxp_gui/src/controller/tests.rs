use super::resize::GuiResizePolicy;
use super::*;
use wrac_host_context::DetectedHost;
use wxp::dpi::LogicalSize;

fn test_layout() -> HostGuiLayout {
    HostGuiLayout::new(
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
    )
}

#[test]
fn keeps_resize_limits_in_platform_units() {
    let layout = test_layout();
    let clamped = layout.clamp_logical_size(LogicalSize::new(700.0, 100.0), 1.5);

    let expected = if cfg!(windows) {
        LogicalSize::new(700.0, 200.0)
    } else {
        LogicalSize::new(600.0, 200.0 / 1.5)
    };
    assert_eq!(clamped, expected);
}

#[cfg(windows)]
#[test]
fn keeps_the_editor_at_its_preferred_logical_size_before_attachment() {
    let layout = test_layout();

    layout.rescale_preferred_size(1.0, 1.25);
    let size = layout.accepted_size();
    assert_eq!((size.width, size.height), (750, 500));
    let minimum = layout.clamp_size(
        GuiSize {
            width: 300,
            height: 200,
        },
        1.25,
    );
    assert_eq!((minimum.width, minimum.height), (375, 250));

    layout.store_accepted_size(GuiSize {
        width: 850,
        height: 610,
    });
    layout.rescale_preferred_size(1.25, 1.5);
    let size = layout.accepted_size();
    assert_eq!((size.width, size.height), (1020, 732));
}

#[test]
fn selects_logical_host_size_for_macos_wrappers() {
    let formats = [PluginFormat::Vst3, PluginFormat::Au, PluginFormat::Aax];

    for plugin_format in formats {
        let (context, _) = HostContext::new(
            DetectedHost {
                family: HostFamily::Unknown,
                display_name: "test".to_string(),
                process_name: "test".to_string(),
                process_path: String::new(),
                version: None,
            },
            plugin_format,
            wrac_host_context::SystemContext::detect(),
        );

        let expected = if cfg!(target_os = "macos") {
            HostGuiSizeUnit::LogicalPoints
        } else {
            HostGuiSizeUnit::PhysicalPixels
        };
        assert_eq!(host_gui_size_unit_for_context(&context), expected);
    }
}
