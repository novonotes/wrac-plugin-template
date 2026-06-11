use std::any::Any;

use crate::PluginResult;
use crate::events::{ProcessEvents, TransportEvent};
use crate::process_buffer::AudioProcessBuffer;

/// Processing object used while the CLAP plugin is active.
///
/// State passed in must be either an immutable snapshot copied at activate time, or
/// atomic/lock-free shared state the audio thread never waits on.
pub trait ActiveProcessor: Send {
    /// Consumes the processor for typed recovery during deactivation. `[control-thread]`
    fn into_any(self: Box<Self>) -> Box<dyn Any + Send>;

    /// Called from CLAP `plugin.reset`. `[audio-thread]`
    fn reset(&mut self) {}

    /// Called from CLAP `plugin.process`. `[audio-thread]`
    fn process(&mut self, context: ProcessContext<'_>) -> PluginResult<ProcessStatus>;

    /// Called from CLAP `params.flush` while active. `[audio-thread]`
    ///
    /// This has the same realtime constraints as `process`.
    fn flush_params(&mut self, context: ParamFlushContext<'_>) -> PluginResult<()>;
}

/// Processing state used while the CLAP plugin is inactive.
pub trait InactiveProcessor: Send {
    /// Consumes the processor for typed recovery during activation. `[control-thread]`
    fn into_any(self: Box<Self>) -> Box<dyn Any + Send>;

    /// Called from CLAP `params.flush` while inactive. `[control-thread]`
    fn flush_params(&mut self, context: ParamFlushContext<'_>) -> PluginResult<()>;
}

pub struct ProcessContext<'a> {
    pub frames_count: u32,
    pub audio: AudioProcessBuffer<'a>,
    pub events: ProcessEvents<'a>,
    pub transport: Option<TransportEvent>,
}

pub struct ParamFlushContext<'a> {
    pub events: ProcessEvents<'a>,
}

#[derive(Debug, Clone, Copy)]
pub enum ProcessStatus {
    Continue,
    ContinueIfNotQuiet,
    Tail,
    Sleep,
}
