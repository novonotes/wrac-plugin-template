//! DSP running on the audio thread.
//!
//! This sample simply multiplies the input by a gain and writes it back.
//! [`ActiveProcessor::process`] is a realtime function called repeatedly for each small buffer,
//! so the rule is to **avoid allocations and locks**. Shared state is read lock-free from
//! [`SharedState`].

use std::any::Any;
use std::sync::Arc;

use wrac_clap_adapter::{
    ActiveProcessor, AudioPairedChannels, AudioPortChannels, AudioProcessBuffer, InactiveProcessor,
    InputEvent, ParamFlushContext, PluginResult, ProcessContext, ProcessStatus,
};

use crate::plugin::{
    PARAM_BYPASS_ID, PARAM_GAIN_ID, WracGainParamOutputQueue, apply_param_input_events,
    parameter_host_input_to_plain,
};
use crate::state::SharedState;

/// The DSP instance created at `activate()` and owned by the host's audio thread.
/// It lives until `deactivate()`, during which `process()` is called repeatedly.
///
/// Fields should contain only things **the audio thread can read without waiting**.
/// `shared` uses atomics and is safe to read during `process()`.
/// `audio_channel_count` is a snapshot copied from the plugin's audio layout store at
/// activate time. Because the adapter rejects layout changes while active, the running
/// active processor's contract cannot change mid-flight. Even when a product's DSP must vary
/// with layout, it is safer to convert the needed settings at activate time and pass them
/// in rather than storing an `Arc<RwLock<Layout>>`.
pub(crate) struct WracGainAudioProcessor {
    shared: Arc<SharedState>,
    param_output_queue: Arc<WracGainParamOutputQueue>,
    // Gain itself does not use channel count, but this field demonstrates the pattern
    // "snapshot layout at activate time and store it as a field."
    // In debug builds, the actual buffer is verified to match this snapshot.
    audio_channel_count: u32,
}

impl WracGainAudioProcessor {
    pub(crate) fn new(
        shared: Arc<SharedState>,
        param_output_queue: Arc<WracGainParamOutputQueue>,
        audio_channel_count: u32,
    ) -> Self {
        Self {
            shared,
            param_output_queue,
            audio_channel_count,
        }
    }
}

impl ActiveProcessor for WracGainAudioProcessor {
    fn into_any(self: Box<Self>) -> Box<dyn Any + Send> {
        self
    }

    /// Processes one block. `context` contains the audio I/O, the parameter event list
    /// `events.input` for this block, and the sample count `frames_count`.
    ///
    /// The buffer is split at each parameter event's timestamp, and the gain current at
    /// that moment is applied to each segment, achieving sample-accurate automation
    /// (gain is constant between events).
    fn process(&mut self, context: ProcessContext<'_>) -> PluginResult<ProcessStatus> {
        #[cfg(debug_assertions)]
        {
            // Abort immediately on any allocation. Wrapping every process() call in
            // debug builds ensures violations are not swallowed by the DAW or adapter.
            assert_no_alloc::assert_no_alloc(|| self.process_no_alloc(context))
        }

        #[cfg(not(debug_assertions))]
        {
            self.process_no_alloc(context)
        }
    }

    fn flush_params(&mut self, mut context: ParamFlushContext<'_>) -> PluginResult<()> {
        apply_param_input_events(&self.shared, &context.events.input);
        self.param_output_queue.drain(&mut context.events.output);
        Ok(())
    }
}

impl WracGainAudioProcessor {
    fn process_no_alloc(&mut self, mut context: ProcessContext<'_>) -> PluginResult<ProcessStatus> {
        #[cfg(debug_assertions)]
        assert_audio_layout_matches_processor_snapshot(
            &mut context.audio,
            self.audio_channel_count,
        );

        self.param_output_queue.drain(&mut context.events.output);

        // Gain at the start of this block; updated each time a parameter event arrives.
        let mut gain = self.shared.gain();
        let mut bypass = self.shared.bypass();
        // Cursor tracking how far into the block has been processed.
        let mut segment_start = 0;
        let frames_count = context.frames_count as usize;

        for event in context.events.input.iter() {
            // Process up to the event position with the current gain.
            // Clamp event time rather than trusting the host, to prevent out-of-bounds access.
            let event_time = (event.time() as usize).min(frames_count);
            if event_time > segment_start {
                let effective_gain = if bypass { 1.0 } else { gain };
                process_audio_range(
                    &mut context.audio,
                    segment_start,
                    event_time,
                    effective_gain,
                )?;
                segment_start = event_time;
            }

            if let InputEvent::ParamValue(event) = event {
                apply_realtime_param_event(
                    &self.shared,
                    event.param_id,
                    event.value,
                    &mut gain,
                    &mut bypass,
                );
            }
        }

        // Process the remaining range from the last event to the end of the block.
        if segment_start < frames_count {
            let effective_gain = if bypass { 1.0 } else { gain };
            process_audio_range(
                &mut context.audio,
                segment_start,
                frames_count,
                effective_gain,
            )?;
        }

        // Signal that processing should continue for the next block unless the input is silent.
        // Returning `Quiet` lets the host use it as a hint for optimisation.
        Ok(ProcessStatus::ContinueIfNotQuiet)
    }
}

pub(crate) struct WracGainInactiveProcessor {
    shared: Arc<SharedState>,
    param_output_queue: Arc<WracGainParamOutputQueue>,
}

impl WracGainInactiveProcessor {
    pub(crate) fn new(
        shared: Arc<SharedState>,
        param_output_queue: Arc<WracGainParamOutputQueue>,
    ) -> Self {
        Self {
            shared,
            param_output_queue,
        }
    }
}

impl InactiveProcessor for WracGainInactiveProcessor {
    fn into_any(self: Box<Self>) -> Box<dyn Any + Send> {
        self
    }

    fn flush_params(&mut self, mut context: ParamFlushContext<'_>) -> PluginResult<()> {
        apply_param_input_events(&self.shared, &context.events.input);
        self.param_output_queue.drain(&mut context.events.output);
        Ok(())
    }
}

fn apply_realtime_param_event(
    shared: &SharedState,
    parameter_id: u32,
    host_value: f64,
    gain: &mut f32,
    bypass: &mut bool,
) {
    // Keep host-domain decoding shared with the non-RT parameter API, but keep DSP meaning
    // explicit here. Gain and bypass affect the current block differently than arbitrary
    // parameters, so hiding this behind a fully generic map would obscure realtime behavior.
    let Ok(plain_value) = parameter_host_input_to_plain(parameter_id, host_value) else {
        return;
    };
    let Some(applied) = shared.set_parameter_value(parameter_id, plain_value) else {
        return;
    };
    match parameter_id {
        PARAM_GAIN_ID => *gain = applied,
        PARAM_BYPASS_ID => *bypass = applied >= 0.5,
        _ => {}
    }
}

#[cfg(debug_assertions)]
fn assert_audio_layout_matches_processor_snapshot(
    audio: &mut AudioProcessBuffer<'_>,
    expected_channel_count: u32,
) {
    // Debug-only verification that the actual buffer matches the activate-time snapshot.
    // The store's lock is not read here. Protecting memory safety from invalid buffers
    // is the adapter's responsibility; this is a demonstration of snapshot usage, not a
    // replacement. Product DSPs that don't use channel count may remove this assertion entirely.
    debug_assert_eq!(
        audio.port_pair_count(),
        1,
        "WRAC Gain expects exactly one main audio port pair"
    );

    for port_index in 0..audio.port_pair_count() {
        let Some(port_pair) = audio.port_pair(port_index) else {
            continue;
        };
        debug_assert_eq!(
            port_pair.channel_pair_count(),
            expected_channel_count as usize,
            "audio buffer channel count must match the layout captured at activate()"
        );
    }
}

/// Applies gain to the `[start, end)` range of each port.
/// The buffer passed by the host can be either `f32` or `f64`, so both are handled.
fn process_audio_range(
    audio: &mut AudioProcessBuffer<'_>,
    start: usize,
    end: usize,
    gain: f32,
) -> PluginResult<()> {
    let len = end.saturating_sub(start);
    for mut port_pair in audio {
        match port_pair.channels()? {
            AudioPortChannels::F32(channels) => process_channels_range(channels, start, len, gain),
            AudioPortChannels::F64(channels) => {
                process_channels_range(channels, start, len, gain as f64)
            }
        }
    }
    Ok(())
}

/// Applies gain to each sample in every channel of one port (paired in/out).
///
/// `map_samples_range` operates in-place and supports "in-place processing"
/// where the input and output point to the same buffer.
fn process_channels_range<T>(
    channels: AudioPairedChannels<'_, T>,
    start: usize,
    len: usize,
    gain: T,
) where
    T: Copy + Default + std::ops::Mul<Output = T>,
{
    for mut channel in channels {
        channel.map_samples_range(start, len, |sample| sample * gain);
    }
}
