//! WRAC plugin implementation が adapter へ提供する安全な境界。
//!
//! Native CLAP の thread annotation だけを信じると、clap-wrapper 経由の
//! VST3/AU/AAX host で成立しない呼び出し順や呼び出し thread が混ざる。ここでは
//! wrapper でも守れる最小契約だけを public API にし、FFI と CLAP callback pointer は
//! adapter 内部に閉じ込める。

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

use crate::events::ProcessEvents;
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

/// adapter から製品 core へ渡す instance ごとの環境。
///
/// host callback pointer などの FFI 詳細を core に渡すと、製品実装が CLAP ABI の
/// lifetime と thread 契約を背負ってしまう。context には製品が安全に保持できる
/// adapter proxy だけを入れる。
#[derive(Clone)]
pub struct PluginCoreContext {
    pub host_parameter_edit_notifier: Arc<dyn HostParameterEditNotifier>,
}

/// GUI など製品側操作で発生した parameter edit を host automation へ通知する。
///
/// これは parameter の SoT を更新する API ではない。製品側は自分の parameter store
/// を先に更新し、その edit を host に返すためにこの notifier を呼ぶ。
pub trait HostParameterEditNotifier: Send + Sync {
    fn begin_edit(&self, parameter_id: u32);
    fn update_edit(&self, parameter_id: u32, value: f64);
    fn end_edit(&self, parameter_id: u32);
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

/// CLAP note dialect bitset の薄い Rust 表現。
///
/// note events 自体は process event stream に流れるが、host がどの event dialect を送れるかは
/// note-ports extension で交渉する。この型はその交渉値を clap-sys の raw bit に戻せる
/// 最小表現に留める。
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

/// CLAP `clap_window_t` を Rust 側で扱うための薄い表現。
///
/// platform handle の意味づけは window toolkit ごとに違うため、この crate では
/// `raw-window-handle` など特定 toolkit の型へ変換しない。
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

/// CLAP plugin instance の lifecycle。
///
/// extension ごとの API をこの trait に押し込むと、audio effect / instrument / GUI なし
/// plugin などの差が曖昧になる。`PluginCore` は lifecycle と capability discovery
/// だけを持ち、extension 固有の契約は `PluginAudioPorts` などへ分ける。
pub trait PluginCore: Send + Sync + 'static {
    fn activate(&mut self, context: ActivateContext) -> PluginResult<Box<dyn Processor>>;
    fn deactivate(&mut self, processor: Box<dyn Processor>) -> PluginResult<()>;

    fn audio_ports(&self) -> Option<&dyn PluginAudioPorts> {
        None
    }

    fn configurable_audio_ports(&mut self) -> Option<&mut dyn PluginConfigurableAudioPorts> {
        None
    }

    fn note_ports(&self) -> Option<&dyn PluginNotePorts> {
        None
    }

    fn parameters(&self) -> Option<&dyn PluginParameters> {
        None
    }

    fn state(&mut self) -> Option<&mut dyn PluginStateSupport> {
        None
    }

    fn gui(&self) -> Option<Arc<dyn PluginGui>> {
        None
    }
}

/// CLAP audio-ports extension に対応する capability。
///
/// wrapper host では port query が任意 thread から呼ばれ得るため、ここは `&self` だけで
/// 完結させる。active 中に変わる layout は、別途明示的な再設定 API を足すまで扱わない。
pub trait PluginAudioPorts {
    fn audio_port_count(&self, is_input: bool) -> u32;
    fn audio_port_info(&self, index: u32, is_input: bool) -> Option<AudioPortInfo>;
}

/// CLAP configurable-audio-ports extension に対応する capability。
///
/// VST3 などの wrapper host は speaker arrangement をあとから交渉する。固定 port 情報だけを
/// 返すと wrapper 内の process adapter が host の実 buffer channel 数とずれ、audio process
/// が呼ばれないことがあるため、対応 plugin はこの capability で非 active 時の layout 変更を
/// 受け入れる。
pub trait PluginConfigurableAudioPorts {
    fn can_apply_audio_port_configuration(
        &self,
        requests: &[AudioPortConfigurationRequest],
    ) -> bool;

    fn apply_audio_port_configuration(
        &mut self,
        requests: &[AudioPortConfigurationRequest],
    ) -> PluginResult<()>;
}

/// CLAP note-ports extension に対応する capability。
///
/// notes は audio callback の event stream に流れるが、port 数と dialect は host query で
/// 先に公開される。audio-ports と同じく wrapper host では任意 thread から読まれ得るため、
/// 実装は `&self` だけで完結させる。
pub trait PluginNotePorts {
    fn note_port_count(&self, is_input: bool) -> u32;
    fn note_port_info(&self, index: u32, is_input: bool) -> Option<NotePortInfo>;
}

/// CLAP params extension に対応する capability。
///
/// parameter query は host の軽量 API から並行に呼ばれやすい。実装は schema と value を
/// thread-safe に読める形へ寄せ、GUI/runtime 専用 state へ入り込まないようにする。
pub trait PluginParameters {
    fn parameter_count(&self) -> u32;
    fn parameter_info(&self, index: u32) -> Option<ParameterInfo>;
    /// Returns the parameter's current plain value, corresponding to CLAP `get_value`.
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

/// CLAP state extension に対応する capability。
///
/// VST3/AU/AAX host は処理が active な間にも state restore し得る。adapter はこの
/// capability を `&mut self` として lifecycle mutation と直列化するが、既に作成済みの
/// `Processor` は並行して処理を続ける可能性がある。
pub trait PluginStateSupport {
    fn save_state(&mut self) -> PluginResult<PluginState>;
    fn restore_state(&mut self, state: PluginState) -> PluginResult<()>;
}

/// CLAP gui extension に対応する capability。
///
/// adapter は GUI callback を main/UI thread に marshal しない。GUI lifecycle / mutation
/// callback は adapter 側で待たずに guard し、再入・並行呼び出しは失敗値として返す。
/// query callback は host layout 中の再入に備え、実装側が host-facing/static state だけで
/// 待たずに答える必要がある。
/// native GUI object の thread affinity は実装側が自分の backend に合わせて守る。
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

/// Audio thread で使う processing object。
///
/// processor を core から分けることで、audio callback は同期済み core の write lock
/// を取らずに済む。この境界を越える state は、明示的に audio-safe なものだけにする。
pub trait Processor: Send {
    fn reset(&mut self) {}
    fn process(&mut self, context: ProcessContext<'_>) -> PluginResult<ProcessStatus>;
}

pub struct ProcessContext<'a> {
    pub frames_count: u32,
    pub audio: AudioProcessBuffer<'a>,
    pub events: ProcessEvents<'a>,
}

#[derive(Debug, Clone, Copy)]
pub enum ProcessStatus {
    Continue,
    ContinueIfNotQuiet,
    Tail,
    Sleep,
}
