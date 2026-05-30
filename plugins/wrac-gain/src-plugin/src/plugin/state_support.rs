use std::sync::Arc;

use serde::{Deserialize, Serialize};
use wrac_clap_adapter::{PluginError, PluginResult, PluginState, PluginStateSupport};

use crate::gui::GuiStateNotifier;
use crate::plugin::{PARAM_BYPASS_ID, PARAM_GAIN_ID};
use crate::state::{
    EditorPage, ParameterStateSnapshot, ProjectState, ProjectStateStore, SharedState,
};

/// Serialisation format (JSON) for the plugin state saved in a DAW project.
///
/// Realtime parameters are snapshotted from [`SharedState`] and editor-only state from
/// [`ProjectStateStore`]; both are merged into this single format before passing to the host.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SavedPluginState {
    pub(crate) gain: f32,
    #[serde(default)]
    pub(crate) bypass: bool,
    #[serde(default)]
    pub(crate) editor_page: EditorPage,
}

pub(super) struct WracGainStateSupport {
    project_state: Arc<ProjectStateStore>,
    shared: Arc<SharedState>,
    gui_notifier: Arc<GuiStateNotifier>,
}

impl WracGainStateSupport {
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
// unrestricted, so JSON is used here for ease of debugging.
impl PluginStateSupport for WracGainStateSupport {
    fn save_state(&self) -> PluginResult<PluginState> {
        let project = self.project_state.snapshot();
        let params = self.shared.snapshot_parameters();
        log::debug!(
            "saving plugin state: gain={}, bypass={}, editor_page={}",
            params.gain,
            params.bypass,
            project.editor_page.as_str()
        );
        let bytes = serde_json::to_vec(&SavedPluginState {
            gain: params.gain,
            bypass: params.bypass,
            editor_page: project.editor_page,
        })
        .map_err(|_| PluginError::InvalidState)?;
        Ok(PluginState { bytes })
    }

    fn restore_state(&self, state: PluginState) -> PluginResult<()> {
        log::debug!("restoring plugin state: byte_count={}", state.bytes.len());
        let state: SavedPluginState =
            serde_json::from_slice(&state.bytes).map_err(|_| PluginError::InvalidState)?;
        let project = ProjectState {
            editor_page: state.editor_page,
        };
        self.project_state.commit(project);
        self.shared.restore_parameters(ParameterStateSnapshot {
            gain: state.gain,
            bypass: state.bypass,
        });
        self.gui_notifier
            .notify_parameter(PARAM_GAIN_ID, self.shared.gain());
        self.gui_notifier
            .notify_parameter(PARAM_BYPASS_ID, f32::from(self.shared.bypass()));
        self.gui_notifier.notify_editor_page(project.editor_page);
        Ok(())
    }
}
