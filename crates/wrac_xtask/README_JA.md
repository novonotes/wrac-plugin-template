# wrac_xtask

> 英語版: [README.md](README.md)

`wrac_xtask` は、repository-local な `cargo xtask` crate から使う WRAC task primitive と
共通実行 helper を提供します。package 選択、policy 判断、task graph 構築は各 repository の
xtask が所有します。

標準の `wrac-plugin.toml` スキーマは
[`wrac_manifest`](../wrac_manifest/README_JA.md) に定義されています。

## 検証 task

この crate は成果物ビルド、WRAC production-readiness check、clap-validator / VST3 validator /
auval / AAX Validator などの外部フォーマット検証を task として公開します。検証 plan にどの
task を含めるかは repository-local な xtask が決定します。
