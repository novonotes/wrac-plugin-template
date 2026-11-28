//! Keeps the host-owned editor frame in step with the size the editor prefers.
//!
//! The host creates and resizes the frame; the controller can only ask for one. These
//! methods decide when to ask and which host-delivered frames to adopt.

use wrac_clap_adapter::interface::{GuiSize, PluginResult};
use wrac_host_context::{HostContext, HostFamily, PluginFormat};

use super::WxpGuiController;
use super::scale::{monitor_scale_for_parent, scale_implied_by_host_resize};

impl WxpGuiController {
    /// Asks the host for the frame that shows the preferred logical size at the active scale.
    ///
    /// Must be called on the main thread with the runtime lock released: hosts may call
    /// `set_size` re-entrantly from within the request.
    pub(super) fn request_preferred_host_frame(&self) -> PluginResult<()> {
        let Some(size) = self.layout.preferred_host_size() else {
            return Ok(());
        };
        log::debug!(
            "wxp controller: requesting preferred host frame: width={}, height={}",
            size.width,
            size.height
        );
        let previous_revision = self.layout.accepted_size_revision();
        let result = self.host_gui.request_resize(size);
        if self.layout.accepted_size_revision() != previous_revision {
            // The host delivered a frame through a re-entrant `set_size`. That frame is the
            // ground truth even when it differs from the request or the request reports
            // failure, so nothing is left to apply here.
            return Ok(());
        }
        match result {
            // Some hosts accept the request but never call `set_size`.
            Ok(()) => self.apply_host_size(size),
            Err(error) if host_resizes_without_confirmation(&self.host_context) => {
                log::debug!(
                    "wxp controller: applying preferred host frame without host confirmation: {error}"
                );
                self.apply_host_size(size)
            }
            Err(error) => {
                // The host keeps its current frame. Growing the child WebView regardless
                // would clip it against the parent, so stay at the last host-confirmed size;
                // `HostGuiLayout::clamp_host_size` keeps accepting that smaller frame.
                log::debug!("wxp controller: host kept its frame after a resize request: {error}");
                Ok(())
            }
        }
    }

    /// Same as [`Self::request_preferred_host_frame`], but skips the request while the
    /// accepted frame already matches so an unchanged scale does not disturb the host.
    pub(super) fn request_preferred_host_frame_if_stale(&self) -> PluginResult<()> {
        let Some(preferred) = self.layout.preferred_host_size() else {
            return Ok(());
        };
        let accepted = self.layout.accepted_size();
        if preferred.width == accepted.width && preferred.height == accepted.height {
            return Ok(());
        }
        self.request_preferred_host_frame()
    }

    /// Adopts a new scale when the host resized the frame for another monitor before
    /// reporting that monitor's content scale. See [`scale_implied_by_host_resize`].
    pub(super) fn adopt_scale_implied_by_host_resize(
        &self,
        requested_size: GuiSize,
    ) -> PluginResult<()> {
        // This ordering has only been observed through the VST3 content-scale interface.
        if self.host_context.plugin_format != PluginFormat::Vst3 {
            return Ok(());
        }
        let (handle, scale) = {
            let mut state = self.runtime.lock();
            let Some(session) = state.session.as_mut() else {
                return Ok(());
            };
            let Some(reported_monitor_scale) = session.monitor_scale_at_scale_report else {
                return Ok(());
            };
            let Some(monitor_scale) = monitor_scale_for_parent(session.parent) else {
                return Ok(());
            };
            if (monitor_scale - reported_monitor_scale).abs() <= f64::EPSILON {
                return Ok(());
            }
            let Some(scale) = scale_implied_by_host_resize(
                self.layout.accepted_size(),
                requested_size,
                self.layout.scale(),
                monitor_scale / reported_monitor_scale,
            ) else {
                return Ok(());
            };
            session.monitor_scale_at_scale_report = Some(monitor_scale);
            self.layout.set_scale_while_attached(scale);
            (session.handle.clone(), scale)
        };
        log::info!("wxp controller: adopted scale {scale} implied by a host resize");
        if let Some(handle) = handle {
            handle.set_scale(scale)?;
        }
        Ok(())
    }

    /// Records a host-confirmed frame and mirrors it to the WebView runtime.
    pub(super) fn apply_host_size(&self, size: GuiSize) -> PluginResult<()> {
        let previous_size = self.layout.accepted_size();
        let size_changed = previous_size.width != size.width || previous_size.height != size.height;
        let handle = self
            .runtime
            .lock()
            .session
            .as_ref()
            .and_then(|session| session.handle.clone());

        // Some hosts repeatedly send the same size until the editor window settles.
        // Re-applying identical bounds does not violate the contract but adds redundant
        // geometry processing to the child view, making resize drags feel laggy. Size is
        // still recorded below so re-entrant `request_resize()` detection can observe
        // host callbacks.
        if let Some(handle) = handle {
            if size_changed {
                handle.set_size(size)?;
            }
            if self.should_async_resync_bounds_after_set_size() {
                // Cubase 10 on macOS can resize the host-owned editor window after
                // delivering `set_size`, leaving the embedded child view one geometry
                // step behind. Re-posting the latest accepted size lets the host finish
                // its adjustment before wxp reapplies child bounds.
                log::debug!(
                    "wxp controller: scheduling Cubase VST3 async bounds resync: width={}, height={}",
                    size.width,
                    size.height
                );
                handle.post_set_size(size)?;
            }
        }
        self.layout.store_accepted_size(size);
        Ok(())
    }

    /// Whether `get_size` must fail until the parent window is attached.
    pub(super) fn defers_size_report_until_attached(&self) -> bool {
        // Ableton Live on Windows queries the VST3 view size before it attaches the view and
        // reports the content scale, then resizes the attached view to that stale size.
        // Failing the early query makes Live ask again once the scaled size is known.
        self.layout.scales_with_host_dpi()
            && self.host_context.plugin_format == PluginFormat::Vst3
            && self.host_context.host().family == HostFamily::AbletonLive
    }
}

/// Hosts that resize the editor window on a VST3 `resizeView` request without confirming
/// the new frame through `onSize`, whatever the request returns.
///
/// Only consulted on Windows, where the controller requests frames on its own.
fn host_resizes_without_confirmation(host_context: &HostContext) -> bool {
    host_context.plugin_format == PluginFormat::Vst3
        && matches!(
            host_context.host().family,
            HostFamily::AbletonLive | HostFamily::BitwigStudio | HostFamily::SteinbergWavelab
        )
}
