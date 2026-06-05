# Production-Readiness Checks

> English version: [production-readiness-checks.md](production-readiness-checks.md)

`cargo xtask validate` は、指定されたプラグイン形式をビルドし、WRAC production-readiness checks を実行したあと、clap-validator、Steinberg VST3 validator、auval などの外部バリデーターを実行します。WRAC のチェックに違反がある場合はエラーとして扱い、コマンドは non-zero exit code を返します。

WRAC production-readiness checks は、商用プラグインのための NovoNotes 独自のリリースポリシーチェックです。プラグイン形式の仕様そのものを検証するバリデーターではありません。このチェックは小さく保ってください。実際に起きていない問題に対するチェックは追加しないでください。production-readiness check として妥当なのは、すでに観測されたリリース、QA、ホスト互換性、サポート上の実問題を防ぐものだけです。

コマンドは各チェックを `pass`、`fail`、`disabled`、`skipped` としてログ出力します。CI ログから、どのリリースポリシーチェックが評価されたかを確認できます。

## チェックの無効化

チェックはプラグイン crate の manifest で rule ID ごとに無効化できます。無効化するルールには、空ではない `reason` が必須です。

```toml
[package.metadata.wrac.validation.disabled_rules.fender-studio-pro-generic-editor-single-knob]
reason = "This product does not support Fender Studio Pro generic editor workflows."
```

未知の rule ID と空の reason はエラーです。

チェックを無効化するのは、意図的な製品判断がある場合だけにしてください。プラグインがそのチェックのリリースポリシーを満たすべき場合は、無効化ではなくプラグインを修正してください。

## チェックの追加

新しいチェックの追加は、単なるコード変更ではなくリリースポリシーの変更です。PR を作る前に、author は以下を完了してください。

- **妥当性:** そのチェックが、実際に起きた問題を扱っていることを確認する。仮説上のリスクに対するチェックは追加しない。
- **重複回避:** 他のバリデーターがすでに検出するチェックと重複させない。観測済みの問題が再現するにもかかわらず `cargo xtask validate` が通ってしまう場合だけ、新しいチェックを追加する。
- **ドキュメント:** このドキュメントの Check List に、期待される状態、理由、エラー条件、修正方法を追加する。
- **Unit Test:** `pass`、`fail`、`disabled`、`skipped`、エッジケースをテストする。
- **Manual Validate 必須:** unit test だけでは不十分です。必ず以下を実施する。
  - 実際のテンプレートプラグインを意図的に壊し、`cargo xtask validate` が期待した rule ID と message で fail することを確認する。
  - プラグインを元に戻し、コマンドがそのチェックを `pass`、`disabled`、または `skipped` としてログ出力することを確認する。

## Check List

### `fender-studio-pro-generic-editor-single-knob`

**期待される状態:** Fender Studio Pro の generic editor workflow をサポートする production plugin は、visible な non-bypass parameter を 0 個、または 2 個以上公開する。

**理由:** Fender Studio Pro 8.0.3 の generic editor は、このパラメーター構成では knob を表示できません。このルールでは bypass parameter は knob 数に含めません。

**エラー条件:** CLAP または VST3 validation が要求されたとき、プラグインが visible な non-bypass parameter をちょうど 1 個公開している。

**修正方法:** visible な non-bypass parameter を 0 個または 2 個以上にする。製品が Fender Studio Pro generic editor workflow を意図的にサポートしない場合は、reason を書いてルールを無効化する。

### `luna-vst3-param-id-must-match-index`

**期待される状態:** VST3-compatible plugin は、public parameter ID を parameter list の index と一致させる。

**理由:** LUNA 2.0.3.4381 では、VST3 parameter ID が parameter list index と異なる場合、VST3 automation write が失敗することがあります。

**エラー条件:** VST3 validation が要求されたとき、public parameter ID が parameter list index と異なる。

**修正方法:** パラメーターを並べ替えるか public parameter ID を調整し、各 public parameter ID が index と一致するようにする。

### `bypass-param-shape`

**期待される状態:** プラグインは bypass parameter を最大 1 個だけ公開し、そのパラメーターが boolean の host bypass control として振る舞う。

**理由:** host bypass UI、bypass automation、generic editor、control surface は、bypass が boolean shape のパラメーターとして 1 つ公開されているときに最も予測しやすく動作します。

**エラー条件:**

- bypass parameter が複数公開されている。
- bypass parameter が stepped enum ではない。
- bypass parameter の range が `0..1` ではない。
- bypass parameter の default が `0` または `1` ではない。

**修正方法:** bypass、stepped、enum flag を持ち、range `0..1`、default `0` または `1` の bypass parameter を 1 つ公開する。

### `plugin-requires-bypass`

**期待される状態:** production plugin は valid な bypass parameter を 1 つ公開する。

**理由:** host bypass UI、bypass automation、generic editor、control surface は、host-visible な bypass control がプラグインから提供されることを期待する場合があります。valid な bypass parameter は実装コストが低く、プラグインのカテゴリーを問わずホスト固有の互換性リスクを下げます。

**エラー条件:** プラグインが bypass parameter を公開していない。

**修正方法:** bypass parameter を 1 つ追加する。製品として host bypass を意図的に提供しない場合は、reason を書いてルールを無効化する。

### `template-placeholders-renamed`

**期待される状態:** テンプレート由来の仮の名前、ID、URL を製品固有の値に置き換える。

**理由:** これは、製品 metadata にテンプレートの識別情報が残るセットアップ失敗が実際に観測されたためのチェックです。仮の company name、plugin ID、plugin name、AU code、repository URL が残ると、ホストの scan cache、plugin menu、AU registration、log、support diagnostics に誤った製品識別情報が出ます。このルールはテンプレートリポジトリ自体では skipped されます。

**エラー条件:** manifest metadata に `Your Company`、`com.your-company`、`WRAC Gain`、`wrac_gain_plugin`、`WtGn`、テンプレートリポジトリ URL などの placeholder が残っている。

**修正方法:** テンプレート由来の metadata を製品固有の metadata に置き換える。テンプレートまたは example repository として意図的に残す場合は、reason を書いてルールを無効化する。
