use clap_sys::plugin::clap_plugin;

#[repr(C)]
pub(super) struct ClapPluginAsVst3 {
    get_num_midi_channels: Option<unsafe extern "C" fn(*const clap_plugin, u32) -> u32>,
    supported_note_expressions: Option<unsafe extern "C" fn(*const clap_plugin) -> u32>,
}

pub(super) static VST3: ClapPluginAsVst3 = ClapPluginAsVst3 {
    get_num_midi_channels: Some(get_num_midi_channels),
    supported_note_expressions: Some(supported_note_expressions),
};

unsafe impl Sync for ClapPluginAsVst3 {}

unsafe extern "C" fn get_num_midi_channels(plugin: *const clap_plugin, note_port: u32) -> u32 {
    // VST3 requires an explicit channel count per event bus. CLAP note ports do not
    // carry that number, and WRAC CoreDevice note processing is channel-agnostic for
    // the builtin/plugin products migrated here. Report one channel instead of
    // falling back to clap-wrapper's 16-channel default, which materializes hundreds
    // of synthetic MIDI controller parameters and can stall the Windows validator.
    let _ = (plugin, note_port);
    1
}

unsafe extern "C" fn supported_note_expressions(_plugin: *const clap_plugin) -> u32 {
    0
}
