# WRAC Plugin Template

WRAC スタックによってオーディオプラグインを実装するためのテンプレートです。
コピーして新規プロジェクトの出発点として使うことが可能です。

> English version: [README.md](README.md)

# WRAC スタックとは

WRAC スタックとは、 **Webview, Rust Audio, CLAP** の三つを中心に構成される、オーディオプラグイン開発の技術スタックです。

**W** (WebView): HTML/CSS/JS を用いたユーザーインターフェースの実装。

**RA** (Rust Audio): Rust 言語による音声信号処理の実装。

**C** (CLAP): CLever Audio Plug-in 規格によるホストアプリケーションとのインターフェース。


## このレポジトリに含まれるもの

- [wxp](https://github.com/novonotes/wxp) を用いた WebView GUI 実装
- [clack](https://github.com/prokopyl/clack) を用いた Rust による CLAP プラグイン実装
- [clap-wrapper](https://github.com/free-audio/clap-wrapper) による VST3 や AU プラグインのビルド


## 新規プロジェクトのセットアップ

このレポジトリを元に、新しい wxp プラグインを作成する手順は [Setup](docs/setup.md) を参照してください。

## 参考

スレッドモデル・通信フロー・パラメータ変更フローの詳細は [docs/architecture.md](docs/architecture.md) を参照してください。

また、wxp クレートの使い方は [wxp の README](https://github.com/novonotes/wxp/tree/main/crates/wxp) に記載しています。
