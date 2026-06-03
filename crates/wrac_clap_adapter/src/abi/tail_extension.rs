use clap_sys::ext::tail::clap_plugin_tail;
use clap_sys::plugin::clap_plugin;

use super::PluginInstance;
use super::ffi::ffi_u32;

pub(super) static TAIL: clap_plugin_tail = clap_plugin_tail {
    get: Some(tail_get),
};

unsafe extern "C" fn tail_get(plugin: *const clap_plugin) -> u32 {
    ffi_u32(|| {
        let Some(instance) = (unsafe { PluginInstance::from_plugin(plugin) }) else {
            wrac_log::rtwarn!("tail.get: missing plugin instance");
            return 0;
        };
        let Some(tail) = instance.tail.as_ref() else {
            wrac_log::rtwarn!("tail.get: plugin has no tail support");
            return 0;
        };
        tail.tail_frames()
    })
}
