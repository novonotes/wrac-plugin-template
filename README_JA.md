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

初期実装として WRAC Gain というシンプルなプラグインが実装されています。
テンプレートとしても使えるように配慮しています。

- [clap-sys](https://github.com/micahrj/clap-sys) を用いた Rust による CLAP プラグイン実装
- [wxp](https://github.com/novonotes/wxp) を用いた WebView GUI 実装
- [clap-wrapper](https://github.com/free-audio/clap-wrapper) による VST3 / AU / Standalone のビルド

## ビルド

代表的なコマンド:

```bash
# 全フォーマットのデバッグビルド
cargo xtask build
# 全フォーマットのリリースビルド
cargo xtask build --release
# VST3 のみデバッグビルド
cargo xtask build --target=vst3
# AU と スタンドアローンをリリースビルド
cargo xtask build --target=au,standalone --release
# ビルド済みプラグインを検証
cargo xtask validate
# ビルド済みプラグインをインストール
cargo xtask install
```

対応フォーマット:

| OS | 対応フォーマット | 
|----|---------------------------|
| macOS | CLAP / VST3 / AU / Standalone |
| Windows | CLAP / VST3 / Standalone |
| Linux | CLAP / VST3 / Standalone |

`--target` オプションには `clap`、`vst3`、`au`、`standalone` をカンマ区切りで指定できます。

詳しい使い方:

```bash
# 全体のヘルプ
cargo xtask --help
# サブコマンドのヘルプ
cargo xtask build --help
```

## 新規プロジェクトのセットアップ

このレポジトリを元に、新しい wxp プラグインを作成する手順は [Setup](docs/setup_JA.md) を参照してください。

## 動作報告を募集中！

このテンプレートは初期実装として Gain プラグインが実装されています。ぜひお手元のDAWでの動作状況を教えてください！
「Logic Pro 10.7 で動きました！」といった一言だけの報告でも、コミュニティにとっては貴重な情報になります。

こちらからお気軽にどうぞ：
👉 [DAW互換性報告](https://github.com/novonotes/wrac-plugin-template/discussions/6)

## 注意事項

汎用的なフレームワークではなく実装例を兼ねた出発点を意図しています。今後の変更に伴う、API の後方互換性やマイグレーションサポートは提供しません。

## 参考

主要 DAW での動作確認状況は [Wiki](https://github.com/novonotes/wrac-plugin-template/wiki/DAW-Compatibility-Matrix) を参照してください。

wxp クレートの使い方は [wxp の README](https://github.com/novonotes/wxp/tree/main/crates/wxp) に記載しています。
