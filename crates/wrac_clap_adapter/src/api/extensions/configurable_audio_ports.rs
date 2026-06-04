use crate::{AudioPortConfigRequest, PluginResult};

/// CLAP configurable-audio-ports extension.
pub trait PluginConfigurableAudioPortsExtension: Send + Sync + 'static {
    /// Called from CLAP `configurable_audio_ports.can_apply_configuration`.
    /// `[thread-safe & control-thread]`
    ///
    /// The adapter rejects this while a processor or lifecycle callback is active.
    fn can_apply_audio_port_configuration(&self, requests: &[AudioPortConfigRequest]) -> bool;

    /// Called from CLAP `configurable_audio_ports.apply_configuration`.
    /// `[thread-safe & control-thread]`
    ///
    /// The adapter rejects this while a processor or lifecycle callback is active.
    fn apply_audio_port_configuration(
        &self,
        requests: &[AudioPortConfigRequest],
    ) -> PluginResult<()>;
}
