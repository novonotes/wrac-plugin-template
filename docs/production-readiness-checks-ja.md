# Production-Readiness Checks

> English version: [production-readiness-checks.md](production-readiness-checks.md)

Production-readiness check は、`cargo xtask validate` の中で実行される WRAC 独自のチェックです。違反がある場合はエラーとして扱い、コマンドは non-zero exit code を返します。

このチェックは、商用プラグインのための NovoNotes 独自のリリースポリシーチェックです。プラグイン形式の仕様そのものを検証するバリデーターではありません。ルールリストは小さく保つ方針です。実際に観測されたホスト互換性、サポート上の実問題を防ぐルールだけ、さらにそのなかでも、他の方法で再発防止が難しい場合だけ新規ルール追加します。

## ルールの無効化

ルールはプラグイン crate の manifest で rule ID ごとに無効化できます。無効化する場合は、空ではない `reason` が必須です。

```toml
[package.metadata.wrac.validation.disabled_rules.fender-studio-pro-generic-editor-single-knob]
reason = "This product does not support Fender Studio Pro generic editor workflows."
```

## ルールの追加

ルールを追加する場合、以下を行ってください。

- **妥当性:** そのルールが、実際に起きた問題を扱っていることを確認する。仮説上のリスクに対するルールは追加しないでください。
- **重複回避:** 他のバリデーターがすでに検出する問題を重複検出しない。観測済みの問題が再現する状態にもかかわらず `cargo xtask validate` が通ってしまう場合だけ、新しいルールを追加してください。
- **ドキュメント:** このドキュメントの Check List に、期待される状態、理由、エラー条件、修正方法を追加してください。
- **手動確認必須:** unit test だけでは不十分です。必ず以下を実施してください。
  - 実際のテンプレートプラグインを意図的に壊し、`cargo xtask validate` が期待した rule ID と message で fail することを確認する。
  - ルールを無効化すれば通ることを確認する。通らない場合は他のバリデーターで重複検出されている可能性が高いのでルール追加しないでください。
  - プラグインを元に戻し、ルールを有効化。チェックが通ることを確認する。

## ルール一覧

### `fender-studio-pro-generic-editor-single-knob`

**期待される状態:** Fender Studio Pro の generic editor をサポートする製品は、bypass 以外の parameter を 0 個、または 2 個以上公開するべきです。

**理由:** Fender Studio Pro 8.0.3 の generic editor は、ノブが一つだけになるパラメーター構成では、一個もノブを表示しません。ユーザーがバグと感じるので、避けるべきです。bypass parameter は元々ノブ表示されないため、カウントに含めません。

**エラー条件:** CLAP または VST3 validation が要求されたとき、プラグインが bypass 以外の parameter をちょうど 1 個公開している。

**修正方法:** bypass 以外の parameter を 0 個または 2 個以上にしてください。

### `luna-vst3-param-id-must-match-index`

**期待される状態:** LUNA の VST3 をサポートする場合は、parameter ID を parameter list の index と一致させるべきです。

**理由:** LUNA 2.0.3.4381 では、VST3 parameter ID が parameter list index と異なる場合、VST3 automation write が失敗することがあります。

**エラー条件:** VST3 ターゲットを含む validation が要求されたとき、public parameter ID が parameter list index と異なる。

**修正方法:** リリース前の場合、パラメーターを並べ替えるか parameter ID を調整し、ID と index と一致するようにしてください。リリース後の場合は、parameter ID を変更せず、このルールの無効化を推奨します。

### `bypass-param-shape`

**期待される状態:** プラグインは bypass parameter を最大 1 個だけ公開し、そのパラメーターが boolean の host bypass control として振る舞うべきです。

**理由:** ホストアプリケーションは、bypass パラメーター専用の UI を持つことがあります。この UI がユーザーの期待通り動作するためには、このルールを満たす parameter shape にすべきです。

**エラー条件:**

- bypass parameter が複数公開されている。
- bypass parameter が stepped enum ではない。
- bypass parameter の range が `0..1` ではない。
- bypass parameter の default が `0` または `1` ではない。

**修正方法:** bypass、stepped、enum flag を持ち、range `0..1`、default `0` または `1` の bypass parameter を 1 つ公開してください。

### `plugin-requires-bypass`

**期待される状態:** bypass parameter を 1 つ公開するべきです。

**理由:** bypass parameter は実装コストが低く、プラグインの種類を問わずホスト固有の互換性リスクを下げます。

**エラー条件:** プラグインが bypass parameter を公開していない。

**修正方法:** bypass parameter を 1 つ追加してください。製品として bypass を意図的に提供しない場合は、reason を書いてルールを無効化してください。

### `template-placeholders-renamed`

**期待される状態:** テンプレート由来の仮の名前、ID、URL は、製品固有の値に置き換えるべきです。

**理由:** テンプレート由来の値は、製品作成時に手作業で置き換える必要があるため、見落としが起きやすいです。

**エラー条件:** manifest metadata に `Your Company`、`YrCo`、`com.your-company`、`example.com`、`WRAC Gain`、`wrac_gain_plugin`、`WtGn`、`WtGM`、`WtGS`、template VST3 component UUID、テンプレートリポジトリ URL などの placeholder が残っている。このルールはテンプレートリポジトリ自体では skipped されます。

**修正方法:** テンプレート由来の metadata を製品固有の metadata に置き換えてください。テンプレートまたは example repository として意図的に残す場合は、reason を書いてルールを無効化してください。
