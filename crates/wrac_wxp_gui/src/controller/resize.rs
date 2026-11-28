use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;
use wrac_clap_adapter::interface::{GuiResizeHints, GuiSize, HostGui, PluginError, PluginResult};
use wxp::{WebViewDispatch, dpi::LogicalSize};

use crate::dpi::{DpiConverter, GuiDpiPolicy};
use wrac_host_context::{HostContext, HostFamily, PluginFormat};

use super::GuiSizeLimits;

struct LayoutState {
    // Active host (or Cubase-corrected) DPI scale shared across CLAP callbacks and resize
    // commands.
    scale: f64,
    // Preferred editor size in 100% logical pixels. Storing the logical baseline instead of
    // rescaling rounded physical integers avoids rounding drift across DPI changes and keeps
    // reopen sizing accurate even when an attached host updates scale without sending set_size.
    preferred_logical_size: LogicalSize<f64>,
    // Scale last negotiated before parent attachment (or confirmed by a subsequent host
    // resize). While attached, a host may report a newer scale without enlarging its parent
    // frame; keeping the minimum clamp anchored to `negotiated_scale.min(scale)` prevents
    // `adjust_size` / `set_size` from expanding the child WebView past the host frame.
    negotiated_scale: f64,
}

pub(super) struct HostGuiLayout {
    // Host-contract size value read by CLAP layout queries without entering the GUI runtime
    // (not a copy of the runtime state).
    accepted_size: AtomicGuiSize,
    state: Mutex<LayoutState>,
    // Some wrappers call `set_size()` re-entrantly from within `request_resize()` (even
    // when the return value is false). This revision counter lets the request side detect
    // "the size the host confirmed" without holding the runtime lock or guessing the return value.
    accepted_size_revision: AtomicU64,
    limits: GuiSizeLimits,
    resize_policy: GuiResizePolicy,
    dpi_policy: GuiDpiPolicy,
}
/// Shared handle to the editor's size negotiation with the host.
///
/// Owns everything a resize request needs: the layout, the host GUI extension, and the host
/// identity that selects host-specific workarounds. The controller and the frontend resize
/// commands both go through this handle so the two cannot disagree on how a host is treated.
#[derive(Clone)]
pub struct WxpGuiResizeHandle {
    pub(super) layout: Arc<HostGuiLayout>,
    pub(super) host_context: HostContext,
    host_gui: Arc<dyn HostGui>,
}

/// How the host answered a frame request.
pub(super) enum HostFrameAnswer {
    /// The host delivered a frame through a re-entrant `set_size`, which is the ground
    /// truth even when it differs from the request. Nothing is left to apply.
    Delivered,
    /// The host took the request without calling `set_size`, so the requester applies the
    /// requested frame itself.
    Granted,
    /// The host keeps its current frame.
    Kept(PluginError),
}

impl HostGuiLayout {
    pub(super) fn new(
        size: GuiSize,
        limits: GuiSizeLimits,
        resize_policy: GuiResizePolicy,
    ) -> Self {
        Self::with_dpi_policy(size, limits, resize_policy, GuiDpiPolicy::current())
    }

    pub(super) fn with_dpi_policy(
        size: GuiSize,
        limits: GuiSizeLimits,
        resize_policy: GuiResizePolicy,
        dpi_policy: GuiDpiPolicy,
    ) -> Self {
        let size = clamp_size_with_limits(size, limits);
        Self {
            accepted_size: AtomicGuiSize::new(size),
            state: Mutex::new(LayoutState {
                scale: 1.0,
                preferred_logical_size: LogicalSize::new(size.width as f64, size.height as f64),
                negotiated_scale: 1.0,
            }),
            accepted_size_revision: AtomicU64::new(0),
            limits,
            resize_policy,
            dpi_policy,
        }
    }

    pub(super) fn accepted_size(&self) -> GuiSize {
        self.accepted_size.load()
    }

    pub(super) fn scale(&self) -> f64 {
        self.state.lock().scale
    }

    /// Whether host frames are derived from a logical baseline multiplied by the host scale.
    pub(super) fn scales_with_host_dpi(&self) -> bool {
        self.dpi_policy.scales_layout_with_host_dpi()
    }

    /// Updates the active scale before a parent window is attached and re-derives
    /// `accepted_size` from the 100% logical baseline so `get_size` returns scaled host
    /// pixels on Windows.
    pub(super) fn set_scale_before_attach(&self, scale: f64) {
        let mut state = self.state.lock();
        state.scale = scale;
        self.apply_preferred_scale_locked(&mut state, scale);
    }

    /// Updates the active scale while a parent window is already attached.
    ///
    /// Deliberately leaves `accepted_size` and `negotiated_scale` untouched: the host owns
    /// the attached frame, so the controller asks it for [`Self::preferred_host_size`] and
    /// only adopts the new frame once the host delivers or accepts it.
    pub(super) fn set_scale_while_attached(&self, scale: f64) {
        self.state.lock().scale = scale;
    }

    /// Returns the host frame that shows the preferred logical size at the active scale.
    ///
    /// `None` on platforms whose layout does not scale with host DPI: there the accepted
    /// size never goes stale when the scale changes, so there is nothing to ask the host for.
    pub(super) fn preferred_host_size(&self) -> Option<GuiSize> {
        if !self.dpi_policy.scales_layout_with_host_dpi() {
            return None;
        }
        let state = self.state.lock();
        Some(self.preferred_host_size_locked(&state, state.scale))
    }

    fn preferred_host_size_locked(&self, state: &LayoutState, scale: f64) -> GuiSize {
        let dpi = DpiConverter::with_policy(scale, self.dpi_policy);
        let scaled = dpi.logical_size_to_gui(state.preferred_logical_size);
        let limits = GuiSizeLimits {
            min: self.scale_layout_size(self.limits.min, scale),
            max: self.scale_layout_size(self.limits.max, scale),
        };
        clamp_size_with_limits(scaled, limits)
    }

    /// Re-syncs `accepted_size` and `negotiated_scale` to the latest scale when starting a
    /// new GUI session, so reopening after an attached scale change starts at the expected
    /// size even if the host skips `set_scale` before `get_size`.
    pub(super) fn prepare_for_new_session(&self) {
        let mut state = self.state.lock();
        let scale = state.scale;
        self.apply_preferred_scale_locked(&mut state, scale);
    }

    /// Clamps a host-supplied size (from `adjust_size` or `set_size`) to the active bounds.
    ///
    /// On Windows, the minimum bound uses `negotiated_scale.min(scale)` and the maximum
    /// bound uses `negotiated_scale.max(scale)` so late scale updates on an unresized parent
    /// frame do not clip the child WebView, while hosts that enlarge their frame to the new
    /// scale are accepted up to the scaled maximum.
    pub(super) fn clamp_host_size(&self, size: GuiSize) -> GuiSize {
        let scale = self.scale();
        self.clamp_host_size_allowing_scale(size, scale)
    }

    /// Like [`Self::clamp_host_size`], but also admits frames that are valid at
    /// `candidate_scale`.
    ///
    /// A host that validates a frame through `adjust_size` before delivering it would
    /// otherwise have a frame sized for a not-yet-adopted scale clamped back to the active
    /// one, and the delivered frame would no longer reveal the scale change.
    pub(super) fn clamp_host_size_allowing_scale(
        &self,
        size: GuiSize,
        candidate_scale: f64,
    ) -> GuiSize {
        if !self.dpi_policy.scales_layout_with_host_dpi() {
            return clamp_size_with_limits(size, self.limits);
        }
        let (scale, negotiated_scale) = {
            let state = self.state.lock();
            (state.scale, state.negotiated_scale)
        };
        let smallest_scale = negotiated_scale.min(scale).min(candidate_scale);
        let largest_scale = negotiated_scale.max(scale).max(candidate_scale);
        let limits = GuiSizeLimits {
            min: self.scale_layout_size(self.limits.min, smallest_scale),
            max: self.scale_layout_size(self.limits.max, largest_scale),
        };
        clamp_size_with_limits(size, limits)
    }

    fn apply_preferred_scale_locked(&self, state: &mut LayoutState, scale: f64) {
        state.negotiated_scale = scale;
        if !self.dpi_policy.scales_layout_with_host_dpi() {
            return;
        }
        self.accepted_size
            .store(self.preferred_host_size_locked(state, scale));
    }

    fn scale_layout_size(&self, size: GuiSize, scale: f64) -> GuiSize {
        let dpi = DpiConverter::with_policy(scale, self.dpi_policy);
        dpi.logical_size_to_gui(LogicalSize::new(size.width as f64, size.height as f64))
    }

    pub(super) fn clamp_logical_size(&self, size: LogicalSize<f64>) -> LogicalSize<f64> {
        let scale = self.scale();
        let dpi = DpiConverter::with_policy(scale, self.dpi_policy);
        // Resize commands receive frontend logical pixels. Convert through the host
        // boundary unit before clamping so limits remain comparable to host callbacks.
        let host_size = dpi.logical_size_to_gui(size);
        let limits = if self.dpi_policy.scales_layout_with_host_dpi() {
            GuiSizeLimits {
                min: self.scale_layout_size(self.limits.min, scale),
                max: self.scale_layout_size(self.limits.max, scale),
            }
        } else {
            self.limits
        };
        let clamped = clamp_size_with_limits(host_size, limits);
        dpi.gui_size_to_logical(clamped)
    }

    pub(super) fn store_accepted_size(&self, size: GuiSize) {
        let previous = self.accepted_size.load();
        let size_changed = previous.width != size.width || previous.height != size.height;
        if size_changed && self.dpi_policy.scales_layout_with_host_dpi() {
            let mut state = self.state.lock();
            let scale = state.scale;
            // A host that grants the frame requested after a scale change echoes the rounded
            // host pixels of the current baseline. Re-deriving the baseline from those
            // integers would accumulate rounding drift across scale changes, so only a frame
            // that differs from the baseline replaces it.
            let baseline_frame = self.preferred_host_size_locked(&state, scale);
            if baseline_frame.width != size.width || baseline_frame.height != size.height {
                let dpi = DpiConverter::with_policy(scale, self.dpi_policy);
                state.preferred_logical_size = dpi.gui_size_to_logical(size);
            }
            // Once the host actually delivers a frame at least as large as the new scale's
            // minimum, promote `negotiated_scale` so subsequent resizes enforce the scaled
            // minimum as well.
            let min_at_scale = self.scale_layout_size(self.limits.min, scale);
            if size.width >= min_at_scale.width && size.height >= min_at_scale.height {
                state.negotiated_scale = scale;
            }
        }
        self.accepted_size.store(size);
        self.accepted_size_revision.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn accepted_size_revision(&self) -> u64 {
        self.accepted_size_revision.load(Ordering::Relaxed)
    }

    pub(super) fn can_resize(&self) -> bool {
        self.resize_policy.can_resize()
    }

    pub(super) fn resize_hints(&self) -> GuiResizeHints {
        self.resize_policy.resize_hints()
    }
}

impl WxpGuiResizeHandle {
    /// Creates a resize handle from the preferred editor size and resize limits.
    ///
    /// On Windows, sizes are interpreted as 100% logical pixels and scaled into host pixels.
    /// On macOS, sizes are Cocoa logical points. On Linux, sizes remain in host units.
    pub fn new(
        initial_size: GuiSize,
        limits: GuiSizeLimits,
        host_context: HostContext,
        host_gui: Arc<dyn HostGui>,
    ) -> Self {
        Self {
            layout: Arc::new(HostGuiLayout::new(
                initial_size,
                limits,
                GuiResizePolicy::RESIZABLE,
            )),
            host_context,
            host_gui,
        }
    }

    #[cfg(test)]
    pub(super) fn with_dpi_policy(
        initial_size: GuiSize,
        limits: GuiSizeLimits,
        dpi_policy: GuiDpiPolicy,
        host_context: HostContext,
        host_gui: Arc<dyn HostGui>,
    ) -> Self {
        Self {
            layout: Arc::new(HostGuiLayout::with_dpi_policy(
                initial_size,
                limits,
                GuiResizePolicy::RESIZABLE,
                dpi_policy,
            )),
            host_context,
            host_gui,
        }
    }

    /// Asks the host for `size` and reports how it answered.
    ///
    /// Must be called on the main thread while the editor is attached, with no controller
    /// lock held: hosts may call `set_size` re-entrantly from within the request.
    pub(super) fn request_host_frame(&self, size: GuiSize) -> HostFrameAnswer {
        let previous_revision = self.layout.accepted_size_revision();
        let result = self.host_gui.request_resize(size);

        // Logic's AUv2 wrapper applies the NSView frame inside `request_resize()`, calls
        // `set_size()` re-entrantly, and then returns false to CLAP. Treat that re-entrant
        // `set_size()` as the ground truth. Optimistically resizing the WebView here would
        // race geometry with the host and cause visual jitter during grip dragging.
        if self.layout.accepted_size_revision() != previous_revision {
            return HostFrameAnswer::Delivered;
        }
        match result {
            // Some hosts accept the request but never call `set_size()`.
            Ok(()) => HostFrameAnswer::Granted,
            Err(error) if self.host_resizes_without_confirmation() => {
                log::debug!(
                    "wxp resize: treating an unconfirmed frame request as granted: {error}"
                );
                HostFrameAnswer::Granted
            }
            Err(error) => HostFrameAnswer::Kept(error),
        }
    }

    /// Whether the host resizes the editor window on a VST3 `resizeView` request without
    /// confirming the new frame through `onSize`, whatever the request returns.
    fn host_resizes_without_confirmation(&self) -> bool {
        // These hosts are only known to behave this way on Windows. The request cannot
        // tell a rejection from a missing host frame, which is why requests are limited to
        // an attached editor (see `request_host_frame`): the frame exists by then.
        self.layout.scales_with_host_dpi()
            && self.host_context.plugin_format == PluginFormat::Vst3
            && matches!(
                self.host_context.host().family,
                HostFamily::AbletonLive | HostFamily::BitwigStudio | HostFamily::SteinbergWavelab
            )
    }

    /// Requests a host-approved resize from the GUI event path and mirrors accepted bounds to wxp.
    ///
    /// `WxpGuiResizeHandle` is `Send + Sync` so command registration can share it, but this method
    /// enters the host GUI resize extension and must only be called from GUI commands/events.
    pub fn request_resize(
        &self,
        requested: LogicalSize<f64>,
        web_view: &WebViewDispatch,
    ) -> PluginResult<LogicalSize<f64>> {
        let scale = self.layout.scale();
        let logical_size = self.layout.clamp_logical_size(requested);
        let dpi = DpiConverter::with_policy(scale, self.layout.dpi_policy);
        let gui_size = dpi.logical_size_to_gui(logical_size);

        match self.request_host_frame(gui_size) {
            HostFrameAnswer::Delivered => Ok(dpi.gui_size_to_logical(self.layout.accepted_size())),
            HostFrameAnswer::Granted => {
                // Update the WebView directly without waiting for an async callback.
                // Pass `WebViewDispatch` rather than the native owner so the command handler
                // can resize without extending the lifetime of a closing editor.
                web_view
                    .post_set_bounds(dpi.create_webview_bounds(logical_size))
                    .map_err(|_| PluginError::Message("failed to resize webview".into()))?;
                self.layout.store_accepted_size(gui_size);
                Ok(logical_size)
            }
            // Rather than speculatively moving the child WebView and rolling it back, keep
            // the last host-confirmed size.
            HostFrameAnswer::Kept(error) => Err(error),
        }
    }
}

struct AtomicGuiSize(AtomicU64);

impl AtomicGuiSize {
    fn new(size: GuiSize) -> Self {
        Self(AtomicU64::new(pack_size(size)))
    }

    fn load(&self) -> GuiSize {
        unpack_size(self.0.load(Ordering::Relaxed))
    }

    fn store(&self, size: GuiSize) {
        self.0.store(pack_size(size), Ordering::Relaxed);
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct GuiResizePolicy {
    can_resize: bool,
}

impl GuiResizePolicy {
    pub(super) const RESIZABLE: Self = Self { can_resize: true };

    fn can_resize(self) -> bool {
        self.can_resize
    }

    fn resize_hints(self) -> GuiResizeHints {
        GuiResizeHints {
            can_resize_horizontally: self.can_resize,
            can_resize_vertically: self.can_resize,
            preserve_aspect_ratio: false,
            aspect_ratio_width: 0,
            aspect_ratio_height: 0,
        }
    }
}

fn pack_size(size: GuiSize) -> u64 {
    ((size.width as u64) << 32) | size.height as u64
}

fn unpack_size(size: u64) -> GuiSize {
    GuiSize {
        width: (size >> 32) as u32,
        height: size as u32,
    }
}

fn clamp_size_with_limits(size: GuiSize, limits: GuiSizeLimits) -> GuiSize {
    GuiSize {
        width: size.width.clamp(limits.min.width, limits.max.width),
        height: size.height.clamp(limits.min.height, limits.max.height),
    }
}
