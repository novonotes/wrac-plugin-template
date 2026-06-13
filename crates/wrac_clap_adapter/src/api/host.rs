use crate::{GuiSize, PluginResult};

/// Requests the host to schedule CLAP params synchronization.
///
/// This maps directly to `clap_host_params.request_flush`. It does not carry
/// parameter values; plugins emit those as output events from `process` or
/// `flush_params`.
pub trait HostParams: Send + Sync {
    /// Calls CLAP `host_params.rescan`. `[main-thread]`
    fn rescan(&self, _flags: u32) {}

    /// Calls CLAP `host_params.clear`. `[main-thread]`
    fn clear(&self, _param_id: u32, _flags: u32) {}

    /// Calls CLAP `host_params.request_flush`. `[thread-safe & control-thread]`
    ///
    /// CLAP marks this callback `!audio-thread`; do not call it from realtime code.
    fn request_flush(&self);
}

/// Notifies the host that non-parameter project state changed and should be saved.
///
/// This maps to CLAP `clap_host_state.mark_dirty()`. Use it for plugin-owned document
/// state, not for parameter automation gestures.
///
/// CLAP requires this notification to be sent from the main thread. The adapter
/// does not marshal calls, so call this from the product's GUI/control path, not
/// directly from `ActiveProcessor::process()` or a background worker.
pub trait HostState: Send + Sync {
    /// Calls CLAP `host_state.mark_dirty`. `[main-thread]`
    fn mark_dirty(&self);
}

/// Requests the host to resize the GUI client area on behalf of the product.
///
/// This trait is `Send + Sync` because it is stored inside the shared plugin context,
/// not because every method is meaningful from every thread. Call `request_resize` only
/// from the product's GUI event path.
pub trait HostGui: Send + Sync {
    /// Calls CLAP `host_gui.resize_hints_changed`. `[main-thread]`
    fn resize_hints_changed(&self) {}

    /// Calls CLAP `host_gui.request_resize`. `[thread-safe & control-thread]`
    ///
    /// Product code should normally call this from its GUI event path.
    fn request_resize(&self, size: GuiSize) -> PluginResult<()>;

    /// Calls CLAP `host_gui.request_show`. `[thread-safe]`
    fn request_show(&self) -> bool {
        false
    }

    /// Calls CLAP `host_gui.request_hide`. `[thread-safe]`
    fn request_hide(&self) -> bool {
        false
    }

    /// Calls CLAP `host_gui.closed`. `[main-thread]`
    fn closed(&self, _was_destroyed: bool) {}
}
