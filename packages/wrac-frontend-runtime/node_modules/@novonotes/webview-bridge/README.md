# @novonotes/webview-bridge

WebView 内のフロントエンドコードとバックエンド間の IPC 通信を共通のインターフェースで利用できるようにする TypeScript ライブラリです。

## 概要

このパッケージは、異なる WebView 環境（Tauri、wxp）において、統一された IPC 通信の API を提供します。実行環境を自動的に検出し、適切なバックエンドの API を使用するため、フロントエンドコードの再利用が可能です。

### サポートするバックエンド

- **Tauri** - Tauri アプリケーション
- **wxp** - WebView X Plugin（オーディオプラグイン開発向け）

## 使用方法

当面は npm publish を行わず、tarball 配布を前提にします。

```sh
npm install
npm run build
npm pack
```

rust 側との統合方法については [wxp](https://github.com/novonotes/wxp) や [tauri](https://github.com/tauri-apps/tauri) のドキュメントを参照してください。

### Command

```typescript
import { invoke, Channel } from "@novonotes/webview-bridge";

// コマンドの呼び出し
const result = await invoke("greet", { name: "World" });
console.log(result); // "Hello, World!"
```

### Channel

```typescript
import { invoke, Channel } from "@novonotes/webview-bridge";

// ストリーミングチャンネル
const channel = new Channel<MyMessageType>((message) => {
  console.log("Received:", message);
});

await invoke("start_streaming", { channel });
```

### 環境検出

```typescript
import { detectEnvironment } from "@novonotes/webview-bridge";

const env = detectEnvironment();
console.log("Current environment:", env); // 'tauri' | 'wxp' | 'unknown'
```
