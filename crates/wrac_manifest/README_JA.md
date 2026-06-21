# wrac_manifest

> 英語版: [README.md](README.md)

`wrac_manifest` は WRAC プラグインのマニフェスト (`wrac-plugin.toml`) を読み込み、
型付きの Rust メタデータへ変換するクレートです。WRAC のビルドスクリプトや xtask コマンドが使う、
標準マニフェストスキーマの一次定義をここに置きます。

## `wrac-plugin.toml`

WRAC プラグインパッケージは、リポジトリが所有する `wrac-plugin.toml` で記述します。
このマニフェストには、ホストから見える製品メタデータと検証時の例外設定を書きます。

### 標準フィールド

- `schema_version`: WRAC マニフェストスキーマのバージョンです。
- `[package]`: パッケージ単位のメタデータ上書きです。`version_source = "cargo"` を指定すると、
  `Cargo.toml` のバージョンをバンドルや各形式のディスクリプターのバージョンとして使います。
- `[bundle]`: バンドル内のすべてのプラグイン製品で共有するメタデータです。会社名、
  バンドル識別子、URL、説明文、著作権表記、`supported_formats` などを含みます。
- `[[plugins]]`: バンドルから公開する、ホストに見える個別のプラグイン製品です。各項目で ID、名前、
  CLAP の feature、ラッパー形式ごとのディスクリプター、必要に応じて AAX メタデータを定義します。
- `[validation]`: production-readiness ルールの無効化や外部バリデーターのスキップ条件など、
  検証時の例外設定を定義します。

### リポジトリ固有の拡張

WRAC は `wrac-plugin.toml` の未知のフィールドやテーブルを無視します。利用側のリポジトリは、
名前空間付きのテーブルに独自メタデータを置き、リポジトリ側の自動化処理から読み取れます。

```toml
[acme.ci]
validation_profile = "prototype"
```

WRAC は拡張テーブルの意味を解釈しません。リポジトリ側の自動化処理で独自ポリシーを読み取り、
明示的な WRAC コマンドラインオプションへ変換してください。
