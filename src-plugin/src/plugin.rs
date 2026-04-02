use std::ffi::CStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use atomic_float::AtomicF32;
use clack_extensions::audio_ports::{
    AudioPortFlags, AudioPortInfo, AudioPortInfoWriter, AudioPortType, PluginAudioPorts,
    PluginAudioPortsImpl,
};
use clack_extensions::gui::PluginGui;
use clack_extensions::params::PluginParams;
use clack_extensions::state::PluginState;
use clack_plugin::factory::plugin::PluginFactoryImpl;
use clack_plugin::host::HostInfo;
use clack_plugin::plugin::PluginInstance;
use clack_plugin::prelude::*;
use novonotes_run_loop::{RunLoop, RunLoopSender};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use wxp::{Channel, WebViewRef, WxpCommandHandler, dpi::LogicalSize};
use wxp_clack::dpi::DpiConverter;

use crate::audio::WxpExampleGainAudioProcessor;

// --- CLAP プラグインメタデータ ---
// PLUGIN_ID はグローバルに一意である必要がある（リバースドメイン形式が慣例）。
pub(crate) const PLUGIN_ID: &str = "com.novo-notes.wxp-example-gain";
pub(crate) const PLUGIN_NAME: &str = "WXP Example Gain";
/// パラメータの一意 ID。ホストはこの ID でパラメータを識別・保存するため、
/// 一度公開したら変更してはならない。
pub(crate) const PARAM_GAIN_ID: ClapId = ClapId::new(1);
/// ゲインのデフォルト値。1.0 = 0dB（原音のまま）。
pub(crate) const DEFAULT_GAIN: f32 = 1.0;
pub(crate) const MIN_GAIN: f32 = 0.0;
/// 最大ゲイン 2.0 = 約 +6dB。
pub(crate) const MAX_GAIN: f32 = 2.0;
pub(crate) const DEFAULT_GUI_SIZE: LogicalSize<f64> = LogicalSize::new(360.0, 360.0);

/// プラグインファクトリ。ホストがプラグインの一覧を問い合わせたり、
/// インスタンスを生成する際に使われる。
pub(crate) struct WxpExampleGainPluginFactory {
    descriptor: PluginDescriptor,
}

/// clack の Plugin トレイトを実装するための型。
/// 関連型でオーディオプロセッサ・共有状態・メインスレッドの型を紐づける。
pub(crate) struct WxpExampleGainPlugin;

// -----------------------------------------------------------------------
// CLAP プラグインのスレッドモデル
// -----------------------------------------------------------------------
// CLAP ではプラグインの状態を 3 つの層に分けて管理する：
//
//   1. SharedState     — 全スレッドから参照可能（Atomic 型で同期）
//   2. MainThread      — メインスレッド専用。GUI やパラメータ情報の操作
//   3. AudioProcessor  — オーディオスレッド専用。リアルタイム処理
//
// SharedState を介してスレッド間の値を受け渡す設計になっている。
// -----------------------------------------------------------------------

/// 全スレッドから共有される状態。Arc で包んで AudioProcessor と MainThread の
/// 両方から参照する。
pub(crate) struct SharedState {
    pub(crate) inner: Arc<SharedStateInner>,
}

pub(crate) struct SharedStateInner {
    /// 現在のゲイン値。オーディオスレッドとメインスレッドの両方からアクセスされるため
    /// AtomicF32 を使用。ロックフリーでリアルタイムスレッドから安全に読み書きできる。
    gain: AtomicF32,
    /// UI から変更されたパラメータを、次回の flush/process でホストに通知するための
    /// フラグ群。「ジェスチャー開始→値変更→ジェスチャー終了」の 3 段階で管理する。
    /// ジェスチャーとは、ユーザーがノブをドラッグするなどの一連の操作のこと。
    /// ホストはジェスチャーの開始・終了を認識し、オートメーション記録の単位とする。
    pending_ui: PendingUiState,
    /// GUI（WebView）への通知に使うチャネル。
    /// GUI が開いていないときは None。
    gui_notifier: Mutex<Option<GuiNotifier>>,
}

/// UI 側からのパラメータ変更をホストに伝えるための pending フラグ。
/// 各フラグは AtomicBool で、オーディオスレッドの process()/flush() で
/// swap(false) して消費される。
struct PendingUiState {
    gesture_begin: AtomicBool,
    value_dirty: AtomicBool,
    gesture_end: AtomicBool,
}

/// GUI 通知用の送信ハンドル。RunLoopSender でメインスレッドにディスパッチし、
/// Channel 経由で WebView の JavaScript に JSON メッセージを送る。
#[derive(Clone)]
struct GuiNotifier {
    /// RunLoopSender は任意のスレッドからメインスレッド（RunLoop）に
    /// クロージャをポストできる。WebView の操作はメインスレッド上でのみ安全なため、
    /// オーディオスレッド等から直接 Channel::send() を呼ばず sender を経由する。
    sender: RunLoopSender,
    /// wxp の Channel。JavaScript 側で subscribe した双方向通信チャネル。
    /// Rust → JS 方向の push 通知に使う。
    channel: Channel,
}

/// プラグインの状態をシリアライズして保存するための構造体。
/// ホストの「プロジェクト保存」機能で永続化される。
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SavedPluginState {
    pub(crate) gain: f32,
}

/// メインスレッド上でのみアクセスされる状態。GUI やパラメータの管理はここで行う。
pub(crate) struct WxpExampleGainMainThread<'a> {
    pub(crate) shared: &'a SharedState,
    /// wxp の WebViewRef。GUI が開いている間だけ Some になる。
    pub(crate) web_view: Option<WebViewRef>,
    /// wry の WebContext。WebView のユーザーデータ（キャッシュ等）の保存先を管理する。
    /// WebView よりも長く生存する必要があるためフィールドとして保持する。
    pub(crate) wry_context: Option<wry::WebContext>,
    /// wxp のコマンドハンドラ。JavaScript から呼び出されるコマンド（RPC）を登録する。
    pub(crate) command_handler: Arc<WxpCommandHandler>,
    pub(crate) gui_size: LogicalSize<f64>,
    /// ホストが提示する DPI スケールと論理サイズ・物理サイズを相互変換するユーティリティ。
    pub(crate) dpi_converter: DpiConverter,
}

impl WxpExampleGainPluginFactory {
    pub(crate) fn new() -> Self {
        Self {
            // AUDIO_EFFECT: エフェクトプラグインであることをホストに伝える。
            // STEREO: ステレオ（2ch）対応であることを示す。
            descriptor: PluginDescriptor::new(PLUGIN_ID, PLUGIN_NAME).with_features([
                clack_plugin::plugin::features::AUDIO_EFFECT,
                clack_plugin::plugin::features::STEREO,
            ]),
        }
    }
}

/// PluginFactoryImpl は CLAP ホストがプラグインを列挙・生成するためのインターフェース。
impl PluginFactoryImpl for WxpExampleGainPluginFactory {
    /// このファクトリが提供するプラグインの数。
    fn plugin_count(&self) -> u32 {
        1
    }

    /// index 番目のプラグインのディスクリプタ（ID, 名前, 機能）を返す。
    fn plugin_descriptor(&self, index: u32) -> Option<&PluginDescriptor> {
        (index == 0).then_some(&self.descriptor)
    }

    /// ホストが実際にプラグインインスタンスを生成する際に呼ばれる。
    /// clack の PluginInstance::new に SharedState と MainThread の
    /// コンストラクタを渡す。
    fn create_plugin<'a>(
        &'a self,
        host_info: HostInfo<'a>,
        plugin_id: &CStr,
    ) -> Option<PluginInstance<'a>> {
        if plugin_id.to_string_lossy() != PLUGIN_ID {
            return None;
        }

        Some(PluginInstance::new::<WxpExampleGainPlugin>(
            host_info,
            &self.descriptor,
            |host| WxpExampleGainPlugin::new_shared(host),
            |host, shared| WxpExampleGainPlugin::new_main_thread(host, shared),
        ))
    }
}

impl Plugin for WxpExampleGainPlugin {
    /// オーディオスレッドで動作するプロセッサ型。
    type AudioProcessor<'a> = WxpExampleGainAudioProcessor<'a>;
    /// 全スレッドから共有される状態型。
    type Shared<'a> = SharedState;
    /// メインスレッド専用の状態型。
    type MainThread<'a> = WxpExampleGainMainThread<'a>;

    /// このプラグインがサポートする CLAP 拡張を宣言する。
    /// ホストはここで登録された拡張だけを問い合わせる。
    fn declare_extensions(
        builder: &mut PluginExtensions<Self>,
        _shared: Option<&Self::Shared<'_>>,
    ) {
        builder
            .register::<PluginAudioPorts>()  // オーディオ入出力ポートの定義
            .register::<PluginParams>()      // パラメータの公開
            .register::<PluginState>()       // 状態の保存・復元
            .register::<PluginGui>();         // GUI の提供
    }
}

impl DefaultPluginFactory for WxpExampleGainPlugin {
    fn get_descriptor() -> PluginDescriptor {
        PluginDescriptor::new(PLUGIN_ID, PLUGIN_NAME).with_features([
            clack_plugin::plugin::features::AUDIO_EFFECT,
            clack_plugin::plugin::features::STEREO,
        ])
    }

    /// SharedState の生成。プラグインインスタンスごとに 1 つ作られる。
    fn new_shared(_host: HostSharedHandle<'_>) -> Result<Self::Shared<'_>, PluginError> {
        Ok(SharedState::new())
    }

    /// メインスレッド状態の生成。ここで wxp のコマンドハンドラを設定し、
    /// JavaScript から呼べる RPC コマンドを登録する。
    fn new_main_thread<'a>(
        _host: HostMainThreadHandle<'a>,
        shared: &'a Self::Shared<'a>,
    ) -> Result<Self::MainThread<'a>, PluginError> {
        // WxpCommandHandler は JavaScript ↔ Rust 間の RPC ブリッジ。
        // register_commands() でコマンド名とハンドラを紐づける。
        let command_handler = Arc::new(WxpCommandHandler::new());
        register_commands(command_handler.clone(), shared.inner.clone());

        Ok(WxpExampleGainMainThread {
            shared,
            web_view: None,
            wry_context: None,
            command_handler,
            gui_size: DEFAULT_GUI_SIZE,
            dpi_converter: DpiConverter::new(1.0),
        })
    }
}

impl PluginShared<'_> for SharedState {}

impl<'a> PluginMainThread<'a, SharedState> for WxpExampleGainMainThread<'a> {}

/// オーディオポートの定義。入力 1 ポート・出力 1 ポート（ともにステレオ）。
impl PluginAudioPortsImpl for WxpExampleGainMainThread<'_> {
    fn count(&mut self, _is_input: bool) -> u32 {
        1
    }

    fn get(&mut self, index: u32, is_input: bool, writer: &mut AudioPortInfoWriter) {
        if index != 0 {
            return;
        }

        writer.set(&AudioPortInfo {
            // 入力と出力で異なる ID を割り当てる。
            id: ClapId::new(if is_input { 1 } else { 2 }),
            name: if is_input { b"Main In" } else { b"Main Out" },
            // ステレオ = 2 チャネル（L, R）。
            channel_count: 2,
            // IS_MAIN: ホストがデフォルトでルーティングするメインポートであることを示す。
            flags: AudioPortFlags::IS_MAIN,
            port_type: Some(AudioPortType::STEREO),
            // in_place_pair を指定すると、入力と出力で同じバッファを使う
            // 「インプレース処理」が可能になる。ここでは None（ホストに任せる）。
            in_place_pair: None,
        });
    }
}

impl SharedState {
    fn new() -> Self {
        Self {
            inner: Arc::new(SharedStateInner::new()),
        }
    }
}

impl SharedStateInner {
    fn new() -> Self {
        Self {
            gain: AtomicF32::new(DEFAULT_GAIN),
            pending_ui: PendingUiState {
                gesture_begin: AtomicBool::new(false),
                value_dirty: AtomicBool::new(false),
                gesture_end: AtomicBool::new(false),
            },
            gui_notifier: Mutex::new(None),
        }
    }

    /// 現在のゲイン値を取得。オーディオスレッドからも呼ばれる。
    /// Acquire ordering で、直前の store が確実に見えることを保証する。
    pub(crate) fn gain(&self) -> f32 {
        self.gain.load(Ordering::Acquire)
    }

    /// ホスト（DAW のオートメーション等）からゲインが変更されたときに呼ばれる。
    /// 値を保存し、GUI が開いていれば WebView に通知する。
    pub(crate) fn set_gain_from_host(&self, gain: f64) -> f32 {
        let gain = clamp_gain(gain as f32);
        self.gain.store(gain, Ordering::Release);
        self.notify_gui();
        gain
    }

    // --- UI → ホストへのパラメータ変更通知 ---
    // CLAP では UI がパラメータを変更する場合、以下の手順でホストに通知する：
    //   1. begin_gesture  — ユーザーがノブ等の操作を開始
    //   2. set_value       — 値を変更（ドラッグ中に複数回呼ばれうる）
    //   3. end_gesture    — 操作を完了
    // これらのフラグは次回の process()/flush() で消費され、
    // output events としてホストに伝えられる。

    pub(crate) fn begin_gesture_from_ui(&self) {
        self.pending_ui.gesture_begin.store(true, Ordering::Release);
    }

    /// UI からゲインを変更。ホスト通知用の value_dirty フラグも立てる。
    pub(crate) fn set_gain_from_ui(&self, gain: f64) -> f32 {
        let gain = self.set_gain_from_host(gain);
        self.pending_ui.value_dirty.store(true, Ordering::Release);
        gain
    }

    pub(crate) fn end_gesture_from_ui(&self) {
        self.pending_ui.gesture_end.store(true, Ordering::Release);
    }

    // take_* メソッド群: swap(false) で「フラグを読みつつリセット」する。
    // process()/flush() から呼ばれ、ホストへの output event 送出に使われる。

    pub(crate) fn take_ui_gesture_begin(&self) -> bool {
        self.pending_ui.gesture_begin.swap(false, Ordering::AcqRel)
    }

    pub(crate) fn take_ui_value_dirty(&self) -> bool {
        self.pending_ui.value_dirty.swap(false, Ordering::AcqRel)
    }

    pub(crate) fn take_ui_gesture_end(&self) -> bool {
        self.pending_ui.gesture_end.swap(false, Ordering::AcqRel)
    }

    /// GUI が開かれたときに、RunLoopSender と Channel を登録する。
    /// これにより、ホストからのパラメータ変更を WebView にプッシュ通知できるようになる。
    pub(crate) fn set_gui_channel(&self, sender: RunLoopSender, channel: Channel) {
        *self.gui_notifier.lock() = Some(GuiNotifier { sender, channel });
    }

    /// GUI が閉じられたときに呼ぶ。通知先をクリアする。
    pub(crate) fn clear_gui_channel(&self) {
        *self.gui_notifier.lock() = None;
    }

    /// ゲイン値が変更されたとき、GUI に通知する。
    /// RunLoopSender を使ってメインスレッドにディスパッチすることで、
    /// オーディオスレッドなど任意のスレッドから安全に WebView にメッセージを送れる。
    fn notify_gui(&self) {
        let Some(notifier) = self.gui_notifier.lock().clone() else {
            return;
        };

        let payload = gain_payload(self.gain());
        // RunLoopSender::send() は非同期。クロージャはメインスレッド上で実行される。
        // Channel::send() で JSON ペイロードを JavaScript 側に送信する。
        notifier.sender.send(move || {
            let _ = notifier.channel.send(payload);
        });
    }
}

impl WxpExampleGainMainThread<'_> {
    /// 現在のプラットフォームに適した GUI API を返す。
    /// is_floating: false はホストのウィンドウに埋め込み表示することを意味する。
    pub(crate) fn preferred_api(&self) -> Option<clack_extensions::gui::GuiConfiguration<'static>> {
        Some(clack_extensions::gui::GuiConfiguration {
            api_type: clack_extensions::gui::GuiApiType::default_for_current_platform()?,
            is_floating: false,
        })
    }

    /// GUI を閉じるときのクリーンアップ。
    /// Channel をクリアしてから WebView と WebContext を破棄する。
    pub(crate) fn reset_webview(&mut self) {
        self.shared.inner.clear_gui_channel();
        self.web_view = None;
        self.wry_context = None;
    }
}

/// JavaScript 側に送る JSON ペイロードを組み立てる。
/// UI はこの形式のメッセージを受け取ってノブやテキスト表示を更新する。
pub(crate) fn gain_payload(gain: f32) -> serde_json::Value {
    json!({
        "type": "gain-state",
        "value": gain,
        "dbText": gain_db_text(gain as f64),
    })
}

pub(crate) fn clamp_gain(gain: f32) -> f32 {
    gain.clamp(MIN_GAIN, MAX_GAIN)
}

/// リニアゲイン値を dB（デシベル）表記の文字列に変換する。
/// dB = 20 * log10(gain) はオーディオ分野の標準的な対数スケール変換。
/// ゲイン 1.0 = 0dB、ゲイン 0.0 = -∞ dB。
pub(crate) fn gain_db_text(gain: f64) -> String {
    if gain <= 0.0 {
        "-inf dB".to_string()
    } else {
        format!("{:.1} dB", 20.0 * gain.log10())
    }
}

// -----------------------------------------------------------------------
// wxp コマンドハンドラの登録
// -----------------------------------------------------------------------
// WxpCommandHandler は JavaScript ↔ Rust 間の RPC メカニズム。
// JavaScript 側から `invoke("command_name", { args })` を呼ぶと、
// ここで登録したハンドラが実行される。
//
// register_sync: 同期コマンド（すぐに結果を返す）
// register_async: 非同期コマンド（Future を返す）
//
// コマンドハンドラ内では SharedStateInner を通じてパラメータを読み書きする。

pub(crate) fn register_commands(
    command_handler: Arc<WxpCommandHandler>,
    shared: Arc<SharedStateInner>,
) {
    // 現在のゲイン状態を取得するコマンド。GUI の初期表示に使われる。
    {
        let shared = shared.clone();
        command_handler.register_sync("get_gain_state", move |_ctx| {
            Ok::<_, String>(gain_payload(shared.gain()))
        });
    }

    // ジェスチャー開始を通知するコマンド。
    // JavaScript 側でノブのドラッグ開始時に呼ぶ。
    {
        let shared = shared.clone();
        command_handler.register_sync("begin_parameter_gesture", move |_ctx| {
            shared.begin_gesture_from_ui();
            Ok::<_, String>(json!({ "ok": true }))
        });
    }

    // ゲイン値を設定するコマンド。ドラッグ中に繰り返し呼ばれる。
    // ctx.arg() で JavaScript から渡された引数を型安全に取得できる。
    {
        let shared = shared.clone();
        command_handler.register_sync("set_gain", move |ctx| {
            let value = ctx.arg::<f64>("value").map_err(|e| e.to_string())?;
            let applied = shared.set_gain_from_ui(value);
            Ok::<_, String>(gain_payload(applied))
        });
    }

    // ジェスチャー終了を通知するコマンド。
    // JavaScript 側でノブのドラッグ終了時に呼ぶ。
    {
        let shared = shared.clone();
        command_handler.register_sync("end_parameter_gesture", move |_ctx| {
            shared.end_gesture_from_ui();
            Ok::<_, String>(json!({ "ok": true }))
        });
    }

    // ゲイン値の変更をサブスクライブするコマンド。
    // JavaScript 側から Channel を渡してもらい、ホストからの値変更を
    // リアルタイムに push 通知する。Rust → JS の非同期通知の典型パターン。
    {
        let shared = shared.clone();
        command_handler.register_sync("subscribe_gain", move |ctx| {
            // Channel は wxp が提供する双方向通信チャネル。
            // JavaScript 側で作成され、コマンド引数として Rust に渡される。
            let channel = ctx.arg::<Channel>("channel").map_err(|e| e.to_string())?;
            // 登録と同時に現在の値を即座に送信する（初期同期）。
            channel
                .send(gain_payload(shared.gain()))
                .map_err(|e| e.to_string())?;

            // RunLoop::sender() でメインスレッドへの送信ハンドルを取得し、
            // Channel と一緒に保存する。以後、オーディオスレッド等から
            // RunLoopSender 経由でメインスレッドに Channel 送信をポストできる。
            shared.set_gui_channel(RunLoop::sender(), channel);

            Ok::<_, String>(json!({ "ok": true }))
        });
    }

    // サブスクリプション解除。GUI が閉じるときに呼ばれる。
    {
        let shared = shared.clone();
        command_handler.register_sync("unsubscribe_gain", move |_ctx| {
            shared.clear_gui_channel();
            Ok::<_, String>(json!({ "ok": true }))
        });
    }
}
