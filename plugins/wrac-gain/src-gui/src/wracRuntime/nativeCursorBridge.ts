import { invoke } from "@novonotes/webview-bridge";

// Cubase VST3 on macOS can fail to propagate WKWebView's CSS cursor to the
// host-owned Cocoa parent. Keep this bridge CSS-driven so template users only
// need to set `cursor` in styles for new hover targets.
export type FrontendRuntimeContext = {
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

type NativeCursorBridge = {
  dispose: () => void;
  refresh: (reason?: string) => void;
};

export function installNativeCursorBridge(
  context: FrontendRuntimeContext,
): NativeCursorBridge | undefined {
  if (!shouldUseNativeCursorBridge(context)) {
    return undefined;
  }

  let lastCssCursor = "";
  let lastPointer: { clientX: number; clientY: number } | undefined;

  const applyNativeCursor = (
    cursorIntent: NativeCursorIntent,
    reason: string,
  ): void => {
    void invoke("apply_native_cursor", {
      cursorIntent,
      reason,
    }).catch(() => undefined);
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

  const handlePointerCursor = (event: PointerEvent | MouseEvent): void => {
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
