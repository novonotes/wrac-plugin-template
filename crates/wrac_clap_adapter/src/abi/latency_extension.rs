use clap_sys::ext::latency::clap_plugin_latency;
use clap_sys::plugin::clap_plugin;

use super::PluginInstance;
use super::ffi::ffi_u32;

pub(super) static LATENCY: clap_plugin_latency = clap_plugin_latency {
    get: Some(latency_get),
};

unsafe extern "C" fn latency_get(plugin: *const clap_plugin) -> u32 {
    ffi_u32(|| {
        let Some(instance) = (unsafe { PluginInstance::from_plugin(plugin) }) else {
            log::warn!("latency.get: missing plugin instance");
            return 0;
        };
        let Some(latency) = instance.latency.as_ref() else {
            log::warn!("latency.get: plugin has no latency support");
            return 0;
        };
        latency.latency_frames()
    })
}
