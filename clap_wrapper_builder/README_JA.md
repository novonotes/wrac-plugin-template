# clap_wrapper_builder

> English version: [README.md](README.md)

ビルド済みの CLAP プラグインの**静的ライブラリ**を VST3 / AUv2 / AAX / Standalone バンドルに変換する、再利用可能な CMake プロジェクトです。

## 構成

- `CMakeLists.txt` - ラッパーのビルド定義。入力変数の一覧はファイル先頭のコメントを参照してください。
- `clap-wrapper` - git subtree として管理する同梱ラッパーのソース。
- `clap` / `vst3sdk` / `AudioUnitSDK` - git submodule として取得する依存 SDK。

## 使い方

少なくとも変換する静的ライブラリを指定して、このプロジェクトを直接呼び出します。

```bash
cmake -S clap_wrapper_builder -B build \
  -DCLAP_WRAPPER_BUILDER_TARGET_LIB=/path/to/libyour_plugin.a \
  -DCLAP_WRAPPER_BUILDER_OUTPUT_NAME="Your Plugin"
cmake --build build
```

出力名、形式の切り替え（VST3/AUv2/AAX/Standalone）、成果物の配置については、`CMakeLists.txt` の `CLAP_WRAPPER_BUILDER_*` 変数一覧を参照してください。

> このテンプレートのプラグインをビルドする場合は、リポジトリのルートで `cargo xtask build` を実行してください。ルートの `README_JA.md` を参照してください。

## AAX

AAX は既定で無効で、Avid AAX SDK が必要です。SDK を用意した後に `-DCLAP_WRAPPER_BUILDER_BUILD_AAX=ON` を指定してください。

## clap-wrapper subtree の保守

`clap-wrapper` は通常のファイルとして同梱されており、アクティブな submodule ではありません。履歴には git-subtree のメタデータが残っています。subtree は upstream のリリースタグを追跡します。`next` には未リリースの機能が含まれ、更新の変更範囲が広がるためです。プロジェクト固有の追加変更は、後続の小さな論理単位のコミットに分けます。次のリリースを待たずに必要な upstream の修正は、次回のタグ更新で判別できるよう `-x` を付けて個別に cherry-pick します。

更新コマンドの例:

```bash
git subtree pull \
  --prefix=clap_wrapper_builder/clap-wrapper \
  https://github.com/free-audio/clap-wrapper.git \
  v0.17.0 \
  -m "Update clap-wrapper subtree to v0.17.0"
```

ローカルパッチが更新と広範囲に衝突する場合、マージコミットでは subtree の内容を upstream タグそのままにし、各パッチを元のコミットを明記した個別コミットとして再適用します。

upstream の単一コミットを取り込む例:

```bash
git cherry-pick -x -Xsubtree=clap_wrapper_builder/clap-wrapper <upstream-commit>
```
