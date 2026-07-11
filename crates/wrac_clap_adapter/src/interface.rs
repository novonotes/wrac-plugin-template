//! Safe interface between product implementations and the adapter.
//!
//! This public API is a thin, safe facade over the CLAP C ABI. Its traits
//! should express existing CLAP entry points, factories, lifecycle callbacks,
//! extensions, event/buffer views, and host callbacks with Rust ownership and
//! defensive thread/call-order handling. Do not add extra abstraction,
//! high-level plugin APIs, or product/domain meaning here. Format conversion is
//! delegated to CLAP plus `clap-wrapper`; this crate must not become a
//! VST3/AU/AAX abstraction layer or plugin framework.
//! The API follows CLAP closely, but may choose pragmatic Rust surfaces over a
//! strict one-to-one mapping when that keeps the adapter thinner and harder to misuse.
//!
//! Method docs use annotations to state the requirements product authors must satisfy:
//! - `[main-thread]`: runs on the main thread, so the implementation may use
//!   main-thread-affine APIs such as GUI operations.
//! - `[non-realtime]`: runs serially on an arbitrary non-realtime thread. The implementation
//!   must not assume affinity to a particular thread.
//! - `[realtime-safe]`: may also run on realtime paths, with calls to the same object
//!   serialized. The implementation must avoid heap allocation, blocking locks, I/O, and
//!   non-realtime logging.
//! - `[non-realtime & thread-safe]`: may run concurrently on multiple non-realtime
//!   threads. The implementation must be thread-safe.
//! - `[realtime-safe & thread-safe]`: may run concurrently on multiple threads, including
//!   realtime paths. The implementation must be both realtime-safe and thread-safe.
//!
//! On product-implemented callbacks, an annotation states the implementation requirement.
//! On `Host*` methods supplied by the adapter, it states where product code may call the
//! method. Calls to the same `Host*` object include calls through cloned `Arc` references
//! to that object. `[main-thread]` permits calls only from the main thread;
//! `[non-realtime]` requires serialized calls from non-realtime paths, and
//! `[realtime-safe]` also permits serialized calls from realtime paths. The thread-safe
//! variants permit concurrent calls in the stated context.
//!
//! Host-facing ABI callbacks that require a synchronous return must not wait for a
//! main-thread or run-loop hop from a method not annotated `[main-thread]`.
//! Some hosts call plugin ABI callbacks from a background thread while blocking the
//! main thread, so waiting for the main thread can deadlock. Use cached state,
//! snapshots, or asynchronous follow-up notifications instead.

mod core;
mod descriptor;
mod entry;
mod error;
mod events;
mod extensions;
mod factory;
mod host;
mod params;
mod process;
mod process_buffer;
mod types;

pub use core::{
    ActivateContext, ActivateNotifications, ActivateResult, PluginInstance, PluginInstanceContext,
};
pub use descriptor::{
    AaxDescriptor, AaxStemConfig, Auv2Descriptor, PluginDescriptor, PluginFeature, Vst3Descriptor,
};
pub use entry::{EntryContext, LogConfig, LogOutput, PluginEntry};
pub use error::{PluginError, PluginResult};
pub use events::{
    EventLists, InputEvent, InputEvents, Midi2Event, MidiEvent, MidiSysexEvent, NoteEvent,
    NoteExpressionEvent, OutputEvent, OutputEvents, ParamGestureEvent, ParamInputEvents,
    ParamModEvent, TransportEvent, TransportFlags, UnknownEvent,
};
pub use extensions::{
    PluginAudioPortsExtension, PluginConfigurableAudioPortsExtension, PluginGuiApiSupportExtension,
    PluginGuiExtension, PluginGuiMainThreadExtension, PluginGuiQueryExtension,
    PluginLatencyExtension, PluginNotePortsExtension, PluginRenderExtension, PluginStateExtension,
    PluginTailExtension, PreparedStateSave, StateSaveCompletion, StateSaveOutcome,
};
pub use factory::PluginFactory;
pub use host::{
    HostAudioPorts, HostGui, HostLifecycle, HostNotePorts, HostParams, HostState, HostTail,
};
pub use params::PluginParamsQuery;
pub use process::{
    ActiveProcessor, InactiveProcessor, ParamFlushContext, ProcessContext, ProcessStatus,
};
#[cfg(feature = "raw-clap-forwarding")]
pub use process::{RawParamFlushContext, RawProcessContext};
pub use process_buffer::{
    AudioBufferError, AudioChannelPair, AudioPairedChannels, AudioPortChannels, AudioPortPair,
    AudioPortPairs, AudioProcessBuffer,
};
pub use types::{
    AudioPortConfigRequest, AudioPortFlags, AudioPortInfo, AudioPortType, GuiApi, GuiConfig,
    GuiResizeHints, GuiSize, HostWindow, NoteDialects, NotePortInfo, ParamFlags, ParamInfo,
    ParamValueEvent, PluginRenderMode, State,
};
pub use wrac_host_context::{
    DetectedHost, HostContext, HostFamily, HostVersion, PluginFormat, SystemContext,
};
