use super::resize::GuiResizePolicy;
use super::*;
use crate::runtime::WxpGuiRuntime;
use crate::window::ParentWindowHandle;
use wrac_host_context::DetectedHost;
use wxp::dpi::LogicalSize;

fn test_layout(unit: HostGuiSizeUnit) -> HostGuiLayout {
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
    layout.set_host_size_unit(unit);
    layout
}

fn test_controller(unit: HostGuiSizeUnit) -> WxpGuiController {
    let (context, _) = HostContext::new(
        DetectedHost {
            family: HostFamily::Unknown,
            display_name: "test".to_string(),
            process_name: "test".to_string(),
            process_path: String::new(),
            version: None,
        },
        PluginFormat::Vst3,
        wrac_host_context::SystemContext::detect(),
    );
    let resize_handle = WxpGuiResizeHandle::new(
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
    );
    let controller = WxpGuiController::new_with_resize_handle(
        |_run_loop: &RunLoopLocal,
         _config: GuiConfig,
         _initial_size: GuiSize,
         _parent: ParentWindowHandle|
         -> PluginResult<Box<dyn WxpGuiRuntime>> { Err(PluginError::InvalidState) },
        resize_handle,
        context,
    );
    controller.layout.set_host_size_unit(unit);
    controller
}

#[test]
fn keeps_resize_limits_in_logical_pixels_across_host_size_units() {
    for unit in [
        HostGuiSizeUnit::PhysicalPixels,
        HostGuiSizeUnit::LogicalPoints,
    ] {
        let layout = test_layout(unit);
        let clamped = layout.clamp_logical_size(LogicalSize::new(700.0, 100.0), 1.5);
        assert_eq!(clamped, LogicalSize::new(700.0, 200.0));
    }
}

#[test]
fn scales_preferred_size_and_limits_for_physical_pixel_hosts() {
    let layout = test_layout(HostGuiSizeUnit::PhysicalPixels);

    layout.apply_preferred_scale(1.25);
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

    layout.store_accepted_size(
        GuiSize {
            width: 850,
            height: 610,
        },
        1.25,
    );
    layout.apply_preferred_scale(1.5);
    let size = layout.accepted_size();
    assert_eq!((size.width, size.height), (1020, 732));
}

#[test]
fn avoids_rounding_drift_across_repeated_scale_changes() {
    let layout = HostGuiLayout::new(
        GuiSize {
            width: 322,
            height: 381,
        },
        GuiSizeLimits {
            min: GuiSize {
                width: 320,
                height: 380,
            },
            max: GuiSize {
                width: 720,
                height: 720,
            },
        },
        GuiResizePolicy::RESIZABLE,
    );
    layout.set_host_size_unit(HostGuiSizeUnit::PhysicalPixels);

    for scale in [1.25, 1.33, 1.75, 1.25, 1.0] {
        layout.apply_preferred_scale(scale);
    }

    let size = layout.accepted_size();
    assert_eq!((size.width, size.height), (322, 381));
}

#[test]
fn keeps_logical_point_hosts_unscaled() {
    let layout = test_layout(HostGuiSizeUnit::LogicalPoints);

    layout.apply_preferred_scale(2.0);
    let size = layout.accepted_size();
    assert_eq!((size.width, size.height), (600, 400));

    let minimum = layout.clamp_size(
        GuiSize {
            width: 200,
            height: 150,
        },
        2.0,
    );
    assert_eq!((minimum.width, minimum.height), (300, 200));
}

#[test]
fn keeps_reopen_size_accurate_after_attached_scale_change_without_set_size() {
    let controller = test_controller(HostGuiSizeUnit::PhysicalPixels);
    let config = default_gui_configuration();

    controller.create(config).unwrap();
    assert_eq!(controller.get_size().unwrap().width, 600);
    assert_eq!(controller.get_size().unwrap().height, 400);

    // Simulate an attached session whose host updates scale without resending set_size.
    controller.runtime.lock().session.as_mut().unwrap().parent =
        Some(StoredParentWindow::Win32 { hwnd: 1 });
    controller.set_scale(1.25).unwrap();
    // Attached host frames stay owned by set_size, so get_size keeps the existing frame.
    let attached_size = controller.get_size().unwrap();
    assert_eq!((attached_size.width, attached_size.height), (600, 400));

    // Reopening at the same scale or back at 100% must derive from the 600x400 logical
    // baseline rather than rescaling 600x400 by `new_scale / 1.25`.
    controller.destroy();
    controller.create(config).unwrap();
    let reopened_at_125 = controller.get_size().unwrap();
    assert_eq!((reopened_at_125.width, reopened_at_125.height), (750, 500));

    controller.set_scale(1.0).unwrap();
    let reopened_at_100 = controller.get_size().unwrap();
    assert_eq!((reopened_at_100.width, reopened_at_100.height), (600, 400));
}

#[test]
fn selects_host_size_unit_by_platform() {
    let formats = [
        PluginFormat::Vst3,
        PluginFormat::Au,
        PluginFormat::Aax,
        PluginFormat::Unknown,
    ];

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
