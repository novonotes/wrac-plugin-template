//! audio thread 上で動く DSP。
//!
//! このサンプルは入力に gain を掛けて書き戻すだけ。[`Processor::process`] は
//! 小さな buffer ごとに繰り返し呼ばれる realtime 関数なので、**allocation と
//! lock を避ける**のが鉄則。共有 state は [`SharedState`] から lock-free に読む。

use std::sync::Arc;

use wrac_clap_adapter::{
    AudioPairedChannels, AudioPortChannels, AudioProcessBuffer, InputEvent, PluginResult,
    ProcessContext, ProcessStatus, Processor,
};

use crate::plugin::{PARAM_BYPASS_ID, PARAM_GAIN_ID, host_value_to_gain};
use crate::state::SharedState;

/// `activate()` で生成され、host の audio thread が所有する DSP 実体。
/// `deactivate()` まで生き続け、その間 `process()` が何度も呼ばれる。
///
/// field は **audio thread が待たずに読めるもの**だけにする。`shared` は atomic
/// なので process 中に読める。`audio_channel_count` は activate 時点で
/// plugin の audio layout store から copy した snapshot。adapter が active 中の
/// layout 変更を拒否するので、走行中の Processor の契約は途中で変わらない。製品で
/// layout に応じ DSP を変える場合も、`Arc<RwLock<Layout>>` を持たせず activate
/// 時に必要な設定へ変換して渡すのが安全。
pub(crate) struct WracGainAudioProcessor {
    shared: Arc<SharedState>,
    // gain 自体は channel count を使わないが、「layout は activate で snapshot して
    // field に持つ」形をテンプレートとして示すために保持する。
    // debug build では実 buffer がこの snapshot と一致するか検査する。
    audio_channel_count: u32,
}

impl WracGainAudioProcessor {
    pub(crate) fn new(shared: Arc<SharedState>, audio_channel_count: u32) -> Self {
        Self {
            shared,
            audio_channel_count,
        }
    }
}

impl Processor for WracGainAudioProcessor {
    /// 1 ブロック分を処理する。`context` には入出力 `audio`、このブロックの
    /// parameter event 列 `events.input`、sample 数 `frames_count` が入る。
    ///
    /// parameter event の発生時刻で buffer を区切り、区間ごとに当時の gain を
    /// 掛けることで sample 精度の automation を実現する (event 間は gain 一定)。
    fn process(&mut self, context: ProcessContext<'_>) -> PluginResult<ProcessStatus> {
        #[cfg(debug_assertions)]
        {
            // allocation 違反を即 abort。DAW/adapter が panic を握りつぶしても
            // 見逃さないため、debug build で全 process を包む。
            assert_no_alloc::assert_no_alloc(|| self.process_no_alloc(context))
        }

        #[cfg(not(debug_assertions))]
        {
            self.process_no_alloc(context)
        }
    }
}

impl WracGainAudioProcessor {
    fn process_no_alloc(&mut self, mut context: ProcessContext<'_>) -> PluginResult<ProcessStatus> {
        #[cfg(debug_assertions)]
        assert_audio_layout_matches_processor_snapshot(
            &mut context.audio,
            self.audio_channel_count,
        );

        // ブロック開始時点の gain。event が来るたびに更新される。
        let mut gain = self.shared.gain();
        let mut bypass = self.shared.bypass();
        // 「ここまで処理した」位置を表すカーソル。
        let mut segment_start = 0;
        let frames_count = context.frames_count as usize;

        for event in context.events.input.iter() {
            // event 発生位置までを現在の gain で処理する。
            // event time は host から信用しない (= buffer 範囲外を防ぐ) ため clamp。
            let event_time = (event.time() as usize).min(frames_count);
            if event_time > segment_start {
                let effective_gain = if bypass { 1.0 } else { gain };
                process_audio_range(
                    &mut context.audio,
                    segment_start,
                    event_time,
                    effective_gain,
                )?;
                segment_start = event_time;
            }

            // 今回扱うのは gain / bypass の parameter event だけ。それ以外 (note 等) は無視。
            if let InputEvent::ParamValue(event) = event {
                if event.parameter_id == PARAM_GAIN_ID {
                    gain = self
                        .shared
                        .set_parameter_value(event.parameter_id, host_value_to_gain(event.value))
                        .unwrap_or(gain);
                } else if event.parameter_id == PARAM_BYPASS_ID {
                    bypass = self
                        .shared
                        .set_parameter_value(event.parameter_id, event.value)
                        .map(|value| value >= 0.5)
                        .unwrap_or(bypass);
                }
            }
        }

        // 最後の event 以降、ブロック末尾まで残った範囲を処理する。
        if segment_start < frames_count {
            let effective_gain = if bypass { 1.0 } else { gain };
            process_audio_range(
                &mut context.audio,
                segment_start,
                frames_count,
                effective_gain,
            )?;
        }

        // 入力が無音でなければ次のブロックも処理を続けてほしい、という宣言。
        // `Quiet` を返すと host が optimization の判断材料に使う。
        Ok(ProcessStatus::ContinueIfNotQuiet)
    }
}

#[cfg(debug_assertions)]
fn assert_audio_layout_matches_processor_snapshot(
    audio: &mut AudioProcessBuffer<'_>,
    expected_channel_count: u32,
) {
    // activate 時の snapshot と実 buffer が一致するかを debug build で確認するだけ。
    // store の lock は読まない。memory safety を不正 buffer から守る責務は adapter
    // 側にあり、これはその代替ではなく snapshot の使い方を示すデモ。channel count
    // を使わない製品 DSP ならこの assertion ごと削ってよい。
    debug_assert_eq!(
        audio.port_pair_count(),
        1,
        "WRAC Gain expects exactly one main audio port pair"
    );

    for port_index in 0..audio.port_pair_count() {
        let Some(port_pair) = audio.port_pair(port_index) else {
            continue;
        };
        debug_assert_eq!(
            port_pair.channel_pair_count(),
            expected_channel_count as usize,
            "audio buffer channel count must match the layout captured at activate()"
        );
    }
}

/// 各 port の `[start, end)` 区間に gain を適用する。
/// host が渡す buffer は `f32` / `f64` どちらもあり得るので両方扱う。
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
