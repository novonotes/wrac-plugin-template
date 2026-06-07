import { invoke } from "@novonotes/webview-bridge";

type ResizeResponse = {
  ok?: boolean;
  width?: number;
  height?: number;
};

type ResizeBridgeOptions = {
  resizeGrip: HTMLElement;
  restoreHostFocus?: (target?: EventTarget | null) => void;
};

export function installResizeBridge({
  resizeGrip,
  restoreHostFocus,
}: ResizeBridgeOptions): void {
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
    restoreHostFocus?.(event.target);
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
    restoreHostFocus?.(event.target);
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
