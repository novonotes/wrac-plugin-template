# AAX Build and Validation

> English version: [aax.md](aax.md)

AAX サポートは、`wrac-plugin.toml` の `formats` に
`aax` を追加することで有効にできます。

## 前提条件

- AAX SDK
- AAX Validator/DSH archive
- CMake と clap-wrapper が対応する platform C++ toolchain

いくつかの環境変数を設定する必要があります。 `.env.example` を参照してください。

## MIDI 入力

ノート入力を持つプラグインで Pro Tools から MIDI を受け取るには、
`wrac-plugin.toml` の対象の `[[plugins]]` に ASCII のノード名を指定します。

```toml
aax_midi_input_name = "MIDI In"
```

AAX はインスタンス生成前に MIDI ノードを登録します。この設定を省略すると、
AAX ではノート入力を無視し、初期化時に宣言がないことをログに出します。
`software-generator` などのカテゴリ指定では MIDI 入力は宣言されません。
MIDI で操作するオーディオエフェクトにもこの設定が必要です。プラグイン側では
ノート入力ポートを公開してください。WRAC は現在、AAX の MIDI 出力宣言に対応していません。

## サイドチェイン入力

ホストのサイドチェインを最初の CLAP 補助入力に接続するには、
`wrac-plugin.toml` の対象の `[[plugins]]` に次を追加します。

```toml
aax_sidechain_input = "first-auxiliary"
```

AAX のサイドチェインはモノラルです。ラッパーは同じ信号を CLAP 入力ポート 1 の
各チャンネルに渡し、それ以外の補助入力には無音を渡します。選択された AAX 構成の
適用後に、ポート 0 にメイン入力、ポート 1 に補助入力が必要です。どちらも
チャンネルを持つ必要があります。宣言が不正な場合は初期化を失敗させますが、
ホストのサイドチェインが未接続の場合は正常に無音を渡します。
このマッピングは、メイン入力を持たないサイドチェインには対応していません。
