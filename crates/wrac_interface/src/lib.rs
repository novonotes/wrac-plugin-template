//! Product-facing contracts for implementing a WRAC plugin.
//!
//! The safe event and audio-buffer views live with the traits because they are part of the
//! callback contract. Their raw CLAP construction entry points exist only so
//! `wrac_clap_adapter` can materialize those borrowed views at the ABI boundary.

mod api;
mod descriptor;
mod entry;
mod events;
mod factory;
mod process_buffer;

pub use api::{
    ActivateContext, ActivateNotifications, ActivateResult, ActiveProcessor,
    AudioPortConfigRequest, AudioPortFlags, AudioPortInfo, AudioPortType, DetectedHost, GuiApi,
    GuiConfig, GuiResizeHints, GuiSize, HostAudioPorts, HostContext, HostFamily, HostGui,
    HostLifecycle, HostNotePorts, HostParams, HostState, HostTail, HostVersion, HostWindow,
    InactiveProcessor, NoteDialects, NotePortInfo, ParamFlags, ParamFlushContext, ParamInfo,
    ParamValueEvent, PluginAudioPortsExtension, PluginConfigurableAudioPortsExtension, PluginError,
    PluginFormat, PluginGuiApiSupportExtension, PluginGuiExtension, PluginGuiMainThreadExtension,
    PluginGuiQueryExtension, PluginInstance, PluginInstanceContext, PluginLatencyExtension,
    PluginNotePortsExtension, PluginParamsQuery, PluginRenderExtension, PluginRenderMode,
    PluginResult, PluginStateExtension, PluginTailExtension, PreparedStateSave, ProcessContext,
    ProcessStatus, State, StateSaveCompletion, StateSaveOutcome, SystemContext,
};
#[cfg(feature = "raw-clap-forwarding")]
pub use api::{RawParamFlushContext, RawProcessContext};

pub use descriptor::{
    AaxDescriptor, AaxStemConfig, Auv2Descriptor, PluginDescriptor, PluginFeature, Vst3Descriptor,
};
pub use entry::{EntryContext, LogConfig, LogOutput, PluginEntry};
pub use events::{
    EventLists, InputEvent, InputEvents, Midi2Event, MidiEvent, MidiSysexEvent, NoteEvent,
    NoteExpressionEvent, OutputEvent, OutputEvents, ParamGestureEvent, ParamInputEvents,
    ParamModEvent, TransportEvent, TransportFlags, UnknownEvent,
};
pub use factory::PluginFactory;
pub use process_buffer::{
    AudioBufferError, AudioChannelPair, AudioPairedChannels, AudioPortChannels, AudioPortPair,
    AudioPortPairs, AudioProcessBuffer,
};
