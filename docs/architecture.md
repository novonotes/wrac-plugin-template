# アーキテクチャ概要

## スレッドモデル

CLAP プラグインは主に 2 つのスレッドで動作します。

```
┌─────────────────────────────────────────────────────────────────┐
│ メインスレッド（= RunLoop スレッド）                                │
│  - GUI の生成・破棄・リサイズ (gui.rs)                             │
│  - パラメータ情報の公開 (params.rs)                                │
│  - WxpCommandHandler によるコマンド処理                            │
│  - 状態の保存・復元                                                │
│  - wxp の WebView イベント処理                                     │
│  - RunLoopSender 経由で他スレッドからタスクを受け取る                 │
│  - Channel::send() による Rust → JS 通知もここで実行               │
├─────────────────────────────────────────────────────────────────┤
│ オーディオスレッド（リアルタイム）                                    │
│  - process() でサンプルにゲインを掛ける (audio.rs)                  │
│  - ロック・メモリ割り当て・I/O は禁止                                │
└─────────────────────────────────────────────────────────────────┘
```

> **補足:** RunLoop はメインスレッド上で初期化されるため、
> このプラグインでは RunLoop スレッド＝メインスレッドです。
> `RunLoopSender` はオーディオスレッドなど別のスレッドから
> メインスレッドにクロージャをポストするために使います。

## Rust ↔ JavaScript 通信

```
JavaScript (main.ts)                    Rust (plugin.rs)
──────────────────                      ────────────────
invoke("set_gain", {value})  ──────►   WxpCommandHandler
                                        └─ register_sync("set_gain", ...)

Channel コールバック        ◄──────    RunLoopSender → Channel::send()
  └─ render(state)                      └─ notify_gui()
```

- **JS → Rust**: `invoke()` で Rust 側に登録されたコマンドを RPC 呼び出し
- **Rust → JS**: `Channel` によるプッシュ通知。ホストがオートメーション等で値を変更したとき、`RunLoopSender` 経由でメインスレッドにディスパッチし、`Channel::send()` で JS に JSON を送信

## パラメータ変更の流れ

**UI → ホスト:**

```
1. ユーザーがノブをドラッグ開始
2. JS: invoke("begin_parameter_gesture")
3. JS: invoke("set_gain", {value})          ← ドラッグ中に繰り返し
4. Rust: SharedStateInner の AtomicF32 を更新 + pending フラグを立てる
5. オーディオスレッド: process() で pending フラグを読み取り、output events としてホストに通知
6. ユーザーがドラッグ終了
7. JS: invoke("end_parameter_gesture")
```

**ホスト → UI:**

```
1. ホストがオートメーション等で値を変更
2. Rust: process() の input events から ParamValue を受け取る
3. Rust: SharedStateInner の AtomicF32 を更新
4. Rust: notify_gui() → RunLoopSender → Channel::send()
5. JS: Channel コールバックで render() が呼ばれ、UI が更新される
```

## wxp 初期化フロー（CLAP コンテキスト）

CLAP の `set_parent()` コールバック内で WebView を生成します（実装は `gui.rs` 参照）。

1. `WebContext::new(data_dir).build_wry_context()` — ユーザーデータディレクトリを設定。返した `wry::WebContext` は WebView の生存期間中保持し続ける必要があるため `self` に格納する
2. `wxp_clack::window::clack_to_wry_window_handle(&parent)` — CLAP の `Window` を wry の `WindowHandle` に変換
3. `WxpWebViewBuilder::new(&mut wry_context)` でビルダーを作成し、コマンドハンドラ・URL・サイズ等を設定
4. `.build_as_child(&parent_handle)` で `WebViewRef` を取得して `self` に格納

`destroy()` では `reset_webview()` を通じて GUI 通知チャネルのクリア、`WebViewRef` の drop による WebView 破棄、`wry::WebContext` の破棄を順に行います。

## 主要な依存クレート

| クレート | 役割 |
|---------|------|
| `clack-plugin` / `clack-extensions` | CLAP プラグイン API の Rust バインディング |
| `wxp` | WebView GUI フレームワーク（WxpWebViewBuilder, WxpCommandHandler, Channel） |
| `wxp_clack` | wxp と CLAP を繋ぐユーティリティ（DPI 変換、ウィンドウハンドル変換） |
| `novonotes_run_loop` | プラットフォーム抽象化されたイベントループ（RunLoop, RunLoopSender） |
| `wry` | WebView エンジン（wxp が内部で使用） |
| `@novonotes/webview-bridge` | JS 側の通信ライブラリ（invoke, Channel） |
