//! 製品実装と adapter の間の safe なインターフェース。
//!
//! 設計の前提を 1 つだけ覚えておけば全 trait の doc が読める:
//! **clap-wrapper 経由の VST3/AU/AAX では CLAP の `[main-thread]` 注釈や
//! lifecycle 順序がそのまま守られない**。そのため query 系 trait は `&self` で、
//! 任意 thread から並行に呼ばれても答えられる実装を要求する。FFI・raw pointer・
//! panic 遮断は adapter 内部に閉じ、製品はこの safe trait だけを実装すればよい。

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

/// instance ごとに adapter から製品 core へ渡す環境。
///
/// FFI pointer を直接渡さず、製品が安全に保持できる adapter proxy だけを入れる。
#[derive(Clone)]
pub struct PluginCoreContext {
    pub host_parameter_edit_notifier: Arc<dyn HostParameterEditNotifier>,
    pub host_gui_resize_requester: Arc<dyn HostGuiResizeRequester>,
}

/// GUI 操作などで起きた parameter edit を host automation へ通知する。
///
/// SoT を更新する API ではない。製品が自分の store を先に更新し、その edit を
/// host へ返すために呼ぶ (begin → update → end が 1 つの undo 単位)。
pub trait HostParameterEditNotifier: Send + Sync {
    fn begin_edit(&self, parameter_id: u32);
    fn update_edit(&self, parameter_id: u32, value: f64);
    fn end_edit(&self, parameter_id: u32);
}

/// GUI など製品側操作から host へ GUI client area の resize を要求する。
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

/// CLAP note dialect bitset の薄い Rust 表現。
/// host とどの note dialect を送受信できるかを note-ports extension で交渉する値。
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

/// CLAP `clap_window_t` の薄い Rust 表現。
/// toolkit 中立にするため、特定 toolkit の型へは変換しない。
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

/// 1 plugin instance の lifecycle と capability の入口。
///
/// ここに全 state を集めない。`&mut self` の `activate`/`deactivate` と、並行に
/// 呼ばれる parameter/state/GUI の query を同じ mutable state に集めると、片方の
/// 実行中にもう片方へ答えられなくなる。各 capability は専用の thread-safe store に
/// 分け、この trait からは `Arc<dyn …>` で返すのが推奨形 (実例は `src-plugin`)。
pub trait PluginCore: Send + Sync + 'static {
    fn activate(&mut self, context: ActivateContext) -> PluginResult<Box<dyn Processor>>;
    fn deactivate(&mut self, processor: Box<dyn Processor>) -> PluginResult<()>;

    /// audio port query の capability。adapter が instance 作成時に Arc を保持し、
    /// 以降 `PluginCore` を借用せず呼ぶ (並行 read できる store に置くこと)。
    fn audio_ports(&self) -> Option<Arc<dyn PluginAudioPorts>> {
        None
    }

    /// host からの port layout 変更要求を扱う capability。
    ///
    /// `&self` だが active 中に変えてよい訳ではない。adapter は processor 存在中や
    /// lifecycle callback 中の apply を拒否する。実装は「次回 activate 用の layout」を
    /// non-realtime store に記録するだけにし、`activate()` で snapshot して
    /// [`Processor`] に渡す (processor 生存中は契約不変、と構造で示せる)。
    fn configurable_audio_ports(&self) -> Option<Arc<dyn PluginConfigurableAudioPorts>> {
        None
    }

    /// note port query の capability。count/dialect は host の routing 判断に使われる。
    /// lifecycle の busy 状態に左右されない schema store から答えること。
    fn note_ports(&self) -> Option<Arc<dyn PluginNotePorts>> {
        None
    }

    /// parameter schema/value と flush 時 input を扱う capability。
    ///
    /// automation・generic editor・restore 後 rescan から並行に触られる。schema は
    /// immutable、現在値は atomic/seqlock に置き、GUI/project state の lock を
    /// ここから辿らないこと。
    fn parameters(&self) -> Option<Arc<dyn PluginParameters>> {
        None
    }

    /// project state の save/restore を扱う capability。ユーザーデータを守る経路。
    /// 再生中・automation 中に呼ばれても committed snapshot を返せること
    /// (`&mut self` 依存にすると host が retry しない場合に編集を失う)。
    fn state(&self) -> Option<Arc<dyn PluginStateSupport>> {
        None
    }

    /// GUI を扱う capability。backend は thread affinity が強い。adapter は
    /// callback を UI thread へ marshal しないので、その契約は実装側で守る。
    fn gui(&self) -> Option<Arc<dyn PluginGui>> {
        None
    }
}

/// CLAP audio-ports extension。host が routing/bus layout を決める metadata を返す。
/// 任意 thread から並行に呼ばれる read 専用 API。busy 状態で値が揺れると host が
/// 正しく配線できないので、安定した値を返すこと。
pub trait PluginAudioPorts: Send + Sync + 'static {
    fn audio_port_count(&self, is_input: bool) -> u32;
    fn audio_port_info(&self, index: u32, is_input: bool) -> Option<AudioPortInfo>;
}

/// CLAP configurable-audio-ports extension。「inactive 時に次回 activate 用の
/// layout store を更新する」API として実装する。file IO・GUI callback・audio が
/// 待つ lock には入らないこと。
///
/// VST3/AU wrapper は host の speaker arrangement をこれに対応付ける。対応 layout を
/// 受け入れないと wrapper の buffer channel 数と合わず、process が呼ばれないことがある。
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

/// CLAP note-ports extension。note event 自体は process stream に流れるが、
/// port 数と dialect は host が先に query する。audio ports 同様、immutable schema /
/// 軽量 read-only store から答えること。
pub trait PluginNotePorts: Send + Sync + 'static {
    fn note_port_count(&self, is_input: bool) -> u32;
    fn note_port_info(&self, index: u32, is_input: bool) -> Option<NotePortInfo>;
}

/// CLAP params extension。schema と現在値を host が任意 thread から読める前提で設計する。
/// 特に `parameter_value` / `apply_parameter_value` は automation/flush と audio processing の
/// 境界に近いので、audio thread が待つ lock を共有しない store に寄せる。
pub trait PluginParameters: Send + Sync + 'static {
    fn parameter_count(&self) -> u32;
    fn parameter_info(&self, index: u32) -> Option<ParameterInfo>;
    /// parameter の現在の plain value (CLAP `get_value` 相当)。
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

/// CLAP state extension。[`PluginCore`] lifecycle から独立した project state 境界として
/// 実装する (host は active 中にも save/restore し得る)。
///
/// `save_state` は committed snapshot を**短時間で**返す。lock 中に serialize/file IO/
/// GUI dispatch を挟むと host の project save を詰まらせる。`restore_state` は decode 済み
/// state を SoT に commit する。audio と共有する値は realtime-safe store、editor-only は
/// project store、と state の種類で同期境界を分けるのが定石。
pub trait PluginStateSupport: Send + Sync + 'static {
    fn save_state(&self) -> PluginResult<PluginState>;
    fn restore_state(&self, state: PluginState) -> PluginResult<()>;
}

/// CLAP gui extension。GUI backend の thread affinity はこの trait 内で守る
/// (adapter は callback を UI thread へ marshal しない)。
///
/// `get_size`/`can_resize`/`resize_hints` は host の layout 計算中に再入し得る。
/// cached size や static hints から答え、重い mutation に入らないこと。
/// `create`/`destroy`/`set_parent` は adapter が再入 guard するが、backend 固有の
/// lifecycle 制約までは隠せない (必要なら controller 内に command queue を持つ)。
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

/// audio thread で動く processing object。
///
/// `PluginCore` と分けてあるのは、audio callback を core の write lock や
/// GUI/project state から切り離すため。渡す state は activate 時に copy した
/// immutable 設定か、audio thread が待たない atomic/lock-free 共有のみ
/// (`Arc<Mutex<_>>` を渡す場合も process 中に lock しない設計にする)。
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
