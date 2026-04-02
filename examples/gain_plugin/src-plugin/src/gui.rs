//! GUI モジュール（CLAP gui 拡張の実装）。
//!
//! wxp の WxpWebViewBuilder を使って WebView を生成し、
//! ホストのプラグインウィンドウに埋め込む。
//! フロントエンドは HTML/CSS/JS で実装され、
//! デバッグ時は Vite dev server、リリース時は ZIP に埋め込まれたアセットを配信する。

use clack_extensions::gui::{
    AspectRatioStrategy, GuiConfiguration, GuiResizeHints, GuiSize, PluginGuiImpl, Window,
};
use clack_plugin::prelude::*;
use wxp::{WebContext, WxpWebViewBuilder, dpi::LogicalSize};
use wxp_clack::dpi::adjust_gui_size;

use crate::plugin::{DEFAULT_GUI_SIZE, WxpExampleGainMainThread};

const MIN_GUI_SIZE: LogicalSize<f64> = LogicalSize::new(280.0, 280.0);
const MAX_GUI_SIZE: LogicalSize<f64> = LogicalSize::new(720.0, 720.0);

/// リリースビルド時のみ、ビルドスクリプトが生成した ZIP をバイナリに埋め込む。
/// この ZIP にはフロントエンドの HTML/JS/CSS がすべて含まれている。
#[cfg(not(debug_assertions))]
const FRONTEND_ZIP: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/wxp_example_gain_plugin_gui.zip"));

/// CLAP gui 拡張の実装。ホストがプラグイン GUI の生成・破棄・リサイズを制御する。
impl PluginGuiImpl for WxpExampleGainMainThread<'_> {
    /// ホストが「この GUI API をサポートしていますか？」と問い合わせる。
    /// macOS では Cocoa、Windows では Win32、Linux では X11 が使われる。
    fn is_api_supported(&mut self, configuration: GuiConfiguration) -> bool {
        let Some(preferred) = self.get_preferred_api() else {
            return false;
        };
        configuration == preferred
    }

    fn get_preferred_api(&mut self) -> Option<GuiConfiguration<'_>> {
        Some(self.preferred_api()?)
    }

    /// GUI リソースの準備。実際の WebView 生成は set_parent() で行う。
    fn create(&mut self, configuration: GuiConfiguration) -> Result<(), PluginError> {
        if !self.is_api_supported(configuration) {
            return Err(PluginError::Message("Unsupported GUI configuration"));
        }
        Ok(())
    }

    /// GUI の破棄。WebView とそのリソースを解放する。
    fn destroy(&mut self) {
        self.reset_webview();
    }

    /// ホストが DPI スケーリング係数を通知する。
    /// Retina ディスプレイでは 2.0、通常ディスプレイでは 1.0 が典型的。
    fn set_scale(&mut self, scale: f64) -> Result<(), PluginError> {
        self.dpi_converter.set_scale(scale);
        Ok(())
    }

    fn can_resize(&mut self) -> bool {
        true
    }

    fn get_resize_hints(&mut self) -> Option<GuiResizeHints> {
        Some(GuiResizeHints {
            can_resize_horizontally: true,
            can_resize_vertically: true,
            // アスペクト比の制約なし。
            strategy: AspectRatioStrategy::Disregard,
        })
    }

    /// ホストがリサイズしようとしたサイズを、最小/最大の制約に収めて返す。
    fn adjust_size(&mut self, size: GuiSize) -> Option<GuiSize> {
        Some(adjust_gui_size(
            size,
            self.dpi_converter.scale_factor(),
            MIN_GUI_SIZE,
            MAX_GUI_SIZE,
        ))
    }

    /// ホストが GUI のサイズを設定する。WebView のサイズも追従させる。
    fn set_size(&mut self, size: GuiSize) -> Result<(), PluginError> {
        self.gui_size = self.dpi_converter.to_logical(size);
        let Some(web_view) = &self.web_view else {
            return Ok(());
        };
        web_view.set_bounds(self.dpi_converter.create_webview_bounds(self.gui_size))?;
        Ok(())
    }

    fn get_size(&mut self) -> Option<GuiSize> {
        Some(self.dpi_converter.to_gui(self.gui_size))
    }

    /// ホストが親ウィンドウを指定して GUI の埋め込みを要求する。
    /// ここが wxp WebView の実際の生成処理。
    fn set_parent(&mut self, parent: Window) -> Result<(), PluginError> {
        // 既存の WebView があれば破棄してからやり直す。
        self.reset_webview();

        // CLAP のウィンドウハンドルを wry が受け取れる形式に変換する。
        // macOS: NSView, Windows: HWND, Linux: X11 Window。
        let parent_handle = wxp_clack::window::clack_to_wry_window_handle(&parent)
            .map_err(|_| PluginError::Message("Window handle conversion failed"))?;

        // WebContext は WebView のユーザーデータ（キャッシュ、localStorage 等）の
        // 保存先を指定する。プラグインごとに分離することで他のプラグインと干渉しない。
        let data_dir = std::env::temp_dir().join("wxp-example-gain-plugin");
        std::fs::create_dir_all(&data_dir)
            .map_err(|_| PluginError::Message("Failed to create data directory"))?;

        let wxp_context = WebContext::new(data_dir);
        // wry_context は WebView よりも長生きする必要があるため self に保持する。
        self.wry_context = Some(wxp_context.build_wry_context());
        let Some(wry_context) = self.wry_context.as_mut() else {
            return Err(PluginError::Message("Failed to create web context"));
        };

        let bounds = self.dpi_converter.create_webview_bounds(self.gui_size);

        // --- デバッグビルド ---
        // Vite dev server（localhost:5173）に接続。ホットリロードが効く。
        #[cfg(debug_assertions)]
        let builder = WxpWebViewBuilder::new(wry_context)
            // command_handler を渡すことで、JavaScript から invoke() で
            // Rust 側のコマンドを呼び出せるようになる。
            .with_command_handler(self.command_handler.clone())
            .with_devtools(cfg!(debug_assertions))
            .with_visible(true)
            .with_bounds(bounds)
            .with_url("http://localhost:5173/");

        // --- リリースビルド ---
        // バイナリに埋め込んだ ZIP からフロントエンドアセットを配信する。
        // with_serve_zip() はカスタムプロトコル "wxp-plugin://" を登録し、
        // WebView がそのスキームに対してリクエストすると ZIP 内のファイルを返す。
        #[cfg(not(debug_assertions))]
        let builder = WxpWebViewBuilder::new(wry_context)
            .with_command_handler(self.command_handler.clone())
            .with_devtools(cfg!(debug_assertions))
            .with_visible(true)
            .with_bounds(bounds)
            .with_serve_zip("wxp-plugin", FRONTEND_ZIP)
            .map_err(|_| PluginError::Message("Failed to set serve directory"))?
            .with_url("wxp-plugin://localhost/");

        // build_as_child() で親ウィンドウの子として WebView を生成する。
        // これによりホストのプラグインウィンドウ内に WebView が埋め込まれる。
        let web_view = builder
            .build_as_child(&parent_handle)
            .map_err(|_| PluginError::Message("Failed to build webview"))?;

        self.web_view = Some(web_view);
        self.gui_size = DEFAULT_GUI_SIZE;
        Ok(())
    }

    /// フローティングウィンドウ（トランジェント）は非対応。
    fn set_transient(&mut self, _window: Window) -> Result<(), PluginError> {
        Err(PluginError::Message("Transient windows are not supported"))
    }

    fn suggest_title(&mut self, _title: &str) {}

    fn show(&mut self) -> Result<(), PluginError> {
        Ok(())
    }

    fn hide(&mut self) -> Result<(), PluginError> {
        Ok(())
    }
}
