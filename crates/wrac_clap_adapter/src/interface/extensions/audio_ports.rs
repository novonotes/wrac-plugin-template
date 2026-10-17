use crate::interface::AudioPortInfo;

/// CLAP audio-ports extension.
///
pub trait PluginAudioPortsExtension: Send + Sync + 'static {
    /// Called from CLAP `audio_ports.count`. `[realtime-safe & thread-safe]`
    fn audio_port_count(&self, is_input: bool) -> u32;

    /// Called from CLAP `audio_ports.get`. `[non-realtime & thread-safe]`
    fn audio_port_info(&self, index: u32, is_input: bool) -> Option<AudioPortInfo>;
}
