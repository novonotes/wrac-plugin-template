//! WRAC Gain の host-facing な plugin core 実装。
//!
//! このファイルは host から見える plugin の契約をまとめる場所です。やっていることを
//! 大雑把に並べると:
//!
//! 1. plugin の自己紹介情報 ([`PLUGIN_DESCRIPTOR`]) を宣言する
//! 2. parameter 定義 (gain ひとつだけ) を host に教える
//! 3. audio thread / GUI / host で共有する [`SharedState`] を作る
//! 4. host から [`PluginCore::activate`] されたら audio 処理用の [`Processor`] を渡す
//! 5. host から GUI を要求されたら `gui.rs` 側の controller を渡す
//! 6. state の save/restore (DAW の project に保存) を実装する
//!
//! CLAP / VST3 / AU といった plugin format の差分は `wrac_clap_adapter` が
//! 吸収するので、ここでは「gain plugin として何を持つか」だけに集中できます。

use std::sync::Arc;

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
use crate::state::SharedState;

// plugin を識別する reverse-DNS 形式の ID。DAW が plugin を一意に判別するために
// 使うので、自分の plugin を作るときはここを必ず変更する。
pub(crate) const PLUGIN_ID: &str = "com.your-company.wrac-gain";

// 各 parameter にも host 内で一意の ID を割り当てる必要がある。
// 新しい parameter を追加するときは、ここに `PARAM_*_ID` を増やし、下の
// `PluginParameters` 実装と `SharedState` の match にも追加する。
pub(crate) const PARAM_GAIN_ID: u32 = 1;

// gain の値域。1.0 が「そのまま (0 dB)」、0.0 が「無音 (-inf dB)」、
// 2.0 が「2 倍 (+6 dB)」を表す線形 amplitude。
pub(crate) const DEFAULT_GAIN: f32 = 1.0;
pub(crate) const MIN_GAIN: f32 = 0.0;
pub(crate) const MAX_GAIN: f32 = 2.0;

// host (DAW) に plugin を自己紹介するための静的データ。
// `wrac_clap_adapter` がこれを CLAP / AUv2 の descriptor 構造体へと変換する。
pub(crate) const PLUGIN_DESCRIPTOR: PluginDescriptor = PluginDescriptor {
    id: PLUGIN_ID,
    name: "WRAC Gain",
    vendor: "Your Company",
    url: "",
    manual_url: "",
    support_url: "",
    version: "0.1.0",
    description: "Simple gain plugin",
    features: &[PluginFeature::AudioEffect, PluginFeature::Stereo],
    // AUv2 (macOS の Audio Unit v2) 用の追加情報。
    // manufacturer_code と plugin_subtype は 4 文字 ASCII の固有 ID で、
    // 同じ会社内で重複しないように決める必要がある。
    auv2: Some(Auv2Descriptor {
        manufacturer_code: *b"YrCo",
        manufacturer_name: "Your Company",
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
    // 現在の audio channel 数 (mono なら 1、stereo なら 2)。
    // configurable audio ports は adapter 側で inactive 時だけ受け付けるので、
    // audio thread と共有する atomic state にはしない。
    audio_channel_count: u32,
    // WebView による GUI を CLAP の GUI extension として扱うための helper。
    // `Arc` にしているのは host が `plugin_gui` を複数回問い合わせるため。
    gui: Arc<WxpGuiController>,
    // GUI が開いているとき、state restore など host-facing 経路から即時反映するための通知口。
    gui_notifier: Arc<GuiStateNotifier>,
}

/// DAW project に保存される plugin state の serialize 形式。
///
/// `serde_json` で JSON にして bytes として host に渡し、復元時は逆に読み戻す。
/// gain の値だけを persist する単純な構造だが、新しい parameter を追加する際は
/// この struct を拡張して `restore_state` で欠落 field を default 値に埋めると
/// 古い project ファイルとの互換性を保ちやすい。
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SavedPluginState {
    pub(crate) gain: f32,
}

impl WracGainPlugin {
    pub(crate) fn new(context: PluginCoreContext) -> Self {
        let shared = Arc::new(SharedState::new());
        let gui = create_gui_integration(shared.clone(), context.host_parameter_edit_notifier);

        Self {
            shared,
            // template の default は stereo。host が configure してくれば書き換わる。
            audio_channel_count: 2,
            gui: gui.controller,
            gui_notifier: gui.notifier,
        }
    }
}

/// [`wrac_clap_adapter::export_clap_plugin!`] から呼ばれる factory 関数。
///
/// host が新しい plugin instance を必要としたタイミングで adapter が呼び出し、
/// trait object として [`PluginCore`] を返す。実装の差し替えはここを変えるだけ。
pub(crate) fn create_plugin_core(context: PluginCoreContext) -> Box<dyn PluginCore> {
    Box::new(WracGainPlugin::new(context))
}

// ---------------------------------------------------------------------------
// PluginCore: plugin の lifecycle と、提供する extension の宣言
// ---------------------------------------------------------------------------
// `PluginCore` は plugin 一個分の lifecycle 全体を見る trait。
impl PluginCore for WracGainPlugin {
    /// host が audio 処理を開始する直前に呼ばれる。
    /// ここで返した `Processor` が以降 audio thread 上で `process()` される。
    fn activate(&mut self, _context: ActivateContext) -> PluginResult<Box<dyn Processor>> {
        Ok(Box::new(WracGainAudioProcessor::new(self.shared.clone())))
    }

    /// host が audio 処理を停止したときに呼ばれる。
    /// `_processor` は `activate` で返した実体。drop すれば clean up される。
    fn deactivate(&mut self, _processor: Box<dyn Processor>) -> PluginResult<()> {
        Ok(())
    }

    // 以下は CLAP の各 extension の宣言。Some を返すと「この extension を実装している」、
    // None を返すと「未対応」になる。実装本体は別 impl ブロックに書く。

    fn audio_ports(&self) -> Option<&dyn PluginAudioPorts> {
        Some(self)
    }

    fn configurable_audio_ports(&mut self) -> Option<&mut dyn PluginConfigurableAudioPorts> {
        Some(self)
    }

    fn parameters(&self) -> Option<&dyn PluginParameters> {
        Some(self)
    }

    fn state(&mut self) -> Option<&mut dyn PluginStateSupport> {
        Some(self)
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
impl PluginAudioPorts for WracGainPlugin {
    fn audio_port_count(&self, _is_input: bool) -> u32 {
        1
    }

    fn audio_port_info(&self, index: u32, is_input: bool) -> Option<AudioPortInfo> {
        let channel_count = self.audio_channel_count;
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
impl PluginConfigurableAudioPorts for WracGainPlugin {
    fn can_apply_audio_port_configuration(
        &self,
        requests: &[AudioPortConfigurationRequest],
    ) -> bool {
        let accepted = resolve_audio_channel_count(self.audio_channel_count, requests);
        accepted.is_some()
    }

    fn apply_audio_port_configuration(
        &mut self,
        requests: &[AudioPortConfigurationRequest],
    ) -> PluginResult<()> {
        // adapter 側が Processor の存在中は configuration apply を拒否するので、
        // core の通常 field を更新すれば後続の port 問い合わせと次回 activate に反映される。
        let channel_count = resolve_audio_channel_count(self.audio_channel_count, requests)
            .ok_or(PluginError::InvalidState)?;
        self.audio_channel_count = channel_count;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// PluginParameters: parameter の宣言と現在値のやり取り
// ---------------------------------------------------------------------------
// host から見える parameter API。template 利用時に parameter を増やす場合は、
// ここが host に公開する schema と文字列表現の追加ポイントになる。
impl PluginParameters for WracGainPlugin {
    fn parameter_count(&self) -> u32 {
        // 新しい parameter を追加するときは、この数と `parameter_info()` の match を
        // 一緒に更新する。
        1
    }

    fn parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        // 新しい parameter を追加するときは、index と stable id の対応をここに追加する。
        // id は DAW project / automation に保存されるので、一度公開した値は変えない。
        match index {
            0 => Some(gain_parameter_info()),
            _ => None,
        }
    }

    /// host が「今この parameter の値はいくつ?」と尋ねてきたときに答える。
    fn parameter_value(&self, parameter_id: u32) -> PluginResult<f64> {
        self.shared
            .parameter_value(parameter_id)
            .map(|value| value as f64)
            .ok_or(PluginError::InvalidParameter)
    }

    /// host から input event として parameter 値が届いたときの経路。
    fn apply_parameter_value(&self, event: ParameterValueEvent) -> PluginResult<f64> {
        let value = self
            .shared
            .set_parameter_value(event.parameter_id, event.value)
            .ok_or(PluginError::InvalidParameter)?;
        Ok(value as f64)
    }

    /// 内部値 → 表示文字列。例: 1.0 → "0.0 dB"。
    fn parameter_value_to_text(&self, parameter_id: u32, value: f64) -> PluginResult<String> {
        parameter_value_text(parameter_id, value)
    }

    /// 表示文字列 → 内部値。ユーザーが host UI に "3 dB" のように入力したとき呼ばれる。
    fn parameter_text_to_value(&self, parameter_id: u32, text: &str) -> PluginResult<f64> {
        parameter_text_value(parameter_id, text)
    }
}

// ---------------------------------------------------------------------------
// PluginStateSupport: state の保存と復元 (DAW project への persist)
// ---------------------------------------------------------------------------
// DAW がプロジェクトを保存するときに `save_state` が、開くときに `restore_state` が
// 呼ばれる。bytes フォーマットは plugin 側で自由に決められるので、ここでは
// JSON にしておく (人が読めるとデバッグが楽)。
impl PluginStateSupport for WracGainPlugin {
    fn save_state(&mut self) -> PluginResult<PluginState> {
        let bytes = serde_json::to_vec(&SavedPluginState {
            gain: self.shared.gain(),
        })
        .map_err(|_| PluginError::InvalidState)?;
        Ok(PluginState { bytes })
    }

    fn restore_state(&mut self, state: PluginState) -> PluginResult<()> {
        let state: SavedPluginState =
            serde_json::from_slice(&state.bytes).map_err(|_| PluginError::InvalidState)?;
        let gain = self
            .shared
            .set_parameter_value(PARAM_GAIN_ID, state.gain as f64)
            .ok_or(PluginError::InvalidParameter)?;
        self.gui_notifier.notify_parameter(PARAM_GAIN_ID, gain);
        Ok(())
    }
}

/// gain を有効範囲に収める。外部から来た値はすべてこれを通してから使う。
pub(crate) fn clamp_gain(gain: f32) -> f32 {
    gain.clamp(MIN_GAIN, MAX_GAIN)
}

fn gain_parameter_info() -> ParameterInfo {
    ParameterInfo {
        id: PARAM_GAIN_ID,
        name: "Gain",
        module: "",
        min_value: MIN_GAIN as f64,
        max_value: MAX_GAIN as f64,
        default_value: DEFAULT_GAIN as f64,
        flags: ParameterFlags {
            // automation 可能であることを host に伝える。これが false だと
            // DAW で parameter を自動化できなくなる。
            is_automatable: true,
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
        _ => Err(PluginError::InvalidParameter),
    }
}

/// parameter の表示文字列を plain value へ戻す。
///
/// 新しい parameter を追加するときは、この `match parameter_id` に parse 処理を追加する。
fn parameter_text_value(parameter_id: u32, text: &str) -> PluginResult<f64> {
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
        _ => Err(PluginError::InvalidParameter),
    }
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
