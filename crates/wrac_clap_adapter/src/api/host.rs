use crate::{GuiSize, NoteDialects, PluginResult};

/// Requests host-side parameter synchronization and invalidation.
///
/// `request_flush` maps directly to `clap_host_params.request_flush` and does not
/// carry parameter values; plugins emit those as output events from `process` or
/// `flush_params`.
///
/// `rescan` and `clear` currently call CLAP `[main-thread]` host callbacks directly.
/// The adapter does not marshal them yet, so call those methods only from a context
/// that is already on the host main thread. The product-facing API intentionally avoids
/// `MainThread` naming because the long-term contract is for the adapter to turn these
/// into queued/coalesced host requests.
pub trait HostParams: Send + Sync {
    /// Calls CLAP `host_params.request_flush`. `[non-audio control path]`
    ///
    /// CLAP allows this from any non-audio thread. WRAC keeps the product-facing
    /// contract narrower because wrapper hosts may not implement the native CLAP
    /// threading contract exactly.
    fn request_flush(&self);

    /// Calls CLAP `host_params.rescan`. `[main-thread]`
    fn rescan(&self, _flags: u32) {}

    /// Calls CLAP `host_params.clear`. `[main-thread]`
    fn clear(&self, _param_id: u32, _flags: u32) {}
}

/// Requests host-side project state notifications.
///
/// `mark_dirty` maps to CLAP `clap_host_state.mark_dirty()`. Use it for plugin-owned
/// document state, not for parameter automation gestures.
///
/// This currently calls a CLAP `[main-thread]` host callback directly. The adapter does
/// not marshal it yet, so call it only from a context that is already on the host main
/// thread. The product-facing API intentionally avoids `MainThread` naming because the
/// long-term contract is for the adapter to turn this into a queued/coalesced host
/// request.
pub trait HostState: Send + Sync {
    /// Calls CLAP `host_state.mark_dirty`. `[main-thread]`
    fn mark_dirty(&self);
}

/// Requests host-side audio port metadata invalidation.
pub trait HostAudioPorts: Send + Sync {
    /// Calls CLAP `host_audio_ports.is_rescan_flag_supported`. `[main-thread]`
    fn is_rescan_flag_supported(&self, _flag: u32) -> bool {
        false
    }

    /// Calls CLAP `host_audio_ports.rescan`. `[main-thread]`
    fn rescan(&self, _flags: u32) {}
}

/// Requests host-side note port metadata invalidation.
pub trait HostNotePorts: Send + Sync {
    /// Calls CLAP `host_note_ports.supported_dialects`. `[main-thread]`
    fn supported_dialects(&self) -> NoteDialects {
        NoteDialects::default()
    }

    /// Calls CLAP `host_note_ports.rescan`. `[main-thread]`
    fn rescan(&self, _flags: u32) {}
}

/// Requests CLAP core host actions.
///
/// These calls are forwarded directly to the host. Native CLAP marks them as
/// thread-safe, but product code should prefer non-realtime control paths unless it
/// is transparently forwarding an inner plugin request.
pub trait HostLifecycle: Send + Sync {
    /// Calls CLAP `host.request_restart`. `[non-audio control path]`
    fn request_restart(&self);

    /// Calls CLAP `host.request_process`. `[thread-safe forwarding path]`
    fn request_process(&self);

    /// Calls CLAP `host.request_callback`. `[thread-safe forwarding path]`
    ///
    /// The host is expected to schedule a later `PluginInstance::on_main_thread` call.
    fn request_callback(&self);
}

/// Host tail notification handle owned by an active processor.
///
/// CLAP marks `host_tail.changed` as `[audio-thread]`. This handle is moved into
/// the active processor at activation time and intentionally does not require `Sync`.
pub trait HostTail: Send + 'static {
    /// Calls CLAP `host_tail.changed`. `[audio-thread]`
    fn changed(&mut self);
}

/// Requests the host to resize the GUI client area on behalf of the product.
///
/// This trait is `Send + Sync` because it is stored inside the shared plugin context,
/// not because every method is meaningful from every thread. Call GUI host callbacks
/// only from the product's GUI event or GUI lifecycle path; several wrapper hosts are
/// less permissive than native CLAP here.
///
pub trait HostGui: Send + Sync {
    /// Calls CLAP `host_gui.resize_hints_changed`. `[GUI lifecycle path]`
    fn resize_hints_changed(&self) {}

    /// Calls CLAP `host_gui.request_resize`. `[GUI event path]`
    ///
    /// Product code should call this from its GUI event path even though native
    /// CLAP permits broader use.
    fn request_resize(&self, size: GuiSize) -> PluginResult<()>;

    /// Calls CLAP `host_gui.request_show`. `[GUI event path]`
    fn request_show(&self) -> bool {
        false
    }

    /// Calls CLAP `host_gui.request_hide`. `[GUI event path]`
    fn request_hide(&self) -> bool {
        false
    }

    /// Calls CLAP `host_gui.closed`. `[GUI lifecycle path]`
    fn closed(&self, _was_destroyed: bool) {}
}
