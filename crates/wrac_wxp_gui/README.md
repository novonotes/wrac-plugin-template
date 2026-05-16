# wrac_wxp_gui

`wrac_wxp_gui` は `wrac_clap_adapter` の `PluginGui` と wxp WebView runtime を接続する helper crate です。

責務は 2 つです。1 つは CLAP の `clap_window_t` 由来の `ClapWindow` を `raw-window-handle` の型に変換して wxp に渡すこと、もう 1 つは特定の thread からしか操作できない WebView runtime を host UI thread 上に保持することです。CLAP C ABI とのインタラクションはこのクレートの責務ではなく、wrac_clap_adapter の責務です。

## 前提

- `set_parent()` で UI thread を固定し、GUI runtime はその thread 上で `show()` 時に作る
- 1 process 内の host UI thread は単一とみなす
- 複数 UI thread を使う host は unsupported として失敗させる
- floating window はこの helper では扱わない
- 汎用的なフレームワークではなく実装例の一部です。今後の変更に伴う、API の後方互換性やマイグレーションサポートは提供しません。

## 参考
wxp クレートの使い方は [wxp の README](https://github.com/novonotes/wxp/tree/main/crates/wxp) に記載しています。
