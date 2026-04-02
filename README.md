# wxp-gain-example

[wxp](https://github.com/novonotes/wxp) を使った gain plugin の参照実装です。
コピーして新規プロジェクトの出発点として使うこともできます。

## 含まれるもの

| パス | 内容 |
|-----|------|
| `src-plugin` | Rust 製の CLAP プラグイン本体 |
| `src-gui` | TypeScript + HTML/CSS で書かれた GUI |
| `script` | ビルド・インストール用スクリプト |
| `clap_wrapper_builder` | CLAP を VST3 / AUv2 / Standalone にラップする補助ビルド環境 |

## 使い方

このレポジトリを元に、新しい wxp プラグインを作成する手順は [Setup](docs/setup.md) を参照してください。

## アーキテクチャ

スレッドモデル・通信フロー・パラメータ変更フローの詳細は [docs/architecture.md](docs/architecture.md) を参照してください。

また、wxp クレートの使い方は [wxp の README](https://github.com/novonotes/wxp/tree/main/crates/wxp) に記載しています。