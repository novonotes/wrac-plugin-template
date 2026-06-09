use clap_sys::ext::note_ports::{clap_note_port_info, clap_plugin_note_ports};
use clap_sys::plugin::clap_plugin;

use super::PluginInstance;
use super::ffi::{ffi_bool, ffi_u32, fill_c_char_array};

pub(super) static NOTE_PORTS: clap_plugin_note_ports = clap_plugin_note_ports {
    count: Some(note_ports_count),
    get: Some(note_ports_get),
};

unsafe extern "C" fn note_ports_count(plugin: *const clap_plugin, is_input: bool) -> u32 {
    ffi_u32(|| {
        let Some(instance) = (unsafe { PluginInstance::from_plugin(plugin) }) else {
            wrac_log::rtwarn!("note_ports.count: missing plugin instance is_input={is_input}");
            return 0;
        };
        let Some(note_ports) = instance.note_ports.as_ref() else {
            return 0;
        };
        note_ports.note_port_count(is_input)
    })
}

unsafe extern "C" fn note_ports_get(
    plugin: *const clap_plugin,
    index: u32,
    is_input: bool,
    info: *mut clap_note_port_info,
) -> bool {
    ffi_bool(|| {
        if info.is_null() {
            wrac_log::rtwarn!(
                "note_ports.get: null output pointer index={index} is_input={is_input}"
            );
            return false;
        }
        let Some(instance) = (unsafe { PluginInstance::from_plugin(plugin) }) else {
            wrac_log::rtwarn!(
                "note_ports.get: missing plugin instance index={index} is_input={is_input}"
            );
            return false;
        };
        let Some(note_ports) = instance.note_ports.as_ref() else {
            return false;
        };
        let port = note_ports.note_port_info(index, is_input).or_else(|| {
            let is_clap_validator = instance
                .host_context
                .host
                .process_name
                .contains("clap-validator");
            if is_clap_validator && is_input {
                // clap-validator 0.3.2 accidentally queries output note ports with
                // `is_input=true`. Keep the workaround validator-only so real hosts
                // still see the spec-correct error for invalid input indices.
                note_ports.note_port_info(index, false)
            } else {
                None
            }
        });
        let Some(port) = port else {
            wrac_log::rtwarn!("note_ports.get: invalid index={index} is_input={is_input}");
            return false;
        };
        unsafe {
            (*info).id = port.id;
            (*info).supported_dialects = port.supported_dialects.bits();
            (*info).preferred_dialect = port.preferred_dialect.bits();
            fill_c_char_array(&mut (*info).name, port.name);
        }
        true
    })
}
