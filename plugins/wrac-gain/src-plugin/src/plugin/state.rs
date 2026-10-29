use std::sync::Arc;

use serde::{Deserialize, Serialize};
use wrac_clap_adapter::interface::{PluginError, PluginResult, PluginStateExtension, State};

use super::latency::{LatencyStore, is_latency_frame_choice};
use crate::gui::GuiStateNotifier;
use crate::plugin::notify_gui_parameters;
use crate::state::{
    EditorPage, ParameterStateSnapshot, ProjectState, ProjectStateStore, SharedState,
};

/// Serialisation format (JSON) for the plugin state saved in a DAW project.
///
/// Realtime parameters are snapshotted from [`SharedState`] and editor-only state from
/// [`ProjectStateStore`]; both are merged into this single format before passing to the host.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SavedState {
    pub(crate) gain: f32,
    // Defaults keep projects saved by the earlier one-parameter template loadable.
    // Removing them requires an explicit state-version migration plan.
    #[serde(default)]
    pub(crate) bypass: bool,
    #[serde(default)]
    pub(crate) editor_page: EditorPage,
    // Absent from projects saved before the delay setting existed, which had none.
    #[serde(default)]
    pub(crate) latency_frames: u32,
}

pub(super) struct WracGainStateExtension {
    project_state: Arc<ProjectStateStore>,
    shared: Arc<SharedState>,
    latency: Arc<LatencyStore>,
    gui_notifier: Arc<GuiStateNotifier>,
}

impl WracGainStateExtension {
    pub(super) fn new(
        project_state: Arc<ProjectStateStore>,
        shared: Arc<SharedState>,
        latency: Arc<LatencyStore>,
        gui_notifier: Arc<GuiStateNotifier>,
    ) -> Self {
        Self {
            project_state,
            shared,
            latency,
            gui_notifier,
        }
    }
}

// `save_state` is called on project save, `restore_state` on load. The byte format is
// unrestricted, so JSON is used here to keep project-state payloads inspectable.
impl PluginStateExtension for WracGainStateExtension {
    fn save_state(&self) -> PluginResult<wrac_clap_adapter::interface::PreparedStateSave> {
        let project = self.project_state.snapshot();
        let params = self.shared.snapshot_parameters();
        let bytes = serde_json::to_vec(&SavedState {
            gain: params.gain,
            bypass: params.bypass,
            editor_page: project.editor_page,
            latency_frames: self.latency.selected_frames(),
        })
        .map_err(|_| PluginError::InvalidState)?;
        Ok(wrac_clap_adapter::interface::PreparedStateSave::new(
            State { bytes },
        ))
    }

    fn restore_state(&self, state: State) -> PluginResult<()> {
        log::debug!("restoring plugin state: byte_count={}", state.bytes.len());
        let state: SavedState =
            serde_json::from_slice(&state.bytes).map_err(|_| PluginError::InvalidState)?;
        // Reject the whole payload rather than loading a delay the processor was never
        // built for; a failed restore leaves the current state untouched.
        if !is_latency_frame_choice(state.latency_frames) {
            return Err(PluginError::InvalidState);
        }
        let project = ProjectState {
            editor_page: state.editor_page,
        };
        self.project_state.commit(project);
        // A restored delay is selected like a GUI choice: the audio follows at once and
        // the reported latency through the reactivation this asks for.
        self.latency.select(state.latency_frames);
        self.shared.restore_parameters(ParameterStateSnapshot {
            gain: state.gain,
            bypass: state.bypass,
        });
        notify_gui_parameters(&self.shared, |parameter_id, value| {
            self.gui_notifier.notify_parameter(parameter_id, value);
        });
        self.gui_notifier.notify_editor_page(project.editor_page);
        self.gui_notifier.notify_latency(state.latency_frames);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use wrac_clap_adapter::interface::{HostLifecycle, HostState};

    use super::*;
    use crate::commands::select_latency_from_gui;

    struct TestHostLifecycle;

    impl HostLifecycle for TestHostLifecycle {
        fn request_restart(&self) {}
        fn request_process(&self) {}
        fn request_callback(&self) {}
    }

    #[derive(Default)]
    struct TestHostState(AtomicUsize);

    impl HostState for TestHostState {
        fn mark_dirty(&self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn latency_edit_marks_host_dirty_but_restore_does_not() {
        let latency = Arc::new(LatencyStore::new(Arc::new(TestHostLifecycle)));
        let host = TestHostState::default();
        let state = WracGainStateExtension::new(
            Arc::new(ProjectStateStore::new()),
            Arc::new(SharedState::new()),
            latency.clone(),
            Arc::new(GuiStateNotifier::new()),
        );

        select_latency_from_gui(&latency, &host, 0).unwrap();
        assert_eq!(host.0.load(Ordering::Relaxed), 0);
        select_latency_from_gui(&latency, &host, 64).unwrap();
        assert_eq!(host.0.load(Ordering::Relaxed), 1);
        let saved = state.save_state().unwrap().state;
        select_latency_from_gui(&latency, &host, 64).unwrap();
        assert!(select_latency_from_gui(&latency, &host, 65).is_err());
        assert_eq!(latency.selected_frames(), 64);
        assert_eq!(host.0.load(Ordering::Relaxed), 1);

        select_latency_from_gui(&latency, &host, 256).unwrap();
        assert_eq!(host.0.load(Ordering::Relaxed), 2);
        state.restore_state(saved).unwrap();
        assert_eq!(latency.selected_frames(), 64);
        assert_eq!(host.0.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn saved_state_accepts_legacy_gain_only_payload() {
        let state: SavedState = serde_json::from_str(r#"{"gain":1.25}"#).unwrap();
        assert_eq!(state.gain, 1.25);
        assert!(!state.bypass);
        assert_eq!(state.editor_page, EditorPage::Controls);
        assert_eq!(state.latency_frames, 0);
    }
}
