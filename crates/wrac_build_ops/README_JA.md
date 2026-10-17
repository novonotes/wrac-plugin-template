# wrac_build_ops

> 英語版: [README.md](README.md)

`wrac_build_ops` は、repository-local な `cargo xtask` crate から使う WRAC の build /
install / launch / validation 操作を typed API として提供します。標準の WRAC package 選択
helper と、package ごとの task ID helper もこの crate から提供します。

各 repository の xtask は、workflow policy、task graph 構築、最終 artifact node の選択、task
実行 dispatch を引き続き所有します。この境界により、標準 package step の後に生成 asset を同梱する
ような製品固有 task を WRAC operation layer の外側に保てます。

標準の `wrac-plugin.toml` スキーマは
[`wrac_manifest`](../wrac_manifest/README_JA.md) に定義されています。

## 検証操作

このcrateはclap-validator / VST3 validator / auval / AAX Validatorなどの外部
フォーマット検証adapterをtyped operationとして公開します。validator設定とworkflow上の
実行順はrepository-localなxtaskが決定します。
