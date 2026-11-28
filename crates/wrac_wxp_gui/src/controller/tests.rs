use super::resize::GuiResizePolicy;
use super::scale::scale_implied_by_host_resize;
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
    )
}

/// How the test host answers `request_resize`. It never calls `set_size` back.
#[derive(Clone, Copy)]
enum HostResizeAnswer {
    Accept,
    Reject,
}

struct TestHostGui {
    answer: HostResizeAnswer,
    requested: Mutex<Vec<(u32, u32)>>,
}

impl TestHostGui {
    fn new(answer: HostResizeAnswer) -> Arc<Self> {
        Arc::new(Self {
            answer,
            requested: Mutex::new(Vec::new()),
        })
    }

    fn requested(&self) -> Vec<(u32, u32)> {
        self.requested.lock().clone()
    }
}

impl HostGui for TestHostGui {
    fn request_resize(&self, size: GuiSize) -> PluginResult<()> {
        self.requested.lock().push((size.width, size.height));
        match self.answer {
            HostResizeAnswer::Accept => Ok(()),
            HostResizeAnswer::Reject => Err(PluginError::Message("rejected".into())),
        }
    }
}

fn test_controller(
    initial_size: GuiSize,
    limits: GuiSizeLimits,
    dpi_policy: GuiDpiPolicy,
) -> WxpGuiController {
    // A host that keeps its frame is the case the clamping rules have to survive.
    test_controller_in_host(
        initial_size,
        limits,
        dpi_policy,
        HostFamily::Unknown,
        TestHostGui::new(HostResizeAnswer::Reject),
    )
}

fn test_controller_in_host(
    initial_size: GuiSize,
    limits: GuiSizeLimits,
    dpi_policy: GuiDpiPolicy,
    family: HostFamily,
    host_gui: Arc<TestHostGui>,
) -> WxpGuiController {
    let (context, _) = HostContext::new(
        DetectedHost {
            family,
            display_name: "test".to_string(),
            process_name: "test".to_string(),
            process_path: String::new(),
            version: None,
        },
        PluginFormat::Vst3,
        wrac_host_context::SystemContext::detect(),
    );
    let resize_handle =
        WxpGuiResizeHandle::with_dpi_policy(initial_size, limits, dpi_policy, context, host_gui);
    WxpGuiController::new(
        |_run_loop: &RunLoopLocal,
         _config: GuiConfig,
         _initial_size: GuiSize,
         _parent: ParentWindowHandle|
         -> PluginResult<Box<dyn WxpGuiRuntime>> { Err(PluginError::InvalidState) },
        resize_handle,
    )
}

fn attach_test_parent(controller: &WxpGuiController) {
    // `set_parent` needs a GUI run loop, so tests mark the session as attached directly.
    controller.runtime.lock().session.as_mut().unwrap().parent =
        Some(StoredParentWindow::Win32 { hwnd: 1 });
}

const EDITOR_SIZE: GuiSize = GuiSize {
    width: 322,
    height: 381,
};
const EDITOR_LIMITS: GuiSizeLimits = GuiSizeLimits {
    min: GuiSize {
        width: 320,
        height: 380,
    },
    max: GuiSize {
        width: 720,
        height: 720,
    },
};

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

#[test]
fn requests_scaled_frame_when_scale_arrives_after_attach() {
    // Hosts may report the content scale after attaching, and again after a monitor change.
    // Without a request the frame keeps its old size and the logical viewport shrinks
    // below the editor minimum.
    let host_gui = TestHostGui::new(HostResizeAnswer::Accept);
    let controller = test_controller_in_host(
        EDITOR_SIZE,
        EDITOR_LIMITS,
        GuiDpiPolicy::Windows,
        HostFamily::Unknown,
        host_gui.clone(),
    );
    controller.create(default_gui_configuration()).unwrap();
    attach_test_parent(&controller);

    controller.set_scale(1.33).unwrap();
    let scaled = controller.get_size().unwrap();
    assert_eq!((scaled.width, scaled.height), (428, 507));

    // An unchanged scale must not disturb the host again.
    controller.set_scale(1.33).unwrap();
    assert_eq!(host_gui.requested(), vec![(428, 507)]);

    // The granted frame is the rounded baseline, so it must not replace the baseline.
    controller.set_scale(1.0).unwrap();
    assert_eq!(host_gui.requested(), vec![(428, 507), (322, 381)]);
}

#[test]
fn leaves_frame_requests_to_platforms_that_scale_layout() {
    for policy in [GuiDpiPolicy::Macos, GuiDpiPolicy::Linux] {
        let host_gui = TestHostGui::new(HostResizeAnswer::Accept);
        let controller = test_controller_in_host(
            EDITOR_SIZE,
            EDITOR_LIMITS,
            policy,
            HostFamily::Unknown,
            host_gui.clone(),
        );
        controller.create(default_gui_configuration()).unwrap();
        attach_test_parent(&controller);

        controller.set_scale(2.0).unwrap();
        assert!(host_gui.requested().is_empty());
    }
}

#[test]
fn applies_requested_frame_in_hosts_that_resize_without_confirmation() {
    let host_gui = TestHostGui::new(HostResizeAnswer::Reject);
    let controller = test_controller_in_host(
        EDITOR_SIZE,
        EDITOR_LIMITS,
        GuiDpiPolicy::Windows,
        HostFamily::BitwigStudio,
        host_gui,
    );
    controller.create(default_gui_configuration()).unwrap();
    attach_test_parent(&controller);

    controller.set_scale(1.25).unwrap();
    let size = controller.get_size().unwrap();
    assert_eq!((size.width, size.height), (403, 476));
}

#[test]
fn defers_size_report_in_ableton_live_until_attached() {
    // Live sizes the attached view from a `getSize` it issued before reporting the scale.
    let controller = test_controller_in_host(
        EDITOR_SIZE,
        EDITOR_LIMITS,
        GuiDpiPolicy::Windows,
        HostFamily::AbletonLive,
        TestHostGui::new(HostResizeAnswer::Accept),
    );
    controller.create(default_gui_configuration()).unwrap();
    assert!(controller.get_size().is_err());

    controller.set_scale(1.25).unwrap();
    attach_test_parent(&controller);
    let size = controller.get_size().unwrap();
    assert_eq!((size.width, size.height), (403, 476));
}

#[test]
fn adopts_monitor_scale_only_when_host_resize_matches_it() {
    let previous = GuiSize {
        width: 1200,
        height: 600,
    };
    // Moving from a 125% to a 150% monitor: the host first resizes the frame by 1.2.
    let rescaled = GuiSize {
        width: 1440,
        height: 720,
    };
    assert_eq!(
        scale_implied_by_host_resize(previous, rescaled, 1.25, 1.2),
        Some(1.5)
    );

    // A user resize on the new monitor must not be mistaken for the monitor change.
    let dragged = GuiSize {
        width: 1230,
        height: 610,
    };
    assert_eq!(
        scale_implied_by_host_resize(previous, dragged, 1.25, 1.2),
        None
    );
}

#[test]
fn admits_frames_sized_for_a_pending_monitor_scale() {
    // The VST3 wrapper passes a host frame through `adjust_size` before `set_size`. An editor
    // at its minimum size that moves from a 150% to a 100% monitor receives the 100% frame
    // while 150% is still active; clamping it back to the 150% minimum would hide the move.
    let layout = HostGuiLayout::with_dpi_policy(
        EDITOR_LIMITS.min,
        EDITOR_LIMITS,
        GuiResizePolicy::RESIZABLE,
        GuiDpiPolicy::Windows,
    );
    layout.set_scale_before_attach(1.5);

    let clamped_to_active_scale = layout.clamp_host_size(EDITOR_LIMITS.min);
    assert_eq!(
        (
            clamped_to_active_scale.width,
            clamped_to_active_scale.height
        ),
        (480, 570)
    );

    let admitted = layout.clamp_host_size_allowing_scale(EDITOR_LIMITS.min, 1.0);
    assert_eq!((admitted.width, admitted.height), (320, 380));
}
