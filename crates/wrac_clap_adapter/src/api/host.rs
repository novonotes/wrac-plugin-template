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
    /// Calls CLAP `host_params.request_flush`. `[thread-safe & control-thread]`
    ///
    /// CLAP marks this callback `!audio-thread`; do not call it from realtime code.
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
pub trait HostLifecycle: Send + Sync {
    /// Calls CLAP `host.request_restart`. `[thread-safe & control-thread]`
    fn request_restart(&self);

    /// Calls CLAP `host.request_process`. `[thread-safe]`
    fn request_process(&self);

    /// Calls CLAP `host.request_callback`. `[thread-safe]`
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
/// not because every method is meaningful from every thread. Call `request_resize` only
/// from the product's GUI event path.
///
/// `resize_hints_changed` and `closed` currently call CLAP `[main-thread]` host
/// callbacks directly. The adapter does not marshal them yet, so call those methods
/// only from a context that is already on the host main thread.
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
