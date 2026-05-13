//! audio / GUI / host から共有される plugin state。
//!
//! この module は「値の SoT」と「状態不整合を防ぐ最小限の操作」だけを持つ。
//! GUI への配送や host への edit 通知は、それぞれ `gui.rs` / `commands.rs` 側で扱う。

use std::sync::atomic::Ordering;

use atomic_float::AtomicF32;

use crate::plugin::{DEFAULT_GAIN, PARAM_GAIN_ID, clamp_gain};

/// audio processor / GUI / host からの問い合わせ が共有する thread-safe な state。
///
/// gain の値などは複数の thread から触られる:
/// - audio thread : [`wrac_clap_adapter::Processor::process`] の中で gain を読んで音に掛ける
/// - GUI thread   : ユーザーが slider を動かして gain を書き換える
/// - host thread  : [`wrac_clap_adapter::PluginParameters::parameter_value`] などで host が値を尋ねてくる
///
/// そのため値の "Single Source of Truth (SoT)" を [`crate::plugin::WracGainPlugin`] の私有
/// field に置くのではなく、[`std::sync::Arc`]<[`SharedState`]> として共有する。lock 不要な
/// [`AtomicF32`] を使うことで audio thread を待たせない実装になっている。
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

    /// 指定された parameter の現在値を返す。
    ///
    /// 新しい parameter を追加するときは、この `match parameter_id` に読み出し処理を
    /// 追加する。GUI command は parameter id だけを見るので、command 名は増やさなくてよい。
    pub(crate) fn parameter_value(&self, parameter_id: u32) -> Option<f32> {
        match parameter_id {
            PARAM_GAIN_ID => Some(self.gain()),
            _ => None,
        }
    }

    /// 外部から来た parameter 値を有効範囲に収めて SoT に保存する。
    ///
    /// 新しい parameter を追加するときは、この `match parameter_id` に保存処理を
    /// 追加する。各 parameter の clamp / normalization はここで完結させる。
    pub(crate) fn set_parameter_value(&self, parameter_id: u32, value: f64) -> Option<f32> {
        match parameter_id {
            PARAM_GAIN_ID => {
                // 範囲外の値が automation/UI から来ても問題ないように、必ず clamp してから保存する。
                let gain = clamp_gain(value as f32);
                self.gain.store(gain, Ordering::Release);
                Some(gain)
            }
            _ => None,
        }
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
