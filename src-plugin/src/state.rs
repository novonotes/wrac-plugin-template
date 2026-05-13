//! audio / GUI / host から共有される plugin state。
//!
//! この module は「値の SoT」と「状態不整合を防ぐ最小限の操作」だけを持つ。
//! GUI への配送や host への edit 通知は、それぞれ `gui.rs` / `commands.rs` 側で扱う。

use std::sync::atomic::Ordering;

use atomic_float::AtomicF32;

use crate::plugin::{DEFAULT_GAIN, clamp_gain};

/// audio processor / GUI / host からの問い合わせ が共有する thread-safe な state。
///
/// gain の値などは複数の thread から触られる:
/// - audio thread : `process()` の中で gain を読んで音に掛ける
/// - GUI thread   : ユーザーが slider を動かして gain を書き換える
/// - host thread  : `parameter_value()` などで host が値を尋ねてくる
///
/// そのため値の "Single Source of Truth (SoT)" を `WxpExampleGainPlugin` の私有
/// field に置くのではなく、`Arc<SharedState>` として共有する。lock 不要な
/// `AtomicF32` を使うことで audio thread を待たせない実装になっている。
pub(crate) struct SharedState {
    // gain の現在値 (線形 amplitude)。lock-free に読み書きする。
    gain: AtomicF32,
}

impl SharedState {
    pub(crate) fn new() -> Self {
        Self {
            gain: AtomicF32::new(DEFAULT_GAIN),
        }
    }

    pub(crate) fn gain(&self) -> f32 {
        self.gain.load(Ordering::Acquire)
    }

    /// 外部から来た gain を有効範囲に収めて SoT に保存する。
    pub(crate) fn set_gain(&self, gain: f64) -> f32 {
        // 範囲外の値が automation/UI から来ても問題ないように、必ず clamp してから保存する。
        let gain = clamp_gain(gain as f32);
        self.gain.store(gain, Ordering::Release);
        gain
    }
}

#[cfg(test)]
mod tests {
    use super::SharedState;

    const fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn shared_state_is_send_sync() {
        assert_send_sync::<SharedState>();
    }
}
