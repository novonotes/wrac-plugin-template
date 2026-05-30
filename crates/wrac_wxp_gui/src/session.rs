use std::path::PathBuf;
use std::rc::Rc;

use directories::ProjectDirs;
use wrac_clap_adapter::{GuiSize, PluginError, PluginResult};
use wxp::{WebContext, WxpCommandHandler, WxpWebView, WxpWebViewBuilder, dpi::LogicalSize};

use crate::controller::GuiSizeLimits;
use crate::dpi::DpiConverter;
use crate::window::ParentWindowHandle;

/// Frontend source loaded by the WebView.
///
/// Product crates decide how to switch debug/release sources. `wrac_wxp_gui` only
/// knows how to open a URL or serve a zip under a custom scheme; frontend structure
/// and command contracts remain product code.
pub enum WxpFrontendSource {
    Url {
        url: &'static str,
    },
    Zip {
        scheme: &'static str,
        url: &'static str,
        bytes: &'static [u8],
    },
}

/// Configuration needed to create a native WebView session.
///
/// `plugin_id` is also used to isolate WebView user-data directories, so pass the same
/// reverse-DNS ID used by the host descriptor. Size limits are interpreted as physical
/// pixels because that is the CLAP host boundary.
pub struct WxpWebViewConfig {
    pub plugin_id: &'static str,
    pub initial_size: GuiSize,
    pub limits: GuiSizeLimits,
    pub parent: ParentWindowHandle,
    pub frontend: WxpFrontendSource,
    pub devtools: bool,
}

/// WebView ownership component embedded by product runtimes.
///
/// This type owns only native WebView state, DPI/bounds application, show/hide, and
/// WebContext drop ordering. Timers, state sync, command registration, and parameter
/// notification stay in product runtimes so each GUI can choose its own sync strategy.
pub struct WxpWebViewSession {
    // WebView teardown can touch the context, so Drop sequences these fields explicitly.
    web_view: Option<WxpWebView>,
    wxp_context: Option<WebContext>,
    // wxp expects the command handler to be owned externally, so keep it alive with the WebView.
    command_handler: Rc<WxpCommandHandler>,
    host_size: GuiSize,
    logical_size: LogicalSize<f64>,
    limits: GuiSizeLimits,
    dpi_converter: DpiConverter,
}

impl WxpWebViewSession {
    pub fn create(
        config: WxpWebViewConfig,
        command_handler: Rc<WxpCommandHandler>,
    ) -> PluginResult<Self> {
        log::debug!(
            "creating wxp WebView session: plugin_id={}, width={}, height={}",
            config.plugin_id,
            config.initial_size.width,
            config.initial_size.height
        );

        let data_dir = webview_data_dir(config.plugin_id);
        std::fs::create_dir_all(&data_dir)
            .map_err(|_| PluginError::Message("failed to create GUI data directory"))?;
        log::debug!("using GUI data directory: {}", data_dir.display());

        let mut wxp_context = WebContext::new(data_dir);
        let dpi_converter = DpiConverter::new(1.0);
        // The host contract is physical pixels. Convert initial bounds here so product
        // runtimes never need platform-specific DPI branches.
        let host_size = clamp_size(config.initial_size, config.limits);
        let logical_size = dpi_converter.gui_size_to_logical(host_size);
        let bounds = dpi_converter.create_webview_bounds(logical_size);

        let builder = match config.frontend {
            WxpFrontendSource::Url { url } => {
                log::debug!("configuring wxp WebView session URL frontend: url={url}");
                WxpWebViewBuilder::new(&mut wxp_context)
                    .with_command_handler(command_handler.clone())
                    .with_devtools(config.devtools)
                    .with_visible(true)
                    .with_bounds(bounds)
                    .with_url(url)
            }
            WxpFrontendSource::Zip { scheme, url, bytes } => {
                log::debug!("configuring wxp WebView session zip frontend: url={url}");
                WxpWebViewBuilder::new(&mut wxp_context)
                    .with_command_handler(command_handler.clone())
                    .with_devtools(config.devtools)
                    .with_visible(true)
                    .with_bounds(bounds)
                    .with_serve_zip(scheme, bytes)
                    .map_err(|_| PluginError::Message("failed to serve GUI assets"))?
                    .with_url(url)
            }
        };

        let web_view = builder
            .build_as_child(&config.parent)
            .map_err(|_| PluginError::Message("failed to build webview"))?;

        log::debug!("creating wxp WebView session completed");
        Ok(Self {
            web_view: Some(web_view),
            wxp_context: Some(wxp_context),
            command_handler,
            host_size,
            logical_size,
            limits: config.limits,
            dpi_converter,
        })
    }

    pub fn set_scale(&mut self, scale: f64) -> PluginResult<()> {
        log::debug!("setting wxp WebView scale: scale={scale}");
        self.dpi_converter.set_scale(scale);
        // Hosts may not resend set_size after a scale change. Reapply bounds immediately
        // so Linux physical bounds and macOS/Windows logical bounds do not keep stale scale.
        self.logical_size = self.dpi_converter.gui_size_to_logical(self.host_size);
        self.apply_bounds()
    }

    pub fn set_size(&mut self, size: GuiSize) -> PluginResult<()> {
        self.host_size = clamp_size(size, self.limits);
        self.logical_size = self.dpi_converter.gui_size_to_logical(self.host_size);
        log::debug!(
            "setting wxp WebView size: requested_width={}, requested_height={}, applied_width={}, applied_height={}",
            size.width,
            size.height,
            self.logical_size.width,
            self.logical_size.height
        );
        self.apply_bounds()
    }

    pub fn show(&mut self) -> PluginResult<()> {
        log::debug!("showing wxp WebView session");
        if let Some(web_view) = &self.web_view {
            web_view
                .dispatch()
                .post_set_visible(true)
                .map_err(|_| PluginError::Message("failed to show webview"))?;
        }
        Ok(())
    }

    pub fn hide(&mut self) -> PluginResult<()> {
        log::debug!("hiding wxp WebView session");
        if let Some(web_view) = &self.web_view {
            web_view
                .dispatch()
                .post_set_visible(false)
                .map_err(|_| PluginError::Message("failed to hide webview"))?;
        }
        Ok(())
    }

    fn apply_bounds(&self) -> PluginResult<()> {
        if let Some(web_view) = &self.web_view {
            // Use the same dispatch path as command handlers and close handling so a closing
            // WebView's native owner is not kept alive by direct access.
            web_view
                .dispatch()
                .post_set_bounds(self.dpi_converter.create_webview_bounds(self.logical_size))
                .map_err(|_| PluginError::Message("failed to resize webview"))?;
        }
        Ok(())
    }
}

impl Drop for WxpWebViewSession {
    fn drop(&mut self) {
        log::debug!("dropping wxp WebView session");
        self.web_view = None;
        log::debug!("dropping wxp WebView session: webview dropped");
        self.wxp_context = None;
        log::debug!("dropping wxp WebView session: web context dropped");
        let _ = Rc::strong_count(&self.command_handler);
    }
}

fn clamp_size(size: GuiSize, limits: GuiSizeLimits) -> GuiSize {
    GuiSize {
        width: size.width.clamp(limits.min.width, limits.max.width),
        height: size.height.clamp(limits.min.height, limits.max.height),
    }
}

fn webview_data_dir(plugin_id: &str) -> PathBuf {
    let plugin_dir = sanitize_plugin_data_dir(plugin_id);
    // Derive the user-data path from plugin_id so a plugin created from the template
    // does not share cookies, cache, or storage with the original template plugin.
    match project_dirs_from_plugin_id(plugin_id) {
        Some(dirs) => dirs.data_dir().join("webview").join(plugin_dir),
        None => std::env::temp_dir()
            .join(plugin_dir)
            .join("webview")
            .join("data"),
    }
}

fn project_dirs_from_plugin_id(plugin_id: &str) -> Option<ProjectDirs> {
    let mut parts = plugin_id.split('.');
    let qualifier = parts.next()?;
    let organization = parts.next()?;
    let application = parts.collect::<Vec<_>>().join("-");
    if application.is_empty() {
        return None;
    }
    ProjectDirs::from(qualifier, organization, &application)
}

fn sanitize_plugin_data_dir(plugin_id: &str) -> String {
    plugin_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_host_size_with_physical_limits() {
        let clamped = clamp_size(
            GuiSize {
                width: 100,
                height: 900,
            },
            GuiSizeLimits {
                min: GuiSize {
                    width: 320,
                    height: 240,
                },
                max: GuiSize {
                    width: 640,
                    height: 480,
                },
            },
        );

        assert_eq!(clamped.width, 320);
        assert_eq!(clamped.height, 480);
    }
}
