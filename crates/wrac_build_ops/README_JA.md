# wrac_build_ops

> 英語版: [README.md](README.md)

`wrac_build_ops` は、各リポジトリの `cargo xtask` クレートから使う WRAC のビルド、
インストール、起動、検証操作を型付き API として提供します。WRAC パッケージの選択や、
パッケージごとのタスク ID の生成に使うヘルパーも提供します。

各リポジトリの xtask は、ワークフローの方針、タスクグラフの構築、最終成果物の選択、
タスクの実行を担当します。この分担により、標準のパッケージ作成後に生成済みアセットを
同梱するような製品固有のタスクを、WRAC の操作層から分離できます。

GUI パッケージがない場合の GUI ビルドや、設定が最新の CMake configure のように、
作業を省略できる操作は `TaskOutcome` を返します。これにより、各リポジトリの
ワークフローは具体的なスキップ理由をタスク結果として表示できます。

標準の `wrac-plugin.toml` スキーマは
[`wrac_manifest`](../wrac_manifest/README_JA.md) に定義されています。

## 検証操作

このクレートは、clap-validator、VST3 validator、auval、AAX Validator などの外部検証ツールを
型付きの操作として利用できるようにします。検証ツールの設定とワークフロー上の実行順は、
各リポジトリの xtask が決定します。
