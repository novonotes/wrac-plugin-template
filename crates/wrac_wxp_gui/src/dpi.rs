use wrac_clap_adapter::GuiSize;
use wxp::dpi::{LogicalPosition, LogicalSize, Size};

/// CLAP GUI size と wxp bounds の変換。
///
/// macOS/Windows と Linux で WebView が期待する単位が違うため、製品ごとに同じ DPI
/// 分岐を持たせず、wxp integration 側に寄せる。
pub struct DpiConverter {
    scale_factor: f64,
    uses_logical: bool,
}

impl DpiConverter {
    pub fn new(scale_factor: f64) -> Self {
        Self {
            scale_factor,
            uses_logical: cfg!(any(target_os = "macos", target_os = "windows")),
        }
    }

    pub fn set_scale(&mut self, scale_factor: f64) {
        self.scale_factor = scale_factor;
    }

    pub fn create_webview_bounds(&self, size: LogicalSize<f64>) -> wxp::Rect {
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

pub fn gui_size_to_logical(size: GuiSize) -> LogicalSize<f64> {
    LogicalSize::new(size.width as f64, size.height as f64)
}

/// wxp の logical size を CLAP `GuiSize` へ戻す helper。
/// WebView/layout 由来のサイズを resize request や state に返すとき、platform
/// ごとの変換を製品側に散らさないよう public API として残している。
pub fn logical_size_to_gui(size: LogicalSize<f64>) -> GuiSize {
    GuiSize {
        width: size.width as u32,
        height: size.height as u32,
    }
}
