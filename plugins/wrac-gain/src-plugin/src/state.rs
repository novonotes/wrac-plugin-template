//! Plugin state shared by the audio thread, GUI, and host.
//!
//! This module holds only the source of truth for values and the minimal operations
//! needed to prevent inconsistency. Delivering changes to the GUI and notifying the host
//! of edits are the responsibility of `gui.rs` and `commands.rs`.

use std::sync::atomic::{AtomicBool, Ordering};

use atomic_float::AtomicF32;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::plugin::{DEFAULT_GAIN, PARAM_BYPASS_ID, PARAM_GAIN_ID, clamp_gain};

/// Example of editor state that is saved with the project but never read from the audio thread.
/// In a real product this category includes things like IR paths, track colours, and
/// editor-only preferences.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum EditorPage {
    Controls,
    About,
}

impl Default for EditorPage {
    fn default() -> Self {
        Self::Controls
    }
}

impl EditorPage {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Controls => "controls",
            Self::About => "about",
        }
    }

    pub(crate) fn from_str(value: &str) -> Option<Self> {
        match value {
            "controls" => Some(Self::Controls),
            "about" => Some(Self::About),
            _ => None,
        }
    }
}

/// Non-realtime state saved with the project.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ProjectState {
    pub(crate) editor_page: EditorPage,
}

/// Source of truth for [`ProjectState`]. The audio thread never touches this lock.
///
/// The lock is used only for snapshot and commit operations. Do not perform serialisation,
/// host callbacks, GUI dispatch, or file I/O while holding it (to minimise lock duration).
pub(crate) struct ProjectStateStore {
    state: RwLock<ProjectState>,
}

impl ProjectStateStore {
    pub(crate) fn new() -> Self {
        Self {
            state: RwLock::new(ProjectState::default()),
        }
    }

    pub(crate) fn snapshot(&self) -> ProjectState {
        *self.state.read()
    }

    pub(crate) fn commit(&self, state: ProjectState) {
        *self.state.write() = state;
    }

    pub(crate) fn editor_page(&self) -> EditorPage {
        self.snapshot().editor_page
    }

    pub(crate) fn set_editor_page(&self, editor_page: EditorPage) {
        self.state.write().editor_page = editor_page;
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ParameterStateSnapshot {
    pub(crate) gain: f32,
    pub(crate) bypass: bool,
}

/// Current values of realtime parameters, accessed concurrently from three threads:
/// - audio thread: reads gain in `process()` and applies it to audio
/// - GUI thread  : writes values on slider interaction
/// - host thread : queries values via `parameter_value()` etc.
///
/// Atomics are used instead of a lock so the audio thread never has to wait.
/// Non-realtime state that belongs only in the project is separated into [`ProjectStateStore`].
pub(crate) struct SharedState {
    // Linear amplitude.
    gain: AtomicF32,
    bypass: AtomicBool,
}

impl SharedState {
    pub(crate) fn new() -> Self {
        Self {
            gain: AtomicF32::new(DEFAULT_GAIN),
            bypass: AtomicBool::new(false),
        }
    }

    pub(crate) fn gain(&self) -> f32 {
        self.gain.load(Ordering::Acquire)
    }

    pub(crate) fn bypass(&self) -> bool {
        self.bypass.load(Ordering::Acquire)
    }

    pub(crate) fn snapshot_parameters(&self) -> ParameterStateSnapshot {
        // No transaction boundary is needed between gain and bypass, so plain atomic
        // loads are sufficient. Products requiring a fully consistent multi-field snapshot
        // should add a seqlock-style generation check here.
        ParameterStateSnapshot {
            gain: self.gain(),
            bypass: self.bypass(),
        }
    }

    pub(crate) fn restore_parameters(&self, snapshot: ParameterStateSnapshot) {
        self.gain
            .store(clamp_gain(snapshot.gain), Ordering::Release);
        self.bypass.store(snapshot.bypass, Ordering::Release);
    }

    /// Returns the current plain value of a parameter. Schema, defaults, and host/text
    /// conversions live in `plugin::params`; this store only owns live values.
    pub(crate) fn parameter_value(&self, parameter_id: u32) -> Option<f32> {
        match parameter_id {
            PARAM_GAIN_ID => Some(self.gain()),
            PARAM_BYPASS_ID => Some(f32::from(self.bypass())),
            _ => None,
        }
    }

    /// Stores a plain parameter value as the source of truth. Keep any host-domain
    /// conversion out of this method so the realtime state remains a value store.
    pub(crate) fn set_parameter_value(&self, parameter_id: u32, value: f64) -> Option<f32> {
        match parameter_id {
            PARAM_GAIN_ID => {
                // Always clamp so out-of-range values from automation or the UI are safe.
                let gain = clamp_gain(value as f32);
                self.gain.store(gain, Ordering::Release);
                Some(gain)
            }
            PARAM_BYPASS_ID => {
                let bypass = value >= 0.5;
                self.bypass.store(bypass, Ordering::Release);
                Some(f32::from(bypass))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ProjectStateStore, SharedState};

    const fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn shared_state_is_send_sync() {
        assert_send_sync::<SharedState>();
    }

    #[test]
    fn project_state_store_is_send_sync() {
        assert_send_sync::<ProjectStateStore>();
    }
}
