import { createResizeController, type ResizeController } from "./resizeController";
import type { WracFrontendRuntime } from "./runtime";

export type ResizeBridge = {
  dispose: () => void;
  controller: ResizeController;
};

export type ResizeBridgeOptions = {
  runtime: WracFrontendRuntime;
  resizeGrip: HTMLElement;
  restoreHostFocus?: (target?: EventTarget | null) => void;
};

export function installResizeBridge({
  runtime,
  resizeGrip,
  restoreHostFocus,
}: ResizeBridgeOptions): ResizeBridge {
  const controller = createResizeController(runtime);
  let pointerId: number | undefined;
  let resizeDragSeq = 0;

  const handlePointerDown = (event: PointerEvent) => {
    pointerId = event.pointerId;
    controller.begin({
      dragId: ++resizeDragSeq,
      width: window.innerWidth,
      height: window.innerHeight,
      screenX: event.screenX,
      screenY: event.screenY,
    });
    resizeGrip.setPointerCapture(event.pointerId);
    event.preventDefault();
  };

  const handlePointerMove = (event: PointerEvent) => {
    if (pointerId !== event.pointerId) {
      return;
    }
    controller.move({
      screenX: event.screenX,
      screenY: event.screenY,
    });
    event.preventDefault();
  };

  const finishResize = (event: PointerEvent) => {
    if (pointerId !== event.pointerId) {
      return;
    }
    pointerId = undefined;
    controller.move({
      screenX: event.screenX,
      screenY: event.screenY,
    });
    controller.end();
    restoreHostFocus?.(event.target);
  };

  const cancelResize = (event: PointerEvent) => {
    if (pointerId !== event.pointerId) {
      return;
    }
    pointerId = undefined;
    controller.cancel();
    restoreHostFocus?.(event.target);
  };

  resizeGrip.addEventListener("pointerdown", handlePointerDown);
  window.addEventListener("pointermove", handlePointerMove);
  window.addEventListener("pointerup", finishResize);
  window.addEventListener("pointercancel", cancelResize);

  return {
    controller,
    dispose() {
      resizeGrip.removeEventListener("pointerdown", handlePointerDown);
      window.removeEventListener("pointermove", handlePointerMove);
      window.removeEventListener("pointerup", finishResize);
      window.removeEventListener("pointercancel", cancelResize);
      controller.cancel();
    },
  };
}
