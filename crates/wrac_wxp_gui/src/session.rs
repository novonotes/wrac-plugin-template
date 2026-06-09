use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use directories::ProjectDirs;
use url::{Host, Url};
use wrac_clap_adapter::{GuiSize, PluginError, PluginResult};
use wxp::{WebContext, WxpCommandHandler, WxpWebView, WxpWebViewBuilder, dpi::LogicalSize};

use crate::controller::GuiSizeLimits;
use crate::dpi::{DpiConverter, HostGuiSizeUnit};
use crate::window::ParentWindowHandle;

const URL_PROBE_TIMEOUT: Duration = Duration::from_millis(500);

/// Frontend source loaded by the WebView.
///
/// Product crates decide how to switch debug/release sources. `wrac_wxp_gui` only
/// knows how to open a URL or serve a zip under a custom scheme; frontend structure
/// and command contracts remain product code.
pub enum WxpFrontendSource {
    Html {
        html: &'static str,
    },
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
/// reverse-DNS ID used by the host descriptor. Size limits use the same unit as the
/// host GUI size callbacks selected by `host_size_unit`.
pub struct WxpWebViewConfig {
    pub plugin_id: &'static str,
    pub initial_size: GuiSize,
    pub limits: GuiSizeLimits,
    pub host_size_unit: HostGuiSizeUnit,
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
        let dpi_converter = DpiConverter::with_host_size_unit(1.0, config.host_size_unit);
        // Convert initial bounds here so product runtimes never need host- or
        // platform-specific DPI branches.
        let host_size = clamp_size(config.initial_size, config.limits);
        let logical_size = dpi_converter.gui_size_to_logical(host_size);
        let bounds = dpi_converter.create_webview_bounds(logical_size);

        let builder = match config.frontend {
            WxpFrontendSource::Html { html } => {
                log::debug!("configuring wxp WebView session HTML frontend");
                WxpWebViewBuilder::new(&mut wxp_context)
                    .with_command_handler(command_handler.clone())
                    .with_devtools(config.devtools)
                    .with_visible(true)
                    .with_bounds(bounds)
                    .with_html(html)
            }
            WxpFrontendSource::Url { url } => {
                log::debug!("configuring wxp WebView session URL frontend: url={url}");
                let builder = WxpWebViewBuilder::new(&mut wxp_context)
                    .with_command_handler(command_handler.clone())
                    .with_devtools(config.devtools)
                    .with_visible(true)
                    .with_bounds(bounds);
                // `Url` is commonly used for development servers. Probe before navigation so
                // connection-level failures become deterministic plugin UI instead of depending
                // on host/WebView-specific blank-page behavior.
                match probe_frontend_url(url, URL_PROBE_TIMEOUT) {
                    Ok(ProbeOutcome::Reachable) | Ok(ProbeOutcome::Skipped) => {
                        builder.with_url(url)
                    }
                    Err(error) => {
                        log::warn!(
                            "wxp WebView URL probe failed: url={}, target={}, error={}",
                            url,
                            error.target,
                            error.error
                        );
                        builder.with_html(url_probe_error_html(url, &error, URL_PROBE_TIMEOUT))
                    }
                }
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

enum ProbeOutcome {
    Reachable,
    Skipped,
}

#[derive(Debug)]
struct UrlProbeError {
    /// The endpoint users should inspect first when the fallback page appears.
    target: String,
    /// Preserves the low-level failure text because this path is primarily a developer diagnostic.
    error: String,
}

#[derive(Debug)]
struct ProbeTarget {
    /// Pre-resolved addresses keep GUI creation from blocking on DNS without a global timeout.
    addresses: Vec<SocketAddr>,
    /// Human-readable endpoint shown in logs and fallback HTML.
    display: String,
}

/// Performs a narrow transport probe for WebView URL frontends.
///
/// This intentionally checks only that a TCP connection can be established. It does not issue
/// an HTTP request, validate TLS, or interpret status codes because the goal is to catch the
/// blank-screen class of failures where no server is listening or the host cannot be reached.
fn probe_frontend_url(url: &str, timeout: Duration) -> Result<ProbeOutcome, UrlProbeError> {
    let parsed = Url::parse(url).map_err(|error| UrlProbeError {
        target: url.to_string(),
        error: format!("URL parse failed: {error}"),
    })?;

    // Custom schemes are resolved by the WebView/custom-protocol layer, so a TCP probe would
    // create false failures for release assets served from `with_serve_zip`.
    if !matches!(parsed.scheme(), "http" | "https") {
        return Ok(ProbeOutcome::Skipped);
    }

    let Some(target) = probe_target(&parsed)? else {
        return Ok(ProbeOutcome::Skipped);
    };

    if target.addresses.is_empty() {
        return Err(UrlProbeError {
            target: target.display,
            error: "probe target produced no socket addresses".to_string(),
        });
    }

    // Embedded WebViews do not consistently render their own network error pages in plugin
    // hosts. Probe first so a refused or unreachable GUI URL becomes visible instead of blank.
    let mut errors = Vec::new();
    let started_at = Instant::now();
    for address in target.addresses {
        let elapsed = started_at.elapsed();
        if elapsed >= timeout {
            errors.push(format!("{address}: probe timeout elapsed"));
            break;
        }

        let remaining = timeout.saturating_sub(elapsed);
        match TcpStream::connect_timeout(&address, remaining) {
            Ok(_) => return Ok(ProbeOutcome::Reachable),
            Err(error) => errors.push(format!("{address}: {error}")),
        }
    }

    Err(UrlProbeError {
        target: target.display,
        error: errors.join("; "),
    })
}

/// Converts a URL into TCP endpoints that are safe to probe synchronously.
///
/// Arbitrary domain names are skipped because std DNS resolution has no timeout control and this
/// code runs while the plugin host is creating the GUI. `localhost` is mapped manually so common
/// dev-server URLs still get the deterministic fallback page without touching DNS.
fn probe_target(url: &Url) -> Result<Option<ProbeTarget>, UrlProbeError> {
    let host = url.host().ok_or_else(|| UrlProbeError {
        target: url.as_str().to_string(),
        error: "URL has no host".to_string(),
    })?;
    let port = url.port_or_known_default().ok_or_else(|| UrlProbeError {
        target: url.as_str().to_string(),
        error: format!("URL scheme has no known default port: {}", url.scheme()),
    })?;

    let (addresses, display_host) = match host {
        Host::Domain(domain) if domain.eq_ignore_ascii_case("localhost") => (
            vec![
                SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
                SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), port),
            ],
            domain.to_string(),
        ),
        Host::Domain(_) => return Ok(None),
        Host::Ipv4(address) => (
            vec![SocketAddr::new(IpAddr::V4(address), port)],
            address.to_string(),
        ),
        Host::Ipv6(address) => {
            let display = format!("[{address}]");
            (vec![SocketAddr::new(IpAddr::V6(address), port)], display)
        }
    };

    let display = format!("{display_host}:{port}");
    Ok(Some(ProbeTarget { addresses, display }))
}

/// Builds the developer-facing fallback page shown when a URL frontend cannot be reached.
///
/// The page includes the raw connection error instead of a friendlier summary because the user
/// usually needs the OS/socket message to distinguish "server not running", DNS, firewall, and
/// wrong-port failures.
fn url_probe_error_html(url: &str, error: &UrlProbeError, timeout: Duration) -> String {
    let timeout_ms = timeout.as_millis();
    let url_html = escape_html(url);
    let target_html = escape_html(&error.target);
    let error_html = escape_html(&error.error);
    let url_js = escape_js_string(url);

    // Keep this page dependency-free because it is the last-resort UI when the real frontend
    // cannot be fetched. Retry probes in-page before navigating so a failed retry keeps the
    // diagnostic visible instead of returning to host/WebView-specific blank-page behavior.
    format!(
        r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Failed to load URL</title>
    <style>
      :root {{
        color-scheme: dark;
        font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
        background: #111;
        color: #eee;
      }}
      body {{
        margin: 0;
        min-height: 100vh;
        display: grid;
        place-items: center;
        background: #111;
      }}
      main {{
        box-sizing: border-box;
        width: min(680px, 100vw);
        padding: 24px;
      }}
      h1 {{
        margin: 0 0 18px;
        font-size: 20px;
        font-weight: 650;
      }}
      dl {{
        display: grid;
        gap: 14px;
        margin: 0;
      }}
      dt {{
        margin-bottom: 5px;
        color: #aaa;
        font-size: 12px;
        font-weight: 700;
        text-transform: uppercase;
      }}
      dd {{
        margin: 0;
      }}
      code, pre {{
        font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
        font-size: 12px;
      }}
      code {{
        overflow-wrap: anywhere;
      }}
      pre {{
        white-space: pre-wrap;
        overflow-wrap: anywhere;
        margin: 0;
        padding: 10px;
        border: 1px solid #3a3a3a;
        background: #181818;
      }}
      button {{
        margin-top: 20px;
        height: 32px;
        padding: 0 14px;
        border: 1px solid #666;
        border-radius: 4px;
        background: #2b2b2b;
        color: #fff;
        font: inherit;
        cursor: pointer;
      }}
      button:hover {{
        background: #383838;
      }}
      button:disabled {{
        cursor: default;
        opacity: 0.65;
      }}
    </style>
  </head>
  <body>
    <main>
      <h1>Failed to load URL</h1>
      <dl>
        <div>
          <dt>URL</dt>
          <dd><code>{url_html}</code></dd>
        </div>
        <div>
          <dt>Probe target</dt>
          <dd><code>{target_html}</code></dd>
        </div>
        <div>
          <dt>Timeout</dt>
          <dd><code>{timeout_ms} ms</code></dd>
        </div>
        <div>
          <dt>Error</dt>
          <dd><pre id="error">{error_html}</pre></dd>
        </div>
      </dl>
      <button type="button" id="retry">Retry</button>
    </main>
    <script>
      const url = "{url_js}";
      const timeoutMs = {timeout_ms};
      const retry = document.getElementById("retry");
      const error = document.getElementById("error");

      retry.addEventListener("click", async () => {{
        retry.disabled = true;
        retry.textContent = "Retrying...";

        const controller = new AbortController();
        const timer = setTimeout(() => controller.abort(), timeoutMs);
        try {{
          await fetch(url, {{
            method: "GET",
            mode: "no-cors",
            cache: "no-store",
            signal: controller.signal,
          }});
          window.location.replace(url);
        }} catch (err) {{
          const message = err && err.message ? err.message : String(err);
          error.textContent = `Retry failed: ${{message}}`;
          retry.disabled = false;
          retry.textContent = "Retry";
        }} finally {{
          clearTimeout(timer);
        }}
      }});
    </script>
  </body>
</html>"#
    )
}

fn escape_html(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

fn escape_js_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '&' => escaped.push_str("\\x26"),
            '<' => escaped.push_str("\\x3C"),
            '>' => escaped.push_str("\\x3E"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\u{2028}' => escaped.push_str("\\u2028"),
            '\u{2029}' => escaped.push_str("\\u2029"),
            _ => escaped.push(character),
        }
    }
    escaped
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

    #[test]
    fn resolves_http_probe_targets() {
        let url = Url::parse("http://127.0.0.1:5173/").unwrap();
        let target = probe_target(&url).unwrap().unwrap();
        assert_eq!(target.display, "127.0.0.1:5173");
        assert_eq!(
            target.addresses,
            vec![SocketAddr::new(
                IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
                5173
            )]
        );

        let url = Url::parse("http://localhost:5173/").unwrap();
        let target = probe_target(&url).unwrap().unwrap();
        assert_eq!(target.display, "localhost:5173");
        assert_eq!(
            target.addresses,
            vec![
                SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 5173),
                SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 5173),
            ]
        );

        let url = Url::parse("http://[::1]:5173/").unwrap();
        let target = probe_target(&url).unwrap().unwrap();
        assert_eq!(target.display, "[::1]:5173");
        assert_eq!(
            target.addresses,
            vec![SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 5173)]
        );
    }

    #[test]
    fn skips_domain_probe_targets() {
        let url = Url::parse("https://example.com/path").unwrap();
        assert!(probe_target(&url).unwrap().is_none());
    }

    #[test]
    fn escapes_error_page_values() {
        let error = UrlProbeError {
            target: "127.0.0.1:5173".to_string(),
            error: "failed <because> \"quoted\" & 'single'".to_string(),
        };
        let html = url_probe_error_html(
            "http://127.0.0.1:5173/?x=\"<&",
            &error,
            Duration::from_millis(500),
        );

        assert!(html.contains("http://127.0.0.1:5173/?x=&quot;&lt;&amp;"));
        assert!(html.contains("failed &lt;because&gt; &quot;quoted&quot; &amp; &#39;single&#39;"));
        assert!(html.contains("const url = \"http://127.0.0.1:5173/?x=\\\"\\x3C\\x26\";"));
        assert!(html.contains("await fetch(url, {"));
        assert!(html.contains("window.location.replace(url);"));
    }
}
