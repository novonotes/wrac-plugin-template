# ラッパーの回帰テスト

> English version: [README.md](README.md)

Windows または macOS で WRAC Gain のリリース版 VST3 または AUv2 をビルドし、
現在の OS のテストを実行します。

```sh
cargo xtask build --release --target=vst3 # Windows。macOS は --target=au
cargo xtask test-wrapper
```

テストの登録先は [CMakeLists.txt](CMakeLists.txt) です。各テストの目的と
一部の検証が SKIP される条件は、それぞれのソースに記載しています。
