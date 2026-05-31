# WRAC Plugin Template

WRAC スタックによってオーディオプラグインを実装するためのテンプレートです。
コピーして新規プロジェクトの出発点として使うことが可能です。

> English version: [README.md](README.md)

# WRAC スタックとは

WRAC スタックとは、 **Webview, Rust Audio, CLAP** の三つを中心に構成される、オーディオプラグイン開発の技術スタックです。

**W** (WebView): HTML/CSS/JS を用いたユーザーインターフェースの実装。

**RA** (Rust Audio): Rust 言語による音声信号処理の実装。

**C** (CLAP): CLever Audio Plug-in 規格によるホストアプリケーションとのインターフェース。

## なぜ WRAC か

オーディオプラグインには、通常のデスクトップ WebView アプリにはない要件があります。多数の DAW やプラグインフォーマットへの対応、ホストアプリケーションとの協調的な動作、audio thread のハードリアルタイム要件などです。

私たちのチームが WebView + Rust の構成でこれらを満たすには試行錯誤が必要でした。
しかし、皆が同じ試行錯誤を繰り返す必要はありません。このテンプレートを使えば、NovoNotes が実運用で使っている「動くコード」から開発を始められます。

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
cargo xtask install --target=clap

# デバッグビルドは Vite dev server から GUI を読み込むため、DAW を起動する前に立ち上げてください
cd plugins/wrac-gain/src-gui
npm install
npm run dev
```

その後、DAW を起動して **WRAC Gain** を挿入してください（プラグインの再スキャンが必要な場合があります）。

動いたら [DAW互換性報告](https://github.com/novonotes/wrac-plugin-template/discussions/6) に一言いただけるとコミュニティにとって大変助かります！

このテンプレートを元に自分のプラグインを作る場合は [Setup](docs/setup_JA.md) を参照してください。

## FAQ

### なぜ GPU ネイティブな UI スタックではなく WebView なのか

実運用のプラグインでは、予測しやすさを重視しました。Web プラットフォームは成熟しており、デスクトップアプリやプラグイン UI の文脈でも利点と制約が比較的よく知られています。wgpu のような GPU ネイティブな UI スタックは有望ですが、DAW にホストされるプラグイン環境では、まだ実運用上の予測材料が少ないと考えています。

### これはフレームワークですか

いいえ。このリポジトリは汎用的なフレームワークではなく、実装例を兼ねた出発点です。そのため、包括的な高レベル API は提供せず、アダプタ層を意図的に薄く保っています。自分のプロジェクトに合わせて調整する負担は小さいはずです。同じ理由で、今後の変更に伴う API の後方互換性やマイグレーションサポートは提供しません。

### 商用プラグインに使えますか

はい。このリポジトリは MIT License で公開されており、商用利用が可能です。このテンプレートを元にしたオープンソース、フリーウェア、商用リリースのいずれも歓迎です。

### AAX / AUv3 対応はありますか

AAX / AUv3 対応は進行中です。`clap-wrapper` の `next` ブランチにはすでに AAX 対応が入っており、NovoNotes 社内では macOS AAX ビルドに利用しています。AUv3 対応も `clap-wrapper` に PR はありますが、NovoNotes ではまだ検証していません。このテンプレートの `xtask` target としては、現時点では CLAP / VST3 / AU / Standalone のみを公開しています。

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
# プラグインをビルドして検証
cargo xtask validate
# プラグインをビルドしてインストール
cargo xtask install
```

Standalone app をビルドして起動できます:

```bash
cargo xtask launch
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

## Built with WRAC

このテンプレートでプラグインを作ったら、ぜひ [Showcase Discussion](https://github.com/novonotes/wrac-plugin-template/discussions/43) で共有してください。
オープンソース、フリーウェア、商用リリースのいずれも歓迎です。

## 参考

主要 DAW での動作確認状況は [Wiki](https://github.com/novonotes/wrac-plugin-template/wiki/DAW-Compatibility-Matrix) を参照してください。

wxp クレートの使い方は [wxp の README](https://github.com/novonotes/wxp/tree/main/crates/wxp) に記載しています。

このテンプレートを元にした追加のプラグイン例は [wrac-examples](https://github.com/novonotes/wrac-examples) を参照してください。
