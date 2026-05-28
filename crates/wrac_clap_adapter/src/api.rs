//! Safe interface between product implementations and the adapter.
//!
//! One design assumption unlocks all trait docs: **VST3/AU/AAX via clap-wrapper does
//! not preserve CLAP `[main-thread]` annotations or lifecycle ordering.** Query traits
//! therefore take `&self` and must be answerable from any thread concurrently. FFI,
//! raw pointers, and panic barriers are contained inside the adapter; products only
//! need to implement these safe traits.

use std::error::Error;
use std::ffi::{CStr, c_void};
use std::fmt::{Display, Formatter};
use std::num::{NonZeroIsize, NonZeroU64};
use std::ptr::NonNull;
use std::sync::Arc;

use clap_sys::ext::note_ports::{
    CLAP_NOTE_DIALECT_CLAP, CLAP_NOTE_DIALECT_MIDI, CLAP_NOTE_DIALECT_MIDI_MPE,
    CLAP_NOTE_DIALECT_MIDI2,
};

use crate::events::{ProcessEvents, TransportEvent};
use crate::process_buffer::{AudioBufferError, AudioProcessBuffer};

#[derive(Debug)]
pub enum PluginError {
    InvalidParameter,
    InvalidState,
    UnsupportedHostGuiThreadingModel,
    RequiresInactive,
    Message(&'static str),
}

impl Display for PluginError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidParameter => f.write_str("invalid parameter"),
            Self::InvalidState => f.write_str("invalid state"),
            Self::UnsupportedHostGuiThreadingModel => {
                f.write_str("unsupported host GUI threading model")
            }
            Self::RequiresInactive => f.write_str("operation requires inactive processing state"),
            Self::Message(message) => f.write_str(message),
        }
    }
}

impl Error for PluginError {}

pub type PluginResult<T> = Result<T, PluginError>;

impl From<AudioBufferError> for PluginError {
    fn from(_value: AudioBufferError) -> Self {
        Self::InvalidState
    }
}

/// Per-instance environment passed from the adapter to the product core.
///
/// Contains only adapter proxies that the product can hold safely, not raw FFI pointers.
#[derive(Clone)]
pub struct PluginCoreContext {
    pub host_parameter_edit_notifier: Arc<dyn HostParameterEditNotifier>,
    pub host_state_dirty_notifier: Arc<dyn HostStateDirtyNotifier>,
    pub host_gui_resize_requester: Arc<dyn HostGuiResizeRequester>,
}

/// Notifies the host automation lane of a parameter edit triggered by the GUI or other
/// product-side action.
///
/// This is not an API to update the source of truth. The product updates its own store
/// first, then calls this to report the edit back to the host
/// (begin → update → end forms one undo unit).
pub trait HostParameterEditNotifier: Send + Sync {
    fn begin_edit(&self, parameter_id: u32);
    fn update_edit(&self, parameter_id: u32, value: f64);
    fn end_edit(&self, parameter_id: u32);
}

/// Notifies the host that non-parameter project state changed and should be saved.
///
/// This maps to CLAP `clap_host_state.mark_dirty()`. Use it for plugin-owned document
/// state, not for parameter automation gestures.
pub trait HostStateDirtyNotifier: Send + Sync {
    fn mark_dirty(&self);
}

/// Requests the host to resize the GUI client area on behalf of the product (e.g., from the GUI).
pub trait HostGuiResizeRequester: Send + Sync {
    fn request_resize(&self, size: GuiSize) -> PluginResult<()>;
}

#[derive(Debug, Clone, Copy)]
pub struct ActivateContext {
    pub sample_rate: f64,
    pub min_frames_count: u32,
    pub max_frames_count: u32,
}

#[derive(Debug, Clone)]
pub struct AudioPortInfo {
    pub id: u32,
    pub name: &'static str,
    pub flags: AudioPortFlags,
    pub channel_count: u32,
    pub port_type: AudioPortType,
    pub in_place_pair: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
pub struct AudioPortConfigurationRequest {
    pub is_input: bool,
    pub port_index: u32,
    pub channel_count: u32,
    pub port_type: AudioPortType,
}

#[derive(Debug, Clone)]
pub struct NotePortInfo {
    pub id: u32,
    pub supported_dialects: NoteDialects,
    pub preferred_dialect: NoteDialects,
    pub name: &'static str,
}

/// Thin Rust representation of the CLAP note dialect bitset.
/// Used in the note-ports extension to negotiate which note dialects can be sent and received.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoteDialects(u32);

impl NoteDialects {
    pub const CLAP: Self = Self(CLAP_NOTE_DIALECT_CLAP);
    pub const MIDI: Self = Self(CLAP_NOTE_DIALECT_MIDI);
    pub const MIDI_MPE: Self = Self(CLAP_NOTE_DIALECT_MIDI_MPE);
    pub const MIDI2: Self = Self(CLAP_NOTE_DIALECT_MIDI2);

    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AudioPortFlags {
    pub is_main: bool,
    pub supports_64bits: bool,
    pub prefers_64bits: bool,
    pub requires_common_sample_size: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum AudioPortType {
    #[default]
    Unspecified,
    Mono,
    Stereo,
    Other(&'static CStr),
}

#[derive(Debug, Clone)]
pub struct ParameterInfo {
    pub id: u32,
    pub name: &'static str,
    pub module: &'static str,
    pub min_value: f64,
    pub max_value: f64,
    pub default_value: f64,
    pub flags: ParameterFlags,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ParameterFlags {
    pub is_stepped: bool,
    pub is_periodic: bool,
    pub is_hidden: bool,
    pub is_readonly: bool,
    pub is_bypass: bool,
    pub is_automatable: bool,
    pub is_automatable_per_note_id: bool,
    pub is_automatable_per_key: bool,
    pub is_automatable_per_channel: bool,
    pub is_automatable_per_port: bool,
    pub is_modulatable: bool,
    pub is_modulatable_per_note_id: bool,
    pub is_modulatable_per_key: bool,
    pub is_modulatable_per_channel: bool,
    pub is_modulatable_per_port: bool,
    pub requires_process: bool,
    pub is_enum: bool,
}

#[derive(Debug, Clone)]
pub struct PluginState {
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
pub struct GuiConfiguration {
    pub api: GuiApi,
    pub is_floating: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiApi {
    Cocoa,
    Win32,
    X11,
}

#[derive(Debug, Clone, Copy)]
pub struct GuiSize {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct GuiResizeHints {
    pub can_resize_horizontally: bool,
    pub can_resize_vertically: bool,
    pub preserve_aspect_ratio: bool,
    pub aspect_ratio_width: u32,
    pub aspect_ratio_height: u32,
}

/// Thin Rust representation of `clap_window_t`.
/// Does not convert to toolkit-specific types in order to remain toolkit-neutral.
#[derive(Debug, Clone, Copy)]
pub enum ClapWindow {
    Cocoa { ns_view: NonNull<c_void> },
    Win32 { hwnd: NonZeroIsize },
    X11 { window: NonZeroU64 },
}

impl ClapWindow {
    pub(crate) fn cocoa(ns_view: *mut c_void) -> Option<Self> {
        Some(Self::Cocoa {
            ns_view: NonNull::new(ns_view)?,
        })
    }

    pub(crate) fn win32(hwnd: *mut c_void) -> Option<Self> {
        Some(Self::Win32 {
            hwnd: NonZeroIsize::new(hwnd as isize)?,
        })
    }

    pub(crate) fn x11(window: u64) -> Option<Self> {
        Some(Self::X11 {
            window: NonZeroU64::new(window)?,
        })
    }
}

/// Entry point for a single plugin instance's lifecycle and capabilities.
///
/// Do not concentrate all state here. Placing `&mut self` `activate`/`deactivate` and
/// concurrently-called parameter/state/GUI queries in the same mutable state would make
/// it impossible to answer one while the other is running. Split each capability into
/// its own thread-safe store and return it as `Arc<dyn …>` from this trait (see
/// `src-plugin` for examples).
///
/// Returning each capability as a separate `Arc` also keeps safe implementations from
/// exposing ordinary `PluginCore` fields directly to concurrent host callbacks: any
/// shared mutable state must cross an explicit thread-safe boundary.
pub trait PluginCore: Send + Sync + 'static {
    fn activate(&mut self, context: ActivateContext) -> PluginResult<Box<dyn Processor>>;
    fn deactivate(&mut self, processor: Box<dyn Processor>) -> PluginResult<()>;

    /// Capability for audio port queries. The adapter holds the Arc at instance creation
    /// and calls it without borrowing `PluginCore` thereafter (store in a parallel-readable store).
    fn audio_ports(&self) -> Option<Arc<dyn PluginAudioPorts>> {
        None
    }

    /// Capability for handling host-initiated port layout change requests.
    ///
    /// Takes `&self` but must not be changed while active. The adapter rejects apply
    /// calls while a Processor exists or during lifecycle callbacks. Implementations
    /// should only record the "layout for the next activate" in a non-realtime store,
    /// snapshot it in `activate()`, and pass it to the [`Processor`] (making the
    /// invariant explicit in the structure: layout is stable while a processor lives).
    fn configurable_audio_ports(&self) -> Option<Arc<dyn PluginConfigurableAudioPorts>> {
        None
    }

    /// Capability for note port queries. Count and dialect inform the host's routing
    /// decisions. Answer from a schema store unaffected by lifecycle busy state.
    fn note_ports(&self) -> Option<Arc<dyn PluginNotePorts>> {
        None
    }

    /// Capability for parameter schema/values and input during flush.
    ///
    /// Accessed concurrently from automation, generic editors, and post-restore rescans.
    /// Schema is immutable; current values live in atomics or a seqlock. Do not reach
    /// into GUI or project state locks from here.
    fn parameters(&self) -> Option<Arc<dyn PluginParameters>> {
        None
    }

    /// Capability for project state save/restore — the path that guards user data.
    /// Must return a committed snapshot even when called during playback or automation
    /// (relying on `&mut self` risks losing edits when the host does not retry).
    fn state(&self) -> Option<Arc<dyn PluginStateSupport>> {
        None
    }

    /// Capability for the GUI. The backend has strong thread affinity. The adapter does
    /// not marshal callbacks to the UI thread; that contract must be upheld by the implementation.
    fn gui(&self) -> Option<Arc<dyn PluginGui>> {
        None
    }

    /// Capability for CLAP render mode changes.
    ///
    /// This mirrors the CLAP render extension: the adapter forwards host mode changes,
    /// and the product decides whether to store that mode for the audio processor.
    fn render(&self) -> Option<Arc<dyn PluginRender>> {
        None
    }

    /// Capability for reporting tail length in frames.
    fn tail(&self) -> Option<Arc<dyn PluginTail>> {
        None
    }

    /// Capability for reporting processing latency in frames.
    fn latency(&self) -> Option<Arc<dyn PluginLatency>> {
        None
    }
}

/// CLAP audio-ports extension. Returns metadata the host uses to determine routing and
/// bus layout. This is a read-only API called concurrently from any thread. Return
/// stable values — fluctuating under busy state prevents the host from wiring correctly.
pub trait PluginAudioPorts: Send + Sync + 'static {
    fn audio_port_count(&self, is_input: bool) -> u32;
    fn audio_port_info(&self, index: u32, is_input: bool) -> Option<AudioPortInfo>;
}

/// CLAP configurable-audio-ports extension. Implement as "update the layout store for
/// the next activate when inactive." Do not enter locks that file IO, GUI callbacks, or
/// audio threads wait on.
///
/// VST3/AU wrappers map the host's speaker arrangement through this extension. Rejecting
/// a supported layout can mismatch the wrapper's buffer channel count, causing process
/// to not be called.
pub trait PluginConfigurableAudioPorts: Send + Sync + 'static {
    fn can_apply_audio_port_configuration(
        &self,
        requests: &[AudioPortConfigurationRequest],
    ) -> bool;

    fn apply_audio_port_configuration(
        &self,
        requests: &[AudioPortConfigurationRequest],
    ) -> PluginResult<()>;
}

/// CLAP note-ports extension. Note events themselves flow in the process stream, but
/// port count and dialect are queried by the host up front. As with audio ports, answer
/// from an immutable schema or lightweight read-only store.
pub trait PluginNotePorts: Send + Sync + 'static {
    fn note_port_count(&self, is_input: bool) -> u32;
    fn note_port_info(&self, index: u32, is_input: bool) -> Option<NotePortInfo>;
}

/// CLAP params extension. Design assuming the host reads schema and current values from
/// any thread. In particular, `parameter_value` / `apply_parameter_value` sit close to
/// the automation/flush and audio processing boundary, so keep them in a store that does
/// not share locks the audio thread waits on.
pub trait PluginParameters: Send + Sync + 'static {
    fn parameter_count(&self) -> u32;
    fn parameter_info(&self, index: u32) -> Option<ParameterInfo>;
    /// Current plain value of a parameter (equivalent to CLAP `get_value`).
    fn parameter_value(&self, parameter_id: u32) -> PluginResult<f64>;
    fn apply_parameter_value(&self, event: ParameterValueEvent) -> PluginResult<f64>;
    fn parameter_value_to_text(&self, parameter_id: u32, value: f64) -> PluginResult<String>;
    fn parameter_text_to_value(&self, parameter_id: u32, text: &str) -> PluginResult<f64>;
}

#[derive(Debug, Clone, Copy)]
pub struct ParameterValueEvent {
    pub time: u32,
    pub parameter_id: u32,
    pub value: f64,
    pub note_id: i32,
    pub port_index: i16,
    pub channel: i16,
    pub key: i16,
}

/// CLAP state extension. Implement as a project state boundary independent of the
/// [`PluginCore`] lifecycle (the host may save/restore while active).
///
/// `save_state` must return a committed snapshot **quickly**. Serializing, doing file IO,
/// or dispatching to the GUI while holding a lock will stall the host's project save.
/// `restore_state` commits the decoded state to the source of truth. The standard
/// pattern is to split sync boundaries by state kind: realtime-safe store for audio-shared
/// values, project store for editor-only values.
pub trait PluginStateSupport: Send + Sync + 'static {
    fn save_state(&self) -> PluginResult<PluginState>;
    fn restore_state(&self, state: PluginState) -> PluginResult<()>;
}

/// CLAP gui extension. GUI backend thread affinity must be enforced within this trait
/// (the adapter does not marshal callbacks to the UI thread).
///
/// `get_size`/`can_resize`/`resize_hints` may be re-entered during host layout
/// computation. Answer from cached size or static hints without entering heavy mutations.
/// `create`/`destroy`/`set_parent` are re-entry-guarded by the adapter, but
/// backend-specific lifecycle constraints are not hidden (use a command queue inside the
/// controller if needed).
pub trait PluginGui: Send + Sync + 'static {
    fn is_api_supported(&self, api: GuiApi, is_floating: bool) -> bool;
    fn preferred_api(&self) -> Option<GuiConfiguration>;
    fn create(&self, configuration: GuiConfiguration) -> PluginResult<()>;
    fn destroy(&self);
    fn set_scale(&self, scale: f64) -> PluginResult<()>;
    fn get_size(&self) -> PluginResult<GuiSize>;
    fn can_resize(&self) -> bool;
    fn resize_hints(&self) -> Option<GuiResizeHints>;
    fn adjust_size(&self, size: GuiSize) -> PluginResult<GuiSize>;
    fn set_size(&self, size: GuiSize) -> PluginResult<()>;
    fn set_parent(&self, window: ClapWindow) -> PluginResult<()>;
    fn set_transient(&self, window: ClapWindow) -> PluginResult<()>;
    fn suggest_title(&self, title: &str);
    fn show(&self) -> PluginResult<()>;
    fn hide(&self) -> PluginResult<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    Realtime,
    Offline,
}

/// CLAP render extension. The host calls this outside `process()` to announce whether
/// upcoming processing is realtime or offline.
pub trait PluginRender: Send + Sync + 'static {
    fn has_hard_realtime_requirement(&self) -> bool {
        false
    }

    fn set_render_mode(&self, mode: RenderMode) -> PluginResult<()>;
}

/// CLAP tail extension. The returned value is a frame count, matching CLAP directly.
pub trait PluginTail: Send + Sync + 'static {
    fn tail_frames(&self) -> u32;
}

/// CLAP latency extension. The returned value is a frame count, matching CLAP directly.
pub trait PluginLatency: Send + Sync + 'static {
    fn latency_frames(&self) -> u32;
}

/// Processing object that runs on the audio thread.
///
/// Kept separate from `PluginCore` to decouple the audio callback from the core's write
/// lock and from GUI/project state. State passed in must be either an immutable snapshot
/// copied at activate time, or atomic/lock-free shared state the audio thread never
/// waits on (even when passing `Arc<Mutex<_>>`, design it so process() never locks).
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
