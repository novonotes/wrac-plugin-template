use wrac_clap_adapter::GuiSize;
use wxp::dpi::{LogicalPosition, LogicalSize, Size};

/// Conversion between CLAP GUI sizes and wxp bounds.
///
/// Most hosts report CLAP GUI sizes in physical pixels, but some wrappers/hosts expose
/// native logical window coordinates. WebView bounds are logical on macOS/Windows and
/// physical on Linux, so keep host and platform-specific pixel arithmetic in one place.
pub(crate) struct DpiConverter {
    scale_factor: f64,
    host_size_unit: HostGuiSizeUnit,
    uses_logical: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostGuiSizeUnit {
    PhysicalPixels,
    LogicalPoints,
}

impl HostGuiSizeUnit {
    pub(crate) fn to_u8(self) -> u8 {
        match self {
            Self::PhysicalPixels => 0,
            Self::LogicalPoints => 1,
        }
    }

    pub(crate) fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::LogicalPoints,
            _ => Self::PhysicalPixels,
        }
    }
}

impl DpiConverter {
    #[cfg(test)]
    pub(crate) fn new(scale_factor: f64) -> Self {
        Self::with_host_size_unit(scale_factor, HostGuiSizeUnit::PhysicalPixels)
    }

    pub(crate) fn with_host_size_unit(scale_factor: f64, host_size_unit: HostGuiSizeUnit) -> Self {
        Self {
            scale_factor,
            host_size_unit,
            uses_logical: cfg!(any(target_os = "macos", target_os = "windows")),
        }
    }

    pub(crate) fn set_scale(&mut self, scale_factor: f64) {
        self.scale_factor = scale_factor;
    }

    /// Converts a host [`GuiSize`] into a logical size for internal layout.
    pub(crate) fn gui_size_to_logical(&self, size: GuiSize) -> LogicalSize<f64> {
        match self.host_size_unit {
            HostGuiSizeUnit::PhysicalPixels => LogicalSize::new(
                size.width as f64 / self.scale_factor,
                size.height as f64 / self.scale_factor,
            ),
            HostGuiSizeUnit::LogicalPoints => {
                LogicalSize::new(size.width as f64, size.height as f64)
            }
        }
    }

    /// Converts a logical size back into the host's GUI size unit.
    pub(crate) fn logical_size_to_gui(&self, size: LogicalSize<f64>) -> GuiSize {
        match self.host_size_unit {
            HostGuiSizeUnit::PhysicalPixels => GuiSize {
                width: (size.width * self.scale_factor).round() as u32,
                height: (size.height * self.scale_factor).round() as u32,
            },
            HostGuiSizeUnit::LogicalPoints => GuiSize {
                width: size.width.round() as u32,
                height: size.height.round() as u32,
            },
        }
    }

    pub(crate) fn create_webview_bounds(&self, size: LogicalSize<f64>) -> wxp::Rect {
        let physical_size = size.to_physical::<f64>(self.scale_factor);
        wxp::Rect {
            position: LogicalPosition::new(0, 0).into(),
            size: if self.uses_logical {
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
        let dpi = DpiConverter::new(1.5);
        let logical = dpi.gui_size_to_logical(GuiSize {
            width: 900,
            height: 600,
        });

        assert_eq!(logical.width, 600.0);
        assert_eq!(logical.height, 400.0);
    }

    #[test]
    fn converts_logical_size_to_host_physical_size() {
        let dpi = DpiConverter::new(1.5);
        let physical = dpi.logical_size_to_gui(LogicalSize::new(600.0, 400.0));

        assert_eq!(physical.width, 900);
        assert_eq!(physical.height, 600);
    }

    #[test]
    fn keeps_luna_style_logical_host_size_unscaled() {
        let dpi = DpiConverter::with_host_size_unit(2.0, HostGuiSizeUnit::LogicalPoints);
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
