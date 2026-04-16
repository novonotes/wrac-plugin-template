//! Audio processing module.
//!
//! This module runs on the audio thread.
//! The audio thread has real-time constraints; the following operations are forbidden:
//!   - Memory allocation / deallocation (malloc / free)
//!   - Lock acquisition (Mutex, RwLock, etc.)
//!   - I/O (file, network)
//!   - System calls in general
//! Performing these operations causes audio dropouts (noise or glitches).
//! For this reason, parameter passing uses lock-free mechanisms such as AtomicF32.

use clack_extensions::params::PluginAudioProcessorParams;
use clack_plugin::prelude::*;
use clack_plugin::process::audio::{ChannelPair, SampleType};

use crate::params::{apply_host_parameter_events, drain_ui_events};
use crate::plugin::{SharedState, WxpExampleGainMainThread};

/// Processor that runs on the audio thread.
/// Holds only a reference to SharedState and reads parameters via Atomics.
pub(crate) struct WxpExampleGainAudioProcessor<'a> {
    shared: &'a SharedState,
}

impl<'a> PluginAudioProcessor<'a, SharedState, WxpExampleGainMainThread<'a>>
    for WxpExampleGainAudioProcessor<'a>
{
    /// Called when the host starts audio processing (activate).
    /// `audio_config` contains sample rate and buffer size information.
    /// This plugin is a simple gain, so those values are not needed.
    fn activate(
        _host: HostAudioProcessorHandle<'a>,
        _main_thread: &mut WxpExampleGainMainThread<'a>,
        shared: &'a SharedState,
        _audio_config: PluginAudioConfiguration,
    ) -> Result<Self, PluginError> {
        Ok(Self { shared })
    }

    /// Called when the host stops audio processing (deactivate).
    fn deactivate(self, _main_thread: &mut WxpExampleGainMainThread<'a>) {}

    /// Main processing function called for every audio buffer.
    /// The host typically calls this at 44100 Hz or 48000 Hz,
    /// in buffer sizes of roughly 64 to 2048 samples.
    fn process(
        &mut self,
        _process: Process,
        mut audio: Audio,
        events: Events,
    ) -> Result<ProcessStatus, PluginError> {
        // Notify the host of parameter changes from the UI (output events).
        drain_ui_events(&self.shared.inner, events.output);
        // Apply parameter changes from the host (automation, etc.).
        apply_host_parameter_events(&self.shared.inner, events.input);

        // Read the current gain value from the Atomic. Lock-free, so safe on the audio thread.
        let gain = self.shared.inner.gain();
        // port_pair(0) retrieves the first audio port pair (input + output).
        let Some(mut port_pair) = audio.port_pair(0) else {
            return Ok(ProcessStatus::ContinueIfNotQuiet);
        };

        // The host may use either f32 or f64 sample format.
        // Handle both cases.
        match port_pair.channels()? {
            SampleType::F32(mut channels) => process_channels_f32(&mut channels, gain),
            SampleType::F64(mut channels) => process_channels_f64(&mut channels, gain as f64),
            // For Both, process the f64 side (hosts prefer f64).
            SampleType::Both(_, mut channels) => process_channels_f64(&mut channels, gain as f64),
        }

        // ContinueIfNotQuiet: tells the host it may skip processing when input is silent.
        // Effects with a tail (e.g., reverb or delay) should return Tail instead.
        Ok(ProcessStatus::ContinueIfNotQuiet)
    }
}

/// Parameter flush on the audio thread.
/// Called by the host when parameter synchronization is needed but process() is not running
/// (e.g., while playback is stopped).
impl PluginAudioProcessorParams for WxpExampleGainAudioProcessor<'_> {
    fn flush(
        &mut self,
        input_parameter_changes: &InputEvents,
        output_parameter_changes: &mut OutputEvents,
    ) {
        drain_ui_events(&self.shared.inner, output_parameter_changes);
        apply_host_parameter_events(&self.shared.inner, input_parameter_changes);
    }
}

/// Channel processing for f32 sample format.
/// ChannelPair handles the four buffer layouts the host may provide:
fn process_channels_f32(
    channels: &mut clack_plugin::process::audio::PairedChannels<'_, f32>,
    gain: f32,
) {
    for pair in channels.iter_mut() {
        match pair {
            // Input only (no output buffer): do nothing.
            ChannelPair::InputOnly(_) => {}
            // Output only (no input buffer): fill with silence.
            ChannelPair::OutputOnly(output) => output.fill(0.0),
            // Separate input and output buffers: apply gain from input to output.
            ChannelPair::InputOutput(input, output) => {
                for (src, dst) in input.iter().zip(output.iter_mut()) {
                    *dst = *src * gain;
                }
            }
            // In-place processing: input and output share the same buffer. Modify in place.
            // Memory-efficient and the format most preferred by hosts.
            ChannelPair::InPlace(buffer) => {
                for sample in buffer.iter_mut() {
                    *sample *= gain;
                }
            }
        }
    }
}

/// Channel processing for f64 sample format. Logic is identical to the f32 version.
fn process_channels_f64(
    channels: &mut clack_plugin::process::audio::PairedChannels<'_, f64>,
    gain: f64,
) {
    for pair in channels.iter_mut() {
        match pair {
            ChannelPair::InputOnly(_) => {}
            ChannelPair::OutputOnly(output) => output.fill(0.0),
            ChannelPair::InputOutput(input, output) => {
                for (src, dst) in input.iter().zip(output.iter_mut()) {
                    *dst = *src * gain;
                }
            }
            ChannelPair::InPlace(buffer) => {
                for sample in buffer.iter_mut() {
                    *sample *= gain;
                }
            }
        }
    }
}
