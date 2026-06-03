use crate::{AudioPortConfigRequest, PluginResult};

/// CLAP configurable-audio-ports extension.
pub trait PluginConfigurableAudioPortsExtension: Send + Sync + 'static {
    fn can_apply_audio_port_configuration(&self, requests: &[AudioPortConfigRequest]) -> bool;

    fn apply_audio_port_configuration(
        &self,
        requests: &[AudioPortConfigRequest],
    ) -> PluginResult<()>;
}
