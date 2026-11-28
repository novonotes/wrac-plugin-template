use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;
use wrac_clap_adapter::interface::{GuiResizeHints, GuiSize, HostGui, PluginError, PluginResult};
use wxp::{WebViewDispatch, dpi::LogicalSize};

use crate::dpi::{DpiConverter, GuiDpiPolicy};

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
    // Both frontend resize commands and the controller's attached-scale resize go through
    // `request_host_resize`, so the host handle lives with the layout it negotiates.
    host_gui: Arc<dyn HostGui>,
}

#[derive(Clone)]
pub struct WxpGuiResizeHandle {
    pub(super) layout: Arc<HostGuiLayout>,
}

impl HostGuiLayout {
    pub(super) fn new(
        size: GuiSize,
        limits: GuiSizeLimits,
        resize_policy: GuiResizePolicy,
        host_gui: Arc<dyn HostGui>,
    ) -> Self {
        Self::with_dpi_policy(
            size,
            limits,
            resize_policy,
            GuiDpiPolicy::current(),
            host_gui,
        )
    }

    pub(super) fn with_dpi_policy(
        size: GuiSize,
        limits: GuiSizeLimits,
        resize_policy: GuiResizePolicy,
        dpi_policy: GuiDpiPolicy,
        host_gui: Arc<dyn HostGui>,
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
            host_gui,
        }
    }

    pub(super) fn accepted_size(&self) -> GuiSize {
        self.accepted_size.load()
    }

    pub(super) fn scale(&self) -> f64 {
        self.state.lock().scale
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
    /// Deliberately leaves `accepted_size` and `negotiated_scale` untouched so an attached
    /// host frame remains governed by `set_size` rather than expanding out of its parent.
    pub(super) fn set_scale_while_attached(&self, scale: f64) {
        self.state.lock().scale = scale;
    }

    /// Returns the host size to request when the accepted frame is below the logical minimum
    /// at the active scale, or `None` when the frame fits.
    ///
    /// After an attached scale increase the frame keeps the size negotiated at the previous
    /// scale, and a frame below the scaled minimum clips the WebView layout. Only that case is
    /// resized: a host may already have delivered a frame for the new scale before reporting
    /// it (`set_size` then `set_scale` on a monitor change). `preferred_logical_size` was then
    /// derived with the previous scale, so resizing to it again would scale the editor twice.
    ///
    /// Trade-off: an editor whose preferred size is well above its minimum can therefore open
    /// smaller than preferred (still at or above the minimum, so not clipped) when the host
    /// reports the scale late. Avoiding clipping and double scaling takes priority, because the
    /// late report cannot be told apart from a frame the host already scaled.
    pub(super) fn host_size_to_unclip_at_active_scale(&self) -> Option<GuiSize> {
        if !self.dpi_policy.scales_layout_with_host_dpi() {
            return None;
        }
        let state = self.state.lock();
        let limits = GuiSizeLimits {
            min: self.scale_layout_size(self.limits.min, state.scale),
            max: self.scale_layout_size(self.limits.max, state.scale),
        };
        // Compare against the truncated minimum rather than the rounded one used for clamping.
        // A host that scales the frame itself before reporting the scale may truncate, landing
        // 1px below the rounded minimum; treating that as clipped would scale the editor twice.
        // A frame left at the previous scale is short by far more than 1px, so it still counts.
        let clip_threshold = GuiSize {
            width: (self.limits.min.width as f64 * state.scale).floor() as u32,
            height: (self.limits.min.height as f64 * state.scale).floor() as u32,
        };
        let accepted = self.accepted_size();
        if accepted.width >= clip_threshold.width && accepted.height >= clip_threshold.height {
            return None;
        }
        let dpi = DpiConverter::with_policy(state.scale, self.dpi_policy);
        Some(clamp_size_with_limits(
            dpi.logical_size_to_gui(state.preferred_logical_size),
            limits,
        ))
    }

    /// Asks the host for `size` and returns the host size that is in effect afterwards.
    ///
    /// Logic's AUv2 wrapper applies the NSView frame inside `request_resize()`, calls
    /// `set_size()` re-entrantly, and then returns false to CLAP. That re-entrant `set_size()`
    /// is the ground truth; optimistically resizing the WebView here would race geometry with
    /// the host and cause visual jitter during grip dragging. Some hosts accept the request but
    /// never call `set_size()`; then `apply` mirrors `size` to the WebView and it is recorded
    /// as accepted. A genuine rejection keeps the last host-confirmed size.
    ///
    /// `[main-thread]`: enters the host GUI extension, so call it only from GUI commands or
    /// run-loop tasks, never from inside a CLAP GUI callback.
    pub(super) fn request_host_resize(
        &self,
        size: GuiSize,
        apply: impl FnOnce(GuiSize) -> PluginResult<()>,
    ) -> PluginResult<GuiSize> {
        let previous_revision = self.accepted_size_revision();
        let resize_result = self.host_gui.request_resize(size);
        if self.accepted_size_revision() != previous_revision {
            return Ok(self.accepted_size());
        }
        resize_result?;
        apply(size)?;
        self.store_accepted_size(size);
        Ok(size)
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
        if !self.dpi_policy.scales_layout_with_host_dpi() {
            return clamp_size_with_limits(size, self.limits);
        }
        let (scale, negotiated_scale) = {
            let state = self.state.lock();
            (state.scale, state.negotiated_scale)
        };
        let limits = GuiSizeLimits {
            min: self.scale_layout_size(self.limits.min, negotiated_scale.min(scale)),
            max: self.scale_layout_size(self.limits.max, negotiated_scale.max(scale)),
        };
        clamp_size_with_limits(size, limits)
    }

    fn apply_preferred_scale_locked(&self, state: &mut LayoutState, scale: f64) {
        state.negotiated_scale = scale;
        if !self.dpi_policy.scales_layout_with_host_dpi() {
            return;
        }
        let dpi = DpiConverter::with_policy(scale, self.dpi_policy);
        let scaled = dpi.logical_size_to_gui(state.preferred_logical_size);
        let limits = GuiSizeLimits {
            min: self.scale_layout_size(self.limits.min, scale),
            max: self.scale_layout_size(self.limits.max, scale),
        };
        self.accepted_size
            .store(clamp_size_with_limits(scaled, limits));
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
            let dpi = DpiConverter::with_policy(scale, self.dpi_policy);
            state.preferred_logical_size = dpi.gui_size_to_logical(size);
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

    fn accepted_size_revision(&self) -> u64 {
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
    ///
    /// `host_gui` is the plugin instance's host GUI extension. Resize requests from frontend
    /// commands and from attached scale changes are sent through it.
    pub fn new(initial_size: GuiSize, limits: GuiSizeLimits, host_gui: Arc<dyn HostGui>) -> Self {
        Self {
            layout: Arc::new(HostGuiLayout::new(
                initial_size,
                limits,
                GuiResizePolicy::RESIZABLE,
                host_gui,
            )),
        }
    }

    #[cfg(test)]
    pub(super) fn with_dpi_policy(
        initial_size: GuiSize,
        limits: GuiSizeLimits,
        dpi_policy: GuiDpiPolicy,
        host_gui: Arc<dyn HostGui>,
    ) -> Self {
        Self {
            layout: Arc::new(HostGuiLayout::with_dpi_policy(
                initial_size,
                limits,
                GuiResizePolicy::RESIZABLE,
                dpi_policy,
                host_gui,
            )),
        }
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

        // Pass `WebViewDispatch` rather than the native owner so the command handler can
        // resize without extending the lifetime of a closing editor.
        let accepted = self.layout.request_host_resize(gui_size, |_| {
            web_view
                .post_set_bounds(dpi.create_webview_bounds(logical_size))
                .map_err(|_| PluginError::Message("failed to resize webview".into()))
        })?;
        if accepted.width == gui_size.width && accepted.height == gui_size.height {
            // Return the unrounded logical size the frontend asked for.
            return Ok(logical_size);
        }
        Ok(dpi.gui_size_to_logical(accepted))
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
