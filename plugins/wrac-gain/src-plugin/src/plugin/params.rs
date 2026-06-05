use std::sync::Arc;

use wrac_clap_adapter::{
    ParamFlags, ParamInfo, ParamValueEvent, PluginError, PluginParamsExtension, PluginResult,
};
use wrac_gain_parameter_contract::{PARAMETER_SPECS, ParameterContract, ParameterKind};

use crate::state::SharedState;

pub(crate) use wrac_gain_parameter_contract::{DEFAULT_GAIN, PARAM_BYPASS_ID, PARAM_GAIN_ID};

const fn param_flags(
    is_automatable: bool,
    is_stepped: bool,
    is_enum: bool,
    is_bypass: bool,
) -> ParamFlags {
    ParamFlags {
        is_stepped,
        is_periodic: false,
        is_hidden: false,
        is_readonly: false,
        is_bypass,
        is_automatable,
        is_automatable_per_note_id: false,
        is_automatable_per_key: false,
        is_automatable_per_channel: false,
        is_automatable_per_port: false,
        is_modulatable: false,
        is_modulatable_per_note_id: false,
        is_modulatable_per_key: false,
        is_modulatable_per_channel: false,
        is_modulatable_per_port: false,
        requires_process: false,
        is_enum,
    }
}

/// The parameter API as seen by the host.
///
/// Schema and values are read concurrently from generic editors, automation, and post-restore
/// rescans. Touching only the atomic source of truth in [`SharedState`] - without reaching
/// into the GUI runtime or project state - decouples host queries from the plugin lifecycle.
pub(super) struct WracGainParamsExtension {
    shared: Arc<SharedState>,
}

impl WracGainParamsExtension {
    pub(super) fn new(shared: Arc<SharedState>) -> Self {
        Self { shared }
    }
}

impl PluginParamsExtension for WracGainParamsExtension {
    fn param_count(&self) -> u32 {
        PARAMETER_SPECS.len() as u32
    }

    fn param_info(&self, index: u32) -> Option<ParamInfo> {
        PARAMETER_SPECS.get(index as usize).map(param_info)
    }

    /// Answers the host's query for the current value of a parameter.
    fn param_value(&self, param_id: u32) -> PluginResult<f64> {
        let spec = param_spec(param_id)?;
        let value = self
            .shared
            .parameter_value(param_id)
            .ok_or(PluginError::InvalidParameter)?;
        Ok(plain_to_host(spec, value))
    }

    /// Called when a parameter value arrives from the host as an input event.
    fn apply_param_value(&self, event: ParamValueEvent) -> PluginResult<f64> {
        let plain_value = parameter_host_input_to_plain(event.param_id, event.value)?;
        let applied = self
            .shared
            .set_parameter_value(event.param_id, plain_value)
            .ok_or(PluginError::InvalidParameter)?;
        parameter_host_value(event.param_id, applied)
    }

    /// Converts a host-domain value to a display string. Example: 0.5 -> "0.0 dB".
    fn value_to_text(&self, param_id: u32, value: f64) -> PluginResult<String> {
        let spec = param_spec(param_id)?;
        value_to_text(spec, host_to_plain(spec, value))
    }

    /// Converts a display string to a host-domain value. Called when the user types "3 dB" into the host UI.
    fn text_to_value(&self, param_id: u32, text: &str) -> PluginResult<f64> {
        let spec = param_spec(param_id)?;
        let plain_value = text_to_plain(spec, text)?;
        Ok(plain_to_host(spec, plain_value as f32))
    }
}

fn param_info(spec: &ParameterContract) -> ParamInfo {
    ParamInfo {
        id: spec.id,
        name: spec.name,
        module: "",
        min_value: spec.host_min,
        max_value: spec.host_max,
        default_value: spec.host_default,
        flags: param_flags(spec.automatable, spec.stepped, spec.is_enum, spec.is_bypass),
    }
}

fn plain_to_host(spec: &ParameterContract, value: f32) -> f64 {
    match spec.kind {
        ParameterKind::Gain => gain_to_host_value(value),
        ParameterKind::Bypass => f64::from(value >= 0.5),
    }
}

fn host_to_plain(spec: &ParameterContract, value: f64) -> f64 {
    match spec.kind {
        ParameterKind::Gain => host_value_to_gain(value),
        ParameterKind::Bypass => f64::from(value >= 0.5),
    }
}

fn value_to_text(spec: &ParameterContract, value: f64) -> PluginResult<String> {
    match spec.kind {
        ParameterKind::Gain => Ok(gain_db_text(clamp_gain(value as f32) as f64)),
        ParameterKind::Bypass => Ok(if value >= 0.5 { "On" } else { "Off" }.to_string()),
    }
}

fn text_to_plain(spec: &ParameterContract, text: &str) -> PluginResult<f64> {
    match spec.kind {
        ParameterKind::Gain => {
            let text = text.trim();
            let text = text.strip_suffix("dB").unwrap_or(text).trim();
            let db = text
                .parse::<f64>()
                .map_err(|_| PluginError::InvalidParameter)?;
            Ok(clamp_gain(10.0_f64.powf(db / 20.0) as f32) as f64)
        }
        ParameterKind::Bypass => match text.trim().to_ascii_lowercase().as_str() {
            "on" | "1" | "true" => Ok(1.0),
            "off" | "0" | "false" => Ok(0.0),
            _ => Err(PluginError::InvalidParameter),
        },
    }
}

fn param_spec(parameter_id: u32) -> PluginResult<&'static ParameterContract> {
    // Host callbacks address parameters by stable id, not by table index. Always look up
    // by id after discovery so inserting a new parameter does not silently reroute edits.
    PARAMETER_SPECS
        .iter()
        .find(|spec| spec.id == parameter_id)
        .ok_or(PluginError::InvalidParameter)
}

/// Clamps gain to the valid range. All externally supplied values must pass through this.
pub(crate) fn clamp_gain(gain: f32) -> f32 {
    let spec = gain_spec();
    gain.clamp(spec.plain_min as f32, spec.plain_max as f32)
}

pub(crate) fn parameter_infos() -> impl Iterator<Item = ParamInfo> {
    PARAMETER_SPECS.iter().map(param_info)
}

/// Converts a plain value to a display string. GUI payloads route through here too, so
/// the host UI and plugin GUI always show the same text.
pub(crate) fn parameter_value_text(parameter_id: u32, value: f64) -> PluginResult<String> {
    value_to_text(param_spec(parameter_id)?, value)
}

/// Default value (plain value) for a parameter. Used by reset features, etc.
pub(crate) fn parameter_default_value(parameter_id: u32) -> PluginResult<f64> {
    Ok(param_spec(parameter_id)?.plain_default)
}

pub(crate) fn parameter_text_value(parameter_id: u32, text: &str) -> PluginResult<f64> {
    text_to_plain(param_spec(parameter_id)?, text)
}

pub(crate) fn parameter_host_value(parameter_id: u32, value: f32) -> PluginResult<f64> {
    Ok(plain_to_host(param_spec(parameter_id)?, value))
}

pub(crate) fn parameter_host_input_to_plain(parameter_id: u32, value: f64) -> PluginResult<f64> {
    Ok(host_to_plain(param_spec(parameter_id)?, value))
}

pub(crate) fn notify_gui_parameters(shared: &SharedState, mut notify: impl FnMut(u32, f32)) {
    // GUI refresh follows the generated TypeScript contract: only parameters with a
    // GUI role are visible controls, while host-only parameters stay out of WebView traffic.
    for spec in PARAMETER_SPECS
        .iter()
        .filter(|spec| spec.gui_role.is_some())
    {
        if let Some(value) = shared.parameter_value(spec.id) {
            notify(spec.id, value);
        }
    }
}

pub(crate) fn gain_to_host_value(gain: f32) -> f64 {
    let spec = gain_spec();
    let min = spec.plain_min as f32;
    let max = spec.plain_max as f32;
    let span = max - min;
    if span <= 0.0 {
        return 0.0;
    }
    ((clamp_gain(gain) - min) / span) as f64
}

pub(crate) fn host_value_to_gain(value: f64) -> f64 {
    let spec = gain_spec();
    let value = value.clamp(0.0, 1.0) as f32;
    let min = spec.plain_min as f32;
    let max = spec.plain_max as f32;
    (min + value * (max - min)) as f64
}

/// Converts a linear amplitude to a dB display string. Values <= 0 return "-inf dB".
pub(crate) fn gain_db_text(gain: f64) -> String {
    if gain <= 0.0 {
        "-inf dB".to_string()
    } else {
        format!("{:.1} dB", 20.0 * gain.log10())
    }
}

fn gain_spec() -> &'static ParameterContract {
    PARAMETER_SPECS
        .iter()
        .find(|spec| spec.id == PARAM_GAIN_ID)
        .expect("PARAM_GAIN_ID must be present in PARAMETER_SPECS")
}
