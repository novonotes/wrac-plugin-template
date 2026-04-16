/**
 * WXP Example Gain Plugin — Frontend (JavaScript side)
 *
 * The GUI of a wxp plugin is implemented as a regular web application.
 * Communication with the Rust side uses invoke() and Channel
 * provided by @novonotes/webview-bridge.
 *
 * invoke(command, args):
 *   Calls a command registered in the Rust-side WxpCommandHandler (RPC).
 *   The return value is a Promise.
 *
 * Channel:
 *   A bidirectional channel for receiving push notifications from Rust → JS.
 *   Pass a callback to the constructor; it is called each time
 *   the Rust side calls Channel::send().
 */
import { Channel, invoke } from "@novonotes/webview-bridge";
import "./style.css";

/** Type definition matching the JSON produced by gain_payload() on the Rust side */
type GainState = {
  type: "gain-state";
  /** Linear gain value (0.0–2.0) */
  value: number;
  /** Gain as a dB string (e.g., "-6.0 dB") */
  dbText: string;
};

// Gain range. Must match MIN_GAIN / MAX_GAIN on the Rust side.
const MIN_GAIN = 0;
const MAX_GAIN = 2;
// Knob rotation range (-135° to +135°, giving 270° of travel)
const MIN_ANGLE = -135;
const MAX_ANGLE = 135;

// --- DOM element references ---
const valueLabel = document.querySelector<HTMLDivElement>("#gain-value");
const dbLabel = document.querySelector<HTMLDivElement>("#gain-db");
const knob = document.querySelector<HTMLButtonElement>("#gain-knob");
const indicator = document.querySelector<HTMLDivElement>("#knob-indicator");
const fill = document.querySelector<HTMLDivElement>("#knob-fill");

if (!valueLabel || !dbLabel || !knob || !indicator || !fill) {
  throw new Error("required elements not found");
}

// --- State ---
let gain = 1;
let dragging = false;
let dragStartY = 0;
let dragStartGain = gain;
/** Whether a gesture (drag interaction) is in progress. Prevents double-sending. */
let gestureActive = false;

// -----------------------------------------------------------------------
// Subscribe to Rust → JS push notifications
// -----------------------------------------------------------------------
// Create a Channel and register it with the Rust side as the target for parameter change
// notifications. When the host changes the gain via automation, this callback updates the UI.
const channel = new Channel<GainState>((message) => {
  if (message && message.type === "gain-state") {
    render(message);
  }
});

// Initialization: fetch the current gain state, render the UI, and subscribe to changes.
void (async () => {
  // Call the Rust "get_gain_state" command via invoke().
  const initialState = await invoke<GainState>("get_gain_state");
  render(initialState);
  // Passing the Channel as an argument lets the Rust side call Channel::send()
  // to push messages to this callback.
  await invoke("subscribe_gain", { channel });
})();

function clamp(value: number): number {
  return Math.min(MAX_GAIN, Math.max(MIN_GAIN, value));
}

/** Converts a linear gain value to a knob rotation angle */
function gainToAngle(value: number): number {
  const normalized = (value - MIN_GAIN) / (MAX_GAIN - MIN_GAIN);
  return MIN_ANGLE + normalized * (MAX_ANGLE - MIN_ANGLE);
}

/** Receives a gain state and updates the UI display */
function render(state: GainState): void {
  gain = clamp(state.value);
  valueLabel.textContent = `${gain.toFixed(2)}x`;
  dbLabel.textContent = state.dbText;
  const angle = gainToAngle(gain);
  indicator.style.transform = `rotate(${angle}deg)`;
  fill.style.transform = `rotate(${angle}deg)`;
}

// -----------------------------------------------------------------------
// Gesture management
// -----------------------------------------------------------------------
// CLAP parameter changes must be wrapped in a gesture begin/end pair.
// The host (DAW) uses gesture begin/end to determine the unit
// for automation recording and undo.

function beginGesture(): void {
  if (gestureActive) {
    return;
  }
  gestureActive = true;
  // Call the Rust begin_parameter_gesture command via invoke().
  // void = fire-and-forget (do not await the result).
  void invoke("begin_parameter_gesture");
}

function endGesture(): void {
  if (!gestureActive) {
    return;
  }
  gestureActive = false;
  void invoke("end_parameter_gesture");
}

/** Sets the gain, immediately updates the UI, and notifies the Rust side */
function applyGain(nextGain: number): void {
  const value = clamp(nextGain);
  // Render locally without waiting for a Rust response, for responsiveness.
  render({
    type: "gain-state",
    value,
    dbText:
      value <= 0 ? "-inf dB" : `${(20 * Math.log10(value)).toFixed(1)} dB`,
  });
  // Update the parameter via the Rust "set_gain" command.
  void invoke("set_gain", { value });
}

// -----------------------------------------------------------------------
// Knob drag interaction
// -----------------------------------------------------------------------
// Uses the Pointer Events API to support both mouse and touch.

knob.addEventListener("pointerdown", (event) => {
  dragging = true;
  dragStartY = event.clientY;
  dragStartGain = gain;
  // setPointerCapture: continue receiving pointermove/pointerup
  // even when the cursor moves outside the button.
  knob.setPointerCapture(event.pointerId);
  beginGesture();
});

knob.addEventListener("pointermove", (event) => {
  if (!dragging) {
    return;
  }
  // Dragging upward increases gain. 180px covers the full range.
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
// Mouse wheel adjustment
// -----------------------------------------------------------------------
knob.addEventListener("wheel", (event) => {
  event.preventDefault();
  beginGesture();
  applyGain(gain - event.deltaY * 0.0015);
  // Wheel events are continuous but have no clear "end", so a 120ms timer
  // is used to end the gesture after the last wheel event.
  window.clearTimeout((knob as unknown as { wheelTimer?: number }).wheelTimer);
  (knob as unknown as { wheelTimer?: number }).wheelTimer = window.setTimeout(
    () => {
      endGesture();
    },
    120,
  );
});

// -----------------------------------------------------------------------
// Cleanup
// -----------------------------------------------------------------------
// End any active gesture and unsubscribe before the WebView closes.
window.addEventListener("beforeunload", () => {
  endGesture();
  void invoke("unsubscribe_gain");
});
