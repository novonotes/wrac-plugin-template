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
//! Method docs use thread annotations for the Rust trait call contract:
//! - `[main-thread]`: native CLAP/UI main thread. Non-realtime and serialized.
//! - `[control-thread]`: non-realtime host/adapter control work. This includes the
//!   main thread and background/control worker threads. Serialized per plugin instance.
//! - `[audio-thread]`: realtime audio callback work. Serialized per plugin instance,
//!   but the OS thread id is not stable.
//! - `[thread-safe & control-thread]`: may be called concurrently from control threads.
//! - `[thread-safe]`: may be called concurrently from any thread, including the audio
//!   thread; implementations must satisfy realtime constraints.
//! - `[control-thread,audio-thread]`: may be called from control or audio threads,
//!   but not concurrently for the same plugin instance.
//!
//! Comma means "or", and `&` adds a condition as in the CLAP headers.
//!
//! Some WRAC contracts are stricter than native CLAP because VST3/AU/AAX wrappers do
//! not reliably preserve CLAP `[main-thread]` callbacks or lifecycle ordering. WRAC
//! uses `[control-thread]` when native CLAP says `[main-thread]` but the exact main
//! thread is not guaranteed. FFI, raw pointers, and panic barriers are contained
//! inside the adapter; products only need to implement these safe traits.

mod core;
mod error;
mod extensions;
mod host;
mod params;
mod process;
mod types;

pub use core::{ActivateContext, PluginInstance, PluginInstanceContext};
pub use error::{PluginError, PluginResult};
pub use extensions::{
    PluginAudioPortsExtension, PluginConfigurableAudioPortsExtension, PluginGuiExtension,
    PluginLatencyExtension, PluginNotePortsExtension, PluginRenderExtension, PluginStateExtension,
    PluginTailExtension,
};
pub use host::{HostGuiResizeRequester, HostParamsFlushRequester, HostStateDirtyNotifier};
pub use params::PluginParamsQuery;
pub use process::{
    ActiveProcessor, InactiveProcessor, ParamFlushContext, ProcessContext, ProcessStatus,
};
pub use types::{
    AudioPortConfigRequest, AudioPortFlags, AudioPortInfo, AudioPortType, GuiApi, GuiConfig,
    GuiResizeHints, GuiSize, HostWindow, NoteDialects, NotePortInfo, ParamFlags, ParamInfo,
    ParamValueEvent, PluginRenderMode, State,
};
pub use wrac_host_context::{
    DetectedHost, HostContext, HostFamily, HostVersion, PluginFormat, SystemContext,
};
