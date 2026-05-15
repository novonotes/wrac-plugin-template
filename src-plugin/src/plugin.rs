//! host から見える plugin の契約をまとめる場所。
//!
//! ここで宣言するもの:
//! 1. plugin の自己紹介 ([`PLUGIN_DESCRIPTOR`])
//! 2. parameter 定義 (gain / bypass)
//! 3. audio / GUI / host で共有する [`SharedState`]
//! 4. activate 時に渡す audio [`Processor`]、GUI controller、state の save/restore
//!
//! CLAP / VST3 / AU の format 差分は `wrac_clap_adapter` が吸収するので、ここは
//! 「この plugin が何を持つか」だけに集中できる。新しい parameter を足すときの
//! 変更箇所は、本ファイル内の `新しい parameter を追加するとき` コメントを辿る。

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

// plugin identity の SoT は src-plugin/Cargo.toml の [package.metadata.wrac]。
// GUI / xtask / wrapper build も同じ metadata を読むので、ここを直書きせず
// env! 経由にすることで rename 時の不整合 (bundle 名や About 表示のズレ) を防ぐ。
pub(crate) const PLUGIN_ID: &str = env!("WRAC_PLUGIN_ID");
pub(crate) const PLUGIN_NAME: &str = env!("WRAC_PLUGIN_NAME");
pub(crate) const COMPANY_NAME: &str = env!("WRAC_COMPANY_NAME");
const AUV2_TYPE: [u8; 4] = four_char_code(env!("WRAC_AUV2_TYPE"));
const AUV2_SUBTYPE: [u8; 4] = four_char_code(env!("WRAC_AUV2_SUBTYPE"));
const AUV2_MANUFACTURER_CODE: [u8; 4] = four_char_code(env!("WRAC_AUV2_MANUFACTURER_CODE"));

// parameter ID は host が automation / project 保存に使う安定値。一度公開したら変えない。
// 新しい parameter を追加するとき: ここに ID を足し、`PluginParameters` 実装と
// `SharedState` の match を揃える。
pub(crate) const PARAM_GAIN_ID: u32 = 1;
pub(crate) const PARAM_BYPASS_ID: u32 = 9;

// gain は線形 amplitude。1.0 = 0 dB (素通し)、0.0 = 無音、2.0 = +6 dB。
pub(crate) const DEFAULT_GAIN: f32 = 1.0;
pub(crate) const MIN_GAIN: f32 = 0.0;
pub(crate) const MAX_GAIN: f32 = 2.0;

// host への自己紹介。adapter がこれを CLAP / AUv2 の descriptor に変換する。
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
    // AUv2 (macOS Audio Unit v2) 用。code 類は 4 文字 ASCII の固有 ID で、
    // 同じ会社の他 plugin と重複させない。
    auv2: Some(Auv2Descriptor {
        manufacturer_code: AUV2_MANUFACTURER_CODE,
        manufacturer_name: COMPANY_NAME,
        plugin_type: AUV2_TYPE,
        plugin_subtype: AUV2_SUBTYPE,
    }),
};

const fn four_char_code(value: &str) -> [u8; 4] {
    let bytes = value.as_bytes();
    if bytes.len() != 4 {
        panic!("AUv2 code must be exactly 4 ASCII bytes");
    }
    [bytes[0], bytes[1], bytes[2], bytes[3]]
}

/// plugin 1 instance。host が plugin を読み込むごとに 1 つ作られる。
///
/// audio 処理本体は [`PluginCore::activate`] で [`Processor`] に切り出すので、
/// この struct 自体は lifecycle と host へ公開する extension の保持だけを担う。
///
/// extension capability を `Arc` で持つのは、host (wrapper) が lifecycle callback の
/// 最中に capability を再入 query してくるため。`PluginCore` の `&mut self` lock を
/// 取らずに到達できる必要がある。
pub(crate) struct WracGainPlugin {
    // audio / GUI / host が共有する parameter state。詳細は [`SharedState`]。
    shared: Arc<SharedState>,
    // host と交渉した audio layout。non-realtime 専用。詳細は [`AudioLayoutStore`]。
    audio_layout: Arc<AudioLayoutStore>,
    audio_ports: Arc<WracGainAudioPorts>,
    configurable_audio_ports: Arc<WracGainConfigurableAudioPorts>,
    parameters: Arc<WracGainParameters>,
    gui: Arc<WxpGuiController>,
    // project state の save/restore。active 中や wrapper 再入中でも committed
    // snapshot を返せるよう、lifecycle lock から独立した専用 capability にしている。
    state_support: Arc<WracGainStateSupport>,
}

struct WracGainStateSupport {
    project_state: Arc<ProjectStateStore>,
    shared: Arc<SharedState>,
    gui_notifier: Arc<GuiStateNotifier>,
}

/// host と交渉した audio layout の SoT。**non-realtime 専用**。
///
/// host の port query と configurable-audio-ports apply がここを読み書きするが、
/// `Processor::process()` は読まない。audio thread から RwLock を読むと priority
/// inversion を招くため、layout は「次に activate する processor の設定」として扱い、
/// `activate()` で snapshot して渡す ([`WracGainAudioProcessor`] 参照)。sidechain や
/// ambisonics など複雑な layout でも、この「store に記録 → activate で snapshot」の
/// 形は同じ。
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

/// host からの layout 変更要求を [`AudioLayoutStore`] に反映する capability。
///
/// `&self` で更新するのは、adapter が `&mut self` lock を通らずに呼ぶため
/// ([`WracGainPlugin`] 参照)。active 中に変えてよい訳ではなく、adapter が
/// Processor 不在 (inactive) のときだけ呼ぶことで安全を保証している。
struct WracGainConfigurableAudioPorts {
    layout: Arc<AudioLayoutStore>,
}

/// host から見える parameter API。
///
/// schema / 値は generic editor・automation・restore 後の rescan から並行に読まれる。
/// [`SharedState`] の atomic SoT だけを触り、GUI runtime や project state には
/// 踏み込まないことで、host query を lifecycle と切り離している。
struct WracGainParameters {
    shared: Arc<SharedState>,
}

/// DAW project に保存する plugin state の serialize 形式 (JSON)。
///
/// realtime parameter は [`SharedState`] から、editor-only state は
/// [`ProjectStateStore`] から snapshot し、この 1 形式に合成して host へ渡す。
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

/// [`wrac_clap_adapter::export_clap_plugin!`] から呼ばれる factory。
/// host が instance を要求するたびに呼ばれ、[`PluginCore`] を返す。
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
impl PluginCore for WracGainPlugin {
    /// host が audio 処理を開始する直前に呼ばれる。
    /// ここで返した [`Processor`] が以降 audio thread 上で `process()` される。
    fn activate(&mut self, context: ActivateContext) -> PluginResult<Box<dyn Processor>> {
        // non-RT layout store と RT processor の境界。
        //
        // adapter は active 中の layout apply を拒否するので、ここで snapshot した
        // channel count は deactivate まで不変な契約になる。`Arc<AudioLayoutStore>`
        // ごと渡すと process() 中に lock を読む余地が残るため、必要な値だけ copy して
        // 渡す。これで「audio thread は immutable な設定だけを見る」が構造で保証される。
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

    /// host が audio 処理を停止したときに呼ばれる。`_processor` は `activate` で
    /// 返した実体で、drop されれば後始末は済む。
    fn deactivate(&mut self, _processor: Box<dyn Processor>) -> PluginResult<()> {
        log::debug!("deactivating audio processor");
        Ok(())
    }

    // 各 extension の宣言。Some = 実装あり / None = 未対応。本体は別 impl ブロック。

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
// gain なので main in / main out が 1 つずつ。channel 数は configurable audio
// ports 経由で host が変更できる。
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
// 例: host が stereo→mono を提案 → 受理可否を `can_apply_*` で答え、
// 実反映を `apply_*` で行う。
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
// 新しい parameter を追加するときの host 公開ポイント (schema と文字列表現)。
impl PluginParameters for WracGainParameters {
    fn parameter_count(&self) -> u32 {
        // 新しい parameter を追加するとき: この数と `parameter_info()` の match を揃える。
        log::debug!("parameter_count -> 2");
        2
    }

    fn parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        // index ↔ stable id の対応表。id は project/automation に残るので変えない。
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
// project 保存で `save_state`、復元で `restore_state`。bytes 形式は自由なので、
// デバッグしやすい JSON にしている。
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
            // false にすると DAW で automation できなくなる。
            is_automatable: true,
            ..ParameterFlags::default()
        },
    }
}

fn bypass_parameter_info() -> ParameterInfo {
    // 一部の host は bypass parameter が無いと generic editor に他の parameter も
    // 出さない。テンプレートでも実際に効く bypass を 1 つ持たせておく。
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
            // 選択肢型は enum も立てる。wrapper が host ネイティブの list
            // metadata に変換し、一部の generic editor がそれに依存する。
            is_enum: true,
            is_bypass: true,
            ..ParameterFlags::default()
        },
    }
}

/// plain value → 表示文字列。GUI payload の `text` もこれを通すので、host UI と
/// plugin GUI の表示が必ず揃う。新しい parameter は match に追加する。
pub(crate) fn parameter_value_text(parameter_id: u32, value: f64) -> PluginResult<String> {
    match parameter_id {
        PARAM_GAIN_ID => Ok(gain_db_text(clamp_gain(value as f32) as f64)),
        PARAM_BYPASS_ID => Ok(if value >= 0.5 { "On" } else { "Off" }.to_string()),
        _ => Err(PluginError::InvalidParameter),
    }
}

/// parameter の default 値 (plain value)。reset 機能などが使う。
/// 新しい parameter は match に追加する。
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

/// port 構成要求を解析し、受理できるなら新しい channel 数を返す。
///
/// 入出力が対称な main port のみ受理する。sidechain のような非対称構成は
/// 製品固有の routing 意味論が必要で、汎用 gain サンプルでは定義できないため。
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
    // host や CLAP runtime 無しで検証できる純粋ロジックの単体テスト例。

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
