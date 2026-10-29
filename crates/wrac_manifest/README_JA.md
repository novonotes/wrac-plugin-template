# wrac_manifest

> 英語版: [README.md](README.md)

`wrac_manifest` は WRAC プラグインのマニフェストを読み込み、検証するクレートです。
スキーマ、検証ルール、利用上の注意は [src/lib.rs](src/lib.rs) に定義しています。

実例は [WRAC Gain のマニフェスト](../../plugins/wrac-gain/src-plugin/wrac-plugin.toml) を参照してください。

xtask によるマニフェストの利用方法は、`wrac_build_ops` の以下のコードを参照してください。

- [メタデータの読み込み (metadata.rs)](../wrac_build_ops/src/metadata.rs)、
- [対象の選択 (target_resolution.rs)](../wrac_build_ops/src/target_resolution.rs)
- [コンテキスト API (context.rs)](../wrac_build_ops/src/context.rs)
