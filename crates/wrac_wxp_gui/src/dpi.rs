use wrac_clap_adapter::GuiSize;
use wxp::dpi::{LogicalPosition, LogicalSize, Size};

/// Conversion between CLAP GUI sizes and wxp bounds.
///
/// [`GuiSize`] values exchanged with the CLAP host are always physical pixels.
/// WebView bounds are logical on macOS/Windows and physical on Linux, so keep
/// platform-specific pixel arithmetic out of product code.
pub(crate) struct DpiConverter {
    scale_factor: f64,
    uses_logical: bool,
}

impl DpiConverter {
    pub(crate) fn new(scale_factor: f64) -> Self {
        Self {
            scale_factor,
            uses_logical: cfg!(any(target_os = "macos", target_os = "windows")),
        }
    }

    pub(crate) fn set_scale(&mut self, scale_factor: f64) {
        self.scale_factor = scale_factor;
    }

    /// Converts a host physical-pixel [`GuiSize`] into a logical size for internal layout.
    pub(crate) fn gui_size_to_logical(&self, size: GuiSize) -> LogicalSize<f64> {
        LogicalSize::new(
            size.width as f64 / self.scale_factor,
            size.height as f64 / self.scale_factor,
        )
    }

    /// Converts a logical size back into a host physical-pixel [`GuiSize`].
    pub(crate) fn logical_size_to_gui(&self, size: LogicalSize<f64>) -> GuiSize {
        GuiSize {
            width: (size.width * self.scale_factor).round() as u32,
            height: (size.height * self.scale_factor).round() as u32,
        }
    }

    pub(crate) fn create_webview_bounds(&self, size: LogicalSize<f64>) -> wxp::Rect {
        wxp::Rect {
            position: LogicalPosition::new(0, 0).into(),
            size: if self.uses_logical {
                Size::Logical(size)
            } else {
                Size::Physical(size.to_physical(self.scale_factor))
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
}
