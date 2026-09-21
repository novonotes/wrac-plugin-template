# @novonotes/wrac-frontend-runtime

[English](README.md)

WRAC/WXP plugin frontend向けの完全なWRAC facadeです。plugin frontendと上位の
frontend packageは、`@novonotes/webview-bridge`を直接importせず、このpackageを
介してhostと通信します。

このpackageは、DAWにhostされるWebView plugin GUIで共通する次のruntime機能を
提供します。

- `write_to_log`によるfrontend log転送
- `focus_host_window`によるhost windowへのfocus復元
- `get_frontend_runtime_context`によるfrontend runtime context取得
- `runtime.invoke<TResponse>()`による型付きcommand呼び出し
- `runtime.createChannel<T>()`によるpush channel生成
- `apply_native_cursor`によるnative cursor連携
- `begin_gui_resize_drag`、`request_gui_resize`、`end_gui_resize_drag`によるhost GUIのresize

製品parameter API、device command schema、telemetry payload、preset動作、client固有の
subscription modelは意図的に定義しません。これらの契約はpluginまたはdevice layerに
属します。

公開するinvokeとchannelの契約はこのpackageが独自に所有し、WRAC frontendに必要な
最小限のinterfaceだけを公開します。default transportは
`@novonotes/webview-bridge`を使用します。テストではWebView hostへ依存せず、
`WracFrontendTransport`を`createWracFrontendRuntime()`へ渡してtransportを
差し替えられます。

## 使用例

```ts
import {
  createHostFocusRestorer,
  createWracFrontendRuntime,
  installConsoleLogPipe,
  installNativeCursorBridge,
  installResizeBridge,
} from "@novonotes/wrac-frontend-runtime";

const runtime = createWracFrontendRuntime();
const events = runtime.createChannel<{ type: string }>((event) => {
  console.info(event.type);
});
await runtime.invoke("subscribe", { channel: events });
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
