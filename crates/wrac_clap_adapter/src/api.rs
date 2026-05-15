//! プラグイン実装と adapter の間のインターフェース。
//!
//! Native CLAP の thread annotation だけを信じると、clap-wrapper 経由の
//! VST3/AU/AAX host で成立しない呼び出し順や呼び出し thread が混ざる。ここでは
//! wrapper でも守れる最小契約だけを public API にし、FFI と CLAP callback pointer は
//! adapter 内部に閉じ込める。
//!
//! adapter は FFI callback で発生した panic を C ABI の外へ伝播させない。製品実装は
//! safe trait だけを実装し、panic / error は callback ごとの失敗値へ変換される前提で扱う。
//!
//! query 系 trait は `&self` を第一引数とし、任意の thread から並行に読める実装を要求する。
//!
//! host / wrapper は CLAP の `[main-thread]` 注釈通りに query を呼ぶとは限らないため、
//! schema や現在値の読み取りなどの軽量クエリは GUI/runtime 専用 state のロックを待たない形へ寄せる。

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
    pub host_gui_resize_requester: Arc<dyn HostGuiResizeRequester>,
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

/// 1 つの plugin instance の lifecycle と capability discovery。
///
/// 実装者はこの trait を「plugin 本体」だと考えがちですが、ここに全 state を集める必要は
/// ありません。むしろ [`PluginCore`] は processor lifecycle と capability の入口だけを持ち、
/// parameter、state、port layout、GUI などはそれぞれ専用の thread-safe store / capability に
/// 分けるのが推奨形です。
///
/// 理由は、VST3/AU などの wrapper 経由では CLAP の thread annotation や lifecycle 順序が
/// そのまま守られないことがあるためです。`activate()` / `deactivate()` のような `&mut self`
/// lifecycle と、parameter query や state save のような host-facing callback を同じ mutable
/// state に集めると、片方が動いている間にもう片方へ答えられなくなります。
pub trait PluginCore: Send + Sync + 'static {
    fn activate(&mut self, context: ActivateContext) -> PluginResult<Box<dyn Processor>>;
    fn deactivate(&mut self, processor: Box<dyn Processor>) -> PluginResult<()>;

    /// Audio port query を実装する object を返す。
    ///
    /// この object は instance 作成時に adapter が保持し、その後の port query では
    /// [`PluginCore`] を借用しません。実装者は port 情報を `PluginCore` の field から直接
    /// 読ませるのではなく、軽量に並行 read できる layout/schema store へ置いてください。
    fn audio_ports(&self) -> Option<Arc<dyn PluginAudioPorts>> {
        None
    }

    /// Host からの port layout 変更要求を扱う object を返す。
    ///
    /// `apply_audio_port_configuration()` は `&self` API ですが、active 中でも自由に layout を
    /// 変えてよいという意味ではありません。adapter は processor が存在する間、または lifecycle
    /// callback が走っている間は `can_apply` / `apply` を拒否します。
    ///
    /// 実装者側では、この object は「次に activate する processor の layout」を non-realtime
    /// store に記録するだけにしてください。audio thread からその store を読ませず、
    /// `activate()` で snapshot して [`Processor`] に渡すと、processor が生きている間に layout
    /// 契約が変わらないことをコード上で表現できます。
    fn configurable_audio_ports(&self) -> Option<Arc<dyn PluginConfigurableAudioPorts>> {
        None
    }

    /// Note port query を実装する object を返す。
    ///
    /// Note port count や dialect は host の routing 判断に使われます。実装者は、lifecycle の
    /// 一時的な busy 状態に左右されない schema store から答えられるようにしてください。
    fn note_ports(&self) -> Option<Arc<dyn PluginNotePorts>> {
        None
    }

    /// Parameter schema/value query と flush-time parameter input を扱う object を返す。
    ///
    /// Parameter は automation、generic editor、state restore 後の rescan などから並行に
    /// 触られます。schema は immutable data、現在値は atomic/seqlock などの realtime-safe store
    /// に置き、GUI runtime や project-only state の lock をここから辿らないようにしてください。
    fn parameters(&self) -> Option<Arc<dyn PluginParameters>> {
        None
    }

    /// Project state の save/restore を扱う object を返す。
    ///
    /// State はユーザーの project data を守る経路です。再生中や automation 中に呼ばれても、
    /// 実装者側の committed state snapshot を返せるようにしてください。`PluginCore` の
    /// `&mut self` が取れないと保存できない設計にすると、host が retry しない場合に編集内容を
    /// 失う可能性があります。
    fn state(&self) -> Option<Arc<dyn PluginStateSupport>> {
        None
    }

    /// GUI を扱う object を返す。
    ///
    /// GUI 実装は native window や WebView runtime など thread affinity の強い資源を持ちます。
    /// 実装者は backend の thread 契約を守りつつ、size query などの軽量 API はなるべく
    /// runtime mutation に依存せず答えられるようにしてください。
    fn gui(&self) -> Option<Arc<dyn PluginGui>> {
        None
    }
}

/// CLAP audio-ports extension に対応する capability。
///
/// 実装者は、この trait を任意 thread から並行に呼ばれてもよい読み取り API として扱ってください。
/// port count / channel count / port name は、host が routing や bus layout を決めるための
/// metadata です。ここで一時的に失敗したり busy 状態によって値が変わったりすると、host が
/// plugin を正しく配線できなくなります。
pub trait PluginAudioPorts: Send + Sync + 'static {
    fn audio_port_count(&self, is_input: bool) -> u32;
    fn audio_port_info(&self, index: u32, is_input: bool) -> Option<AudioPortInfo>;
}

/// CLAP configurable-audio-ports extension に対応する capability。
///
/// 実装者は、この trait を「inactive 時に次回 activate 用の layout store を更新する API」として
/// 実装してください。adapter は active 中の apply を拒否しますが、この trait の中でも file IO、
/// GUI callback、audio thread が待つ lock などには入らないでください。
///
/// VST3/AU wrapper は host-native の speaker arrangement と CLAP audio ports を対応させます。
/// ここで対応 layout を受け入れないと、wrapper 内の process adapter が host の実 buffer channel
/// 数と合わず、音声処理が呼ばれないことがあります。
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

/// CLAP note-ports extension に対応する capability。
///
/// Note events 自体は process event stream に流れますが、port 数と dialect は host が先に
/// query します。実装者は audio ports と同じく、ここを immutable schema か軽量 read-only store
/// から答える API として扱ってください。
pub trait PluginNotePorts: Send + Sync + 'static {
    fn note_port_count(&self, is_input: bool) -> u32;
    fn note_port_info(&self, index: u32, is_input: bool) -> Option<NotePortInfo>;
}

/// CLAP params extension に対応する capability。
///
/// 実装者は parameter schema と current value を、host が任意 thread から読めるものとして
/// 設計してください。特に `parameter_value()` と `apply_parameter_value()` は automation /
/// flush / generic editor と audio processing の境界に近いため、audio thread が待つ lock を
/// 共有しない store に寄せるのが安全です。
pub trait PluginParameters: Send + Sync + 'static {
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
/// 実装者は、この trait を [`PluginCore`] の lifecycle から独立した project state 境界として
/// 実装してください。VST3/AU/AAX host は処理が active な間にも state save/restore し得ます。
///
/// `save_state()` は「今 committed になっている project state」を短時間で snapshot して返す
/// API です。serialize、file IO、GUI dispatch などを lock 中に行うと、host の project save を
/// 詰まらせたり deadlock の原因になります。
///
/// `restore_state()` は decoded state を SoT に commit する API です。parameter のように audio
/// thread と共有する値は atomic/seqlock などの realtime-safe store へ、editor-only state は
/// audio thread から読まない project store へ、というように state の種類で同期境界を分けると
/// 製品実装が破綻しにくくなります。
pub trait PluginStateSupport: Send + Sync + 'static {
    fn save_state(&self) -> PluginResult<PluginState>;
    fn restore_state(&self, state: PluginState) -> PluginResult<()>;
}

/// CLAP gui extension に対応する capability。
///
/// 実装者は、GUI backend の thread affinity をこの trait の内側で守ってください。adapter は
/// callback を UI thread に marshal しません。
///
/// `get_size()` / `can_resize()` / `resize_hints()` のような query は、host の layout 計算中に
/// 再入することがあります。できるだけ cached size や static hints から答え、WebView/native
/// runtime の重い mutation に入らない設計にしてください。
///
/// `create()` / `destroy()` / `set_parent()` のような mutation は adapter 側でも再入 guard しますが、
/// backend 固有の lifecycle 制約までは隠せません。必要なら GUI controller 内に command queue や
/// main-thread executor を持たせます。
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
/// 実装者は、[`Processor`] を realtime path の所有物として扱ってください。`PluginCore` から
/// 分けているのは、audio callback が core の write lock や GUI/project state に入らずに済むように
/// するためです。
///
/// `Processor` に渡す state は、`activate()` 時点で copy した immutable 設定、または atomic /
/// lock-free queue など audio thread が待たない共有状態に限るのが基本です。`Arc<Mutex<_>>` や
/// `Arc<RwLock<_>>` を渡す場合でも、process 中に lock しない設計にしてください。
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
