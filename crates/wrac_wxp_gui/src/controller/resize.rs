use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

use parking_lot::Mutex;
use wrac_clap_adapter::{GuiResizeHints, GuiSize, HostGui, PluginError, PluginResult};
use wxp::{WebViewDispatch, dpi::LogicalSize};

use crate::dpi::{DpiConverter, HostGuiSizeUnit};

use super::GuiSizeLimits;

pub(super) struct HostGuiLayout {
    // Host-contract size value read by CLAP layout queries without entering the GUI runtime
    // (not a copy of the runtime state).
    accepted_size: AtomicGuiSize,
    // Some wrappers call `set_size()` re-entrantly from within `request_resize()` (even
    // when the return value is false). This revision counter lets the request side detect
    // "the size the host confirmed" without holding the runtime lock or guessing the return value.
    accepted_size_revision: AtomicU64,
    limits: GuiSizeLimits,
    resize_policy: GuiResizePolicy,
    host_size_unit: AtomicU8,
}

#[derive(Clone)]
pub struct WxpGuiResizeHandle {
    pub(super) layout: Arc<HostGuiLayout>,
    pub(super) scale: Arc<Mutex<f64>>,
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
            accepted_size_revision: AtomicU64::new(0),
            limits,
            resize_policy,
            host_size_unit: AtomicU8::new(HostGuiSizeUnit::PhysicalPixels.to_u8()),
        }
    }

    pub(super) fn accepted_size(&self) -> GuiSize {
        self.accepted_size.load()
    }

    pub(super) fn clamp_size(&self, size: GuiSize) -> GuiSize {
        clamp_size_with_limits(size, self.limits)
    }

    pub(super) fn clamp_logical_size(
        &self,
        size: LogicalSize<f64>,
        scale: f64,
    ) -> LogicalSize<f64> {
        let dpi = DpiConverter::with_host_size_unit(scale, self.host_size_unit());
        // Resize commands receive frontend logical pixels. Convert through the host
        // boundary unit before clamping so limits remain comparable to host callbacks.
        let physical = dpi.logical_size_to_gui(size);
        let clamped = clamp_size_with_limits(physical, self.limits);
        dpi.gui_size_to_logical(clamped)
    }

    pub(super) fn set_host_size_unit(&self, unit: HostGuiSizeUnit) {
        self.host_size_unit.store(unit.to_u8(), Ordering::Relaxed);
    }

    pub(super) fn host_size_unit(&self) -> HostGuiSizeUnit {
        HostGuiSizeUnit::from_u8(self.host_size_unit.load(Ordering::Relaxed))
    }

    pub(super) fn store_accepted_size(&self, size: GuiSize) {
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
    pub fn new(initial_size: GuiSize, limits: GuiSizeLimits) -> Self {
        Self {
            layout: Arc::new(HostGuiLayout::new(
                initial_size,
                limits,
                GuiResizePolicy::RESIZABLE,
            )),
            scale: Arc::new(Mutex::new(1.0)),
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
        let scale = *self.scale.lock();
        let logical_size = self.layout.clamp_logical_size(requested, scale);
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
                    .map_err(|_| PluginError::Message("failed to resize webview"))?;
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
