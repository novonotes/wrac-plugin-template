# AAX Build and Validation

> English version: [aax.md](aax.md)

AAX サポートは、`wrac-plugin.toml` の `formats` に
`aax` を追加することで有効にできます。

## 前提条件

- AAX SDK
- AAX Validator/DSH archive
- CMake と clap-wrapper が対応する platform C++ toolchain

いくつかの環境変数を設定する必要があります。 `.env.example` を参照してください。。

## サイドチェイン入力

ホストのサイドチェインを最初の CLAP 補助入力に接続するには、
`wrac-plugin.toml` の対象の `[[plugins]]` に次を追加します。

```toml
aax_sidechain_input = "first-auxiliary"
```

AAX のサイドチェインはモノラルです。ラッパーは同じ信号を CLAP 入力ポート 1 の
各チャンネルに渡し、それ以外の補助入力には無音を渡します。プラグイン側で
この入力ポートを公開する必要があります。
