use crate::NotePortInfo;

/// CLAP note-ports extension.
///
/// Native CLAP marks this extension `[main-thread]`, but WRAC requires audio-thread
/// compatibility because wrappers may query it from audio/render workers.
/// Implementations must not block, allocate, or take contended locks.
pub trait PluginNotePortsExtension: Send + Sync + 'static {
    /// Called from CLAP `note_ports.count`. `[thread-safe]`
    fn note_port_count(&self, is_input: bool) -> u32;

    /// Called from CLAP `note_ports.get`. `[thread-safe]`
    fn note_port_info(&self, index: u32, is_input: bool) -> Option<NotePortInfo>;
}
