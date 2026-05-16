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

## クイックスタート

自分のプラグインを作る前に、同梱の WRAC Gain プラグインをまず動かしてみたい方は、以下の最小手順をお試しください。
最低限 Rust と Node.js があれば、CLAP はビルドできるはずです。
VST3 / AU / Standalone をビルドする場合の前提条件は [Setup ドキュメント](docs/setup_JA.md#前提条件) を参照してください。

```sh
# サブモジュール込みで clone（サブモジュールは VST3 / AU / Standalone のときのみ必要）
git clone --recursive https://github.com/novonotes/wrac-plugin-template.git
cd wrac-plugin-template

# プラグインをビルドしてインストール
# AU や VST3 が必要な場合は、target 引数を変更してください。
cargo xtask build --target=clap
cargo xtask install --target=clap

# デバッグビルドは Vite dev server から GUI を読み込むため、DAW を起動する前に立ち上げてください
cd src-gui
npm install
npm run dev
```

その後、DAW を起動して **WRAC Gain** を挿入してください（プラグインの再スキャンが必要な場合があります）。

動いたら [DAW互換性報告](https://github.com/novonotes/wrac-plugin-template/discussions/6) に一言いただけるとコミュニティにとって大変助かります！

このテンプレートを元に自分のプラグインを作る場合は [Setup](docs/setup_JA.md) を参照してください。

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

| OS | サポートフォーマット  |
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

## 注意事項

汎用的なフレームワークではなく実装例を兼ねた出発点を意図しています。今後の変更に伴う、API の後方互換性やマイグレーションサポートは提供しません。

## 参考

主要 DAW での動作確認状況は [Wiki](https://github.com/novonotes/wrac-plugin-template/wiki/DAW-Compatibility-Matrix) を参照してください。

wxp クレートの使い方は [wxp の README](https://github.com/novonotes/wxp/tree/main/crates/wxp) に記載しています。
