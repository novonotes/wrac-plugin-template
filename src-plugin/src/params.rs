//! Parameter exposure and state save/restore.
//!
//! In CLAP, a plugin exposes values to the host as "parameters".
//! The host uses these for automation recording, preset management, and GUI display.
//!
//! PluginStateImpl serializes plugin state as a byte stream so it can be saved
//! to and restored from a DAW project file.

use std::ffi::CStr;
use std::fmt::Write as _;
use std::io::{Read, Write as _};

use clack_extensions::params::{
    ParamDisplayWriter, ParamInfo, ParamInfoFlags, ParamInfoWriter, PluginMainThreadParams,
};
use clack_extensions::state::PluginStateImpl;
use clack_plugin::events::event_types::{
    ParamGestureBeginEvent, ParamGestureEndEvent, ParamValueEvent,
};
use clack_plugin::events::spaces::CoreEventSpace;
use clack_plugin::prelude::*;
use clack_plugin::stream::{InputStream, OutputStream};
use serde_json::from_slice;
use serde_json::to_vec;

use crate::plugin::{
    DEFAULT_GAIN, PARAM_GAIN_ID, SavedPluginState, SharedStateInner, WxpExampleGainMainThread,
    clamp_gain, gain_db_text,
};

/// Plugin state save/restore (CLAP state extension).
/// Called when the DAW saves or loads a project.
/// The format is flexible; here we use `[length (4-byte LE)] + [JSON bytes]`.
impl PluginStateImpl for WxpExampleGainMainThread<'_> {
    fn save(&mut self, output: &mut OutputStream) -> Result<(), PluginError> {
        let bytes = to_vec(&SavedPluginState {
            gain: self.shared.inner.gain(),
        })
        .map_err(|_| PluginError::Message("Failed to serialize plugin state"))?;

        // The length prefix allows safe reading even if more fields are added in the future.
        output.write_all(&(bytes.len() as u32).to_le_bytes())?;
        output.write_all(&bytes)?;
        Ok(())
    }

    fn load(&mut self, input: &mut InputStream) -> Result<(), PluginError> {
        let mut len_buffer = [0_u8; 4];
        input.read_exact(&mut len_buffer)?;
        let len = u32::from_le_bytes(len_buffer) as usize;

        let mut bytes = vec![0_u8; len];
        input.read_exact(&mut bytes)?;

        let state: SavedPluginState = from_slice(&bytes)
            .map_err(|_| PluginError::Message("Failed to deserialize plugin state"))?;
        // Apply the restored value to SharedState and notify the GUI.
        self.shared.inner.set_gain_from_host(state.gain as f64);
        Ok(())
    }
}

/// Main thread implementation of CLAP parameters.
/// Used by the host to list parameters and read/write values and text representations.
impl PluginMainThreadParams for WxpExampleGainMainThread<'_> {
    /// Number of parameters exposed by this plugin.
    fn count(&mut self) -> u32 {
        1
    }

    /// Provides the host with parameter information (ID, name, range, flags, etc.).
    fn get_info(&mut self, param_index: u32, info: &mut ParamInfoWriter) {
        if param_index != 0 {
            return;
        }

        info.set(&ParamInfo {
            id: PARAM_GAIN_ID,
            // IS_AUTOMATABLE: the host can draw an automation curve for this parameter.
            flags: ParamInfoFlags::IS_AUTOMATABLE,
            cookie: Default::default(),
            name: b"Gain",
            // module is a path for grouping parameters (e.g., "EQ/Band1").
            // This plugin has only one parameter, so it is empty.
            module: b"",
            min_value: 0.0,
            max_value: 2.0,
            default_value: DEFAULT_GAIN as f64,
        });
    }

    /// Called when the host queries the current value of a parameter.
    fn get_value(&mut self, param_id: ClapId) -> Option<f64> {
        (param_id == PARAM_GAIN_ID).then(|| self.shared.inner.gain() as f64)
    }

    /// Converts a parameter's numeric value to a display string.
    /// Used by the host UI to show a label such as "−6.0 dB" next to the parameter value.
    fn value_to_text(
        &mut self,
        param_id: ClapId,
        value: f64,
        writer: &mut ParamDisplayWriter,
    ) -> std::fmt::Result {
        if param_id != PARAM_GAIN_ID {
            return Err(std::fmt::Error);
        }

        writer.write_str(&gain_db_text(clamp_gain(value as f32) as f64))
    }

    /// Converts a text input to a parameter value (inverse of value_to_text).
    /// Used when the user types a value such as "-6 dB" in the host UI.
    fn text_to_value(&mut self, param_id: ClapId, text: &CStr) -> Option<f64> {
        if param_id != PARAM_GAIN_ID {
            return None;
        }

        let text = text.to_str().ok()?.trim();
        let text = text.strip_suffix("dB").unwrap_or(text).trim();
        let db = text.parse::<f64>().ok()?;
        // Inverse conversion from dB to linear gain: gain = 10^(dB/20)
        Some(clamp_gain(10.0_f64.powf(db / 20.0) as f32) as f64)
    }

    /// Parameter flush on the main thread.
    /// Called by the host when audio processing is inactive.
    fn flush(
        &mut self,
        input_parameter_changes: &InputEvents,
        output_parameter_changes: &mut OutputEvents,
    ) {
        drain_ui_events(&self.shared.inner, output_parameter_changes);
        apply_host_parameter_events(&self.shared.inner, input_parameter_changes);
    }
}

/// Reads the pending UI flags and emits them as output events to the host.
/// Called at the start of process() and flush().
///
/// Event ordering matters: begin → value → end.
/// take_* uses swap(false), so each flag is consumed once read.
pub(crate) fn drain_ui_events(
    shared: &SharedStateInner,
    output_parameter_changes: &mut OutputEvents,
) {
    if shared.take_ui_gesture_begin() {
        let _ = output_parameter_changes.try_push(ParamGestureBeginEvent::new(0, PARAM_GAIN_ID));
    }

    if shared.take_ui_value_dirty() {
        let _ = output_parameter_changes.try_push(ParamValueEvent::new(
            0,
            PARAM_GAIN_ID,
            // Pckn::match_all() matches all MIDI channels/ports.
            // The gain parameter is unrelated to MIDI, so a wildcard is fine.
            clack_plugin::events::Pckn::match_all(),
            shared.gain() as f64,
            clack_plugin::utils::Cookie::empty(),
        ));
    }

    if shared.take_ui_gesture_end() {
        let _ = output_parameter_changes.try_push(ParamGestureEndEvent::new(0, PARAM_GAIN_ID));
    }
}

/// Processes input events from the host (automation, etc.) and applies them to SharedState.
/// Extracts only ParamValue events from the event stream.
pub(crate) fn apply_host_parameter_events(shared: &SharedStateInner, events: &InputEvents) {
    for event in events {
        // Skip non-core events (e.g., MIDI).
        let Some(core_event) = event.as_core_event() else {
            continue;
        };

        // Skip core events other than ParamValue (e.g., NoteOn).
        let CoreEventSpace::ParamValue(param) = core_event else {
            continue;
        };
        let Some(param_id) = param.param_id() else {
            continue;
        };
        // Skip parameter IDs this plugin does not recognize.
        if param_id != PARAM_GAIN_ID {
            continue;
        }

        shared.set_gain_from_host(param.value());
    }
}
