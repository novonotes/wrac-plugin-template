use std::sync::Arc;

use wrac_clap_adapter::{
    ParamFlags, ParamInfo, ParamInputEvents, PluginError, PluginParamsExtension, PluginResult,
};

use crate::state::SharedState;

// Parameter IDs are host/project ABI. Never renumber an existing id after publishing;
// add new ids instead so saved automation keeps targeting the same control.
pub(crate) const PARAM_BYPASS_ID: u32 = 0;
pub(crate) const PARAM_GAIN_ID: u32 = 1;

// Gain is a linear amplitude. 1.0 = 0 dB (unity), 0.0 = silence, 2.0 = +6 dB.
pub(crate) const MIN_GAIN: f32 = 0.0;
pub(crate) const MAX_GAIN: f32 = 2.0;
pub(crate) const DEFAULT_GAIN: f32 = 1.0;
const DEFAULT_GAIN_HOST_VALUE: f64 = ((DEFAULT_GAIN - MIN_GAIN) / (MAX_GAIN - MIN_GAIN)) as f64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParameterKind {
    Bypass,
    Gain,
}

#[derive(Debug, Clone, Copy)]
struct ParameterSpec {
    kind: ParameterKind,
    info: ParamInfo,
    // CLAP exposes normalized host-domain values, while the DSP and GUI use plain
    // product-domain values. Keeping both domains in the spec prevents host metadata,
    // GUI mapping, defaults, and DSP ranges from drifting inside the Rust contract.
    plain_min: f64,
    plain_max: f64,
    plain_default: f64,
    // Only parameters with a GUI role are sent through periodic GUI sync. A host-visible
    // parameter can stay out of WebView traffic if the custom GUI has no control for it.
    gui_role: Option<&'static str>,
}

// This is the host-facing publication table for parameters. Adding a parameter should
// start here; count, get_info, lookup-based conversions, defaults, and GUI sync all
// derive from this table instead of parallel matches.
const PARAM_SPECS: &[ParameterSpec] = &[
    ParameterSpec {
        kind: ParameterKind::Bypass,
        info: ParamInfo {
            id: PARAM_BYPASS_ID,
            name: "Bypass",
            module: "",
            min_value: 0.0,
            max_value: 1.0,
            default_value: 0.0,
            flags: param_flags(true, true, true, true),
        },
        plain_min: 0.0,
        plain_max: 1.0,
        plain_default: 0.0,
        // Keep bypass host-visible and enum-shaped so wrappers/hosts can expose native
        // bypass controls instead of treating it as an ordinary continuous parameter.
        gui_role: None,
    },
    ParameterSpec {
        kind: ParameterKind::Gain,
        info: ParamInfo {
            id: PARAM_GAIN_ID,
            name: "Gain",
            module: "",
            min_value: 0.0,
            max_value: 1.0,
            default_value: DEFAULT_GAIN_HOST_VALUE,
            flags: param_flags(true, false, false, false),
        },
        plain_min: MIN_GAIN as f64,
        plain_max: MAX_GAIN as f64,
        plain_default: DEFAULT_GAIN as f64,
        // Gain is the template's primary editable GUI control.
        gui_role: Some("gain"),
    },
];

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
        PARAM_SPECS.len() as u32
    }

    fn param_info(&self, index: u32) -> Option<ParamInfo> {
        PARAM_SPECS.get(index as usize).map(|spec| spec.info)
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

    /// Called when parameter values arrive from the host as input events.
    fn apply_param_events(&self, events: ParamInputEvents<'_>) -> PluginResult<()> {
        for event in events.values() {
            let Ok(plain_value) = parameter_host_input_to_plain(event.param_id, event.value) else {
                wrac_log::rtwarn!(
                    "params.flush: ignoring invalid parameter input param_id={} value={}",
                    event.param_id,
                    event.value
                );
                continue;
            };
            if self
                .shared
                .set_parameter_value(event.param_id, plain_value)
                .is_none()
            {
                wrac_log::rtwarn!(
                    "params.flush: ignoring unknown parameter input param_id={} value={}",
                    event.param_id,
                    event.value
                );
            }
        }
        Ok(())
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

fn plain_to_host(spec: &ParameterSpec, value: f32) -> f64 {
    match spec.kind {
        ParameterKind::Gain => gain_to_host_value(value),
        ParameterKind::Bypass => f64::from(value >= 0.5),
    }
}

fn host_to_plain(spec: &ParameterSpec, value: f64) -> f64 {
    match spec.kind {
        ParameterKind::Gain => host_value_to_gain(value),
        ParameterKind::Bypass => f64::from(value >= 0.5),
    }
}

fn value_to_text(spec: &ParameterSpec, value: f64) -> PluginResult<String> {
    match spec.kind {
        ParameterKind::Gain => Ok(gain_db_text(clamp_gain(value as f32) as f64)),
        ParameterKind::Bypass => Ok(if value >= 0.5 { "On" } else { "Off" }.to_string()),
    }
}

fn text_to_plain(spec: &ParameterSpec, text: &str) -> PluginResult<f64> {
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

fn param_spec(parameter_id: u32) -> PluginResult<&'static ParameterSpec> {
    // Host callbacks address parameters by stable id, not by table index. Always look up
    // by id after discovery so inserting a new parameter does not silently reroute edits.
    PARAM_SPECS
        .iter()
        .find(|spec| spec.info.id == parameter_id)
        .ok_or(PluginError::InvalidParameter)
}

/// Clamps gain to the valid range. All externally supplied values must pass through this.
pub(crate) fn clamp_gain(gain: f32) -> f32 {
    let spec = gain_spec();
    gain.clamp(spec.plain_min as f32, spec.plain_max as f32)
}

pub(crate) fn parameter_infos() -> impl Iterator<Item = ParamInfo> {
    PARAM_SPECS.iter().map(|spec| spec.info)
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
    // GUI refresh follows the Rust parameter table's GUI role marker. The custom GUI
    // still maps the gain control explicitly in TypeScript, but Rust owns the list of
    // parameters worth pushing over the WebView channel.
    for spec in PARAM_SPECS.iter().filter(|spec| spec.gui_role.is_some()) {
        if let Some(value) = shared.parameter_value(spec.info.id) {
            notify(spec.info.id, value);
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

fn gain_spec() -> &'static ParameterSpec {
    PARAM_SPECS
        .iter()
        .find(|spec| spec.info.id == PARAM_GAIN_ID)
        .expect("PARAM_GAIN_ID must be present in PARAM_SPECS")
}
