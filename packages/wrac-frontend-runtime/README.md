# @novonotes/wrac-frontend-runtime

Shared TypeScript helpers for WRAC/WXP plugin frontends.

This package contains runtime behavior that is common to DAW-hosted WebView
plugin GUIs:

- frontend log forwarding via `write_to_log`
- host focus restoration via `focus_host_window`
- frontend runtime context via `get_frontend_runtime_context`
- native cursor bridging via `apply_native_cursor`
- host GUI resizing via `begin_gui_resize_drag`, `request_gui_resize`, and
  `end_gui_resize_drag`

It intentionally does not define product parameter APIs, device command
schemas, telemetry payloads, preset behavior, or client subscription models.
Those contracts belong to the plugin or device layer.

## Example

```ts
import {
  createHostFocusRestorer,
  createWracFrontendRuntime,
  installConsoleLogPipe,
  installNativeCursorBridge,
  installResizeBridge,
} from "@novonotes/wrac-frontend-runtime";

const runtime = createWracFrontendRuntime();
installConsoleLogPipe(runtime.writeToLog);

const restoreHostFocus = createHostFocusRestorer(runtime);

installResizeBridge({
  runtime,
  resizeGrip,
  restoreHostFocus,
});

const context = await runtime.getFrontendRuntimeContext().catch(() => ({}));
installNativeCursorBridge({
  runtime,
  context,
});
```
