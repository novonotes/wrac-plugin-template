use std::sync::Arc;

use serde::{Deserialize, Serialize};
use wrac_clap_adapter::{PluginError, PluginResult, PluginStateExtension, State};

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
}

pub(super) struct WracGainStateExtension {
    project_state: Arc<ProjectStateStore>,
    shared: Arc<SharedState>,
    gui_notifier: Arc<GuiStateNotifier>,
}

impl WracGainStateExtension {
    pub(super) fn new(
        project_state: Arc<ProjectStateStore>,
        shared: Arc<SharedState>,
        gui_notifier: Arc<GuiStateNotifier>,
    ) -> Self {
        Self {
            project_state,
            shared,
            gui_notifier,
        }
    }
}

// `save_state` is called on project save, `restore_state` on load. The byte format is
// unrestricted, so JSON is used here to keep project-state payloads inspectable.
impl PluginStateExtension for WracGainStateExtension {
    fn save_state(&self) -> PluginResult<State> {
        let project = self.project_state.snapshot();
        let params = self.shared.snapshot_parameters();
        let bytes = serde_json::to_vec(&SavedState {
            gain: params.gain,
            bypass: params.bypass,
            editor_page: project.editor_page,
        })
        .map_err(|_| PluginError::InvalidState)?;
        Ok(State { bytes })
    }

    fn restore_state(&self, state: State) -> PluginResult<()> {
        log::debug!("restoring plugin state: byte_count={}", state.bytes.len());
        let state: SavedState =
            serde_json::from_slice(&state.bytes).map_err(|_| PluginError::InvalidState)?;
        let project = ProjectState {
            editor_page: state.editor_page,
        };
        self.project_state.commit(project);
        self.shared.restore_parameters(ParameterStateSnapshot {
            gain: state.gain,
            bypass: state.bypass,
        });
        notify_gui_parameters(&self.shared, |parameter_id, value| {
            self.gui_notifier.notify_parameter(parameter_id, value);
        });
        self.gui_notifier.notify_editor_page(project.editor_page);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_state_accepts_legacy_gain_only_payload() {
        let state: SavedState = serde_json::from_str(r#"{"gain":1.25}"#).unwrap();
        assert_eq!(state.gain, 1.25);
        assert!(!state.bypass);
        assert_eq!(state.editor_page, EditorPage::Controls);
    }
}
