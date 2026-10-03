import {
  Channel as WebviewBridgeChannel,
  invoke as webviewBridgeInvoke,
} from "@novonotes/webview-bridge";

type RuntimeInvokeArgs = Record<string, unknown>;

type RuntimeChannelMessageHandler<T> = (message: T) => void;

// Keep transport serialization private: frontend code may replace the receiver and pass
// this handle through invoke arguments, but must not inspect or construct transport state.
export type RuntimeChannel<T> = {
  onmessage: RuntimeChannelMessageHandler<T> | undefined;
};

// invoke and createChannel must belong to the same transport. A channel is only guaranteed
// to serialize correctly when it is passed back through the invoke implementation that
// created it; mixing handles across transports is outside this contract.
export type WracFrontendTransport = {
  // Product packages own command, payload, and response schemas. The transport forwards
  // them unchanged and does not treat TResponse as runtime validation.
  invoke<TResponse = unknown>(
    command: string,
    args?: RuntimeInvokeArgs,
  ): Promise<TResponse>;
  createChannel<T = unknown>(
    onMessage?: RuntimeChannelMessageHandler<T>,
  ): RuntimeChannel<T>;
};

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
  // The runtime deliberately preserves the generic command surface so product contracts
  // remain in higher-level packages instead of accumulating in this shared facade.
  invoke<TResponse = unknown>(
    command: string,
    args?: RuntimeInvokeArgs,
  ): Promise<TResponse>;
  createChannel<T = unknown>(
    onMessage?: RuntimeChannelMessageHandler<T>,
  ): RuntimeChannel<T>;
  writeToLog: (entry: NativeLogEntry) => Promise<RuntimeOkResponse>;
  focusHostWindow: () => Promise<RuntimeOkResponse>;
  getFrontendRuntimeContext: () => Promise<FrontendRuntimeContext>;
  beginGuiResizeDrag: (
    request: BeginResizeDragRequest,
  ) => Promise<RuntimeOkResponse>;
  requestGuiResize: (request: ResizeRequest) => Promise<ResizeResponse>;
  endGuiResizeDrag: (
    request: EndResizeDragRequest,
  ) => Promise<RuntimeOkResponse>;
  applyNativeCursor: (
    cursorIntent: NativeCursorIntent,
    reason: string,
  ) => Promise<ApplyNativeCursorResponse>;
};

// This is the only bridge adapter in the frontend stack. Keeping it here prevents bridge
// version and environment-specific channel construction from leaking into consumers.
const defaultTransport: WracFrontendTransport = {
  invoke<TResponse>(command: string, args?: RuntimeInvokeArgs) {
    return webviewBridgeInvoke<TResponse>(command, args);
  },
  createChannel<T>(onMessage?: RuntimeChannelMessageHandler<T>) {
    return new WebviewBridgeChannel<T>(onMessage);
  },
};

export function createWracFrontendRuntime(
  transport: WracFrontendTransport = defaultTransport,
): WracFrontendRuntime {
  // Every facade method closes over one transport so common host commands, raw product
  // commands, and channel handles cannot accidentally use different implementations.
  return {
    invoke(command, args) {
      return transport.invoke(command, args);
    },
    createChannel(onMessage) {
      return transport.createChannel(onMessage);
    },
    writeToLog(entry) {
      return transport.invoke<RuntimeOkResponse>("write_to_log", { entry });
    },
    focusHostWindow() {
      return transport.invoke<RuntimeOkResponse>("focus_host_window");
    },
    getFrontendRuntimeContext() {
      return transport.invoke<FrontendRuntimeContext>(
        "get_frontend_runtime_context",
      );
    },
    beginGuiResizeDrag(request) {
      return transport.invoke<RuntimeOkResponse>("begin_gui_resize_drag", {
        request,
      });
    },
    requestGuiResize(request) {
      return transport.invoke<ResizeResponse>("request_gui_resize", {
        request,
      });
    },
    endGuiResizeDrag(request) {
      return transport.invoke<RuntimeOkResponse>("end_gui_resize_drag", {
        request,
      });
    },
    applyNativeCursor(cursorIntent, reason) {
      return transport.invoke<ApplyNativeCursorResponse>(
        "apply_native_cursor",
        {
          cursorIntent,
          reason,
        },
      );
    },
  };
}
