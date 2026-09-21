//! Host callback proxies available to product code.
//!
//! Method annotations describe how product code may call the same `Host*` object.
//!
//! - `[main-thread]`: avoid calls from any thread other than the main thread.
//! - `[non-realtime]`: avoid calls from realtime paths and avoid concurrent calls to the same
//!   object.
//! - `[realtime-safe]`: calls from realtime paths are allowed, but avoid concurrent calls to the
//!   same object.
//! - `[non-realtime & thread-safe]`: concurrent calls from non-realtime threads are allowed;
//!   avoid calls from realtime paths.
//! - `[realtime-safe & thread-safe]`: calls from realtime paths and concurrent calls are allowed.

use crate::interface::{GuiSize, NoteDialects, PluginResult};

/// Requests host-side parameter synchronization and invalidation.
///
/// `request_flush` maps directly to `clap_host_params.request_flush` and does not
/// carry parameter values; plugins emit those as output events from `process` or
/// `flush_params`.
pub trait HostParams: Send + Sync {
    /// Calls CLAP `host_params.request_flush`. `[non-realtime & thread-safe]`
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
    /// Calls CLAP `host.request_restart`. `[non-realtime & thread-safe]`
    fn request_restart(&self);

    /// Calls CLAP `host.request_process`. `[non-realtime & thread-safe]`
    fn request_process(&self);

    /// Calls CLAP `host.request_callback`. `[non-realtime & thread-safe]`
    ///
    /// The host is expected to schedule a later `PluginInstance::on_main_thread` call.
    fn request_callback(&self);
}

/// Host tail notification object owned by an active processor.
///
/// This object is moved into the active processor at activation time and intentionally
/// does not require `Sync`.
pub trait HostTail: Send + 'static {
    /// Calls CLAP `host_tail.changed`. `[realtime-safe]`
    fn changed(&mut self);
}

/// Requests the host to resize the GUI client area on behalf of the product.
pub trait HostGui: Send + Sync {
    /// Calls CLAP `host_gui.resize_hints_changed`. `[main-thread]`
    fn resize_hints_changed(&self) {}

    /// Calls CLAP `host_gui.request_resize`. `[main-thread]`
    fn request_resize(&self, size: GuiSize) -> PluginResult<()>;

    /// Calls CLAP `host_gui.request_show`. `[main-thread]`
    fn request_show(&self) -> bool {
        false
    }

    /// Calls CLAP `host_gui.request_hide`. `[main-thread]`
    fn request_hide(&self) -> bool {
        false
    }

    /// Calls CLAP `host_gui.closed`. `[main-thread]`
    fn closed(&self, _was_destroyed: bool) {}
}
