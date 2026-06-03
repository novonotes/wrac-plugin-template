//! Safe interface between product implementations and the adapter.
//!
//! One design assumption unlocks all trait docs: **VST3/AU/AAX via clap-wrapper does
//! not preserve CLAP `[main-thread]` annotations or lifecycle ordering.** Query traits
//! therefore take `&self` and must be answerable from any thread concurrently. FFI,
//! raw pointers, and panic barriers are contained inside the adapter; products only
//! need to implement these safe traits.

mod core;
mod error;
mod extensions;
mod host;
mod process;
mod types;

pub use core::{ActivateContext, PluginCore, PluginCoreContext};
pub use error::{PluginError, PluginResult};
pub use extensions::{
    PluginAudioPortsExtension, PluginConfigurableAudioPortsExtension, PluginGuiExtension,
    PluginLatencyExtension, PluginNotePortsExtension, PluginParamsExtension, PluginRenderExtension,
    PluginStateExtension, PluginTailExtension,
};
pub use host::{HostGuiResizeRequester, HostParamsEditNotifier, HostStateDirtyNotifier};
pub use process::{ProcessContext, ProcessStatus, Processor};
pub use types::{
    AudioPortConfigRequest, AudioPortFlags, AudioPortInfo, AudioPortType, GuiApi, GuiConfig,
    GuiResizeHints, GuiSize, HostWindow, NoteDialects, NotePortInfo, ParamFlags, ParamInfo,
    ParamValueEvent, PluginRenderMode, State,
};
