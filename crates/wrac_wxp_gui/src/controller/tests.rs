use super::resize::GuiResizePolicy;
use super::*;
use crate::dpi::GuiDpiPolicy;
use crate::runtime::WxpGuiRuntime;
use crate::window::ParentWindowHandle;
use wrac_clap_adapter::interface::HostGui;
use wrac_host_context::DetectedHost;
use wxp::dpi::LogicalSize;

fn test_layout(dpi_policy: GuiDpiPolicy) -> HostGuiLayout {
    HostGuiLayout::with_dpi_policy(
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
        dpi_policy,
        Arc::new(UnusedHostGui),
    )
}

// Mirrors an editor whose default width equals its minimum width, so a frame negotiated at
// 100% falls below the minimum as soon as the scale increases.
fn min_width_layout(dpi_policy: GuiDpiPolicy) -> HostGuiLayout {
    HostGuiLayout::with_dpi_policy(
        GuiSize {
            width: 960,
            height: 480,
        },
        GuiSizeLimits {
            min: GuiSize {
                width: 960,
                height: 400,
            },
            max: GuiSize {
                width: 2760,
                height: 1260,
            },
        },
        GuiResizePolicy::RESIZABLE,
        dpi_policy,
        Arc::new(UnusedHostGui),
    )
}

// These tests run without a GUI run loop, so posted host resize requests never execute.
struct UnusedHostGui;

impl HostGui for UnusedHostGui {
    fn request_resize(&self, _size: GuiSize) -> PluginResult<()> {
        Err(PluginError::InvalidState)
    }
}

fn test_controller(
    initial_size: GuiSize,
    limits: GuiSizeLimits,
    dpi_policy: GuiDpiPolicy,
) -> WxpGuiController {
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
    let resize_handle = WxpGuiResizeHandle::with_dpi_policy(
        initial_size,
        limits,
        dpi_policy,
        Arc::new(UnusedHostGui),
    );
    WxpGuiController::new(
        |_run_loop: &RunLoopLocal,
         _config: GuiConfig,
         _initial_size: GuiSize,
         _parent: ParentWindowHandle|
         -> PluginResult<Box<dyn WxpGuiRuntime>> { Err(PluginError::InvalidState) },
        resize_handle,
        context,
    )
}

#[test]
fn requests_scaled_size_when_attached_frame_falls_below_minimum() {
    // Ableton Live with Auto-Scale off: get_size, set_parent and set_size happen at 100%,
    // then set_scale(1.25) arrives while the 960x480 frame is attached.
    let layout = min_width_layout(GuiDpiPolicy::Windows);
    layout.store_accepted_size(GuiSize {
        width: 960,
        height: 480,
    });
    layout.set_scale_while_attached(1.25);

    let size = layout.host_size_to_unclip_at_active_scale().unwrap();
    assert_eq!((size.width, size.height), (1200, 600));
}

#[test]
fn leaves_frame_alone_when_host_resized_before_reporting_scale() {
    // A host that sends the new frame before set_scale (set_size then set_scale on a monitor
    // change) must not be asked to scale the already scaled frame a second time.
    let layout = min_width_layout(GuiDpiPolicy::Windows);
    layout.store_accepted_size(GuiSize {
        width: 1200,
        height: 600,
    });
    layout.set_scale_while_attached(1.25);

    assert!(layout.host_size_to_unclip_at_active_scale().is_none());
}

#[test]
fn tolerates_host_truncating_the_scaled_frame_before_reporting_scale() {
    // 961 * 1.5 = 1441.5. The clamp rounds the minimum to 1442, but a host that truncates
    // sends 1441 before set_scale; that frame already fits and must not be scaled again.
    let layout = HostGuiLayout::with_dpi_policy(
        GuiSize {
            width: 961,
            height: 480,
        },
        GuiSizeLimits {
            min: GuiSize {
                width: 961,
                height: 400,
            },
            max: GuiSize {
                width: 2760,
                height: 1260,
            },
        },
        GuiResizePolicy::RESIZABLE,
        GuiDpiPolicy::Windows,
        Arc::new(UnusedHostGui),
    );
    layout.store_accepted_size(GuiSize {
        width: 1441,
        height: 720,
    });
    layout.set_scale_while_attached(1.5);

    assert!(layout.host_size_to_unclip_at_active_scale().is_none());
}

#[test]
fn never_requests_attached_scale_resize_on_macos_or_linux() {
    for policy in [GuiDpiPolicy::Macos, GuiDpiPolicy::Linux] {
        let layout = min_width_layout(policy);
        layout.set_scale_while_attached(2.0);

        assert!(layout.host_size_to_unclip_at_active_scale().is_none());
    }
}

#[test]
fn clamps_logical_resize_request_per_platform_policy() {
    let windows_layout = test_layout(GuiDpiPolicy::Windows);
    windows_layout.set_scale_before_attach(1.5);
    assert_eq!(
        windows_layout.clamp_logical_size(LogicalSize::new(700.0, 100.0)),
        LogicalSize::new(700.0, 200.0)
    );

    let linux_layout = test_layout(GuiDpiPolicy::Linux);
    linux_layout.set_scale_before_attach(1.5);
    assert_eq!(
        linux_layout.clamp_logical_size(LogicalSize::new(700.0, 100.0)),
        LogicalSize::new(600.0, 200.0 / 1.5)
    );

    let macos_layout = test_layout(GuiDpiPolicy::Macos);
    macos_layout.set_scale_before_attach(2.0);
    assert_eq!(
        macos_layout.clamp_logical_size(LogicalSize::new(700.0, 100.0)),
        LogicalSize::new(700.0, 200.0)
    );
}

#[test]
fn scales_preferred_size_and_limits_on_windows() {
    let layout = test_layout(GuiDpiPolicy::Windows);

    layout.set_scale_before_attach(1.25);
    let size = layout.accepted_size();
    assert_eq!((size.width, size.height), (750, 500));

    let minimum = layout.clamp_host_size(GuiSize {
        width: 300,
        height: 200,
    });
    assert_eq!((minimum.width, minimum.height), (375, 250));

    layout.store_accepted_size(GuiSize {
        width: 850,
        height: 610,
    });
    layout.set_scale_before_attach(1.5);
    let size = layout.accepted_size();
    assert_eq!((size.width, size.height), (1020, 732));
}

#[test]
fn keeps_macos_and_linux_host_sizes_and_limits_unchanged_across_scale() {
    for policy in [GuiDpiPolicy::Macos, GuiDpiPolicy::Linux] {
        let layout = test_layout(policy);

        layout.set_scale_before_attach(2.0);
        let size = layout.accepted_size();
        assert_eq!((size.width, size.height), (600, 400));

        let clamped = layout.clamp_host_size(GuiSize {
            width: 200,
            height: 700,
        });
        assert_eq!((clamped.width, clamped.height), (300, 600));
    }
}

#[test]
fn avoids_rounding_drift_across_repeated_scale_changes() {
    let layout = HostGuiLayout::with_dpi_policy(
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
        GuiDpiPolicy::Windows,
        Arc::new(UnusedHostGui),
    );

    for scale in [1.25, 1.33, 1.75, 1.25, 1.0] {
        layout.set_scale_before_attach(scale);
    }

    let size = layout.accepted_size();
    assert_eq!((size.width, size.height), (322, 381));
}

#[test]
fn keeps_attached_frame_unclipped_and_reopen_accurate_after_late_scale_increase() {
    // Mirrors WRAC Gain (min == default == 320x380) when a Windows host creates a 100%
    // parent frame first and then updates scale (or Cubase 10 corrects scale in set_parent).
    let controller = test_controller(
        GuiSize {
            width: 320,
            height: 380,
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
        GuiDpiPolicy::Windows,
    );
    let config = default_gui_configuration();

    controller.create(config).unwrap();
    let initial = controller.get_size().unwrap();
    assert_eq!((initial.width, initial.height), (320, 380));

    // Simulate an attached session whose host updates scale to 125% without enlarging the
    // 320x380 parent frame.
    controller.runtime.lock().session.as_mut().unwrap().parent =
        Some(StoredParentWindow::Win32 { hwnd: 1 });
    controller.set_scale(1.25).unwrap();

    // Neither adjust_size nor set_size may push the child WebView above the 320x380 host
    // frame (which would clip the bottom-right area).
    let adjusted = controller
        .adjust_size(GuiSize {
            width: 320,
            height: 380,
        })
        .unwrap();
    assert_eq!((adjusted.width, adjusted.height), (320, 380));
    controller.set_size(adjusted).unwrap();
    let attached = controller.get_size().unwrap();
    assert_eq!((attached.width, attached.height), (320, 380));

    // Reopening at 125% or 100% still derives from the preserved 320x380 logical baseline.
    controller.destroy();
    controller.create(config).unwrap();
    let reopened_at_125 = controller.get_size().unwrap();
    assert_eq!((reopened_at_125.width, reopened_at_125.height), (400, 475));

    controller.set_scale(1.0).unwrap();
    let reopened_at_100 = controller.get_size().unwrap();
    assert_eq!((reopened_at_100.width, reopened_at_100.height), (320, 380));
}

#[test]
fn preserves_logical_baseline_when_attached_host_echoes_unscaled_frame() {
    // When default > min, echoing the existing 600x400 frame after a late scale update must
    // not overwrite the 600x400 logical baseline with 480x320.
    let controller = test_controller(
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
        GuiDpiPolicy::Windows,
    );
    let config = default_gui_configuration();

    controller.create(config).unwrap();
    controller.runtime.lock().session.as_mut().unwrap().parent =
        Some(StoredParentWindow::Win32 { hwnd: 1 });
    controller.set_scale(1.25).unwrap();
    controller
        .set_size(GuiSize {
            width: 600,
            height: 400,
        })
        .unwrap();

    controller.destroy();
    controller.create(config).unwrap();
    let reopened = controller.get_size().unwrap();
    assert_eq!((reopened.width, reopened.height), (750, 500));
}

#[test]
fn allows_attached_fixed_size_frame_to_grow_when_host_scales_parent() {
    let controller = test_controller(
        GuiSize {
            width: 320,
            height: 380,
        },
        GuiSizeLimits {
            min: GuiSize {
                width: 320,
                height: 380,
            },
            max: GuiSize {
                width: 320,
                height: 380,
            },
        },
        GuiDpiPolicy::Windows,
    );
    let config = default_gui_configuration();

    controller.create(config).unwrap();
    controller.runtime.lock().session.as_mut().unwrap().parent =
        Some(StoredParentWindow::Win32 { hwnd: 1 });
    controller.set_scale(1.25).unwrap();

    // If the host does resize its parent to 125% (400x475), the controller must accept it
    // instead of clamping back to 320x380.
    let adjusted = controller
        .adjust_size(GuiSize {
            width: 400,
            height: 475,
        })
        .unwrap();
    assert_eq!((adjusted.width, adjusted.height), (400, 475));
    controller.set_size(adjusted).unwrap();
    let grown = controller.get_size().unwrap();
    assert_eq!((grown.width, grown.height), (400, 475));
}
