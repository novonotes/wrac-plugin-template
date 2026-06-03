use std::sync::Arc;

use wrac_clap_adapter::{
    ParamFlags, ParamInfo, ParamValueEvent, PluginError, PluginParamsExtension, PluginResult,
};

use crate::state::SharedState;

// Parameter IDs are stable values used by the host for automation and project saving.
// Never change them after publishing. To add a new parameter: append an ID here and
// keep the `PluginParamsExtension` impl and `SharedState` match arms in sync.
pub(crate) const PARAM_GAIN_ID: u32 = 1;
pub(crate) const PARAM_BYPASS_ID: u32 = 9;

// Gain is a linear amplitude. 1.0 = 0 dB (unity), 0.0 = silence, 2.0 = +6 dB.
pub(crate) const DEFAULT_GAIN: f32 = 1.0;
pub(crate) const MIN_GAIN: f32 = 0.0;
pub(crate) const MAX_GAIN: f32 = 2.0;

/// The parameter API as seen by the host.
///
/// Schema and values are read concurrently from generic editors, automation, and post-restore
/// rescans. Touching only the atomic source of truth in [`SharedState`] — without reaching
/// into the GUI runtime or project state — decouples host queries from the plugin lifecycle.
pub(super) struct WracGainParamsExtension {
    shared: Arc<SharedState>,
}

impl WracGainParamsExtension {
    pub(super) fn new(shared: Arc<SharedState>) -> Self {
        Self { shared }
    }
}

// The host-facing publication point for new parameters (schema and string representation).
impl PluginParamsExtension for WracGainParamsExtension {
    fn param_count(&self) -> u32 {
        // When adding a new parameter: keep this count in sync with the `param_info()` match.
        2
    }

    fn param_info(&self, index: u32) -> Option<ParamInfo> {
        // Mapping of sequential index to stable ID. IDs persist in project/automation data — never change them.
        match index {
            0 => Some(gain_param_info()),
            1 => Some(bypass_param_info()),
            _ => None,
        }
    }

    /// Answers the host's query for the current value of a parameter.
    fn param_value(&self, param_id: u32) -> PluginResult<f64> {
        match param_id {
            PARAM_GAIN_ID => self
                .shared
                .parameter_value(param_id)
                .map(gain_to_host_value)
                .ok_or(PluginError::InvalidParameter),
            PARAM_BYPASS_ID => self
                .shared
                .parameter_value(param_id)
                .map(|value| value as f64)
                .ok_or(PluginError::InvalidParameter),
            _ => Err(PluginError::InvalidParameter),
        }
    }

    /// Called when a parameter value arrives from the host as an input event.
    fn apply_param_value(&self, event: ParamValueEvent) -> PluginResult<f64> {
        if event.param_id == PARAM_BYPASS_ID {
            return self
                .shared
                .set_parameter_value(event.param_id, event.value)
                .map(|value| value as f64)
                .ok_or(PluginError::InvalidParameter);
        }
        let value = self
            .shared
            .set_parameter_value(event.param_id, host_value_to_gain(event.value))
            .ok_or(PluginError::InvalidParameter)?;
        Ok(gain_to_host_value(value))
    }

    /// Converts an internal value to a display string. Example: 1.0 → "0.0 dB".
    fn value_to_text(&self, param_id: u32, value: f64) -> PluginResult<String> {
        match param_id {
            PARAM_GAIN_ID => parameter_value_text(param_id, host_value_to_gain(value)),
            PARAM_BYPASS_ID => Ok(if value >= 0.5 { "On" } else { "Off" }.to_string()),
            _ => Err(PluginError::InvalidParameter),
        }
    }

    /// Converts a display string to an internal value. Called when the user types "3 dB" into the host UI.
    fn text_to_value(&self, param_id: u32, text: &str) -> PluginResult<f64> {
        match param_id {
            PARAM_GAIN_ID => {
                parameter_text_value(param_id, text).map(|value| gain_to_host_value(value as f32))
            }
            PARAM_BYPASS_ID => match text.trim().to_ascii_lowercase().as_str() {
                "on" | "1" | "true" => Ok(1.0),
                "off" | "0" | "false" => Ok(0.0),
                _ => Err(PluginError::InvalidParameter),
            },
            _ => Err(PluginError::InvalidParameter),
        }
    }
}

/// Clamps gain to the valid range. All externally supplied values must pass through this.
pub(crate) fn clamp_gain(gain: f32) -> f32 {
    gain.clamp(MIN_GAIN, MAX_GAIN)
}

pub(crate) fn gain_param_info() -> ParamInfo {
    ParamInfo {
        id: PARAM_GAIN_ID,
        name: "Gain",
        module: "",
        min_value: 0.0,
        max_value: 1.0,
        default_value: gain_to_host_value(DEFAULT_GAIN),
        flags: ParamFlags {
            // Setting this false prevents automation in the DAW.
            is_automatable: true,
            ..ParamFlags::default()
        },
    }
}

pub(crate) fn bypass_param_info() -> ParamInfo {
    // Some hosts suppress all parameters in their generic editor if there is no bypass
    // parameter. Include a working bypass even in the template.
    ParamInfo {
        id: PARAM_BYPASS_ID,
        name: "Bypass",
        module: "",
        min_value: 0.0,
        max_value: 1.0,
        default_value: 0.0,
        flags: ParamFlags {
            is_automatable: true,
            is_stepped: true,
            // Also set the enum flag for stepped choice params. The wrapper converts
            // this to the host's native list metadata, which some generic editors rely on.
            is_enum: true,
            is_bypass: true,
            ..ParamFlags::default()
        },
    }
}

/// Converts a plain value to a display string. GUI payloads route through here too, so
/// the host UI and plugin GUI always show the same text. Add new parameters to the match arm.
pub(crate) fn parameter_value_text(parameter_id: u32, value: f64) -> PluginResult<String> {
    match parameter_id {
        PARAM_GAIN_ID => Ok(gain_db_text(clamp_gain(value as f32) as f64)),
        PARAM_BYPASS_ID => Ok(if value >= 0.5 { "On" } else { "Off" }.to_string()),
        _ => Err(PluginError::InvalidParameter),
    }
}

/// Default value (plain value) for a parameter. Used by reset features, etc.
/// Add new parameters to the match arm.
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
            // Convert dB to linear amplitude, then clamp.
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

/// Converts a linear amplitude to a dB display string. Values ≤ 0 return "-inf dB".
pub(crate) fn gain_db_text(gain: f64) -> String {
    if gain <= 0.0 {
        "-inf dB".to_string()
    } else {
        format!("{:.1} dB", 20.0 * gain.log10())
    }
}
