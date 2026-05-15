//! WRAC Gain の host-facing な plugin core 実装。
//!
//! このファイルは host から見える plugin の契約をまとめる場所です。やっていることを
//! 大雑把に並べると:
//!
//! 1. plugin の自己紹介情報 ([`PLUGIN_DESCRIPTOR`]) を宣言する
//! 2. parameter 定義 (gain と bypass) を host に教える
//! 3. audio thread / GUI / host で共有する [`SharedState`] を作る
//! 4. host から [`PluginCore::activate`] されたら audio 処理用の [`Processor`] を渡す
//! 5. host から GUI を要求されたら `gui.rs` 側の controller を渡す
//! 6. state の save/restore (DAW の project に保存) を実装する
//!
//! CLAP / VST3 / AU といった plugin format の差分は `wrac_clap_adapter` が
//! 吸収するので、ここでは「gain plugin として何を持つか」だけに集中できます。

use std::sync::Arc;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use wrac_clap_adapter::{
    ActivateContext, AudioPortConfigurationRequest, AudioPortFlags, AudioPortInfo, AudioPortType,
    Auv2Descriptor, ParameterFlags, ParameterInfo, ParameterValueEvent, PluginAudioPorts,
    PluginConfigurableAudioPorts, PluginCore, PluginCoreContext, PluginDescriptor, PluginError,
    PluginFeature, PluginGui, PluginParameters, PluginResult, PluginState, PluginStateSupport,
    Processor,
};
use wrac_wxp_gui::WxpGuiController;

use crate::audio::WracGainAudioProcessor;
use crate::gui::{GuiStateNotifier, create_gui_integration};
use crate::state::{
    EditorPage, ParameterStateSnapshot, ProjectState, ProjectStateStore, SharedState,
};

// plugin identity は src-plugin/Cargo.toml の [package.metadata.wrac] から来る。
// GUI 側も同じ metadata を読むことで、host-facing descriptor と About 表示の
// 改名漏れを防ぐ。
pub(crate) const PLUGIN_ID: &str = env!("WRAC_PLUGIN_ID");
pub(crate) const PLUGIN_NAME: &str = env!("WRAC_PLUGIN_NAME");
pub(crate) const COMPANY_NAME: &str = env!("WRAC_COMPANY_NAME");

// 各 parameter にも host 内で一意の ID を割り当てる必要がある。
// 新しい parameter を追加するときは、ここに `PARAM_*_ID` を増やし、下の
// `PluginParameters` 実装と `SharedState` の match にも追加する。
pub(crate) const PARAM_GAIN_ID: u32 = 1;
pub(crate) const PARAM_BYPASS_ID: u32 = 9;

// gain の値域。1.0 が「そのまま (0 dB)」、0.0 が「無音 (-inf dB)」、
// 2.0 が「2 倍 (+6 dB)」を表す線形 amplitude。
pub(crate) const DEFAULT_GAIN: f32 = 1.0;
pub(crate) const MIN_GAIN: f32 = 0.0;
pub(crate) const MAX_GAIN: f32 = 2.0;

// host (DAW) に plugin を自己紹介するための静的データ。
// `wrac_clap_adapter` がこれを CLAP / AUv2 の descriptor 構造体へと変換する。
pub(crate) const PLUGIN_DESCRIPTOR: PluginDescriptor = PluginDescriptor {
    id: PLUGIN_ID,
    name: PLUGIN_NAME,
    vendor: COMPANY_NAME,
    url: "",
    manual_url: "",
    support_url: "",
    version: env!("CARGO_PKG_VERSION"),
    description: "Simple gain plugin",
    features: &[
        PluginFeature::AudioEffect,
        PluginFeature::Utility,
        PluginFeature::Stereo,
    ],
    // AUv2 (macOS の Audio Unit v2) 用の追加情報。
    // manufacturer_code と plugin_subtype は 4 文字 ASCII の固有 ID で、
    // 同じ会社内で重複しないように決める必要がある。
    auv2: Some(Auv2Descriptor {
        manufacturer_code: *b"YrCo",
        manufacturer_name: COMPANY_NAME,
        plugin_type: *b"aufx", // "aufx" = audio effect
        plugin_subtype: *b"WtGn",
    }),
};

/// plugin 1 instance を表す型。
///
/// host (DAW) が plugin を読み込むごとにこの struct が 1 つずつ作られる。
/// audio 処理本体は [`PluginCore::activate`] で別途 [`Processor`] として切り出すので、
/// この struct は lifecycle と、host に公開する extension trait 群を実装する。
pub(crate) struct WracGainPlugin {
    // audio thread / GUI / host から共有して触る状態。
    // 詳細は `SharedState` の doc を参照。
    shared: Arc<SharedState>,
    // Port layout は activate 前後の host query / configurable ports で使う非 RT 状態。
    //
    // 製品 plugin では channel count や speaker layout を DSP が参照することがよくありますが、
    // audio thread からこの store を直接読ませないのが重要です。layout 変更は host との交渉で
    // 決まる non-realtime な出来事で、処理中に RwLock を読みに行くと priority inversion や
    // callback 再入時の停止要因になります。
    //
    // そのため、この store は「次に activate する processor 用の設定」として扱い、`activate()`
    // で値を snapshot して [`WracGainAudioProcessor`] へ渡します。
    audio_layout: Arc<AudioLayoutStore>,
    // Host-facing capability は `PluginCore` の borrow と切り離して Arc で持つ。
    // wrapper が lifecycle callback 中に query を再入させても、adapter は core lock を取らずに
    // ここへ到達できる。
    audio_ports: Arc<WracGainAudioPorts>,
    configurable_audio_ports: Arc<WracGainConfigurableAudioPorts>,
    parameters: Arc<WracGainParameters>,
    // WebView による GUI を CLAP の GUI extension として扱うための helper。
    // `Arc` にしているのは host が `plugin_gui` を複数回問い合わせるため。
    gui: Arc<WxpGuiController>,
    // Project state は lifecycle 用の PluginCore lock から切り離して保存・復元する。
    // project save はユーザーデータ保護の経路なので、active 中や wrapper 再入中でも
    // committed snapshot を返せる専用 capability にしている。
    state_support: Arc<WracGainStateSupport>,
}

struct WracGainStateSupport {
    project_state: Arc<ProjectStateStore>,
    shared: Arc<SharedState>,
    gui_notifier: Arc<GuiStateNotifier>,
}

/// Host と交渉した audio layout の SoT。
///
/// この store は non-realtime 専用です。`audio_ports.get()` のような host query と
/// `configurable_audio_ports.apply()` はここを読み書きしますが、`Processor::process()` は
/// 読みません。
///
/// 製品 plugin で stereo/mono 以外の layout、sidechain、ambisonics などを扱う場合も、
/// まずはこのような layout store に「次の processor をどう作るか」を記録し、`activate()`
/// で DSP 用の immutable/snapshot state に変換するのが基本です。
struct AudioLayoutStore {
    channel_count: RwLock<u32>,
}

impl AudioLayoutStore {
    fn new(channel_count: u32) -> Self {
        Self {
            channel_count: RwLock::new(channel_count),
        }
    }

    fn channel_count(&self) -> u32 {
        *self.channel_count.read()
    }

    fn set_channel_count(&self, channel_count: u32) {
        *self.channel_count.write() = channel_count;
    }
}

struct WracGainAudioPorts {
    layout: Arc<AudioLayoutStore>,
}

/// Host からの layout 変更要求を `AudioLayoutStore` に反映する capability。
///
/// この型が `&self` で更新できるようにしているのは、adapter が capability を `Arc` で固定し、
/// `PluginCore` の `&mut self` lifecycle lock を通らずに呼ぶためです。active 中に変更してよい
/// という意味ではなく、adapter 側が Processor の有無を見て inactive 時だけ呼びます。
struct WracGainConfigurableAudioPorts {
    layout: Arc<AudioLayoutStore>,
}

/// Host-facing parameter API。
///
/// Parameter schema/value は DAW の generic editor、automation、state restore 後の rescan から
/// 並行に読まれます。ここは `SharedState` の atomic parameter SoT だけを触り、GUI runtime や
/// project-only state には入らないようにしておくと、host query が lifecycle に巻き込まれません。
struct WracGainParameters {
    shared: Arc<SharedState>,
}

/// DAW project に保存される plugin state の serialize 形式。
///
/// `serde_json` で JSON にして bytes として host に渡し、復元時は逆に読み戻す。realtime な
/// parameter は [`SharedState`] から、editor-only state は [`ProjectStateStore`] から snapshot
/// して、この保存形式へ合成する。
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SavedPluginState {
    pub(crate) gain: f32,
    #[serde(default)]
    pub(crate) bypass: bool,
    #[serde(default)]
    pub(crate) editor_page: EditorPage,
}

impl WracGainPlugin {
    pub(crate) fn new(context: PluginCoreContext) -> Self {
        let shared = Arc::new(SharedState::new());
        let audio_layout = Arc::new(AudioLayoutStore::new(2));
        let audio_ports = Arc::new(WracGainAudioPorts {
            layout: audio_layout.clone(),
        });
        let configurable_audio_ports = Arc::new(WracGainConfigurableAudioPorts {
            layout: audio_layout.clone(),
        });
        let parameters = Arc::new(WracGainParameters {
            shared: shared.clone(),
        });
        let project_state = Arc::new(ProjectStateStore::new());
        let gui = create_gui_integration(
            project_state.clone(),
            shared.clone(),
            context.host_parameter_edit_notifier,
            context.host_gui_resize_requester,
        );
        let state_support = Arc::new(WracGainStateSupport {
            project_state: project_state.clone(),
            shared: shared.clone(),
            gui_notifier: gui.notifier.clone(),
        });

        Self {
            shared,
            audio_layout,
            audio_ports,
            configurable_audio_ports,
            parameters,
            gui: gui.controller,
            state_support,
        }
    }
}

/// [`wrac_clap_adapter::export_clap_plugin!`] から呼ばれる factory 関数。
///
/// host が新しい plugin instance を必要としたタイミングで adapter が呼び出し、
/// trait object として [`PluginCore`] を返す。実装の差し替えはここを変えるだけ。
pub(crate) fn create_plugin_core(context: PluginCoreContext) -> Box<dyn PluginCore> {
    crate::logging::init_debug_logging_once(PLUGIN_DESCRIPTOR.name);

    log::debug!(
        "creating plugin core: id={}, name={}",
        PLUGIN_DESCRIPTOR.id,
        PLUGIN_DESCRIPTOR.name
    );
    for parameter in [gain_parameter_info(), bypass_parameter_info()] {
        log::info!(
            "host parameter schema: id={}, name={}, min={}, max={}, default={}, automatable={}, stepped={}, enum={}, bypass={}",
            parameter.id,
            parameter.name,
            parameter.min_value,
            parameter.max_value,
            parameter.default_value,
            parameter.flags.is_automatable,
            parameter.flags.is_stepped,
            parameter.flags.is_enum,
            parameter.flags.is_bypass
        );
    }
    Box::new(WracGainPlugin::new(context))
}

// ---------------------------------------------------------------------------
// PluginCore: plugin の lifecycle と、提供する extension の宣言
// ---------------------------------------------------------------------------
// `PluginCore` は plugin 一個分の lifecycle 全体を見る trait。
impl PluginCore for WracGainPlugin {
    /// host が audio 処理を開始する直前に呼ばれる。
    /// ここで返した `Processor` が以降 audio thread 上で `process()` される。
    fn activate(&mut self, context: ActivateContext) -> PluginResult<Box<dyn Processor>> {
        // ここが non-RT layout store と RT processor の境界です。
        //
        // adapter は Processor が存在する active 期間中の configurable-audio-ports apply を
        // 拒否します。そのため、このタイミングで snapshot した channel count は、この
        // Processor が deactivate されるまで変わらない layout 契約として扱えます。
        //
        // Processor に Arc<AudioLayoutStore> を渡すと、将来の DSP 実装者が process() 中に
        // うっかり lock を読む余地が残ります。activate() で必要な値だけを copy して渡せば、
        // audio thread 側は immutable な設定として扱えるため、realtime safety の責務が
        // コード構造として見えます。
        let audio_channel_count = self.audio_layout.channel_count();
        log::debug!(
            "activating audio processor: sample_rate={}, min_frames_count={}, max_frames_count={}, audio_channel_count={}",
            context.sample_rate,
            context.min_frames_count,
            context.max_frames_count,
            audio_channel_count
        );
        Ok(Box::new(WracGainAudioProcessor::new(
            self.shared.clone(),
            audio_channel_count,
        )))
    }

    /// host が audio 処理を停止したときに呼ばれる。
    /// `_processor` は `activate` で返した実体。drop すれば clean up される。
    fn deactivate(&mut self, _processor: Box<dyn Processor>) -> PluginResult<()> {
        log::debug!("deactivating audio processor");
        Ok(())
    }

    // 以下は CLAP の各 extension の宣言。Some を返すと「この extension を実装している」、
    // None を返すと「未対応」になる。実装本体は別 impl ブロックに書く。

    fn audio_ports(&self) -> Option<Arc<dyn PluginAudioPorts>> {
        Some(self.audio_ports.clone())
    }

    fn configurable_audio_ports(&self) -> Option<Arc<dyn PluginConfigurableAudioPorts>> {
        Some(self.configurable_audio_ports.clone())
    }

    fn parameters(&self) -> Option<Arc<dyn PluginParameters>> {
        Some(self.parameters.clone())
    }

    fn state(&self) -> Option<Arc<dyn PluginStateSupport>> {
        Some(self.state_support.clone())
    }

    fn gui(&self) -> Option<Arc<dyn PluginGui>> {
        Some(self.gui.clone())
    }
}

// ---------------------------------------------------------------------------
// PluginAudioPorts: audio 入出力 port の宣言
// ---------------------------------------------------------------------------
// gain plugin なので「main in 1 つ」「main out 1 つ」のシンプルな構成。
// channel 数は configurable audio ports 経由で host が変更できる。
impl PluginAudioPorts for WracGainAudioPorts {
    fn audio_port_count(&self, _is_input: bool) -> u32 {
        1
    }

    fn audio_port_info(&self, index: u32, is_input: bool) -> Option<AudioPortInfo> {
        let channel_count = self.layout.channel_count();
        (index == 0).then_some(if is_input {
            AudioPortInfo {
                id: 1,
                name: "Main In",
                flags: AudioPortFlags {
                    is_main: true,
                    ..AudioPortFlags::default()
                },
                channel_count,
                port_type: audio_port_type(channel_count),
                in_place_pair: None,
            }
        } else {
            AudioPortInfo {
                id: 2,
                name: "Main Out",
                flags: AudioPortFlags {
                    is_main: true,
                    ..AudioPortFlags::default()
                },
                channel_count,
                port_type: audio_port_type(channel_count),
                in_place_pair: None,
            }
        })
    }
}

// ---------------------------------------------------------------------------
// PluginConfigurableAudioPorts: host が port 構成を変えに来たときの応答
// ---------------------------------------------------------------------------
// 例えば host が「stereo から mono に切り替えたい」と提案してきたとき、
// plugin はそれを受理できるかを `can_apply_*` で答え、`apply_*` で実際に反映する。
impl PluginConfigurableAudioPorts for WracGainConfigurableAudioPorts {
    fn can_apply_audio_port_configuration(
        &self,
        requests: &[AudioPortConfigurationRequest],
    ) -> bool {
        let accepted = resolve_audio_channel_count(self.layout.channel_count(), requests);
        accepted.is_some()
    }

    fn apply_audio_port_configuration(
        &self,
        requests: &[AudioPortConfigurationRequest],
    ) -> PluginResult<()> {
        // adapter 側が Processor の存在中は configuration apply を拒否する。ここは非 RT
        // query 専用 store だけを更新し、audio thread は activate 時の snapshot を使う。
        let previous_channel_count = self.layout.channel_count();
        let channel_count =
            resolve_audio_channel_count(previous_channel_count, requests).ok_or_else(|| {
                log::warn!(
                    "rejecting unsupported audio port configuration: request_count={}, current_channel_count={}",
                    requests.len(),
                    previous_channel_count
                );
                PluginError::InvalidState
            })?;
        log::debug!(
            "applying audio port configuration: previous_channel_count={previous_channel_count}, channel_count={channel_count}"
        );
        self.layout.set_channel_count(channel_count);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// PluginParameters: parameter の宣言と現在値のやり取り
// ---------------------------------------------------------------------------
// host から見える parameter API。template 利用時に parameter を増やす場合は、
// ここが host に公開する schema と文字列表現の追加ポイントになる。
impl PluginParameters for WracGainParameters {
    fn parameter_count(&self) -> u32 {
        // 新しい parameter を追加するときは、この数と `parameter_info()` の match を
        // 一緒に更新する。
        log::debug!("parameter_count -> 2");
        2
    }

    fn parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        // 新しい parameter を追加するときは、index と stable id の対応をここに追加する。
        // id は DAW project / automation に保存されるので、一度公開した値は変えない。
        let info = match index {
            0 => Some(gain_parameter_info()),
            1 => Some(bypass_parameter_info()),
            _ => None,
        };
        log::debug!(
            "parameter_info: index={index} -> {:?}",
            info.as_ref().map(|info| (info.id, info.name))
        );
        info
    }

    /// host が「今この parameter の値はいくつ?」と尋ねてきたときに答える。
    fn parameter_value(&self, parameter_id: u32) -> PluginResult<f64> {
        match parameter_id {
            PARAM_GAIN_ID => self
                .shared
                .parameter_value(parameter_id)
                .map(gain_to_host_value)
                .ok_or(PluginError::InvalidParameter),
            PARAM_BYPASS_ID => self
                .shared
                .parameter_value(parameter_id)
                .map(|value| value as f64)
                .ok_or(PluginError::InvalidParameter),
            _ => Err(PluginError::InvalidParameter),
        }
    }

    /// host から input event として parameter 値が届いたときの経路。
    fn apply_parameter_value(&self, event: ParameterValueEvent) -> PluginResult<f64> {
        if event.parameter_id == PARAM_BYPASS_ID {
            return self
                .shared
                .set_parameter_value(event.parameter_id, event.value)
                .map(|value| value as f64)
                .ok_or(PluginError::InvalidParameter);
        }
        let value = self
            .shared
            .set_parameter_value(event.parameter_id, host_value_to_gain(event.value))
            .ok_or(PluginError::InvalidParameter)?;
        Ok(gain_to_host_value(value))
    }

    /// 内部値 → 表示文字列。例: 1.0 → "0.0 dB"。
    fn parameter_value_to_text(&self, parameter_id: u32, value: f64) -> PluginResult<String> {
        match parameter_id {
            PARAM_GAIN_ID => parameter_value_text(parameter_id, host_value_to_gain(value)),
            PARAM_BYPASS_ID => Ok(if value >= 0.5 { "On" } else { "Off" }.to_string()),
            _ => Err(PluginError::InvalidParameter),
        }
    }

    /// 表示文字列 → 内部値。ユーザーが host UI に "3 dB" のように入力したとき呼ばれる。
    fn parameter_text_to_value(&self, parameter_id: u32, text: &str) -> PluginResult<f64> {
        match parameter_id {
            PARAM_GAIN_ID => parameter_text_value(parameter_id, text)
                .map(|value| gain_to_host_value(value as f32)),
            PARAM_BYPASS_ID => match text.trim().to_ascii_lowercase().as_str() {
                "on" | "1" | "true" => Ok(1.0),
                "off" | "0" | "false" => Ok(0.0),
                _ => Err(PluginError::InvalidParameter),
            },
            _ => Err(PluginError::InvalidParameter),
        }
    }
}

// ---------------------------------------------------------------------------
// PluginStateSupport: state の保存と復元 (DAW project への persist)
// ---------------------------------------------------------------------------
// DAW がプロジェクトを保存するときに `save_state` が、開くときに `restore_state` が
// 呼ばれる。bytes フォーマットは plugin 側で自由に決められるので、ここでは
// JSON にしておく (人が読めるとデバッグが楽)。
impl PluginStateSupport for WracGainStateSupport {
    fn save_state(&self) -> PluginResult<PluginState> {
        let project = self.project_state.snapshot();
        let params = self.shared.snapshot_parameters();
        log::debug!(
            "saving plugin state: gain={}, bypass={}, editor_page={}",
            params.gain,
            params.bypass,
            project.editor_page.as_str()
        );
        let bytes = serde_json::to_vec(&SavedPluginState {
            gain: params.gain,
            bypass: params.bypass,
            editor_page: project.editor_page,
        })
        .map_err(|_| PluginError::InvalidState)?;
        Ok(PluginState { bytes })
    }

    fn restore_state(&self, state: PluginState) -> PluginResult<()> {
        log::debug!("restoring plugin state: byte_count={}", state.bytes.len());
        let state: SavedPluginState =
            serde_json::from_slice(&state.bytes).map_err(|_| PluginError::InvalidState)?;
        let project = ProjectState {
            editor_page: state.editor_page,
        };
        self.project_state.commit(project);
        self.shared.restore_parameters(ParameterStateSnapshot {
            gain: state.gain,
            bypass: state.bypass,
        });
        self.gui_notifier
            .notify_parameter(PARAM_GAIN_ID, self.shared.gain());
        self.gui_notifier
            .notify_parameter(PARAM_BYPASS_ID, f32::from(self.shared.bypass()));
        self.gui_notifier.notify_editor_page(project.editor_page);
        Ok(())
    }
}

/// gain を有効範囲に収める。外部から来た値はすべてこれを通してから使う。
pub(crate) fn clamp_gain(gain: f32) -> f32 {
    gain.clamp(MIN_GAIN, MAX_GAIN)
}

pub(crate) fn gain_parameter_info() -> ParameterInfo {
    ParameterInfo {
        id: PARAM_GAIN_ID,
        name: "Gain",
        module: "",
        min_value: 0.0,
        max_value: 1.0,
        default_value: gain_to_host_value(DEFAULT_GAIN),
        flags: ParameterFlags {
            // automation 可能であることを host に伝える。これが false だと
            // DAW で parameter を自動化できなくなる。
            is_automatable: true,
            ..ParameterFlags::default()
        },
    }
}

fn bypass_parameter_info() -> ParameterInfo {
    // 一部の host は CLAP generic editor の表示対象を compact parameter set から作る。
    // bypass parameter がないと、他に automatable parameter があっても editor に
    // 表示しない host があるため、template でも実際に効く bypass を持っておく。
    ParameterInfo {
        id: PARAM_BYPASS_ID,
        name: "Bypass",
        module: "",
        min_value: 0.0,
        max_value: 1.0,
        default_value: 0.0,
        flags: ParameterFlags {
            is_automatable: true,
            is_stepped: true,
            // Choice-style parameters should also be marked enum. Wrappers map this
            // to host-native list metadata, and some generic editors rely on it.
            is_enum: true,
            is_bypass: true,
            ..ParameterFlags::default()
        },
    }
}

/// parameter の plain value を表示文字列へ変換する。
///
/// 新しい parameter を追加するときは、この `match parameter_id` に表示変換を追加する。
/// GUI payload の `text` もこの関数を使うため、host UI と plugin GUI の表示が揃う。
pub(crate) fn parameter_value_text(parameter_id: u32, value: f64) -> PluginResult<String> {
    match parameter_id {
        PARAM_GAIN_ID => Ok(gain_db_text(clamp_gain(value as f32) as f64)),
        PARAM_BYPASS_ID => Ok(if value >= 0.5 { "On" } else { "Off" }.to_string()),
        _ => Err(PluginError::InvalidParameter),
    }
}

/// parameter の表示文字列を plain value へ戻す。
///
/// 新しい parameter を追加するときは、この `match parameter_id` に parse 処理を追加する。
pub(crate) fn parameter_default_value(parameter_id: u32) -> PluginResult<f64> {
    match parameter_id {
        PARAM_GAIN_ID => Ok(DEFAULT_GAIN as f64),
        PARAM_BYPASS_ID => Ok(0.0),
        _ => Err(PluginError::InvalidParameter),
    }
}

pub(crate) fn parameter_text_value(parameter_id: u32, text: &str) -> PluginResult<f64> {
    match parameter_id {
        PARAM_GAIN_ID => {
            let text = text.trim();
            let text = text.strip_suffix("dB").unwrap_or(text).trim();
            let db = text
                .parse::<f64>()
                .map_err(|_| PluginError::InvalidParameter)?;
            // dB → 線形 amplitude に変換してから clamp。
            Ok(clamp_gain(10.0_f64.powf(db / 20.0) as f32) as f64)
        }
        PARAM_BYPASS_ID => match text.trim().to_ascii_lowercase().as_str() {
            "on" | "1" | "true" => Ok(1.0),
            "off" | "0" | "false" => Ok(0.0),
            _ => Err(PluginError::InvalidParameter),
        },
        _ => Err(PluginError::InvalidParameter),
    }
}

pub(crate) fn parameter_host_value(parameter_id: u32, value: f32) -> PluginResult<f64> {
    match parameter_id {
        PARAM_GAIN_ID => Ok(gain_to_host_value(value)),
        PARAM_BYPASS_ID => Ok(f64::from(value >= 0.5)),
        _ => Err(PluginError::InvalidParameter),
    }
}

pub(crate) fn gain_to_host_value(gain: f32) -> f64 {
    let span = MAX_GAIN - MIN_GAIN;
    if span <= 0.0 {
        return 0.0;
    }
    ((clamp_gain(gain) - MIN_GAIN) / span) as f64
}

pub(crate) fn host_value_to_gain(value: f64) -> f64 {
    let value = value.clamp(0.0, 1.0) as f32;
    (MIN_GAIN + value * (MAX_GAIN - MIN_GAIN)) as f64
}

fn audio_port_type(channel_count: u32) -> AudioPortType {
    match channel_count {
        1 => AudioPortType::Mono,
        2 => AudioPortType::Stereo,
        _ => AudioPortType::Unspecified,
    }
}

/// host から渡された port 構成要求を解析し、受理可能なら新しい channel 数を返す。
///
/// このサンプルでは入出力が「対称な main port」のケースしか受け付けない。
/// sidechain のような非対称構成を受理してしまうと製品固有の routing 意味論が
/// 必要になり、汎用の gain サンプルでは正しく定義できないため。
fn resolve_audio_channel_count(
    current_channel_count: u32,
    requests: &[AudioPortConfigurationRequest],
) -> Option<u32> {
    let mut input_channel_count = current_channel_count;
    let mut output_channel_count = current_channel_count;
    for request in requests {
        if request.port_index != 0 {
            return None;
        }
        if !is_supported_audio_port_request(request) {
            return None;
        }
        if request.is_input {
            input_channel_count = request.channel_count;
        } else {
            output_channel_count = request.channel_count;
        }
    }

    // 入出力で channel 数が一致しているときだけ受理する。
    (input_channel_count == output_channel_count).then_some(input_channel_count)
}

fn is_supported_audio_port_request(request: &AudioPortConfigurationRequest) -> bool {
    matches!(
        (request.channel_count, request.port_type),
        (1, AudioPortType::Mono | AudioPortType::Unspecified)
            | (2, AudioPortType::Stereo | AudioPortType::Unspecified)
    )
}

/// 線形 amplitude を dB 表示の文字列に変換する。0 以下は "-inf dB"。
pub(crate) fn gain_db_text(gain: f64) -> String {
    if gain <= 0.0 {
        "-inf dB".to_string()
    } else {
        format!("{:.1} dB", 20.0 * gain.log10())
    }
}

#[cfg(test)]
mod tests {
    // 単体テスト例: `resolve_audio_channel_count` の対称性チェックなど、
    // host や CLAP runtime を立ち上げずに検証できるロジックをここに置く。

    use wrac_clap_adapter::{AudioPortConfigurationRequest, AudioPortType};

    use super::resolve_audio_channel_count;

    #[test]
    fn accepts_matching_mono_configuration() {
        let requests = [
            AudioPortConfigurationRequest {
                is_input: true,
                port_index: 0,
                channel_count: 1,
                port_type: AudioPortType::Mono,
            },
            AudioPortConfigurationRequest {
                is_input: false,
                port_index: 0,
                channel_count: 1,
                port_type: AudioPortType::Mono,
            },
        ];

        assert_eq!(resolve_audio_channel_count(2, &requests), Some(1));
    }

    #[test]
    fn rejects_mismatched_input_output_configuration() {
        let requests = [
            AudioPortConfigurationRequest {
                is_input: true,
                port_index: 0,
                channel_count: 1,
                port_type: AudioPortType::Mono,
            },
            AudioPortConfigurationRequest {
                is_input: false,
                port_index: 0,
                channel_count: 2,
                port_type: AudioPortType::Stereo,
            },
        ];

        assert_eq!(resolve_audio_channel_count(2, &requests), None);
    }
}
