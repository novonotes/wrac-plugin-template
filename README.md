# wxp-examples

`wxp` のサンプル・参照実装を独立して管理するためのリポジトリです。

## 含まれるもの

- `examples/gain_plugin` - wxp を使った CLAP プラグインの入門サンプル
- `clap_wrapper_builder` - CLAP を VST3 / AUv2 / Standalone にラップする補助ビルド環境

## 依存方針

- Rust クレートは `git + rev` で `wxp` / `wxp_clack` / `wry` / `run_loop` を参照します。
- `@novonotes/webview-bridge` は配布 tarball URL から取得します。
