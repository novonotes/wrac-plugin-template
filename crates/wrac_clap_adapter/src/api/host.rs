use crate::{GuiSize, PluginResult};

/// Notifies the host automation lane of a parameter edit triggered by the GUI or other
/// product-side action.
///
/// This is not an API to update the source of truth. The product updates its own store
/// first, then calls this to report the edit back to the host
/// (begin -> update -> end forms one undo unit).
///
/// `Send + Sync` allows GUI or control callbacks to share the notifier. It is not
/// realtime-safe; do not call it from `Processor::process()`.
pub trait HostParamsEditNotifier: Send + Sync {
    /// Queues a CLAP `PARAM_GESTURE_BEGIN` event and requests `host_params.request_flush`.
    /// `[thread-safe & control-thread]`
    fn begin_edit(&self, param_id: u32);

    /// Queues a CLAP `PARAM_VALUE` event and requests `host_params.request_flush`.
    /// `[thread-safe & control-thread]`
    fn update_edit(&self, param_id: u32, value: f64);

    /// Queues a CLAP `PARAM_GESTURE_END` event and requests `host_params.request_flush`.
    /// `[thread-safe & control-thread]`
    fn end_edit(&self, param_id: u32);
}

/// Notifies the host that non-parameter project state changed and should be saved.
///
/// This maps to CLAP `clap_host_state.mark_dirty()`. Use it for plugin-owned document
/// state, not for parameter automation gestures.
///
/// CLAP requires this notification to be sent from the main thread. The adapter
/// does not marshal calls, so call this from the product's GUI/control path, not
/// directly from `Processor::process()` or a background worker.
pub trait HostStateDirtyNotifier: Send + Sync {
    /// Calls CLAP `host_state.mark_dirty`. `[main-thread]`
    fn mark_dirty(&self);
}

/// Requests the host to resize the GUI client area on behalf of the product.
///
/// This trait is `Send + Sync` because it is stored inside the shared plugin context,
/// not because every method is meaningful from every thread. Call `request_resize` only
/// from the product's GUI event path.
pub trait HostGuiResizeRequester: Send + Sync {
    /// Calls CLAP `host_gui.request_resize`. `[thread-safe & control-thread]`
    ///
    /// Product code should normally call this from its GUI event path.
    fn request_resize(&self, size: GuiSize) -> PluginResult<()>;
}
