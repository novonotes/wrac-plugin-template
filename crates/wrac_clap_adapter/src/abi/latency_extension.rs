use std::sync::atomic::{AtomicBool, Ordering};

use clap_sys::ext::latency::clap_plugin_latency;
use clap_sys::plugin::clap_plugin;

use super::PluginInstanceState;
use super::ffi::ffi_u32;

pub(super) static LATENCY: clap_plugin_latency = clap_plugin_latency {
    get: Some(latency_get),
};

static MISSING_LATENCY_WARNED: AtomicBool = AtomicBool::new(false);

unsafe extern "C" fn latency_get(plugin: *const clap_plugin) -> u32 {
    ffi_u32(|| {
        let Some(instance) = (unsafe { PluginInstanceState::from_plugin(plugin) }) else {
            log::warn!("latency.get: missing plugin instance");
            return 0;
        };
        let Some(latency) = instance.latency.as_ref() else {
            // The implementation error is reported at plugin creation by a debug assertion and
            // this one release warning. `latency.get` can be polled continuously by wrappers/hosts,
            // so keep the ABI fallback visible without flooding logs.
            if !MISSING_LATENCY_WARNED.swap(true, Ordering::Relaxed) {
                log::warn!("latency.get: plugin has no latency support; returning zero latency");
            }
            return 0;
        };
        latency.latency_frames()
    })
}
