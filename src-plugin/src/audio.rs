//! オーディオ処理モジュール。
//!
//! このモジュールはオーディオスレッド上で動作する。
//! オーディオスレッドはリアルタイム制約があり、以下の操作は禁止：
//!   - メモリ割り当て / 解放（malloc / free）
//!   - ロック取得（Mutex, RwLock 等）
//!   - I/O（ファイル、ネットワーク）
//!   - システムコール全般
//! これらを行うとオーディオのドロップアウト（ノイズ・途切れ）が発生する。
//! そのため、パラメータの受け渡しには AtomicF32 のようなロックフリーな仕組みを使う。

use clack_extensions::params::PluginAudioProcessorParams;
use clack_plugin::prelude::*;
use clack_plugin::process::audio::{ChannelPair, SampleType};

use crate::params::{apply_host_parameter_events, drain_ui_events};
use crate::plugin::{SharedState, WxpExampleGainMainThread};

/// オーディオスレッドで動作するプロセッサ。
/// SharedState への参照のみを持ち、Atomic 経由でパラメータを読み取る。
pub(crate) struct WxpExampleGainAudioProcessor<'a> {
    shared: &'a SharedState,
}

impl<'a> PluginAudioProcessor<'a, SharedState, WxpExampleGainMainThread<'a>>
    for WxpExampleGainAudioProcessor<'a>
{
    /// ホストがオーディオ処理を開始するときに呼ばれる（activate）。
    /// audio_config にはサンプルレートやバッファサイズの情報が含まれる。
    /// このプラグインはシンプルなゲインなのでこれらの情報は不要。
    fn activate(
        _host: HostAudioProcessorHandle<'a>,
        _main_thread: &mut WxpExampleGainMainThread<'a>,
        shared: &'a SharedState,
        _audio_config: PluginAudioConfiguration,
    ) -> Result<Self, PluginError> {
        Ok(Self { shared })
    }

    /// ホストがオーディオ処理を停止するときに呼ばれる（deactivate）。
    fn deactivate(self, _main_thread: &mut WxpExampleGainMainThread<'a>) {}

    /// 毎オーディオバッファごとに呼ばれるメインの処理関数。
    /// ホストは通常 44100Hz や 48000Hz のサンプルレートで、
    /// 64〜2048 サンプル程度のバッファ単位でこの関数を呼び出す。
    fn process(
        &mut self,
        _process: Process,
        mut audio: Audio,
        events: Events,
    ) -> Result<ProcessStatus, PluginError> {
        // UI からのパラメータ変更をホストに通知する（output events）。
        drain_ui_events(&self.shared.inner, events.output);
        // ホストからのパラメータ変更（オートメーション等）を反映する。
        apply_host_parameter_events(&self.shared.inner, events.input);

        // Atomic から現在のゲイン値を読み取る。ロックフリーなので安全。
        let gain = self.shared.inner.gain();
        // port_pair(0) で最初のオーディオポート（入力と出力のペア）を取得。
        let Some(mut port_pair) = audio.port_pair(0) else {
            return Ok(ProcessStatus::ContinueIfNotQuiet);
        };

        // ホストはサンプル形式として f32 または f64 を使用する。
        // どちらが来ても対応できるようにマッチする。
        match port_pair.channels()? {
            SampleType::F32(mut channels) => process_channels_f32(&mut channels, gain),
            SampleType::F64(mut channels) => process_channels_f64(&mut channels, gain as f64),
            // Both の場合は f64 側を処理する（ホストは f64 を優先する）。
            SampleType::Both(_, mut channels) => process_channels_f64(&mut channels, gain as f64),
        }

        // ContinueIfNotQuiet: 入力が無音になったら処理をスキップしてよいとホストに伝える。
        // 残響やディレイのように音が残るエフェクトでは Tail を返す必要がある。
        Ok(ProcessStatus::ContinueIfNotQuiet)
    }
}

/// オーディオスレッド上でのパラメータ flush。
/// process() が呼ばれていない間（再生停止中など）でも
/// パラメータの同期が必要な場合にホストから呼ばれる。
impl PluginAudioProcessorParams for WxpExampleGainAudioProcessor<'_> {
    fn flush(
        &mut self,
        input_parameter_changes: &InputEvents,
        output_parameter_changes: &mut OutputEvents,
    ) {
        drain_ui_events(&self.shared.inner, output_parameter_changes);
        apply_host_parameter_events(&self.shared.inner, input_parameter_changes);
    }
}

/// f32 サンプル形式のチャネル処理。
/// ChannelPair はホストがバッファを提供する 4 つのパターンに対応する：
fn process_channels_f32(
    channels: &mut clack_plugin::process::audio::PairedChannels<'_, f32>,
    gain: f32,
) {
    for pair in channels.iter_mut() {
        match pair {
            // 入力のみ（出力バッファなし）: 何もしない。
            ChannelPair::InputOnly(_) => {}
            // 出力のみ（入力バッファなし）: 無音で埋める。
            ChannelPair::OutputOnly(output) => output.fill(0.0),
            // 入出力が別バッファ: 入力にゲインを掛けて出力に書き込む。
            ChannelPair::InputOutput(input, output) => {
                for (src, dst) in input.iter().zip(output.iter_mut()) {
                    *dst = *src * gain;
                }
            }
            // インプレース処理: 入出力が同じバッファ。直接書き換える。
            // メモリ効率が良く、ホストが最も好む形式。
            ChannelPair::InPlace(buffer) => {
                for sample in buffer.iter_mut() {
                    *sample *= gain;
                }
            }
        }
    }
}

/// f64 サンプル形式のチャネル処理。ロジックは f32 版と同じ。
fn process_channels_f64(
    channels: &mut clack_plugin::process::audio::PairedChannels<'_, f64>,
    gain: f64,
) {
    for pair in channels.iter_mut() {
        match pair {
            ChannelPair::InputOnly(_) => {}
            ChannelPair::OutputOnly(output) => output.fill(0.0),
            ChannelPair::InputOutput(input, output) => {
                for (src, dst) in input.iter().zip(output.iter_mut()) {
                    *dst = *src * gain;
                }
            }
            ChannelPair::InPlace(buffer) => {
                for sample in buffer.iter_mut() {
                    *sample *= gain;
                }
            }
        }
    }
}
