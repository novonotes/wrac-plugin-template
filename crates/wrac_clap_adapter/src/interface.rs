//! Safe interface between product implementations and the adapter.
//!
//! When developing a plugin using `wrac_clap_adapter`, each product implements this interface.
//!
//! This interface is not a VST3/AU/AAX abstraction layer or plugin framework. It is intentionally
//! designed to limit abstraction and high-level APIs and to correspond to the CLAP ABI. Format
//! conversion is delegated to `clap-wrapper` and is not the responsibility of this crate.
//!
//! However, to keep the adapter thinner and make misuse easier to prevent, it may choose a
//! practical Rust interface over a strict one-to-one mapping.
//!
//! Method documentation uses the following annotations to specify the requirements product
//! developers must satisfy:
//! - `[main-thread]`: always runs on the main thread. The implementation may use
//!   main-thread-affine APIs such as GUI operations.
//! - `[non-realtime]`: implement it so that it can run on any non-realtime thread. It must not
//!   assume affinity to a specific thread.
//! - `[realtime-safe]`: implement it so that it can also run on realtime paths. Avoid heap
//!   allocation and locks.
//! - `[non-realtime & thread-safe]`: implement it so that it can be called concurrently from
//!   multiple non-realtime threads.
//! - `[realtime-safe & thread-safe]`: implement it so that it satisfies both realtime safety and
//!   thread safety.
//!
//! Host-facing ABI callbacks that require a synchronous return value must not hop to and wait for
//! the main thread or run loop from a method not annotated `[main-thread]`. Some hosts call plugin
//! ABI callbacks from a background thread while blocking the main thread, so waiting for the main
//! thread may cause a deadlock. Use cached state, snapshots, or asynchronous follow-up
//! notifications instead.

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
