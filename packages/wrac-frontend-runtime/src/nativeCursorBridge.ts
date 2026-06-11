import type {
  FrontendRuntimeContext,
  NativeCursorIntent,
  WracFrontendRuntime,
} from "./runtime";

export type NativeCursorBridge = {
  dispose: () => void;
  refresh: (reason?: string) => void;
};

export type NativeCursorBridgeOptions = {
  runtime: WracFrontendRuntime;
  context: FrontendRuntimeContext;
  shouldEnable?: (context: FrontendRuntimeContext) => boolean;
};

export function installNativeCursorBridge({
  runtime,
  context,
  shouldEnable = defaultShouldEnableNativeCursorBridge,
}: NativeCursorBridgeOptions): NativeCursorBridge | undefined {
  if (!shouldEnable(context)) {
    return undefined;
  }

  let lastCssCursor = "";
  let lastPointer: { clientX: number; clientY: number } | undefined;

  const applyNativeCursor = (
    cursorIntent: NativeCursorIntent,
    reason: string,
  ): void => {
    void runtime.applyNativeCursor(cursorIntent, reason).catch(() => undefined);
  };

  const applyCursorAtPoint = (
    clientX: number,
    clientY: number,
    reason: string,
  ): void => {
    const hitElement = document.elementFromPoint(clientX, clientY);
    const hitCursor = hitElement
      ? window.getComputedStyle(hitElement).cursor
      : "none";
    if (hitCursor === lastCssCursor) {
      return;
    }
    lastCssCursor = hitCursor;
    applyNativeCursor(nativeCursorIntentFromCss(hitCursor), reason);
  };

  const handlePointerCursor: EventListener = (event): void => {
    if (!(event instanceof MouseEvent)) {
      return;
    }
    lastPointer = {
      clientX: event.clientX,
      clientY: event.clientY,
    };
    applyCursorAtPoint(
      event.clientX,
      event.clientY,
      `css-change:${event.type}`,
    );
  };

  const resetNativeCursor = (reason: string): void => {
    lastCssCursor = "auto";
    applyNativeCursor("arrow", reason);
  };
  const handleDocumentMouseLeave = () =>
    resetNativeCursor("document-mouseleave");
  const handleWindowBlur = () => resetNativeCursor("window-blur");
  const handlePageHide = () => resetNativeCursor("pagehide");
  const handlePointerCancel = () => resetNativeCursor("pointercancel");

  for (const type of ["pointerover", "pointermove", "pointerout"]) {
    document.addEventListener(type, handlePointerCursor, {
      capture: true,
    });
  }

  document.addEventListener("mouseleave", handleDocumentMouseLeave, {
    capture: true,
  });
  window.addEventListener("blur", handleWindowBlur);
  window.addEventListener("pagehide", handlePageHide);
  window.addEventListener("pointercancel", handlePointerCancel);

  return {
    dispose: () => {
      for (const type of ["pointerover", "pointermove", "pointerout"]) {
        document.removeEventListener(type, handlePointerCursor, {
          capture: true,
        });
      }
      document.removeEventListener("mouseleave", handleDocumentMouseLeave, {
        capture: true,
      });
      window.removeEventListener("blur", handleWindowBlur);
      window.removeEventListener("pagehide", handlePageHide);
      window.removeEventListener("pointercancel", handlePointerCancel);
      resetNativeCursor("dispose");
    },
    refresh: (reason = "manual-refresh") => {
      if (lastPointer) {
        applyCursorAtPoint(lastPointer.clientX, lastPointer.clientY, reason);
      }
    },
  };
}

export function defaultShouldEnableNativeCursorBridge(
  context: FrontendRuntimeContext,
): boolean {
  return (
    context.os === "macos" &&
    context.pluginFormat === "vst3" &&
    (context.hostFamily === "steinberg-cubase" ||
      context.hostFamily === "steinberg-cubase-bridged")
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
