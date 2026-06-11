import { invoke } from "@novonotes/webview-bridge";

export type RuntimeOkResponse = {
  ok?: boolean;
};

export type FrontendRuntimeContext = {
  os?: string;
  pluginFormat?: string;
  hostFamily?: string;
  hostName?: string;
  processName?: string;
};

export type NativeLogLevel = "debug" | "info" | "warn" | "error";

export type NativeLogData =
  | null
  | string
  | number
  | boolean
  | NativeLogData[]
  | { [key: string]: NativeLogData };

export type NativeLogEntry = {
  level: NativeLogLevel;
  message: string;
  data?: NativeLogData;
};

export type ResizeRequest = {
  width: number;
  height: number;
  dragId?: number;
};

export type ResizeResponse = RuntimeOkResponse & {
  width?: number;
  height?: number;
};

export type BeginResizeDragRequest = {
  dragId: number;
  width: number;
  height: number;
};

export type EndResizeDragRequest = {
  dragId: number;
};

export type NativeCursorIntent =
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

export type ApplyNativeCursorResponse = RuntimeOkResponse & {
  applied?: boolean;
};

export type WracFrontendRuntime = {
  invoke: typeof invoke;
  writeToLog: (entry: NativeLogEntry) => Promise<RuntimeOkResponse>;
  focusHostWindow: () => Promise<RuntimeOkResponse>;
  getFrontendRuntimeContext: () => Promise<FrontendRuntimeContext>;
  beginGuiResizeDrag: (
    request: BeginResizeDragRequest,
  ) => Promise<RuntimeOkResponse>;
  requestGuiResize: (request: ResizeRequest) => Promise<ResizeResponse>;
  endGuiResizeDrag: (request: EndResizeDragRequest) => Promise<RuntimeOkResponse>;
  applyNativeCursor: (
    cursorIntent: NativeCursorIntent,
    reason: string,
  ) => Promise<ApplyNativeCursorResponse>;
};

export function createWracFrontendRuntime(): WracFrontendRuntime {
  return {
    invoke,
    writeToLog(entry) {
      return invoke("write_to_log", { entry }) as Promise<RuntimeOkResponse>;
    },
    focusHostWindow() {
      return invoke("focus_host_window") as Promise<RuntimeOkResponse>;
    },
    getFrontendRuntimeContext() {
      return invoke("get_frontend_runtime_context") as Promise<FrontendRuntimeContext>;
    },
    beginGuiResizeDrag(request) {
      return invoke("begin_gui_resize_drag", {
        request,
      }) as Promise<RuntimeOkResponse>;
    },
    requestGuiResize(request) {
      return invoke("request_gui_resize", {
        request,
      }) as Promise<ResizeResponse>;
    },
    endGuiResizeDrag(request) {
      return invoke("end_gui_resize_drag", {
        request,
      }) as Promise<RuntimeOkResponse>;
    },
    applyNativeCursor(cursorIntent, reason) {
      return invoke("apply_native_cursor", {
        cursorIntent,
        reason,
      }) as Promise<ApplyNativeCursorResponse>;
    },
  };
}
