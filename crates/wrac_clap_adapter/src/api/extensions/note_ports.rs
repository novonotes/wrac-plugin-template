use crate::NotePortInfo;

/// CLAP note-ports extension.
///
/// Count queries may be reached from wrapper code that must stay realtime-safe.
/// Metadata queries are non-realtime host/control queries and may allocate owned
/// strings before the ABI layer copies them into CLAP buffers.
pub trait PluginNotePortsExtension: Send + Sync + 'static {
    /// Called from CLAP `note_ports.count`. `[thread-safe]`
    fn note_port_count(&self, is_input: bool) -> u32;

    /// Called from CLAP `note_ports.get`. `[thread-safe & control-thread]`
    fn note_port_info(&self, index: u32, is_input: bool) -> Option<NotePortInfo>;
}
