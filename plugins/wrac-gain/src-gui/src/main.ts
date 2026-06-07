/**
 * WRAC Gain Plugin — Frontend (JavaScript side)
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

type PluginMetadata = {
  pluginId: string;
  pluginName: string;
  companyName: string;
  version: string;
};

/** Type definition matching the JSON produced by parameter_payload() on the Rust side */
type ParameterState = {
  type: "parameter-value";
  /** Stable parameter id used by the native plugin and host automation */
  parameterId: number;
  /** Plain parameter value */
  value: number;
  /** Parameter value formatted by the Rust side */
  text: string;
};

type EditorPage = "controls" | "about";

type EditorPageState = {
  type: "editor-page";
  page: EditorPage;
};

type SubscribeParametersResponse = {
  ok?: boolean;
  subscriptionId: number;
};

// Knob rotation range (-135° to +135°, giving 270° of travel)
const MIN_ANGLE = -135;
const MAX_ANGLE = 135;

const GAIN_PARAMETER = {
  // GUI mapping for the single gain control. Keep this in sync with PARAM_GAIN_ID and
  // the plain gain range in src-plugin/src/plugin/params.rs.
  id: 1,
  minValue: 0,
  maxValue: 2,
  defaultValue: 1,
} as const;

// --- DOM element references ---
const dbLabel = document.querySelector<HTMLButtonElement>("#gain-db");
const gainInput = document.querySelector<HTMLInputElement>("#gain-input");
const headerAction =
  document.querySelector<HTMLButtonElement>("#header-action");
const pluginName = document.querySelector<HTMLButtonElement>("#plugin-name");
const aboutTitle = document.querySelector<HTMLElement>("#about-title");
const aboutPluginName =
  document.querySelector<HTMLElement>("#about-plugin-name");
const aboutVersion = document.querySelector<HTMLElement>("#about-version");
const aboutCompanyName = document.querySelector<HTMLElement>(
  "#about-company-name",
);
const aboutBuild = document.querySelector<HTMLElement>("#about-build");
const knob = document.querySelector<HTMLButtonElement>("#gain-knob");
const indicator = document.querySelector<HTMLDivElement>("#knob-indicator");
const resizeGrip = document.querySelector<HTMLButtonElement>("#resize-grip");
const pageControls = document.querySelector<HTMLElement>("#page-controls");
const pageAbout = document.querySelector<HTMLElement>("#page-about");

if (
  !dbLabel ||
  !gainInput ||
  !headerAction ||
  !pluginName ||
  !aboutTitle ||
  !aboutPluginName ||
  !aboutVersion ||
  !aboutCompanyName ||
  !aboutBuild ||
  !knob ||
  !indicator ||
  !resizeGrip ||
  !pageControls ||
  !pageAbout
) {
  throw new Error("required elements not found");
}

const buildType = import.meta.env.PROD ? "Release" : "Debug";

// --- State ---
let gain = 1;
let dragging = false;
let dragStartX = 0;
let dragStartY = 0;
let dragStartGain = gain;
/** Whether a gesture (drag interaction) is in progress. Prevents double-sending. */
let gestureActive = false;
let parameterSubscriptionId: number | undefined;
let editorPageSubscriptionId: number | undefined;
let pluginMetadata: PluginMetadata | undefined;
let nativeCursorBridgeEnabled = false;

type ResizeResponse = {
  ok?: boolean;
  width?: number;
  height?: number;
};

type FrontendRuntimeContext = {
  os?: string;
  pluginFormat?: string;
  hostFamily?: string;
  hostName?: string;
  processName?: string;
};

type NativeCursorIntent =
  | "alias"
  | "all-scroll"
  | "arrow"
  | "cell"
  | "col-resize"
  | "context-menu"
  | "copy"
  | "crosshair"
  | "e-resize"
  | "ew-resize"
  | "grab"
  | "grabbing"
  | "help"
  | "move"
  | "n-resize"
  | "ne-resize"
  | "nesw-resize"
  | "no-drop"
  | "none"
  | "not-allowed"
  | "ns-resize"
  | "nw-resize"
  | "nwse-resize"
  | "pointer"
  | "progress"
  | "row-resize"
  | "s-resize"
  | "se-resize"
  | "sw-resize"
  | "text"
  | "vertical-text"
  | "w-resize"
  | "wait"
  | "zoom-in"
  | "zoom-out"
  | "unsupported";

function writeFrontendLog(message: string): void {
  void invoke("write_to_log", { message }).catch(() => undefined);
}

function shouldUseNativeCursorBridge(context: FrontendRuntimeContext): boolean {
  return (
    context.os === "macos" &&
    context.pluginFormat === "vst3" &&
    context.hostFamily === "steinberg-cubase"
  );
}

function nativeCursorIntentFromCss(cssCursor: string): NativeCursorIntent {
  switch (cssCursor) {
    case "auto":
    case "default":
      return "arrow";
    case "alias":
    case "all-scroll":
    case "cell":
    case "col-resize":
    case "context-menu":
    case "copy":
    case "crosshair":
    case "e-resize":
    case "ew-resize":
    case "grab":
    case "grabbing":
    case "help":
    case "move":
    case "n-resize":
    case "ne-resize":
    case "nesw-resize":
    case "no-drop":
    case "none":
    case "not-allowed":
    case "ns-resize":
    case "nw-resize":
    case "nwse-resize":
    case "pointer":
    case "progress":
    case "row-resize":
    case "s-resize":
    case "se-resize":
    case "sw-resize":
    case "text":
    case "vertical-text":
    case "w-resize":
    case "wait":
    case "zoom-in":
    case "zoom-out":
      return cssCursor;
    default:
      return "unsupported";
  }
}

function isEditableElement(target: EventTarget | null): boolean {
  return (
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement ||
    target instanceof HTMLSelectElement ||
    (target instanceof HTMLElement && target.isContentEditable)
  );
}

function restoreHostFocusIfNeeded(target?: EventTarget | null): void {
  if (
    isEditableElement(target ?? null) ||
    isEditableElement(document.activeElement)
  ) {
    return;
  }
  window.setTimeout(() => {
    if (isEditableElement(document.activeElement)) {
      return;
    }
    void invoke("focus_host_window");
  }, 0);
}

function editableText(source: string): string {
  const match = source.match(/[-+]?\d*\.?\d+/);
  return match?.[0] ?? source;
}

function isEditableContextMenuTarget(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) {
    return false;
  }
  return Boolean(
    target.closest(
      'input, textarea, select, [contenteditable=""], [contenteditable="true"], [data-allow-context-menu="true"]',
    ),
  );
}

if (import.meta.env.PROD) {
  window.addEventListener(
    "contextmenu",
    (event) => {
      if (isEditableContextMenuTarget(event.target)) {
        return;
      }
      event.preventDefault();
    },
    { capture: true },
  );
}

// -----------------------------------------------------------------------
// Subscribe to Rust → JS push notifications
// -----------------------------------------------------------------------
// Create a Channel and register it with the Rust side as the target for parameter change
// notifications. When the host changes the gain via automation, this callback updates the UI.
const channel = new Channel<ParameterState>((message) => {
  if (message && message.type === "parameter-value") {
    render(message);
  }
});

const editorPageChannel = new Channel<EditorPageState>((message) => {
  if (message && message.type === "editor-page") {
    renderEditorPage(message.page);
  }
});

// Initialization: fetch the current gain state, render the UI, and subscribe to changes.
void (async () => {
  pluginMetadata = await invoke<PluginMetadata>("get_plugin_metadata");
  renderPluginMetadata(pluginMetadata);

  gain = clamp(GAIN_PARAMETER.defaultValue);
  // Call the Rust "get_parameter_state" command via invoke().
  const initialState = await invoke<ParameterState>("get_parameter_state", {
    parameterId: GAIN_PARAMETER.id,
  });
  render(initialState);
  // Register the Channel on the Rust side and remember the returned subscriptionId.
  // Passing that id back on unsubscribe guarantees we tear down only our own
  // subscription, even if a remount created another one in the meantime.
  const subscription = await invoke<SubscribeParametersResponse>(
    "subscribe_parameters",
    {
      channel,
    },
  );
  parameterSubscriptionId = subscription.subscriptionId;

  const initialPage = await invoke<EditorPageState>("get_editor_page");
  renderEditorPage(initialPage.page);
  const editorPageSubscription = await invoke<SubscribeParametersResponse>(
    "subscribe_editor_page",
    {
      channel: editorPageChannel,
    },
  );
  editorPageSubscriptionId = editorPageSubscription.subscriptionId;
  // Log frontend initialization to the native log without relying on the WebView console.
  // Some environments inside a DAW do not allow opening devtools, so this boundary log is preserved.
  writeFrontendLog("GUI initialization completed");
  const runtimeContext = await invoke<FrontendRuntimeContext>(
    "get_frontend_runtime_context",
  ).catch(() => ({}));
  nativeCursorBridgeEnabled = shouldUseNativeCursorBridge(runtimeContext);
})();

function clamp(value: number): number {
  return Math.min(
    GAIN_PARAMETER.maxValue,
    Math.max(GAIN_PARAMETER.minValue, value),
  );
}

/** Converts a linear gain value to a knob rotation angle */
function gainToAngle(value: number): number {
  const span = GAIN_PARAMETER.maxValue - GAIN_PARAMETER.minValue;
  const normalized = span > 0 ? (value - GAIN_PARAMETER.minValue) / span : 0;
  return MIN_ANGLE + normalized * (MAX_ANGLE - MIN_ANGLE);
}

function requirePluginMetadata(): PluginMetadata {
  if (!pluginMetadata) {
    throw new Error("plugin metadata not loaded");
  }
  return pluginMetadata;
}

/** Receives a parameter state and updates the matching UI display */
function render(state: ParameterState): void {
  if (state.parameterId !== GAIN_PARAMETER.id) {
    return;
  }
  gain = clamp(state.value);
  dbLabel.textContent = state.text;
  const angle = gainToAngle(gain);
  indicator.style.transform = `rotate(${angle}deg)`;
}

function renderPluginMetadata(metadata: PluginMetadata): void {
  pluginName.textContent = metadata.pluginName;
  aboutTitle.textContent = metadata.pluginName;
  aboutPluginName.textContent = metadata.pluginName;
  aboutVersion.textContent = metadata.version;
  aboutCompanyName.textContent = metadata.companyName;
  aboutBuild.textContent = `${buildType} build`;
  document.title = metadata.pluginName;
}

function renderEditorPage(page: EditorPage): void {
  const showControls = page === "controls";
  pageControls.hidden = !showControls;
  pageAbout.hidden = showControls;
  pageControls.classList.toggle("is-active", showControls);
  pageAbout.classList.toggle("is-active", !showControls);
  pluginName.setAttribute(
    "aria-label",
    showControls ? "Show about page" : "Show controls",
  );
  headerAction.textContent = showControls
    ? `v${requirePluginMetadata().version}`
    : "×";
  headerAction.disabled = showControls;
  headerAction.classList.toggle("is-close", !showControls);
  headerAction.setAttribute(
    "aria-label",
    showControls ? "Plugin version" : "Close about page",
  );
}

function setEditorPage(page: EditorPage): void {
  renderEditorPage(page);
  void invoke<EditorPageState>("set_editor_page", { page })
    .then((state) => renderEditorPage(state.page))
    .catch(() => undefined);
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
  void invoke("begin_parameter_gesture", {
    parameterId: GAIN_PARAMETER.id,
  });
}

function endGesture(): void {
  if (!gestureActive) {
    return;
  }
  gestureActive = false;
  void invoke("end_parameter_gesture", {
    parameterId: GAIN_PARAMETER.id,
  });
}

/** Sets the gain, immediately updates the UI, and notifies the Rust side */
function applyGain(nextGain: number): void {
  const value = clamp(nextGain);
  // Render locally without waiting for a Rust response, for responsiveness.
  render({
    type: "parameter-value",
    parameterId: GAIN_PARAMETER.id,
    value,
    text: value <= 0 ? "-inf dB" : `${(20 * Math.log10(value)).toFixed(1)} dB`,
  });
  // Update the parameter via the Rust "set_parameter_value" command.
  void invoke("set_parameter_value", {
    parameterId: GAIN_PARAMETER.id,
    value,
  });
}

function renderResponse(promise: Promise<ParameterState>): void {
  void promise.then(render).catch(() => undefined);
}

function enterTextInput(): void {
  gainInput.hidden = false;
  dbLabel.hidden = true;
  gainInput.value = editableText(dbLabel.textContent ?? "");
  gainInput.focus();
  gainInput.select();
}

function commitTextInput(): void {
  if (gainInput.hidden) {
    return;
  }
  const text = gainInput.value;
  gainInput.hidden = true;
  dbLabel.hidden = false;
  renderResponse(
    invoke<ParameterState>("set_parameter_text", {
      parameterId: GAIN_PARAMETER.id,
      text,
    }),
  );
  restoreHostFocusIfNeeded();
}

function cancelTextInput(): void {
  gainInput.hidden = true;
  dbLabel.hidden = false;
  restoreHostFocusIfNeeded();
}

// -----------------------------------------------------------------------
// Knob drag interaction
// -----------------------------------------------------------------------
// Uses the Pointer Events API to support both mouse and touch.

knob.addEventListener("pointerdown", (event) => {
  dragging = true;
  dragStartX = event.clientX;
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
  // Dragging right or upward increases gain. 180px covers the full range.
  const deltaX = event.clientX - dragStartX;
  const deltaY = dragStartY - event.clientY;
  const delta = (deltaX + deltaY) / 180;
  applyGain(dragStartGain + delta);
});

const finishDrag = (event: PointerEvent) => {
  if (!dragging) {
    return;
  }
  dragging = false;
  knob.releasePointerCapture(event.pointerId);
  endGesture();
  restoreHostFocusIfNeeded(event.target);
};

knob.addEventListener("pointerup", finishDrag);
knob.addEventListener("pointercancel", finishDrag);

knob.addEventListener("dblclick", (event) => {
  event.preventDefault();
  renderResponse(
    invoke<ParameterState>("reset_parameter_to_default", {
      parameterId: GAIN_PARAMETER.id,
    }),
  );
  restoreHostFocusIfNeeded(event.target);
});

// -----------------------------------------------------------------------
// Mouse wheel adjustment
// -----------------------------------------------------------------------
knob.addEventListener("wheel", (event) => {
  event.preventDefault();
  beginGesture();
  applyGain(gain + event.deltaY * 0.0015);
  // Wheel events are continuous but have no clear "end", so a 120ms timer
  // is used to end the gesture after the last wheel event.
  window.clearTimeout((knob as unknown as { wheelTimer?: number }).wheelTimer);
  (knob as unknown as { wheelTimer?: number }).wheelTimer = window.setTimeout(
    () => {
      endGesture();
      restoreHostFocusIfNeeded(event.target);
    },
    120,
  );
});

dbLabel.addEventListener("pointerdown", (event) => {
  event.stopPropagation();
  event.preventDefault();
  enterTextInput();
});

dbLabel.addEventListener("keydown", (event) => {
  if (event.key === "Enter" || event.key === " ") {
    event.preventDefault();
    enterTextInput();
  }
});

gainInput.addEventListener("blur", commitTextInput);
gainInput.addEventListener("keydown", (event) => {
  if (event.key === "Enter") {
    event.preventDefault();
    commitTextInput();
  }
  if (event.key === "Escape") {
    event.preventDefault();
    cancelTextInput();
  }
});
gainInput.addEventListener("pointerdown", (event) => event.stopPropagation());

// About is a detail view of plugin identity rather than a settings screen, so the plugin name
// itself is used as the entry point/toggle instead of a permanent tab, to avoid an extra
// segmented control on the main controls surface.
pluginName.addEventListener("click", (event) => {
  setEditorPage(pageAbout.hidden ? "about" : "controls");
  restoreHostFocusIfNeeded(event.target);
});

// About behaves like a full-screen modal overlay, so the explicit close
// affordance in the top-right returns to controls. The plugin name in the center is kept
// as an information display only, to avoid conflating it with the close action.
headerAction.addEventListener("click", (event) => {
  setEditorPage("controls");
  restoreHostFocusIfNeeded(event.target);
});

{
  let lastCssCursor = "";

  const applyNativeCursor = (
    cursorIntent: NativeCursorIntent,
    reason: string,
  ): void => {
    void invoke("apply_native_cursor", {
      cursorIntent,
      reason,
    }).catch(() => undefined);
  };

  const logCursorPropagationTrigger = (event: PointerEvent | MouseEvent): void => {
    if (!nativeCursorBridgeEnabled) {
      return;
    }
    const hitElement = document.elementFromPoint(event.clientX, event.clientY);
    const hitCursor = hitElement
      ? window.getComputedStyle(hitElement).cursor
      : "none";
    if (hitCursor === lastCssCursor) {
      return;
    }
    const cursorIntent = nativeCursorIntentFromCss(hitCursor);
    lastCssCursor = hitCursor;
    applyNativeCursor(cursorIntent, `css-change:${event.type}`);
  };

  for (const type of ["pointerover", "pointermove", "pointerout"]) {
    document.addEventListener(type, logCursorPropagationTrigger, {
      capture: true,
    });
  }

  const resetNativeCursor = (reason: string): void => {
    if (!nativeCursorBridgeEnabled) {
      return;
    }
    if (lastCssCursor !== "auto") {
      lastCssCursor = "auto";
    }
    applyNativeCursor("arrow", reason);
  };

  document.addEventListener("mouseleave", () => resetNativeCursor("document-mouseleave"), {
    capture: true,
  });
  window.addEventListener("blur", () => resetNativeCursor("window-blur"));
  window.addEventListener("pagehide", () => resetNativeCursor("pagehide"));
  window.addEventListener("pointercancel", () => resetNativeCursor("pointercancel"), {
    capture: true,
  });
}

{
  let dragStart:
    | {
        pointerId: number;
        dragId: number;
        width: number;
        height: number;
        lastX: number;
        lastY: number;
      }
    | null = null;
  let inFlight = false;
  let drainResizeQueue: Promise<void> | null = null;
  let resizeDragSeq = 0;
  let queuedSize:
    | {
        width: number;
        height: number;
        dragId: number;
      }
    | null = null;

  const flushResize = () => {
    if (inFlight) {
      return drainResizeQueue ?? Promise.resolve();
    }
    inFlight = true;
    drainResizeQueue = (async () => {
      try {
        while (queuedSize) {
          const size = queuedSize;
          queuedSize = null;
          await invoke<ResizeResponse>("request_gui_resize", {
            request: size,
          }).catch(() => undefined);
        }
      } finally {
        inFlight = false;
      }
      if (queuedSize) {
        await flushResize();
      }
    })().finally(() => {
      if (!inFlight && !queuedSize) {
        drainResizeQueue = null;
      }
    });
    return drainResizeQueue;
  };

  const requestResize = (width: number, height: number) => {
    queuedSize = {
      width: Math.max(1, Math.round(width)),
      height: Math.max(1, Math.round(height)),
      dragId: dragStart?.dragId ?? 0,
    };
    return flushResize();
  };

  const endResizeDragAfterDrain = (dragId: number) => {
    void (async () => {
      // Keep the native drag snapshot alive until the final queued resize request
      // has returned. Otherwise a slow host can make the last request fall back to
      // JS coordinates, exactly the coordinate source this path is trying to avoid.
      await flushResize();
      await invoke("end_gui_resize_drag", {
        request: { dragId },
      }).catch(() => undefined);
    })();
  };

  const applyResizeDelta = (event: PointerEvent) => {
    if (!dragStart || dragStart.pointerId !== event.pointerId) {
      return false;
    }

    // Treat browser pointer events as resize triggers, not the source of truth for
    // coordinates. The host can move or relayout this WebView while processing the
    // same resize request, so the next browser coordinate may include movement of the
    // child view itself. We keep this JS delta only as the non-native fallback; on
    // macOS the Rust command uses dragId to replace it with a desktop cursor delta.
    const deltaX = event.screenX - dragStart.lastX;
    const deltaY = event.screenY - dragStart.lastY;
    if (deltaX === 0 && deltaY === 0) {
      return true;
    }

    dragStart.width += deltaX;
    dragStart.height += deltaY;
    dragStart.lastX = event.screenX;
    dragStart.lastY = event.screenY;
    requestResize(dragStart.width, dragStart.height);
    return true;
  };

  const finishResize = (event: PointerEvent) => {
    if (!applyResizeDelta(event)) {
      return;
    }
    const dragId = dragStart?.dragId;
    dragStart = null;
    if (dragId !== undefined) {
      endResizeDragAfterDrain(dragId);
    }
    restoreHostFocusIfNeeded(event.target);
  };

  const cancelResize = (event: PointerEvent) => {
    if (!dragStart || dragStart.pointerId !== event.pointerId) {
      return;
    }
    const dragId = dragStart.dragId;
    dragStart = null;
    void invoke("end_gui_resize_drag", {
      request: { dragId },
    }).catch(() => undefined);
    restoreHostFocusIfNeeded(event.target);
  };

  resizeGrip.addEventListener("pointerdown", (event) => {
    const dragId = ++resizeDragSeq;
    dragStart = {
      pointerId: event.pointerId,
      dragId,
      width: window.innerWidth,
      height: window.innerHeight,
      lastX: event.screenX,
      lastY: event.screenY,
    };
    void invoke("begin_gui_resize_drag", {
      request: {
        dragId,
        width: dragStart.width,
        height: dragStart.height,
      },
    }).catch(() => undefined);
    resizeGrip.setPointerCapture(event.pointerId);
    event.preventDefault();
  });

  window.addEventListener("pointermove", (event) => {
    if (!dragStart || dragStart.pointerId !== event.pointerId) {
      return;
    }
    applyResizeDelta(event);
    event.preventDefault();
  });

  window.addEventListener("pointerup", finishResize);
  window.addEventListener("pointercancel", cancelResize);
}

// -----------------------------------------------------------------------
// Cleanup
// -----------------------------------------------------------------------
// End any active gesture and unsubscribe before the WebView closes.
window.addEventListener("beforeunload", () => {
  endGesture();
  if (parameterSubscriptionId !== undefined) {
    void invoke("unsubscribe_gui_subscription", {
      subscriptionId: parameterSubscriptionId,
    });
  }
  if (editorPageSubscriptionId !== undefined) {
    void invoke("unsubscribe_gui_subscription", {
      subscriptionId: editorPageSubscriptionId,
    });
  }
});
