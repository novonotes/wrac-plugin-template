use clap_sys::ext::state::clap_plugin_state;
use clap_sys::plugin::clap_plugin;
use clap_sys::stream::{clap_istream, clap_ostream};

use super::PluginInstanceState;
use super::ffi::{ffi_bool, read_stream_to_end, write_stream};

pub(super) static STATE: clap_plugin_state = clap_plugin_state {
    save: Some(state_save),
    load: Some(state_load),
};

const MAX_STATE_BYTES: usize = 64 * 1024 * 1024;

// State callbacks may arrive while the plugin is active, depending on the host format.
// Waiting for or giving up on the `PluginInstance` write lock here could silently drop a project save,
// so only the thread-safe state capability fixed during plugin initialization is called.
unsafe extern "C" fn state_save(plugin: *const clap_plugin, stream: *const clap_ostream) -> bool {
    ffi_bool(|| {
        if stream.is_null() {
            log::warn!("state.save: null stream");
            return false;
        }
        let Some(instance) = (unsafe { PluginInstanceState::from_plugin(plugin) }) else {
            log::warn!("state.save: missing plugin instance");
            return false;
        };
        let Some(state_support) = instance
            .runtime
            .get()
            .and_then(|runtime| runtime.state.as_ref())
        else {
            log::debug!("state.save: plugin has no state support");
            return false;
        };
        let prepared = match state_support.save_state() {
            Ok(prepared) => prepared,
            Err(error) => {
                log::warn!("state.save: plugin save_state failed: {error}");
                return false;
            }
        };
        let ok = unsafe { write_stream(stream, &prepared.state.bytes) };
        if !ok {
            log::warn!(
                "state.save: writing state stream failed byte_len={}",
                prepared.state.bytes.len()
            );
        } else {
            log::debug!("state.save: wrote byte_len={}", prepared.state.bytes.len());
        }
        complete_state_save(prepared.completion, ok);
        ok
    })
}

fn complete_state_save(
    completion: Option<Box<dyn crate::StateSaveCompletion>>,
    write_succeeded: bool,
) {
    if let Some(completion) = completion {
        completion.complete(if write_succeeded {
            crate::StateSaveOutcome::Written
        } else {
            crate::StateSaveOutcome::StreamWriteFailed
        });
    }
}

unsafe extern "C" fn state_load(plugin: *const clap_plugin, stream: *const clap_istream) -> bool {
    ffi_bool(|| {
        if stream.is_null() {
            log::warn!("state.load: null stream");
            return false;
        }
        let Some(instance) = (unsafe { PluginInstanceState::from_plugin(plugin) }) else {
            log::warn!("state.load: missing plugin instance");
            return false;
        };
        if instance.is_in_realtime_callback() {
            wrac_log::rtwarn!("state.load: rejected from realtime callback");
            return false;
        }
        let Some(bytes) = (unsafe { read_stream_to_end(stream, MAX_STATE_BYTES) }) else {
            log::warn!("state.load: failed to read state stream");
            return false;
        };

        let Some(state_support) = instance
            .runtime
            .get()
            .and_then(|runtime| runtime.state.as_ref())
        else {
            log::debug!("state.load: plugin has no state support");
            return false;
        };
        let byte_len = bytes.len();
        let Some(_guard) = instance.enter_lifecycle_blocking_or_reject_reentry() else {
            log::warn!("state.load: rejected lifecycle re-entry");
            return false;
        };
        if let Err(error) = state_support.restore_state(crate::State { bytes }) {
            log::warn!("state.load: plugin restore_state failed: {error}");
            return false;
        }
        instance.host_params.rescan_values();
        log::debug!("state.load: restored byte_len={byte_len}");
        true
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use crate::{StateSaveCompletion, StateSaveOutcome};

    use super::complete_state_save;

    struct RecordingCompletion(Arc<Mutex<Vec<StateSaveOutcome>>>);

    impl StateSaveCompletion for RecordingCompletion {
        fn complete(self: Box<Self>, outcome: StateSaveOutcome) {
            self.0.lock().unwrap().push(outcome);
        }
    }

    #[test]
    fn completion_receives_the_matching_stream_write_outcome() {
        let outcomes = Arc::new(Mutex::new(Vec::new()));
        complete_state_save(
            Some(Box::new(RecordingCompletion(Arc::clone(&outcomes)))),
            true,
        );
        complete_state_save(
            Some(Box::new(RecordingCompletion(Arc::clone(&outcomes)))),
            false,
        );

        assert_eq!(
            *outcomes.lock().unwrap(),
            vec![
                StateSaveOutcome::Written,
                StateSaveOutcome::StreamWriteFailed
            ]
        );
    }
}
