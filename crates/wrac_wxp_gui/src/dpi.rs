use wrac_clap_adapter::interface::GuiSize;
use wxp::dpi::{LogicalPosition, LogicalSize, Size};

/// Platform-level policy for CLAP GUI size units, layout DPI scaling, and WebView bounds.
///
/// Each target platform maps to a fixed combination:
/// - Windows: physical-pixel host callbacks, 100% logical layout/limits scaled by host DPI,
///   and physical WebView2 bounds (preventing wry from re-applying monitor DPI).
/// - macOS: Cocoa logical points at both the CLAP and WKWebView boundaries, unscaled.
/// - Linux: physical-pixel host callbacks and WebKitGTK bounds, keeping layout sizes and
///   limits unscaled in host units because WebKitGTK follows GDK integer scale rather than
///   CLAP host scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GuiDpiPolicy {
    Windows,
    Macos,
    Linux,
}

impl GuiDpiPolicy {
    pub(crate) fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Linux
        }
    }

    pub(crate) fn uses_logical_host_coords(self) -> bool {
        matches!(self, Self::Macos)
    }

    pub(crate) fn scales_layout_with_host_dpi(self) -> bool {
        matches!(self, Self::Windows)
    }

    pub(crate) fn uses_logical_webview_bounds(self) -> bool {
        matches!(self, Self::Macos)
    }
}

/// Conversion between CLAP GUI sizes and wxp bounds.
pub(crate) struct DpiConverter {
    scale_factor: f64,
    policy: GuiDpiPolicy,
}

impl DpiConverter {
    pub(crate) fn new(scale_factor: f64) -> Self {
        Self::with_policy(scale_factor, GuiDpiPolicy::current())
    }

    pub(crate) fn with_policy(scale_factor: f64, policy: GuiDpiPolicy) -> Self {
        Self {
            scale_factor,
            policy,
        }
    }

    pub(crate) fn set_scale(&mut self, scale_factor: f64) {
        self.scale_factor = scale_factor;
    }

    /// Converts a host [`GuiSize`] into a logical size for internal layout.
    pub(crate) fn gui_size_to_logical(&self, size: GuiSize) -> LogicalSize<f64> {
        if self.policy.uses_logical_host_coords() {
            LogicalSize::new(size.width as f64, size.height as f64)
        } else {
            LogicalSize::new(
                size.width as f64 / self.scale_factor,
                size.height as f64 / self.scale_factor,
            )
        }
    }

    /// Converts a logical size back into the host's GUI size unit.
    pub(crate) fn logical_size_to_gui(&self, size: LogicalSize<f64>) -> GuiSize {
        if self.policy.uses_logical_host_coords() {
            GuiSize {
                width: size.width.round() as u32,
                height: size.height.round() as u32,
            }
        } else {
            GuiSize {
                width: (size.width * self.scale_factor).round() as u32,
                height: (size.height * self.scale_factor).round() as u32,
            }
        }
    }

    pub(crate) fn create_webview_bounds(&self, size: LogicalSize<f64>) -> wxp::Rect {
        let physical_size = size.to_physical::<f64>(self.scale_factor);
        wxp::Rect {
            position: LogicalPosition::new(0, 0).into(),
            size: if self.policy.uses_logical_webview_bounds() {
                Size::Logical(size)
            } else {
                Size::Physical(physical_size.cast())
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_host_physical_size_to_logical_size() {
        let dpi = DpiConverter::with_policy(1.5, GuiDpiPolicy::Windows);
        let logical = dpi.gui_size_to_logical(GuiSize {
            width: 900,
            height: 600,
        });

        assert_eq!(logical.width, 600.0);
        assert_eq!(logical.height, 400.0);
    }

    #[test]
    fn converts_logical_size_to_host_physical_size() {
        let dpi = DpiConverter::with_policy(1.5, GuiDpiPolicy::Windows);
        let physical = dpi.logical_size_to_gui(LogicalSize::new(600.0, 400.0));

        assert_eq!(physical.width, 900);
        assert_eq!(physical.height, 600);
    }

    #[test]
    fn keeps_macos_logical_host_size_unscaled() {
        let dpi = DpiConverter::with_policy(2.0, GuiDpiPolicy::Macos);
        let logical = dpi.gui_size_to_logical(GuiSize {
            width: 320,
            height: 380,
        });

        assert_eq!(logical.width, 320.0);
        assert_eq!(logical.height, 380.0);

        let host = dpi.logical_size_to_gui(LogicalSize::new(320.0, 380.0));
        assert_eq!(host.width, 320);
        assert_eq!(host.height, 380);
    }
}
