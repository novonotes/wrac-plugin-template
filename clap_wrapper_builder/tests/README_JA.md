# ラッパーの回帰テスト

> English version: [README.md](README.md)

Windows または macOS で WRAC Gain のリリース版 VST3 または AUv2 をビルドし、
現在の OS のテストを実行します。

```sh
cargo xtask build --release --target=vst3 # Windows。macOS は --target=au
cargo xtask test-wrapper
```

`vst3_inactive_buses` は実際の VST3 変換処理を通し、非アクティブな補助バスの
`null`・省略、再有効化、不正なバッファー、処理エラーからの復帰を検証します。
32-bit／64-bit と in-place 処理を対象とし、音声処理中のメモリー確保も検出します。

テストの登録先は [CMakeLists.txt](CMakeLists.txt) です。各テストの目的と
一部の検証が SKIP される条件は、それぞれのソースに記載しています。
