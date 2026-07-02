# wrac_build_ops

> 英語版: [README.md](README.md)

`wrac_build_ops` は、repository-local な `cargo xtask` crate から使う WRAC の build /
install / launch / validation 操作を typed API として提供します。package 選択、workflow
policy、task graph 構築、task 実行 dispatch は各 repository の xtask が所有します。

標準の `wrac-plugin.toml` スキーマは
[`wrac_manifest`](../wrac_manifest/README_JA.md) に定義されています。

## 検証操作

この crate は WRAC production-readiness check、clap-validator / VST3 validator / auval /
AAX Validator などの外部フォーマット検証を typed operation として公開します。これらの操作を
どの順序で workflow に含めるかは repository-local な xtask が決定します。
