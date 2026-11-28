use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

use parking_lot::Mutex;
use wrac_clap_adapter::interface::{GuiResizeHints, GuiSize, HostGui, PluginError, PluginResult};
use wxp::{WebViewDispatch, dpi::LogicalSize};

use crate::dpi::{DpiConverter, HostGuiSizeUnit};

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
    host_size_unit: AtomicU8,
    // Only Windows WebView2 automatically matches its CSS viewport scale to the host's
    // per-monitor DPI. Linux WebKitGTK renders at the GDK integer scale regardless of the
    // CLAP host scale, so scaling preferred sizes and limits by host scale stays Windows-only
    // until Linux zoom synchronization is implemented and verified on X11.
    scales_with_host_dpi: AtomicBool,
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
            host_size_unit: AtomicU8::new(HostGuiSizeUnit::PhysicalPixels.to_u8()),
            scales_with_host_dpi: AtomicBool::new(cfg!(windows)),
        }
    }

    pub(super) fn accepted_size(&self) -> GuiSize {
        self.accepted_size.load()
    }

    pub(super) fn scale(&self) -> f64 {
        self.state.lock().scale
    }

    pub(super) fn set_scale(&self, scale: f64, is_unattached: bool) {
        let mut state = self.state.lock();
        state.scale = scale;
        if is_unattached {
            self.apply_preferred_scale_locked(&mut state, scale);
        }
    }

    pub(super) fn prepare_for_new_session(&self) {
        let mut state = self.state.lock();
        let scale = state.scale;
        self.apply_preferred_scale_locked(&mut state, scale);
    }

    pub(super) fn clamp_host_size(&self, size: GuiSize) -> GuiSize {
        if !self.uses_host_dpi_layout_scaling() {
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
        if !self.uses_host_dpi_layout_scaling() {
            return;
        }
        let dpi = DpiConverter::with_host_size_unit(scale, self.host_size_unit());
        let scaled = dpi.logical_size_to_gui(state.preferred_logical_size);
        let limits = GuiSizeLimits {
            min: self.scale_layout_size(self.limits.min, scale),
            max: self.scale_layout_size(self.limits.max, scale),
        };
        self.accepted_size
            .store(clamp_size_with_limits(scaled, limits));
    }

    fn scale_layout_size(&self, size: GuiSize, scale: f64) -> GuiSize {
        let dpi = DpiConverter::with_host_size_unit(scale, self.host_size_unit());
        dpi.logical_size_to_gui(LogicalSize::new(size.width as f64, size.height as f64))
    }

    fn uses_host_dpi_layout_scaling(&self) -> bool {
        self.scales_with_host_dpi.load(Ordering::Relaxed)
            && self.host_size_unit() == HostGuiSizeUnit::PhysicalPixels
    }

    pub(super) fn clamp_logical_size(&self, size: LogicalSize<f64>) -> LogicalSize<f64> {
        let scale = self.scale();
        let dpi = DpiConverter::with_host_size_unit(scale, self.host_size_unit());
        // Resize commands receive frontend logical pixels. Convert through the host
        // boundary unit before clamping so limits remain comparable to host callbacks.
        let host_size = dpi.logical_size_to_gui(size);
        let limits = if self.uses_host_dpi_layout_scaling() {
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

    pub(super) fn set_host_size_unit(&self, unit: HostGuiSizeUnit) {
        self.host_size_unit.store(unit.to_u8(), Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(super) fn set_scales_with_host_dpi(&self, enabled: bool) {
        self.scales_with_host_dpi.store(enabled, Ordering::Relaxed);
    }

    pub(super) fn host_size_unit(&self) -> HostGuiSizeUnit {
        HostGuiSizeUnit::from_u8(self.host_size_unit.load(Ordering::Relaxed))
    }

    pub(super) fn store_accepted_size(&self, size: GuiSize) {
        let previous = self.accepted_size.load();
        let size_changed = previous.width != size.width || previous.height != size.height;
        if size_changed && self.uses_host_dpi_layout_scaling() {
            let mut state = self.state.lock();
            let scale = state.scale;
            let dpi = DpiConverter::with_host_size_unit(scale, self.host_size_unit());
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
    pub fn new(initial_size: GuiSize, limits: GuiSizeLimits) -> Self {
        Self {
            layout: Arc::new(HostGuiLayout::new(
                initial_size,
                limits,
                GuiResizePolicy::RESIZABLE,
            )),
        }
    }

    pub fn host_size_unit(&self) -> HostGuiSizeUnit {
        self.layout.host_size_unit()
    }

    pub(super) fn set_host_size_unit(&self, unit: HostGuiSizeUnit) {
        self.layout.set_host_size_unit(unit);
    }

    /// Requests a host-approved resize from the GUI event path and mirrors accepted bounds to wxp.
    ///
    /// `WxpGuiResizeHandle` is `Send + Sync` so command registration can share it, but this method
    /// enters the host GUI resize extension and must only be called from GUI commands/events.
    pub fn request_resize(
        &self,
        requested: LogicalSize<f64>,
        web_view: &WebViewDispatch,
        host_gui: &dyn HostGui,
    ) -> PluginResult<LogicalSize<f64>> {
        // `HostGui` can be shared from Send/Sync product state, but the target
        // API is a host GUI extension. Keep the "GUI command only" threading contract at the
        // command registration boundary rather than making this a generic background-thread API.
        let scale = self.layout.scale();
        let logical_size = self.layout.clamp_logical_size(requested);
        let dpi = DpiConverter::with_host_size_unit(scale, self.layout.host_size_unit());
        let gui_size = dpi.logical_size_to_gui(logical_size);

        let previous_revision = self.layout.accepted_size_revision();
        let resize_result = host_gui.request_resize(gui_size);
        let current_revision = self.layout.accepted_size_revision();

        // Logic's AUv2 wrapper applies the NSView frame inside `request_resize()`, calls
        // `set_size()` re-entrantly, and then returns false to CLAP. Treat that re-entrant
        // `set_size()` as the ground truth. Optimistically resizing the WebView here would
        // race geometry with the host and cause visual jitter during grip dragging.
        if current_revision != previous_revision {
            return Ok(dpi.gui_size_to_logical(self.layout.accepted_size()));
        }

        match resize_result {
            Ok(()) => {
                // Some hosts accept the request but never call `set_size()`. In that case,
                // update the WebView directly without waiting for an async callback.
                // Pass `WebViewDispatch` rather than the native owner so the command handler
                // can resize without extending the lifetime of a closing editor.
                web_view
                    .post_set_bounds(dpi.create_webview_bounds(logical_size))
                    .map_err(|_| PluginError::Message("failed to resize webview".into()))?;
                self.layout.store_accepted_size(gui_size);
                Ok(logical_size)
            }
            Err(error) => {
                // A genuine rejection is distinct from the AUv2 re-entry case above. Rather
                // than speculatively moving the child WebView and rolling it back, keep the
                // last host-confirmed size.
                Err(error)
            }
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
