/**
 * WXP Example Gain Plugin — フロントエンド（JavaScript 側）
 *
 * wxp プラグインの GUI は通常の Web アプリとして実装する。
 * Rust 側との通信には @novonotes/webview-bridge が提供する
 * invoke() と Channel を使う。
 *
 * invoke(command, args):
 *   Rust 側の WxpCommandHandler に登録されたコマンドを呼び出す（RPC）。
 *   戻り値は Promise で返される。
 *
 * Channel:
 *   Rust → JS 方向のプッシュ通知を受け取るための双方向チャネル。
 *   コンストラクタにコールバックを渡すと、Rust 側から Channel::send() された
 *   メッセージを受信するたびにコールバックが呼ばれる。
 */
import { Channel, invoke } from "@novonotes/webview-bridge";
import "./style.css";

/** Rust 側の gain_payload() が生成する JSON と同じ型定義 */
type GainState = {
  type: "gain-state";
  /** リニアゲイン値（0.0〜2.0） */
  value: number;
  /** dB 表記のテキスト（例: "-6.0 dB"） */
  dbText: string;
};

// ゲインの値域。Rust 側の MIN_GAIN / MAX_GAIN と一致させること。
const MIN_GAIN = 0;
const MAX_GAIN = 2;
// ノブの回転角度の範囲（-135° 〜 +135° で 270° の可動域）
const MIN_ANGLE = -135;
const MAX_ANGLE = 135;

// --- DOM 要素の取得 ---
const valueLabel = document.querySelector<HTMLDivElement>("#gain-value");
const dbLabel = document.querySelector<HTMLDivElement>("#gain-db");
const knob = document.querySelector<HTMLButtonElement>("#gain-knob");
const indicator = document.querySelector<HTMLDivElement>("#knob-indicator");
const fill = document.querySelector<HTMLDivElement>("#knob-fill");

if (!valueLabel || !dbLabel || !knob || !indicator || !fill) {
  throw new Error("required elements not found");
}

// --- 状態管理 ---
let gain = 1;
let dragging = false;
let dragStartY = 0;
let dragStartGain = gain;
/** ジェスチャー（ドラッグ操作）が進行中かどうか。二重送信を防ぐ。 */
let gestureActive = false;

// -----------------------------------------------------------------------
// Rust → JS プッシュ通知の受信セットアップ
// -----------------------------------------------------------------------
// Channel を生成し、Rust 側にパラメータ変更の通知先として登録する。
// ホストがオートメーションでゲインを変更したとき、このコールバックで
// UI が自動更新される。
const channel = new Channel<GainState>((message) => {
  if (message && message.type === "gain-state") {
    render(message);
  }
});

// 初期化: 現在のゲイン状態を取得して UI を描画し、変更通知を購読する。
void (async () => {
  // invoke() で Rust 側の "get_gain_state" コマンドを呼び出す。
  const initialState = await invoke<GainState>("get_gain_state");
  render(initialState);
  // Channel を引数として渡すことで、Rust 側が Channel::send() で
  // メッセージを送れるようになる。
  await invoke("subscribe_gain", { channel });
})();

function clamp(value: number): number {
  return Math.min(MAX_GAIN, Math.max(MIN_GAIN, value));
}

/** リニアゲイン値をノブの回転角度に変換する */
function gainToAngle(value: number): number {
  const normalized = (value - MIN_GAIN) / (MAX_GAIN - MIN_GAIN);
  return MIN_ANGLE + normalized * (MAX_ANGLE - MIN_ANGLE);
}

/** ゲイン状態を受け取って UI の表示を更新する */
function render(state: GainState): void {
  gain = clamp(state.value);
  valueLabel.textContent = `${gain.toFixed(2)}x`;
  dbLabel.textContent = state.dbText;
  const angle = gainToAngle(gain);
  indicator.style.transform = `rotate(${angle}deg)`;
  fill.style.transform = `rotate(${angle}deg)`;
}

// -----------------------------------------------------------------------
// ジェスチャー管理
// -----------------------------------------------------------------------
// CLAP のパラメータ変更は「ジェスチャー」として begin/end で囲む必要がある。
// ホスト（DAW）はジェスチャーの開始・終了を認識し、
// オートメーション記録やアンドゥの単位として扱う。

function beginGesture(): void {
  if (gestureActive) {
    return;
  }
  gestureActive = true;
  // invoke() で Rust 側の begin_parameter_gesture コマンドを呼ぶ。
  // void で fire-and-forget（戻り値を待たない）。
  void invoke("begin_parameter_gesture");
}

function endGesture(): void {
  if (!gestureActive) {
    return;
  }
  gestureActive = false;
  void invoke("end_parameter_gesture");
}

/** ゲインを設定し、即座に UI を更新しつつ Rust 側に通知する */
function applyGain(nextGain: number): void {
  const value = clamp(nextGain);
  // 応答性のため、Rust 側の応答を待たずにローカルで即座に描画する。
  render({
    type: "gain-state",
    value,
    dbText:
      value <= 0 ? "-inf dB" : `${(20 * Math.log10(value)).toFixed(1)} dB`,
  });
  // Rust 側の "set_gain" コマンドでパラメータを更新。
  void invoke("set_gain", { value });
}

// -----------------------------------------------------------------------
// ノブのドラッグ操作
// -----------------------------------------------------------------------
// Pointer Events API を使用。マウスとタッチの両方に対応する。

knob.addEventListener("pointerdown", (event) => {
  dragging = true;
  dragStartY = event.clientY;
  dragStartGain = gain;
  // setPointerCapture: ボタンの外にカーソルが出ても
  // pointermove/pointerup を受け取り続ける。
  knob.setPointerCapture(event.pointerId);
  beginGesture();
});

knob.addEventListener("pointermove", (event) => {
  if (!dragging) {
    return;
  }
  // 上方向にドラッグ = ゲイン増加。180px で全範囲を操作できる感度。
  const delta = (dragStartY - event.clientY) / 180;
  applyGain(dragStartGain + delta);
});

const finishDrag = (event: PointerEvent) => {
  if (!dragging) {
    return;
  }
  dragging = false;
  knob.releasePointerCapture(event.pointerId);
  endGesture();
};

knob.addEventListener("pointerup", finishDrag);
knob.addEventListener("pointercancel", finishDrag);

// -----------------------------------------------------------------------
// マウスホイールでの調整
// -----------------------------------------------------------------------
knob.addEventListener("wheel", (event) => {
  event.preventDefault();
  beginGesture();
  applyGain(gain - event.deltaY * 0.0015);
  // ホイール操作は連続イベントだが、明確な「終了」がないため、
  // 120ms のタイマーで「最後のホイールイベントから一定時間経過したら終了」とする。
  window.clearTimeout((knob as unknown as { wheelTimer?: number }).wheelTimer);
  (knob as unknown as { wheelTimer?: number }).wheelTimer = window.setTimeout(
    () => {
      endGesture();
    },
    120,
  );
});

// -----------------------------------------------------------------------
// クリーンアップ
// -----------------------------------------------------------------------
// WebView が閉じられる前にジェスチャーを終了し、サブスクリプションを解除する。
window.addEventListener("beforeunload", () => {
  endGesture();
  void invoke("unsubscribe_gain");
});
