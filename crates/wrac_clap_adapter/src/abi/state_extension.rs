use clap_sys::ext::state::clap_plugin_state;
use clap_sys::plugin::clap_plugin;
use clap_sys::stream::{clap_istream, clap_ostream};

use super::PluginInstance;
use super::ffi::{ffi_bool, read_stream_to_end, write_stream};
use crate::State;

pub(super) static STATE: clap_plugin_state = clap_plugin_state {
    save: Some(state_save),
    load: Some(state_load),
};

const MAX_STATE_BYTES: usize = 64 * 1024 * 1024;

// State callbacks may arrive while the plugin is active, depending on the host format.
// Waiting for or giving up on the `PluginCore` write lock here could silently drop a project save,
// so only the thread-safe state capability fixed at instance creation is called.
unsafe extern "C" fn state_save(plugin: *const clap_plugin, stream: *const clap_ostream) -> bool {
    ffi_bool(|| {
        if stream.is_null() {
            log::warn!("state.save: null stream");
            return false;
        }
        let Some(instance) = (unsafe { PluginInstance::from_plugin(plugin) }) else {
            log::warn!("state.save: missing plugin instance");
            return false;
        };
        let Some(state_support) = instance.state.as_ref() else {
            log::debug!("state.save: plugin has no state support");
            return false;
        };
        let state = match state_support.save_state() {
            Ok(state) => state,
            Err(error) => {
                log::warn!("state.save: plugin save_state failed: {error}");
                return false;
            }
        };
        let ok = unsafe { write_stream(stream, &state.bytes) };
        if !ok {
            log::warn!(
                "state.save: writing state stream failed byte_len={}",
                state.bytes.len()
            );
        } else {
            log::debug!("state.save: wrote byte_len={}", state.bytes.len());
        }
        ok
    })
}

unsafe extern "C" fn state_load(plugin: *const clap_plugin, stream: *const clap_istream) -> bool {
    ffi_bool(|| {
        if stream.is_null() {
            log::warn!("state.load: null stream");
            return false;
        }
        let Some(instance) = (unsafe { PluginInstance::from_plugin(plugin) }) else {
            log::warn!("state.load: missing plugin instance");
            return false;
        };
        let Some(bytes) = (unsafe { read_stream_to_end(stream, MAX_STATE_BYTES) }) else {
            log::warn!("state.load: failed to read state stream");
            return false;
        };

        let Some(state_support) = instance.state.as_ref() else {
            log::debug!("state.load: plugin has no state support");
            return false;
        };
        let byte_len = bytes.len();
        if let Err(error) = state_support.restore_state(State { bytes }) {
            log::warn!("state.load: plugin restore_state failed: {error}");
            return false;
        }
        instance.parameter_edits.rescan_values();
        log::debug!("state.load: restored byte_len={byte_len}");
        true
    })
}
