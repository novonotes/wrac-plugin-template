import type { WracFrontendRuntime } from "./runtime";

type ResizeDragState = {
  dragId: number;
  width: number;
  height: number;
  lastX: number;
  lastY: number;
};

export type ResizeController = {
  begin: (args: {
    dragId: number;
    width: number;
    height: number;
    screenX: number;
    screenY: number;
  }) => void;
  move: (args: { screenX: number; screenY: number }) => boolean;
  end: () => void;
  cancel: () => void;
  requestResize: (width: number, height: number, dragId?: number) => Promise<void>;
  flush: () => Promise<void>;
};

export function createResizeController(
  runtime: WracFrontendRuntime,
): ResizeController {
  let dragState: ResizeDragState | null = null;
  let inFlight = false;
  let drainResizeQueue: Promise<void> | null = null;
  let queuedSize:
    | {
        width: number;
        height: number;
        dragId?: number;
      }
    | null = null;

  const flush = () => {
    if (inFlight) {
      return drainResizeQueue ?? Promise.resolve();
    }
    inFlight = true;
    drainResizeQueue = (async () => {
      try {
        while (queuedSize) {
          const size = queuedSize;
          queuedSize = null;
          await runtime.requestGuiResize(size).catch(() => undefined);
        }
      } finally {
        inFlight = false;
      }
      if (queuedSize) {
        await flush();
      }
    })().finally(() => {
      if (!inFlight && !queuedSize) {
        drainResizeQueue = null;
      }
    });
    return drainResizeQueue;
  };

  const requestResize = (width: number, height: number, dragId?: number) => {
    queuedSize = {
      width: Math.max(1, Math.round(width)),
      height: Math.max(1, Math.round(height)),
      dragId,
    };
    return flush();
  };

  const endNativeDragAfterDrain = (dragId: number) => {
    void (async () => {
      // Keep the native drag snapshot alive until the final queued resize request
      // returns; otherwise the last request may fall back to WebView-relative deltas.
      await flush();
      await runtime.endGuiResizeDrag({ dragId }).catch(() => undefined);
    })();
  };

  return {
    begin({ dragId, width, height, screenX, screenY }) {
      dragState = {
        dragId,
        width,
        height,
        lastX: screenX,
        lastY: screenY,
      };
      void runtime
        .beginGuiResizeDrag({
          dragId,
          width,
          height,
        })
        .catch(() => undefined);
    },
    move({ screenX, screenY }) {
      if (!dragState) {
        return false;
      }

      const deltaX = screenX - dragState.lastX;
      const deltaY = screenY - dragState.lastY;
      if (deltaX === 0 && deltaY === 0) {
        return true;
      }

      dragState.width += deltaX;
      dragState.height += deltaY;
      dragState.lastX = screenX;
      dragState.lastY = screenY;
      void requestResize(dragState.width, dragState.height, dragState.dragId);
      return true;
    },
    end() {
      const dragId = dragState?.dragId;
      dragState = null;
      if (dragId !== undefined) {
        endNativeDragAfterDrain(dragId);
      }
    },
    cancel() {
      const dragId = dragState?.dragId;
      dragState = null;
      if (dragId !== undefined) {
        void runtime.endGuiResizeDrag({ dragId }).catch(() => undefined);
      }
    },
    requestResize,
    flush,
  };
}
