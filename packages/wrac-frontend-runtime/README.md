# @novonotes/wrac-frontend-runtime

[日本語](README_JA.md)

The complete WRAC-facing facade for WRAC/WXP plugin frontends. Plugin
frontends and higher-level frontend packages communicate with the host through
this package instead of importing `@novonotes/webview-bridge` directly.

This package contains runtime behavior that is common to DAW-hosted WebView
plugin GUIs:

- frontend log forwarding via `write_to_log`
- host key routing that keeps keys such as Space with the host and returns
  focus via `focus_host_window` after GUI operations
- frontend runtime context via `get_frontend_runtime_context`
- typed command invocation via `runtime.invoke<TResponse>()`
- push channels via `runtime.createChannel<T>()`
- native cursor bridging via `apply_native_cursor`
- host GUI resizing via `begin_gui_resize_drag`, `request_gui_resize`, and
  `end_gui_resize_drag`

It intentionally does not define product parameter APIs, device command
schemas, telemetry payloads, preset behavior, or client subscription models.
Those contracts belong to the plugin or device layer.

The public invocation and channel contracts are deliberately owned by this
package and expose only what WRAC frontends need. The default transport uses
`@novonotes/webview-bridge`, while tests can pass a `WracFrontendTransport` to
`createWracFrontendRuntime()` without depending on a WebView host.

## Example

```ts
import {
  createWracFrontendRuntime,
  installConsoleLogPipe,
  installHostKeyRouting,
  installNativeCursorBridge,
  installResizeBridge,
} from "@novonotes/wrac-frontend-runtime";

const runtime = createWracFrontendRuntime();
const events = runtime.createChannel<{ type: string }>((event) => {
  console.info(event.type);
});
await runtime.invoke("subscribe", { channel: events });
installConsoleLogPipe(runtime.writeToLog);

const { restoreHostFocus } = installHostKeyRouting(runtime);

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
