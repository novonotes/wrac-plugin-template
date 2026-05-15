//! audio / GUI / host が共有する plugin state。
//!
//! ここは「値の SoT」と「不整合を防ぐ最小限の操作」だけを持つ。GUI への配送や
//! host への edit 通知は `gui.rs` / `commands.rs` の責務。

use std::sync::atomic::{AtomicBool, Ordering};

use atomic_float::AtomicF32;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::plugin::{DEFAULT_GAIN, PARAM_BYPASS_ID, PARAM_GAIN_ID, clamp_gain};

/// project に保存するが audio thread からは読まない editor state の例。
/// 製品では IR path、track color、editor-only preference などがこの系統。
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

/// project に保存する non-realtime state。
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ProjectState {
    pub(crate) editor_page: EditorPage,
}

/// [`ProjectState`] の SoT。audio thread はこの lock を触らない。
///
/// lock は snapshot / commit だけに使い、lock 中に serialize・host callback・
/// GUI dispatch・file IO を挟まないこと (lock 保持時間を最短に保つため)。
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

/// realtime parameter の現在値。3 つの thread から並行に触られる:
/// - audio thread: `process()` で gain を読んで音に掛ける
/// - GUI thread  : slider 操作で書き換える
/// - host thread : `parameter_value()` などで host が問い合わせる
///
/// audio thread が lock を待たないよう、lock ではなく atomic を使う。
/// project にだけ残す non-realtime state は [`ProjectStateStore`] 側に分離する。
pub(crate) struct SharedState {
    // 線形 amplitude。
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
        // gain/bypass 間に transaction 境界が無いので単純な atomic load で十分。
        // 複数 field の完全一貫 snapshot が要る製品では、ここに seqlock 風の
        // generation check を閉じ込める。
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

    /// parameter の現在値。新しい parameter は match に追加する
    /// (GUI command は id だけを見るので command 名は増えない)。
    pub(crate) fn parameter_value(&self, parameter_id: u32) -> Option<f32> {
        match parameter_id {
            PARAM_GAIN_ID => Some(self.gain()),
            PARAM_BYPASS_ID => Some(f32::from(self.bypass())),
            _ => None,
        }
    }

    /// 外部から来た値を範囲に収めて SoT に保存する。各 parameter の clamp /
    /// normalization はここで完結させる。新しい parameter は match に追加する。
    pub(crate) fn set_parameter_value(&self, parameter_id: u32, value: f64) -> Option<f32> {
        match parameter_id {
            PARAM_GAIN_ID => {
                // automation/UI から範囲外が来ても安全なよう必ず clamp。
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
