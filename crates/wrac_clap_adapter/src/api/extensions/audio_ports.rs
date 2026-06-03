use crate::AudioPortInfo;

/// CLAP audio-ports extension.
pub trait PluginAudioPortsExtension: Send + Sync + 'static {
    fn audio_port_count(&self, is_input: bool) -> u32;
    fn audio_port_info(&self, index: u32, is_input: bool) -> Option<AudioPortInfo>;
}
