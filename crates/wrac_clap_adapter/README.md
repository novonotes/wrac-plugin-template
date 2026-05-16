# wrac_clap_adapter

各製品コードが実装するべき trait 群を定義し、
その trait 実装を CLAP ABI に適合させ、プラグインとして利用可能にするアダプターを提供します。

VST3 / AU / AAX への変換は `clap-wrapper` の責務で、本 crate は CLAP plugin および CLAP extension を Rust 側で実装することに専念します。

## 目的

CLAP プラグインを clap-wrapper 経由で他フォーマットのホストから利用する場合、 CLAP の定義するスレッドモデルや呼び出し順序の契約が一部守られない場合があります。それらに対して防御的な機構で対応することを目的としています。

## clack との違い

CLAP のヘッダには、各関数を呼び出してよい thread が `[main-thread]` `[audio-thread]` `[thread-safe]` といったコメントで示されています。例えば `init` は `[main-thread]`、`process` は `[audio-thread]`、`get_extension` は `[thread-safe]` です。

`clack` は host がこのコメント通りに関数を呼び出す前提で型を設計しており、Native CLAP host 向けには素直に動作します。

一方、本 crate は `clap-wrapper` 経由の VST3 / AU / AAX host も対象とします。これらの host を経由すると、`[main-thread]` 指定の query が別 thread から呼ばれるなど、コメント通りの呼び出し順・呼び出し thread にならない場合があります。本 crate はこれを adapter 側の lock や panic 捕捉などで受け、製品コードに `unsafe` を露出させずに動作させることを目的としています。

## 謝辞

`wrac_clap_adapter` は、`clack` の safe で low-level な CLAP wrapper 設計、特に CLAP extension 境界と audio buffer access の考え方を参考にさせていただきました。本 crate は `clack` のコードの派生ではなく、`clap-sys` を直接用いた独立した実装です。

## Public API

- `PluginCore`: instance lifecycle と、サポートする extension の宣言
- `PluginAudioPorts`: CLAP `audio-ports`
- `PluginConfigurableAudioPorts`: CLAP `configurable-audio-ports`
- `PluginNotePorts`: CLAP `note-ports`
- `PluginParameters`: CLAP `params`
- `PluginStateSupport`: CLAP `state`
- `PluginGui`: CLAP `gui`
- `export_clap_plugin!`: CLAP entry point の export

各 trait は CLAP C ABI に対応する薄い Rust 表現です。独自の plugin framework としては設計しません。

## 制約

この crate は汎用フレームワークではなく、実装例の一部として提供しています。今後の変更では、API の後方互換性やマイグレーションサポートは提供しません。

また、現時点では CLAP の全 ABI には対応できておらず、以下の制限があります。

- `audio-ports`: 現在の port metadata の公開のみ。port 構成の動的変更通知 (rescan) は未対応
- `configurable-audio-ports`: 非 active 時の layout 交渉のみ対応
- `params`: state restore 後の value rescan は対応するが、parameter schema 自体の動的 rescan API は未提供
- raw MIDI bytes 向けの helper は未実装（note-ports 経由の note event のみ対応）
- output event の batching helper は最小限（sample accurate な event 整列は製品側の責務）
- `audio-ports-activation` extension は未実装
- 複数 plugin を 1 binary から export する構成は未対応
