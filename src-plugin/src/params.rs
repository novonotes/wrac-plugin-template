//! パラメータの公開と状態の保存・復元。
//!
//! CLAP ではプラグインが「パラメータ」としてホストに値を公開する。
//! ホストはこれを使ってオートメーション記録、プリセット管理、GUI 表示を行う。
//!
//! また、PluginStateImpl でプラグインの状態をバイト列としてシリアライズし、
//! DAW のプロジェクトファイルに保存・復元する機能を提供する。

use std::ffi::CStr;
use std::fmt::Write as _;
use std::io::{Read, Write as _};

use clack_extensions::params::{
    ParamDisplayWriter, ParamInfo, ParamInfoFlags, ParamInfoWriter, PluginMainThreadParams,
};
use clack_extensions::state::PluginStateImpl;
use clack_plugin::events::event_types::{
    ParamGestureBeginEvent, ParamGestureEndEvent, ParamValueEvent,
};
use clack_plugin::events::spaces::CoreEventSpace;
use clack_plugin::prelude::*;
use clack_plugin::stream::{InputStream, OutputStream};
use serde_json::from_slice;
use serde_json::to_vec;

use crate::plugin::{
    DEFAULT_GAIN, PARAM_GAIN_ID, SavedPluginState, SharedStateInner, WxpExampleGainMainThread,
    clamp_gain, gain_db_text,
};

/// プラグイン状態の保存・復元（CLAP state 拡張）。
/// DAW がプロジェクトを保存/読み込みする際に呼ばれる。
/// フォーマットは自由だが、ここでは `[長さ (4 byte LE)] + [JSON バイト列]` を使用。
impl PluginStateImpl for WxpExampleGainMainThread<'_> {
    fn save(&mut self, output: &mut OutputStream) -> Result<(), PluginError> {
        let bytes = to_vec(&SavedPluginState {
            gain: self.shared.inner.gain(),
        })
        .map_err(|_| PluginError::Message("Failed to serialize plugin state"))?;

        // 長さプレフィックスを付けることで、将来フィールドが増えても安全に読める。
        output.write_all(&(bytes.len() as u32).to_le_bytes())?;
        output.write_all(&bytes)?;
        Ok(())
    }

    fn load(&mut self, input: &mut InputStream) -> Result<(), PluginError> {
        let mut len_buffer = [0_u8; 4];
        input.read_exact(&mut len_buffer)?;
        let len = u32::from_le_bytes(len_buffer) as usize;

        let mut bytes = vec![0_u8; len];
        input.read_exact(&mut bytes)?;

        let state: SavedPluginState = from_slice(&bytes)
            .map_err(|_| PluginError::Message("Failed to deserialize plugin state"))?;
        // 復元した値を SharedState に反映し、GUI にも通知する。
        self.shared.inner.set_gain_from_host(state.gain as f64);
        Ok(())
    }
}

/// CLAP パラメータのメインスレッド側実装。
/// ホストがパラメータの一覧取得、値の読み書き、テキスト変換を行う際に使う。
impl PluginMainThreadParams for WxpExampleGainMainThread<'_> {
    /// このプラグインが公開するパラメータの数。
    fn count(&mut self) -> u32 {
        1
    }

    /// パラメータの情報（ID, 名前, 範囲, フラグ等）をホストに伝える。
    fn get_info(&mut self, param_index: u32, info: &mut ParamInfoWriter) {
        if param_index != 0 {
            return;
        }

        info.set(&ParamInfo {
            id: PARAM_GAIN_ID,
            // IS_AUTOMATABLE: ホストがオートメーションカーブを描けるパラメータ。
            flags: ParamInfoFlags::IS_AUTOMATABLE,
            cookie: Default::default(),
            name: b"Gain",
            // module はパラメータをグループ化するパス（例: "EQ/Band1"）。
            // このプラグインはパラメータが 1 つだけなので空。
            module: b"",
            min_value: 0.0,
            max_value: 2.0,
            default_value: DEFAULT_GAIN as f64,
        });
    }

    /// ホストがパラメータの現在値を問い合わせるときに呼ばれる。
    fn get_value(&mut self, param_id: ClapId) -> Option<f64> {
        (param_id == PARAM_GAIN_ID).then(|| self.shared.inner.gain() as f64)
    }

    /// パラメータの数値を表示用テキストに変換する。
    /// ホストの UI がパラメータ値の横に "−6.0 dB" のように表示するために使う。
    fn value_to_text(
        &mut self,
        param_id: ClapId,
        value: f64,
        writer: &mut ParamDisplayWriter,
    ) -> std::fmt::Result {
        if param_id != PARAM_GAIN_ID {
            return Err(std::fmt::Error);
        }

        writer.write_str(&gain_db_text(clamp_gain(value as f32) as f64))
    }

    /// テキスト入力からパラメータ値に変換する（value_to_text の逆変換）。
    /// ユーザーがホスト UI で "-6 dB" と入力したときなどに使われる。
    fn text_to_value(&mut self, param_id: ClapId, text: &CStr) -> Option<f64> {
        if param_id != PARAM_GAIN_ID {
            return None;
        }

        let text = text.to_str().ok()?.trim();
        let text = text.strip_suffix("dB").unwrap_or(text).trim();
        let db = text.parse::<f64>().ok()?;
        // dB からリニアゲインに逆変換: gain = 10^(dB/20)
        Some(clamp_gain(10.0_f64.powf(db / 20.0) as f32) as f64)
    }

    /// メインスレッド上でのパラメータ flush。
    /// オーディオ処理が停止しているときにホストから呼ばれる。
    fn flush(
        &mut self,
        input_parameter_changes: &InputEvents,
        output_parameter_changes: &mut OutputEvents,
    ) {
        drain_ui_events(&self.shared.inner, output_parameter_changes);
        apply_host_parameter_events(&self.shared.inner, input_parameter_changes);
    }
}

/// UI からの pending フラグを読み取り、ホストへの output events として送出する。
/// process() や flush() の冒頭で呼ばれる。
///
/// イベントの順序は重要: begin → value → end の順でなければならない。
/// take_* は swap(false) なので、一度読んだフラグは消費される。
pub(crate) fn drain_ui_events(
    shared: &SharedStateInner,
    output_parameter_changes: &mut OutputEvents,
) {
    if shared.take_ui_gesture_begin() {
        let _ = output_parameter_changes.try_push(ParamGestureBeginEvent::new(0, PARAM_GAIN_ID));
    }

    if shared.take_ui_value_dirty() {
        let _ = output_parameter_changes.try_push(ParamValueEvent::new(
            0,
            PARAM_GAIN_ID,
            // Pckn::match_all() はすべての MIDI チャネル/ポートにマッチする指定。
            // ゲインパラメータは MIDI と無関係なのでワイルドカードで良い。
            clack_plugin::events::Pckn::match_all(),
            shared.gain() as f64,
            clack_plugin::utils::Cookie::empty(),
        ));
    }

    if shared.take_ui_gesture_end() {
        let _ = output_parameter_changes.try_push(ParamGestureEndEvent::new(0, PARAM_GAIN_ID));
    }
}

/// ホストからの入力イベント（オートメーション等）を処理し、SharedState に反映する。
/// イベントストリームから ParamValue イベントだけを抽出して適用する。
pub(crate) fn apply_host_parameter_events(shared: &SharedStateInner, events: &InputEvents) {
    for event in events {
        // コアイベント以外（MIDI 等）はスキップ。
        let Some(core_event) = event.as_core_event() else {
            continue;
        };

        // ParamValue 以外のコアイベント（NoteOn 等）もスキップ。
        let CoreEventSpace::ParamValue(param) = core_event else {
            continue;
        };
        let Some(param_id) = param.param_id() else {
            continue;
        };
        // このプラグインが知らないパラメータ ID はスキップ。
        if param_id != PARAM_GAIN_ID {
            continue;
        }

        shared.set_gain_from_host(param.value());
    }
}
