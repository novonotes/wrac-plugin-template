# wrac_xtask

> 英語版: [README.md](README.md)

`wrac_xtask` は、WRAC プラグイン成果物のビルド、インストール、起動、クリーンアップ、検証に使う
共通の `cargo xtask` コマンド群を提供します。

標準の `wrac-plugin.toml` スキーマは
[`wrac_manifest`](../wrac_manifest/README_JA.md) に定義されています。

## 検証の段階

`cargo xtask validate` は通常、選択された成果物をビルドし、WRAC の製品出荷前チェックと
外部フォーマット検証ツールを実行します。

- `--skip-readiness-checks`: WRAC の製品出荷前チェックをスキップします。
- `--skip-external-validators`: clap-validator、VST3 validator、auval、AAX Validator などの
  外部フォーマット検証ツールをスキップします。

両方のフラグを指定した場合、選択された対象のビルドとパッケージングだけを確認する簡易チェックとして
動作します。
