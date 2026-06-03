use crate::PluginResult;
use crate::events::{ProcessEvents, TransportEvent};
use crate::process_buffer::AudioProcessBuffer;

/// Processing object that runs on the audio thread.
///
/// Kept separate from `PluginCore` to decouple the audio callback from the core's write
/// lock and from GUI/project state. State passed in must be either an immutable snapshot
/// copied at activate time, or atomic/lock-free shared state the audio thread never
/// waits on.
pub trait Processor: Send {
    fn reset(&mut self) {}
    fn process(&mut self, context: ProcessContext<'_>) -> PluginResult<ProcessStatus>;
}

pub struct ProcessContext<'a> {
    pub frames_count: u32,
    pub audio: AudioProcessBuffer<'a>,
    pub events: ProcessEvents<'a>,
    pub transport: Option<TransportEvent>,
}

#[derive(Debug, Clone, Copy)]
pub enum ProcessStatus {
    Continue,
    ContinueIfNotQuiet,
    Tail,
    Sleep,
}
