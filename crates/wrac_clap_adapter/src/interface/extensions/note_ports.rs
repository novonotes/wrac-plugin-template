use crate::interface::NotePortInfo;

/// CLAP note-ports extension.
///
pub trait PluginNotePortsExtension: Send + Sync + 'static {
    /// Called from CLAP `note_ports.count`. `[realtime-safe & thread-safe]`
    fn note_port_count(&self, is_input: bool) -> u32;

    /// Called from CLAP `note_ports.get`. `[non-realtime & thread-safe]`
    fn note_port_info(&self, index: u32, is_input: bool) -> Option<NotePortInfo>;
}
