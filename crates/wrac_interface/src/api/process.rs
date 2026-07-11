use std::any::Any;
#[cfg(feature = "raw-clap-forwarding")]
use std::{marker::PhantomData, rc::Rc};

use crate::PluginResult;
use crate::events::{EventLists, TransportEvent};
use crate::process_buffer::AudioProcessBuffer;
#[cfg(feature = "raw-clap-forwarding")]
use clap_sys::{
    events::{clap_input_events, clap_output_events},
    process::clap_process,
};

/// Processing object used while the CLAP plugin is active.
///
/// State passed in must be either an immutable snapshot copied at activate time, or
/// atomic/lock-free shared state the audio thread never waits on.
pub trait ActiveProcessor: Send {
    /// Converts to `Any` so `deactivate` can recover owned state. `[default]`
    fn into_any(self: Box<Self>) -> Box<dyn Any + Send>;

    /// Called from CLAP `plugin.reset`. `[realtime-safe]`
    fn reset(&mut self) {}

    /// Called from CLAP `plugin.process`. `[realtime-safe]`
    fn process(&mut self, context: ProcessContext<'_>) -> PluginResult<ProcessStatus>;

    /// Called from CLAP `params.flush` while active. `[realtime-safe]`
    fn flush_params(&mut self, context: ParamFlushContext<'_>) -> PluginResult<()>;
}

/// Processing state used while the CLAP plugin is inactive.
pub trait InactiveProcessor: Send {
    /// Converts to `Any` so `activate` can recover owned state. `[default]`
    fn into_any(self: Box<Self>) -> Box<dyn Any + Send>;

    /// Called from CLAP `params.flush` while inactive. `[default]`
    fn flush_params(&mut self, context: ParamFlushContext<'_>) -> PluginResult<()>;
}

pub struct ProcessContext<'a> {
    pub frames_count: u32,
    pub audio: AudioProcessBuffer<'a>,
    pub events: EventLists<'a>,
    pub transport: Option<TransportEvent>,
    #[cfg(feature = "raw-clap-forwarding")]
    pub raw: RawProcessContext<'a>,
}

pub struct ParamFlushContext<'a> {
    pub events: EventLists<'a>,
    #[cfg(feature = "raw-clap-forwarding")]
    pub raw: RawParamFlushContext<'a>,
}

#[derive(Debug, Clone, Copy)]
pub enum ProcessStatus {
    Continue,
    ContinueIfNotQuiet,
    Tail,
    Sleep,
}

#[cfg(feature = "raw-clap-forwarding")]
impl<'a> ProcessContext<'a> {
    /// Returns the exact CLAP process pointer received by the WRAC adapter.
    ///
    /// This is intentionally available only behind `raw-clap-forwarding`. It exists for
    /// CLAP-to-CLAP proxy products that must synchronously forward process data without
    /// re-encoding events or buffers. Do not store the returned view beyond the callback.
    pub fn raw_forwarding(&self) -> RawProcessContext<'a> {
        self.raw
    }
}

#[cfg(feature = "raw-clap-forwarding")]
impl<'a> ParamFlushContext<'a> {
    /// Returns the exact CLAP params.flush event lists received by the WRAC adapter.
    ///
    /// The view is callback-lifetime bound and must only be used for synchronous
    /// forwarding into another CLAP plugin instance.
    pub fn raw_forwarding(&self) -> RawParamFlushContext<'a> {
        self.raw
    }
}

#[cfg(feature = "raw-clap-forwarding")]
#[derive(Clone, Copy)]
pub struct RawProcessContext<'a> {
    process: *const clap_process,
    _marker: PhantomData<(&'a clap_process, Rc<()>)>,
}

#[cfg(feature = "raw-clap-forwarding")]
impl<'a> RawProcessContext<'a> {
    /// Creates a raw forwarding view for one process callback.
    ///
    /// # Safety
    ///
    /// `process` must remain valid for `'a` and must only be forwarded synchronously.
    pub unsafe fn from_raw(process: *const clap_process) -> Self {
        Self {
            process,
            _marker: PhantomData,
        }
    }

    /// Raw CLAP `clap_process` pointer valid only for the current process callback.
    pub fn as_ptr(self) -> *const clap_process {
        self.process
    }
}

#[cfg(feature = "raw-clap-forwarding")]
#[derive(Clone, Copy)]
pub struct RawParamFlushContext<'a> {
    input_events: *const clap_input_events,
    output_events: *const clap_output_events,
    _marker: PhantomData<(&'a clap_input_events, &'a mut clap_output_events, Rc<()>)>,
}

#[cfg(feature = "raw-clap-forwarding")]
impl<'a> RawParamFlushContext<'a> {
    /// Creates raw forwarding views for one parameter-flush callback.
    ///
    /// # Safety
    ///
    /// Both pointers must remain valid for `'a`, and `output_events` must be exclusively
    /// writable for the duration of the synchronous forwarding call.
    pub unsafe fn from_raw(
        input_events: *const clap_input_events,
        output_events: *const clap_output_events,
    ) -> Self {
        Self {
            input_events,
            output_events,
            _marker: PhantomData,
        }
    }

    /// Raw CLAP `clap_input_events` pointer valid only for the current flush callback.
    pub fn input_events(self) -> *const clap_input_events {
        self.input_events
    }

    /// Raw CLAP `clap_output_events` pointer valid only for the current flush callback.
    pub fn output_events(self) -> *const clap_output_events {
        self.output_events
    }
}
