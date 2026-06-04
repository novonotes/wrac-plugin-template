use crate::AudioPortInfo;

/// CLAP audio-ports extension.
///
/// Native CLAP marks this extension `[main-thread]`, but WRAC requires audio-thread
/// compatibility because wrappers may query it from audio/render workers.
/// Implementations must not block, allocate, or take contended locks.
pub trait PluginAudioPortsExtension: Send + Sync + 'static {
    /// Called from CLAP `audio_ports.count`. `[thread-safe]`
    fn audio_port_count(&self, is_input: bool) -> u32;

    /// Called from CLAP `audio_ports.get`. `[thread-safe]`
    fn audio_port_info(&self, index: u32, is_input: bool) -> Option<AudioPortInfo>;
}
