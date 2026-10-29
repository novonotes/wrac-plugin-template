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

/// Delay settings offered to the user, in samples.
pub(crate) const LATENCY_FRAME_CHOICES: [u32; 5] = [0, 64, 256, 1024, 4096];

/// The longest selectable delay. The processor sizes its delay buffers for it, so a new
/// selection can take effect without allocating on the audio thread.
pub(crate) const MAX_LATENCY_FRAMES: u32 = 4096;

pub(crate) fn is_latency_frame_choice(frames: u32) -> bool {
    LATENCY_FRAME_CHOICES.contains(&frames)
}

/// The selected delay and the latency currently reported for it.
///
/// A new selection reaches the audio immediately: the plugin does not make its own
/// processing wait for the host. CLAP only allows the reported latency to change while
/// the plugin is being activated, so the report follows through the reactivation this
/// store asks the host for. Until the host performs it, the audio is delayed by the new
/// amount while the old latency is still reported. A host that never reactivates the
/// plugin keeps compensating the old latency, which is the host's part to correct.
pub(crate) struct LatencyStore {
    host_lifecycle: Arc<dyn HostLifecycle>,
    // Read by the audio thread every block.
    selected_frames: AtomicU32,
    active_frames: AtomicU32,
}

impl LatencyStore {
    pub(crate) fn new(host_lifecycle: Arc<dyn HostLifecycle>) -> Self {
        Self {
            host_lifecycle,
            selected_frames: AtomicU32::new(0),
            active_frames: AtomicU32::new(0),
        }
    }

    /// The delay the audio is processed with. `[realtime-safe & thread-safe]`
    pub(crate) fn selected_frames(&self) -> u32 {
        self.selected_frames.load(Ordering::Acquire)
    }

    /// Selects a delay from [`LATENCY_FRAME_CHOICES`] (a GUI choice or a restored
    /// state) and asks the host to reactivate the plugin when it differs from the
    /// reported latency. `[non-realtime]`
    pub(crate) fn select(&self, frames: u32) {
        self.selected_frames.store(frames, Ordering::Release);
        if frames != self.active_frames.load(Ordering::Acquire) {
            // Some hosts ignore a restart request while the plugin is inactive. The
            // report still catches up: activate() reads the selection every time.
            self.host_lifecycle.request_restart();
        }
    }

    /// Reports the selected delay as the latency of the processor being activated.
    /// Returns whether it differs from the latency reported before.
    pub(crate) fn activate(&self) -> bool {
        let frames = self.selected_frames.load(Ordering::Acquire);
        self.active_frames.swap(frames, Ordering::AcqRel) != frames
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
        // The latency reported at the last activation, as CLAP requires, not the
        // selection that may be waiting for the next one.
        self.latency.active_frames.load(Ordering::Acquire)
    }
}
