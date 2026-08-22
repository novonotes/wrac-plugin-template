# wrac_manifest

> 英語版: [README.md](README.md)

`wrac_manifest` は WRAC プラグインのマニフェスト (`wrac-plugin.toml`) を読み込み、
型付きの Rust メタデータへ変換するクレートです。この README は、WRAC が所有する
マニフェストフィールドの一次スキーマ定義です。

## 対象範囲

`wrac-plugin.toml` には、ホストから見えるプラグイン識別情報、ラッパー用メタデータ、
既定のビルド対象を記述します。

WRAC は未知のフィールドやテーブルを無視します。利用者は、名前空間付きの
拡張テーブルを追加できます。

```toml
[acme.ci]
validation_profile = "prototype"
```

## スキーマ

### ルートテーブル

| フィールド | 型 | 必須 | 受け付ける値 | 意味 |
| --- | --- | --- | --- | --- |
| `schema_version` | integer | はい | `1` | WRACマニフェストスキーマのバージョンです。未知の値や省略は拒否します。 |

### `[package]`

`[package]` は任意です。`wrac_build_ops` が Cargo パッケージからマニフェストを読む場合、
`name`、`version`、`repository` が省略されていれば、パッケージの `Cargo.toml` から補完します。

| フィールド | 型 | 必須 | 受け付ける値 | 意味 |
| --- | --- | --- | --- | --- |
| `name` | string | いいえ | 指定する場合は空文字不可 | WRAC の成果物メタデータで使う Cargo パッケージ名を上書きします。通常は省略し、`Cargo.toml` を使います。 |
| `version` | string | いいえ | 指定する場合は空文字不可 | 生成されるプラグインディスクリプターやバンドルに使うバージョンを上書きします。通常は省略し、`Cargo.toml` を使います。 |
| `repository` | string | いいえ | 指定する場合は空文字不可 | Cargo パッケージの repository 情報を上書きします。 |
| `version_source` | string | いいえ | 慣例として `cargo` | 互換性と意図表示のために受け付けているフィールドです。WRAC は現在、省略された `version` を Cargo メタデータから補完しますが、他のバージョン取得元は扱いません。 |

### `[bundle]`

`[bundle]` は必須です。同じバイナリバンドルから公開されるすべてのプラグイン製品で共有する
メタデータを定義します。

| フィールド | 型 | 必須 | 受け付ける値 | 意味 |
| --- | --- | --- | --- | --- |
| `company_name` | string | はい | 空文字不可 | ホスト向けのベンダー/メーカー名です。CLAP ディスクリプターとラッパー用メタデータに生成されます。ホストのブラウザーやスキャナーでユーザーに見える可能性があります。 |
| `auv2_manufacturer_code` | string | はい | 4 バイトの ASCII | AUv2 manufacturer code です。AUv2 ラッパーディスクリプターに生成されます。リリース後は安定させてください。 |
| `aax_manufacturer_id` | string | 条件付き | 4 バイトの ASCII | `supported_formats` に `aax` を含める場合は必須です。それ以外では使われません。リリース後は安定させてください。 |
| `bundle_name` | string | はい | 空文字不可 | 製品バンドル名です。`.clap`、`.vst3`、`.component`、`.aaxplugin` などの成果物名と、macOS バンドルの表示名に使われます。 |
| `bundle_identifier` | string | はい | 空文字不可。reverse-DNS 形式を推奨 | macOS CLAP バンドルの `CFBundleIdentifier` です。個別プラグイン ID ではなく、バンドルの識別子です。リリース後は安定させてください。 |
| `homepage_url` | string | はい | 空文字不可の URL 文字列 | CLAP ディスクリプターの `url` に生成されます。ホストやスキャナーがユーザーに表示する可能性があります。 |
| `manual_url` | string | はい | 空文字不可の URL 文字列 | CLAP ディスクリプターの `manual_url` に生成されます。ホストやスキャナーがユーザーに表示する可能性があります。 |
| `support_url` | string | はい | 空文字不可の URL 文字列 | CLAP ディスクリプターの `support_url` に生成されます。ホストやスキャナーがユーザーに表示する可能性があります。 |
| `description` | string | はい | 空文字不可 | CLAP ディスクリプターの `description` に生成されます。ホストやスキャナーの UI に表示され得る、ユーザー向けの短い製品説明です。 |
| `copyright` | string | はい | 空文字不可 | macOS バンドルの著作権メタデータに生成されます。 |
| `supported_formats` | array of `PluginFormat` | はい | `clap`、`vst3`、`au`、`aax`。空配列と重複は不可 | 既定の `cargo xtask build`、`install`、`validate` のプラグイン対象を決める製品方針です。`--target` で明示したプラグイン形式も、この配列に含まれている必要があります。 |

### `[[plugins]]`

`[[plugins]]` は 1 件以上必要です。各項目は、バンドルから公開されるホスト向けのプラグイン製品を
1 つ表します。1 つのバンドルから複数のプラグイン製品を公開できます。

| フィールド | 型 | 必須 | 受け付ける値 | 意味 |
| --- | --- | --- | --- | --- |
| `plugin_id` | string | はい | 空文字不可。マニフェスト内で一意 | CLAP plugin ID です。ホストはこれを製品識別子として扱います。リリース後の変更は、保存済みセッションとの互換性を壊します。 |
| `plugin_name` | string | はい | 空文字不可 | ホスト向けのプラグイン名です。CLAP ディスクリプターに生成され、ホストのブラウザーやスキャナーに表示されます。 |
| `clap_features` | array of `ClapFeature` | はい | 事前定義された CLAP feature 文字列を 1 個以上。[CLAP feature の値](#clap-feature-の値) を参照 | CLAP ディスクリプターに生成されます。CLAP ホストが直接読むため、実際の音声/MIDI 挙動と一致させてください。 |
| `vst3_subcategories` | string | はい | 空文字不可。例: `Fx&#124;Dynamics` のように、Steinberg 形式の縦棒区切り文字列 | VST3 ラッパーディスクリプターに生成されます。WRAC は空文字でないことだけを検証します。VST3 ホストが受け付ける値を指定してください。 |
| `vst3_component_id` | string | はい | UUID 文字列 | VST3 のコンポーネント識別子です。WRAC は clap-wrapper が期待するバイト順に変換します。リリース前に一度生成し、同じ製品では安定させてください。 |
| `standalone_name` | string | はい | 空文字不可。マニフェスト内で一意 | スタンドアロンアプリの成果物名です。`.app`、`.exe`、Linux のスタンドアロンファイル名に使われます。 |
| `standalone_audio_input` | boolean | いいえ | `true`または`false`。既定値は`true` | 開発用スタンドアロンアプリが物理音声入力デバイスを開けるかを指定します。DAWホストへ公開するプラグインのバス構成には影響しません。 |
| `auv2_type` | string | はい | 4 バイトの ASCII | AUv2 component type です。例として `aufx` や `aumu` を指定します。AUv2 ラッパーディスクリプターに生成されます。 |
| `auv2_subtype` | string | はい | 4 バイトの ASCII。`(auv2_type, auv2_subtype)` の組はマニフェスト内で一意 | AUv2 component subtype です。ホストが識別子として使うため、リリース後は安定させてください。 |
| `aax_categories` | array of `AaxCategory` | 条件付き | 事前定義された AAX category 文字列を 1 個以上。[AAX category の値](#aax-category-の値) を参照 | `supported_formats` に `aax` を含める場合は必須です。それ以外では使われません。AAX ラッパーディスクリプターに生成されます。 |
| `aax_product_id` | string | 条件付き | 4 バイトの ASCII | `supported_formats` に `aax` を含める場合は必須です。それ以外では使われません。AAX の製品識別子なので、リリース後は安定させてください。 |
| `aax_stem_configs` | array of tables | 条件付き | `supported_formats` に `aax` を含める場合は空配列不可 | `supported_formats` に `aax` を含める場合は必須です。それ以外では使われません。AAX の入出力 stem 構成を定義します。 |

### `[[plugins.aax_stem_configs]]`

| フィールド | 型 | 必須 | 受け付ける値 | 意味 |
| --- | --- | --- | --- | --- |
| `name` | string | はい | 空文字不可 | AAX ステム構成の表示名です。 |
| `input` | `AaxStemFormat` | はい | `mono`、`stereo` | AAX の入力ステム形式です。 |
| `output` | `AaxStemFormat` | はい | `mono`、`stereo` | AAX の出力ステム形式です。 |
| `plugin_id` | string | はい | 4 バイトの ASCII。親プラグインのステム構成内で一意 | AAX ステムごとの plugin ID です。リリース後は安定させてください。 |

## 事前定義値

### `PluginFormat` の値

- `clap`
- `vst3`
- `au`
- `aax`

### CLAP feature の値

- `audio-effect`
- `analyzer`
- `ambisonic`
- `chorus`
- `compressor`
- `de-esser`
- `delay`
- `instrument`
- `note-effect`
- `note-detector`
- `drum`
- `drum-machine`
- `equalizer`
- `expander`
- `filter`
- `flanger`
- `frequency-shifter`
- `gate`
- `glitch`
- `granular`
- `distortion`
- `limiter`
- `mastering`
- `mixing`
- `mono`
- `multi-effects`
- `phaser`
- `phase-vocoder`
- `pitch-correction`
- `pitch-shifter`
- `restoration`
- `reverb`
- `sampler`
- `stereo`
- `surround`
- `synthesizer`
- `transient-shaper`
- `tremolo`
- `utility`

### AAX category の値

- `eq`
- `dynamics`
- `pitch-shift`
- `reverb`
- `delay`
- `modulation`
- `harmonic`
- `noise-reduction`
- `dither`
- `sound-field`
- `hardware-generator`
- `software-generator`
- `wrapped-plugin`
- `effect`
- `midi-effect`

### AAX stem format の値

- `mono`
- `stereo`
