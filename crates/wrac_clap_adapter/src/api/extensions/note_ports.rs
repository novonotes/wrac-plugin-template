use crate::NotePortInfo;

/// CLAP note-ports extension.
pub trait PluginNotePortsExtension: Send + Sync + 'static {
    fn note_port_count(&self, is_input: bool) -> u32;
    fn note_port_info(&self, index: u32, is_input: bool) -> Option<NotePortInfo>;
}
