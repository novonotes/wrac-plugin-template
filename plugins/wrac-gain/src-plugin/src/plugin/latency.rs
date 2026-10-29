//! Latency reporting, and the delay setting that changes it.
//!
//! WRAC Gain itself needs no latency. The delay setting is here to show how a plugin
//! changes its reported latency, and to give wrappers and hosts a plugin to test that
//! path with.
//!
//! The setting is editor state rather than a host parameter. A setting that changes
//! latency makes the host re-run its delay compensation, which hosts do not expect from
//! automation, so plugins commonly keep such settings out of the parameter list.
//! Changing it only ever happens on a non-realtime thread (a GUI command or a state
//! restore), which is also where `host.request_restart()` may be called from.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use wrac_clap_adapter::interface::{HostLifecycle, PluginLatencyExtension};

use crate::state::ProjectStateStore;

/// Delay settings offered to the user, in samples.
pub(crate) const LATENCY_FRAME_CHOICES: [u32; 5] = [0, 64, 256, 1024, 4096];

pub(crate) fn is_latency_frame_choice(frames: u32) -> bool {
    LATENCY_FRAME_CHOICES.contains(&frames)
}

/// Owns the gap between the delay the user selected and the latency currently reported.
///
/// CLAP only allows the reported latency to change while the plugin is being activated.
/// A new selection is therefore stored in the project state and applied by the next
/// activation, which this store asks the host for. Until then the plugin keeps
/// processing with the latency it reported, so audio and the host's compensation stay
/// consistent.
pub(crate) struct LatencyStore {
    project_state: Arc<ProjectStateStore>,
    host_lifecycle: Arc<dyn HostLifecycle>,
    active_frames: AtomicU32,
}

impl LatencyStore {
    pub(crate) fn new(
        project_state: Arc<ProjectStateStore>,
        host_lifecycle: Arc<dyn HostLifecycle>,
    ) -> Self {
        Self {
            project_state,
            host_lifecycle,
            active_frames: AtomicU32::new(0),
        }
    }

    /// Records the selected delay and asks the host to reactivate the plugin when it
    /// differs from the reported latency. `[non-realtime]`
    pub(crate) fn select(&self, frames: u32) {
        self.project_state.set_latency_frames(frames);
        self.request_restart_if_pending();
    }

    /// Asks for a reactivation when the stored selection is not the reported latency,
    /// for example after a state restore replaced the selection. `[non-realtime]`
    pub(crate) fn request_restart_if_pending(&self) {
        if self.project_state.latency_frames() != self.active_frames.load(Ordering::Acquire) {
            // Some hosts ignore a restart request while the plugin is inactive. The
            // selection is still applied: activate() reads it every time.
            self.host_lifecycle.request_restart();
        }
    }

    /// Takes the selected delay as the latency of the processor being activated.
    /// Returns the delay and whether it differs from the latency reported before.
    pub(crate) fn activate(&self) -> (u32, bool) {
        let frames = self.project_state.latency_frames();
        let previous = self.active_frames.swap(frames, Ordering::AcqRel);
        (frames, frames != previous)
    }
}

pub(super) struct WracGainLatencyExtension {
    latency: Arc<LatencyStore>,
}

impl WracGainLatencyExtension {
    pub(super) fn new(latency: Arc<LatencyStore>) -> Self {
        Self { latency }
    }
}

impl PluginLatencyExtension for WracGainLatencyExtension {
    fn latency_frames(&self) -> u32 {
        // The latency of the active processor, not the pending selection: hosts query
        // this to compensate the audio the plugin is producing now.
        self.latency.active_frames.load(Ordering::Acquire)
    }
}
