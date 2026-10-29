# wrac_clap_adapter

> English version: [README.md](README.md)

`wrac_clap_adapter::interface` が定義する製品向けの契約を CLAP ABI に対応付けます。

VST3 / AU / AAX への変換は `clap-wrapper` の役割です。この crate は Rust 側での CLAP プラグインと CLAP 拡張の実装に専念します。

## 目的

clap-wrapper を通じて VST3/AU/AAX ホストで CLAP プラグインを使う場合、CLAP のスレッドモデルや呼び出し順序の保証が守られないことがあります。この crate はそうした状況に対して防御的に動作することを目指します。

## clack との違い

CLAP ヘッダーは各関数で許されるスレッドを `[main-thread]`、`[audio-thread]`、`[thread-safe]` などのコメントで示します。例えば `init` は `[main-thread]`、`process` は `[audio-thread]`、`get_extension` は `[thread-safe]` です。

`clack` はホストがこれらの注釈に従って関数を呼び出すことを前提として設計されており、ネイティブ CLAP ホストで素直に動作します。

一方、この crate は `clap-wrapper` 経由の VST3/AU/AAX ホストも対象とします。これらのホストを通すと、`[main-thread]` と注釈された問い合わせが別のスレッドから呼ばれるなど、仕様との差異が生じることがあります。この crate は adapter 側の lock や panic の捕捉で対応し、製品コードに `unsafe` を公開せずに正しく動作します。

## 謝辞

`wrac_clap_adapter` は安全で低レベルな CLAP ラッパーとしての `clack` の設計、特に CLAP 拡張の境界と音声バッファーへのアクセス方法から着想を得ています。この crate は `clack` のコードから派生したものではなく、`clap-sys` を直接使った独立した実装です。

## 公開 API

`export_clap_entry!` は CLAP エントリーポイントを公開します。製品実装は `wrac_clap_adapter::interface` の trait と関連型を使用します。残りのモジュールは ABI callback、登録データの保持、具体的な host proxy 実装を担います。

## 内部の境界

- `interface`: 製品に公開する契約全体
- `abi`: 製品実装を呼び出す CLAP callback
- `host_proxy`: 製品が CLAP host callback を呼ぶための非公開の具体的な proxy

## インスタンスのライフサイクル

`clap.plugin-factory.create_plugin` は CLAP ABI の外殻だけを作成します。製品インスタンスの構築は `plugin.init` まで延期され、そこで CLAP と clap-wrapper がプラグイン向けのライフサイクルを初期化します。ポート、パラメーター、状態、GUI、レイテンシー、tail などの capability オブジェクトは `plugin.init` 中に確定され、後続の host callback は製品のライフサイクル lock を取得せずに応答できます。

capability の確定前に到着した呼び出しは初期化を待たず、CLAP callback の形式に応じて即座に失敗、`null` / `false` / `0` の返却、または何もしない処理となります。製品が保持する host extension proxy も capability 確定まで無効に保たれ、初期化中の host callback が中途半端な capability を観測しないようにします。破棄 callback は例外で、`deactivate` と `destroy` は処理中のアクセスが完了するのを待ち、ホストがインスタンスを解放する前に adapter が processor を回収できるようにします。

## 制限

この crate は汎用フレームワークではなく、実装例の一部として提供されています。今後の変更では API の後方互換性や移行サポートは提供しません。

CLAP ABI 全体の実装もまだ完了していません。既知の制限:

- `configurable-audio-ports`: 非アクティブ時のレイアウト交渉のみ対応
- host callback proxy は薄いラッパーであり、呼び出しを別スレッドへ移しません
- 出力イベントのバッチ処理 helper は最小限です（サンプル精度のイベント順序は製品側の責任です）
- `audio-ports-activation` 拡張は未実装です
- plugin factory と AUv2 wrapper info 以外の型付き factory は未実装です
