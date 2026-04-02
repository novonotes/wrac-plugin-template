# wxp-gain-example

`wxp` を使った gain plugin の参照実装を独立して管理するためのリポジトリです。

## Getting Started

新しい wxp プラグインの作成手順は [Getting Started](docs/getting-started.md) を参照してください。

## 含まれるもの

| パス | 内容 |
|-----|------|
| `src-plugin` | Rust 製の CLAP プラグイン本体 |
| `src-gui` | TypeScript + HTML/CSS で書かれた GUI |
| `script` | ビルド・インストール用スクリプト |
| `clap_wrapper_builder` | CLAP を VST3 / AUv2 / Standalone にラップする補助ビルド環境 |

## Positioning

このリポジトリは公式スターターキットではなく、`wxp` を使った最小構成の参照実装です。
必要に応じてコピーして調整する前提で管理しています。

## Architecture Overview

この gain plugin は、入力信号にゲインを掛けるだけのシンプルなエフェクトですが、
`wxp` を使ったプラグイン開発に必要な要素を一通り含んでいます。

- Rust 側で CLAP プラグインとパラメータ処理を実装
- `wxp` と `@novonotes/webview-bridge` を使って WebView GUI と通信
- `clap_wrapper_builder` を使って VST3 / AUv2 / Standalone を生成

## 依存方針

- Rust クレートは `git + rev` で `wxp` / `wxp_clack` / `wry` / `run_loop` を参照します。
- `@novonotes/webview-bridge` は配布 tarball URL から取得します。
