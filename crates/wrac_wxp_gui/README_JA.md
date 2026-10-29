# wrac_wxp_gui

> English version: [README.md](README.md)

`wrac_wxp_gui` は `wrac_clap_adapter::interface::PluginGuiExtension` と wxp WebView runtime を接続する helper crate です。

役割は2つあります。`wrac_clap_adapter::interface` が定義するホストウィンドウのハンドルを、wxp に渡すための `raw-window-handle` 型へ変換することと、特定のスレッドからのみ操作できる WebView runtime をホストの UI スレッドで保持することです。CLAP C ABI とのやり取りはこの crate ではなく `wrac_clap_adapter` が担います。

## 前提

- `set_parent()` で UI スレッドを確定し、`show()` 呼び出し時にそのスレッド上で GUI runtime を作成します
- 1プロセスにつきホストの UI スレッドは1つと想定します
- 複数の UI スレッドを使うホストは非対応として失敗します
- この helper はフローティングウィンドウを扱いません
- これは汎用フレームワークではなく実装例の一部です。今後の変更では API の後方互換性や移行サポートは提供しません

## 参考

wxp crate の使い方は [wxp README](https://github.com/novonotes/wxp/tree/main/crates/wxp) を参照してください。
