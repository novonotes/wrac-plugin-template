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
            log::warn!("note_ports.count: missing plugin instance is_input={is_input}");
            return 0;
        };
        // note port query は軽量 metadata call です。wrapper 経由では lifecycle/state
        // 作業と競合し得るため、この path では `PluginCore` を待たない。
        let Some(core) = instance.core.try_read() else {
            log::warn!(
                "note_ports.count: core try_read failed is_input={is_input} thread={:?}",
                std::thread::current().id()
            );
            return 0;
        };
        let Some(note_ports) = core.note_ports() else {
            log::debug!("note_ports.count: plugin has no note ports is_input={is_input}");
            return 0;
        };
        let count = note_ports.note_port_count(is_input);
        log::debug!(
            "note_ports.count: is_input={is_input} count={count} thread={:?}",
            std::thread::current().id()
        );
        count
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
            log::warn!("note_ports.get: null output pointer index={index} is_input={is_input}");
            return false;
        }
        let Some(instance) = (unsafe { PluginInstance::from_plugin(plugin) }) else {
            log::warn!("note_ports.get: missing plugin instance index={index} is_input={is_input}");
            return false;
        };
        let Some(core) = instance.core.try_read() else {
            log::warn!(
                "note_ports.get: core try_read failed index={index} is_input={is_input} thread={:?}",
                std::thread::current().id()
            );
            return false;
        };
        let Some(note_ports) = core.note_ports() else {
            log::debug!(
                "note_ports.get: plugin has no note ports index={index} is_input={is_input}"
            );
            return false;
        };
        let Some(port) = note_ports.note_port_info(index, is_input) else {
            log::warn!("note_ports.get: invalid index={index} is_input={is_input}");
            return false;
        };
        log::debug!(
            "note_ports.get: index={index} is_input={is_input} id={} dialects={} thread={:?}",
            port.id,
            port.supported_dialects.bits(),
            std::thread::current().id()
        );

        unsafe {
            (*info).id = port.id;
            (*info).supported_dialects = port.supported_dialects.bits();
            (*info).preferred_dialect = port.preferred_dialect.bits();
            fill_c_char_array(&mut (*info).name, port.name);
        }
        true
    })
}
