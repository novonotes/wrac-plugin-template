# @novonotes/wrac-frontend-runtime

[English](README.md)

WRAC/WXP プラグインのフロントエンド向けに、WRAC の機能をまとめて提供するパッケージです。
プラグインのフロントエンドや上位のパッケージは、`@novonotes/webview-bridge` を直接
インポートせず、このパッケージを介してホストと通信します。

DAW で動作する WebView プラグイン GUI に共通する、次の機能を提供します。

- `write_to_log` によるフロントエンドのログ転送
- Space などのキーをホストに渡し、GUI 操作後に `focus_host_window` でフォーカスをホストに戻すキー処理
- `get_frontend_runtime_context` による実行環境情報の取得
- `runtime.invoke<TResponse>()` による型付きコマンドの呼び出し
- `runtime.createChannel<T>()` によるプッシュ通知用チャネルの生成
- `apply_native_cursor` によるネイティブカーソルとの連携
- `begin_gui_resize_drag`、`request_gui_resize`、`end_gui_resize_drag` によるホスト GUI のサイズ変更

パラメーター API、コマンド、プリセットの動作など、プラグイン固有の契約はこの共通
ランタイムではなく、プラグイン側で定義します。

コマンド呼び出しとチャネルの契約はこのパッケージが管理し、WRAC のフロントエンドに
必要な最小限のインターフェースを公開します。通信には既定で
`@novonotes/webview-bridge` を使用します。テストでは、
`createWracFrontendRuntime()` に `WracFrontendTransport` を渡して通信手段を差し替えることで、
WebView ホストに依存せずに実行できます。

## 使用例

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
