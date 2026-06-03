use clap_sys::ext::state::clap_plugin_state;
use clap_sys::plugin::clap_plugin;
use clap_sys::stream::{clap_istream, clap_ostream};

use super::PluginInstance;
use super::ffi::{ffi_bool, read_stream_exact, write_stream};
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
        let len = state.bytes.len() as u32;
        let ok = unsafe {
            write_stream(stream, &len.to_le_bytes()) && write_stream(stream, &state.bytes)
        };
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
        let Some(len_bytes) = (unsafe { read_stream_exact(stream, 4) }) else {
            log::warn!("state.load: failed to read state length");
            return false;
        };
        let len_bytes: [u8; 4] = match len_bytes.try_into() {
            Ok(bytes) => bytes,
            Err(_) => {
                log::warn!("state.load: invalid state length prefix");
                return false;
            }
        };
        let len = u32::from_le_bytes(len_bytes) as usize;
        if len > MAX_STATE_BYTES {
            log::warn!("state.load: state too large byte_len={len}");
            return false;
        }
        let Some(bytes) = (unsafe { read_stream_exact(stream, len) }) else {
            log::warn!("state.load: failed to read state payload byte_len={len}");
            return false;
        };

        let Some(state_support) = instance.state.as_ref() else {
            log::debug!("state.load: plugin has no state support");
            return false;
        };
        if let Err(error) = state_support.restore_state(State { bytes }) {
            log::warn!("state.load: plugin restore_state failed: {error}");
            return false;
        }
        instance.parameter_edits.rescan_values();
        log::debug!("state.load: restored byte_len={len}");
        true
    })
}
