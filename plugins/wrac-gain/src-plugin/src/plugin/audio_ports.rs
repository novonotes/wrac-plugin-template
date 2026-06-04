use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use wrac_clap_adapter::{
    AudioPortConfigRequest, AudioPortFlags, AudioPortInfo, AudioPortType,
    PluginAudioPortsExtension, PluginConfigurableAudioPortsExtension, PluginError, PluginResult,
};

/// Source of truth for the audio layout negotiated with the host.
///
/// Host port queries and configurable-audio-ports apply operations read/write this store,
/// and wrappers may issue those queries from audio/render workers. `Processor::process()`
/// still uses the snapshot captured at `activate()`, so layout changes cannot alter the
/// running processor's buffer contract.
pub(super) struct AudioLayoutStore {
    channel_count: AtomicU32,
}

impl AudioLayoutStore {
    pub(super) fn new(channel_count: u32) -> Self {
        Self {
            channel_count: AtomicU32::new(channel_count),
        }
    }

    pub(super) fn channel_count(&self) -> u32 {
        self.channel_count.load(Ordering::Relaxed)
    }

    fn set_channel_count(&self, channel_count: u32) {
        self.channel_count.store(channel_count, Ordering::Relaxed);
    }
}

pub(super) struct WracGainAudioPorts {
    layout: Arc<AudioLayoutStore>,
}

impl WracGainAudioPorts {
    pub(super) fn new(layout: Arc<AudioLayoutStore>) -> Self {
        Self { layout }
    }
}

// Gain has one main input and one main output. Channel count can be changed by the
// host via configurable audio ports.
impl PluginAudioPortsExtension for WracGainAudioPorts {
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

/// Capability that applies host-requested layout changes to [`AudioLayoutStore`].
///
/// Mutation via `&self` is intentional: the adapter calls this without acquiring the
/// `&mut self` lock (see [`WracGainPlugin`](super::WracGainPlugin)). This does not mean
/// changes are allowed while active — the adapter enforces that this is only called when
/// no `Processor` exists (inactive).
pub(super) struct WracGainConfigurableAudioPorts {
    layout: Arc<AudioLayoutStore>,
}

impl WracGainConfigurableAudioPorts {
    pub(super) fn new(layout: Arc<AudioLayoutStore>) -> Self {
        Self { layout }
    }
}

// Example: host proposes stereo→mono → answer feasibility via `can_apply_*`,
// commit the change via `apply_*`.
impl PluginConfigurableAudioPortsExtension for WracGainConfigurableAudioPorts {
    fn can_apply_audio_port_configuration(&self, requests: &[AudioPortConfigRequest]) -> bool {
        let accepted = resolve_audio_channel_count(self.layout.channel_count(), requests);
        accepted.is_some()
    }

    fn apply_audio_port_configuration(
        &self,
        requests: &[AudioPortConfigRequest],
    ) -> PluginResult<()> {
        // The adapter rejects configuration apply while a Processor exists. This updates
        // only the non-RT query store; the audio thread uses the snapshot captured at activate.
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

fn audio_port_type(channel_count: u32) -> AudioPortType {
    match channel_count {
        1 => AudioPortType::Mono,
        2 => AudioPortType::Stereo,
        _ => AudioPortType::Unspecified,
    }
}

/// Parses port configuration requests and returns the new channel count if acceptable.
///
/// Only symmetric main-port configurations (same channel count for input and output) are
/// accepted. Asymmetric configurations such as sidechain require product-specific routing
/// semantics that cannot be defined in a generic gain sample.
fn resolve_audio_channel_count(
    current_channel_count: u32,
    requests: &[AudioPortConfigRequest],
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

    // Accept only when input and output channel counts match.
    (input_channel_count == output_channel_count).then_some(input_channel_count)
}

fn is_supported_audio_port_request(request: &AudioPortConfigRequest) -> bool {
    matches!(
        (request.channel_count, request.port_type),
        (1, AudioPortType::Mono | AudioPortType::Unspecified)
            | (2, AudioPortType::Stereo | AudioPortType::Unspecified)
    )
}

#[cfg(test)]
mod tests {
    // Unit test examples for pure logic that can be verified without a host or CLAP runtime.

    use wrac_clap_adapter::{AudioPortConfigRequest, AudioPortType};

    use super::resolve_audio_channel_count;

    #[test]
    fn accepts_matching_mono_configuration() {
        let requests = [
            AudioPortConfigRequest {
                is_input: true,
                port_index: 0,
                channel_count: 1,
                port_type: AudioPortType::Mono,
            },
            AudioPortConfigRequest {
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
            AudioPortConfigRequest {
                is_input: true,
                port_index: 0,
                channel_count: 1,
                port_type: AudioPortType::Mono,
            },
            AudioPortConfigRequest {
                is_input: false,
                port_index: 0,
                channel_count: 2,
                port_type: AudioPortType::Stereo,
            },
        ];

        assert_eq!(resolve_audio_channel_count(2, &requests), None);
    }
}
