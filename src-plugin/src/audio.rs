//! audio thread 上で動く DSP。
//!
//! このサンプルでは「入力 sample に gain を掛けて出力に書き戻す」だけの
//! 単純な処理を行う。[`Processor::process`] は host が決めた小さな buffer
//! (例: 512 sample) ごとに繰り返し呼び出される real-time な関数なので、
//! ここでは allocation や lock を避けるのが原則。
//!
//! 共有 state ([`SharedState`]) は [`atomic_float::AtomicF32`] などで lock-free に
//! 読めるようになっており、GUI thread が gain を更新しても audio 側が
//! ブロックされない設計になっている。

use std::sync::Arc;

use wrac_clap_adapter::{
    AudioPairedChannels, AudioPortChannels, AudioProcessBuffer, InputEvent, PluginResult,
    ProcessContext, ProcessStatus, Processor,
};

use crate::plugin::PARAM_GAIN_ID;
use crate::state::SharedState;

/// [`wrac_clap_adapter::PluginCore::activate`] で生成され、host の audio thread に所有される DSP 実体。
///
/// 中身は共有 state への [`Arc`] だけ。[`Processor`] instance は host が
/// [`wrac_clap_adapter::PluginCore::deactivate`] するまで生き続け、その間に何度も
/// [`Processor::process`] が呼ばれる。
pub(crate) struct WracGainAudioProcessor {
    shared: Arc<SharedState>,
}

impl WracGainAudioProcessor {
    pub(crate) fn new(shared: Arc<SharedState>) -> Self {
        Self { shared }
    }
}

impl Processor for WracGainAudioProcessor {
    /// 1 ブロック分の音を処理する。host から渡される `context` には:
    /// - `audio` : 入出力 buffer (channel ごとの sample 列)
    /// - `events.input` : このブロック内で発生する parameter event の列
    /// - `frames_count` : この呼び出しで処理する sample 数
    ///
    /// が入っている。
    ///
    /// このサンプルでは parameter event の発生時刻ごとに buffer を区切り、
    /// 区間ごとに当時の gain を掛けることで「sample 精度の automation」を
    /// 実現している (event 間は gain 一定として扱う)。
    fn process(&mut self, mut context: ProcessContext<'_>) -> PluginResult<ProcessStatus> {
        // ブロック開始時点の gain。event が来るたびに更新される。
        let mut gain = self.shared.gain();
        // 「ここまで処理した」位置を表すカーソル。
        let mut segment_start = 0;
        let frames_count = context.frames_count as usize;

        for event in context.events.input.iter() {
            // event 発生位置までを現在の gain で処理する。
            // event time は host から信用しない (= buffer 範囲外を防ぐ) ため clamp。
            let event_time = (event.time() as usize).min(frames_count);
            if event_time > segment_start {
                process_audio_range(&mut context.audio, segment_start, event_time, gain)?;
                segment_start = event_time;
            }

            // 今回扱うのは gain の parameter event だけ。それ以外 (note 等) は無視。
            if let InputEvent::ParamValue(event) = event {
                if event.parameter_id == PARAM_GAIN_ID {
                    gain = self
                        .shared
                        .set_parameter_value(event.parameter_id, event.value)
                        .unwrap_or(gain);
                }
            }
        }

        // 最後の event 以降、ブロック末尾まで残った範囲を処理する。
        if segment_start < frames_count {
            process_audio_range(&mut context.audio, segment_start, frames_count, gain)?;
        }

        // 入力が無音でなければ次のブロックも処理を続けてほしい、という宣言。
        // `Quiet` を返すと host が optimization の判断材料に使う。
        Ok(ProcessStatus::ContinueIfNotQuiet)
    }
}

/// [`AudioProcessBuffer`] 内の各 port について `[start, end)` の区間に gain を適用する。
///
/// host によっては buffer が `f32` のことも `f64` のこともあるので、両方の
/// ケースを [`AudioPortChannels`] の variant で処理する。
fn process_audio_range(
    audio: &mut AudioProcessBuffer<'_>,
    start: usize,
    end: usize,
    gain: f32,
) -> PluginResult<()> {
    let len = end.saturating_sub(start);
    for mut port_pair in audio {
        match port_pair.channels()? {
            AudioPortChannels::F32(channels) => process_channels_range(channels, start, len, gain),
            AudioPortChannels::F64(channels) => {
                process_channels_range(channels, start, len, gain as f64)
            }
        }
    }
    Ok(())
}

/// 1 つの port (paired in/out) の各 channel について sample に gain を掛ける。
///
/// `map_samples_range` は in-place 書き換えで、in/out が同じ buffer を指す
/// "in-place processing" にも対応している。
fn process_channels_range<T>(
    channels: AudioPairedChannels<'_, T>,
    start: usize,
    len: usize,
    gain: T,
) where
    T: Copy + Default + std::ops::Mul<Output = T>,
{
    for mut channel in channels {
        channel.map_samples_range(start, len, |sample| sample * gain);
    }
}
