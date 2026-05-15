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

use crate::plugin::{PARAM_BYPASS_ID, PARAM_GAIN_ID, host_value_to_gain};
use crate::state::SharedState;

/// [`wrac_clap_adapter::PluginCore::activate`] で生成され、host の audio thread に所有される DSP 実体。
///
/// [`Processor`] instance は host が [`wrac_clap_adapter::PluginCore::deactivate`] するまで
/// 生き続け、その間に何度も [`Processor::process`] が呼ばれる。
///
/// この型に入れる field は「audio thread が待たずに読めるもの」だけにします。
/// `shared` は atomic parameter store なので process 中に読めます。一方で
/// `audio_channel_count` は [`PluginCore::activate`](wrac_clap_adapter::PluginCore::activate)
/// 時点で non-realtime layout store から copy した snapshot です。
///
/// この snapshot が適切なのは、adapter が Processor の存在中に configurable-audio-ports の
/// layout apply を拒否するからです。つまり、layout store は次回 activate 用には更新されますが、
/// すでに走っている Processor の契約は途中で書き換わりません。
///
/// 製品 plugin で layout に応じて DSP graph や channel mapping を変える場合も、ここに
/// `Arc<RwLock<Layout>>` を渡すのではなく、activate 時に必要な設定へ変換して processor に
/// 持たせるのが安全です。
pub(crate) struct WracGainAudioProcessor {
    shared: Arc<SharedState>,
    // Gain の DSP 自体は channel count を必要としないが、template として
    // 「layout は activate 時に snapshot して Processor field にする」形を示すために保持する。
    // debug build では host から来た実 buffer がこの snapshot と合っているかを検査する。
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
    fn process(&mut self, context: ProcessContext<'_>) -> PluginResult<ProcessStatus> {
        #[cfg(debug_assertions)]
        {
            // 違反時は allocator error と backtrace で即座に失敗させる。
            // DAW や adapter が panic を握りつぶしても allocation 違反を見逃さないため。
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
    // Port layout は `activate()` 時に non-RT store から snapshot して Processor へ渡す。
    // audio thread では store の lock を読まず、この snapshot と実 buffer の整合だけを見る。
    //
    // これは adapter の buffer validation を補うものではなく、activate 時に取得した layout
    // snapshot を Processor 側でどう使うかを示すデモです。実際の製品 DSP が channel count を
    // 必要としないなら、この assertion 自体は削って構いません。host/wrapper 由来の不正 buffer
    // から memory safety を守る責務は adapter 側にあります。
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
