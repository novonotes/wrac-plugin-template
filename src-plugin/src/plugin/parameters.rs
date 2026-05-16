use std::sync::Arc;

use wrac_clap_adapter::{
    ParameterFlags, ParameterInfo, ParameterValueEvent, PluginError, PluginParameters, PluginResult,
};

use crate::state::SharedState;

// parameter ID は host が automation / project 保存に使う安定値。一度公開したら変えない。
// 新しい parameter を追加するとき: ここに ID を足し、`PluginParameters` 実装と
// `SharedState` の match を揃える。
pub(crate) const PARAM_GAIN_ID: u32 = 1;
pub(crate) const PARAM_BYPASS_ID: u32 = 9;

// gain は線形 amplitude。1.0 = 0 dB (素通し)、0.0 = 無音、2.0 = +6 dB。
pub(crate) const DEFAULT_GAIN: f32 = 1.0;
pub(crate) const MIN_GAIN: f32 = 0.0;
pub(crate) const MAX_GAIN: f32 = 2.0;

/// host から見える parameter API。
///
/// schema / 値は generic editor・automation・restore 後の rescan から並行に読まれる。
/// [`SharedState`] の atomic SoT だけを触り、GUI runtime や project state には
/// 踏み込まないことで、host query を lifecycle と切り離している。
pub(super) struct WracGainParameters {
    shared: Arc<SharedState>,
}

impl WracGainParameters {
    pub(super) fn new(shared: Arc<SharedState>) -> Self {
        Self { shared }
    }
}

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

pub(crate) fn bypass_parameter_info() -> ParameterInfo {
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

/// 線形 amplitude を dB 表示の文字列に変換する。0 以下は "-inf dB"。
pub(crate) fn gain_db_text(gain: f64) -> String {
    if gain <= 0.0 {
        "-inf dB".to_string()
    } else {
        format!("{:.1} dB", 20.0 * gain.log10())
    }
}
