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
- [clap-wrapper](https://github.com/free-audio/clap-wrapper) による VST3 / AU / AAX プラグインのビルドと開発用 standalone app

## クイックスタート

自分のプラグインを作る前に、同梱の WRAC Gain プラグインをまず動かしてみたい方は、以下の最小手順をお試しください。
最低限 Rust と Node.js があれば、CLAP はビルドできるはずです。
VST3 / AU / AAX、または開発用 standalone app をビルドする場合の前提条件は [Setup ドキュメント](docs/setup-ja.md#前提条件) を参照してください。

```sh
# テンプレートを clone（VST3 / AU / AAX、または開発用 standalone app をビルドする場合は --recursive を追加してください）
git clone https://github.com/novonotes/wrac-plugin-template.git
cd wrac-plugin-template

# 最小確認として CLAP だけをビルドしてインストール
# supported_formats に書かれた全 format をインストールする場合は `cargo xtask install`
cargo xtask install --target=clap

# デバッグビルドは Vite dev server から GUI を読み込むため、DAW を起動する前に立ち上げてください
cd plugins/wrac-gain/src-gui
npm install
npm run dev
```

その後、DAW を起動して **WRAC Gain** を挿入してください（プラグインの再スキャンが必要な場合があります）。

このテンプレートを元に自分のプラグインを作る場合は [Setup](docs/setup-ja.md) を参照してください。

## FAQ

### なぜ GPU ネイティブな UI スタックではなく WebView なのか

実運用のプラグインでは、予測しやすさを重視しました。Web プラットフォームは成熟しており、デスクトップアプリやプラグイン UI の文脈でも利点と制約が比較的よく知られています。wgpu のような GPU ネイティブな UI スタックは有望ですが、DAW にホストされるプラグイン環境では、まだ実運用上の予測材料が少ないと考えています。

### これはフレームワークですか

いいえ。このリポジトリは汎用的なフレームワークではなく、実装例を兼ねた出発点です。そのため、包括的な高レベル API は提供せず、アダプタ層を意図的に薄く保っています。自分のプロジェクトに合わせて調整する負担は小さいはずです。同じ理由で、今後の変更に伴う API の後方互換性やマイグレーションサポートは提供しません。

### 商用プラグインに使えますか

はい。このリポジトリは MIT License で公開されており、商用利用が可能です。このテンプレートを元にしたオープンソース、フリーウェア、商用リリースのいずれも歓迎です。

### AAX / AUv3 対応はありますか

AAX は macOS / Windows で明示的な build / install / validate target として対応しています。詳細は [AAX Build and Validation](docs/aax-ja.md) を参照してください。
AUv3 対応はまだ進行中です。

## ビルド

代表的なコマンド:

```bash
# 全プラグインフォーマットと開発用 standalone app のデバッグビルド
cargo xtask build
# 全プラグインフォーマットと開発用 standalone app のリリースビルド
cargo xtask build --release
# VST3 のみデバッグビルド
cargo xtask build --target=vst3
# AU をリリースビルド
cargo xtask build --target=au --release
# プラグインをビルドして検証
cargo xtask validate
# プラグインをビルドしてインストール
cargo xtask install
```

`cargo xtask validate` は外部フォーマット validator の前に WRAC の production-readiness check を実行します。
check 一覧と disable 形式は [Production-Readiness Checks](docs/production-readiness-checks.md) を参照してください。

開発用 standalone app をビルドして起動できます:

```bash
cargo xtask launch
```

Standalone app は軽量な開発・smoke test 用 host です。リリース用のプラグインフォーマットや出荷 artifact ではありません。

対応プラグインフォーマット:

| OS | サポートフォーマット  |
|----|---------------------------|
| macOS | CLAP / VST3 / AU / AAX |
| Windows | CLAP / VST3 / AAX |
| Linux | CLAP / VST3 |

既定の build / install / validate target は `wrac-plugin.toml` の `supported_formats` から決まります。
`--target` を使うと特定の subset だけを指定できます。明示した plugin format target は `supported_formats` に含まれている必要があります。
`cargo xtask build` は既定で開発用 standalone app もビルドします。build コマンドでは、開発専用 target として `standalone` も指定できます。
build / install / validate では `--dry-run` を使って、実行前に task graph を確認できます。

詳しい使い方:

```bash
# 全体のヘルプ
cargo xtask --help
# サブコマンドのヘルプ
cargo xtask build --help
```

## 参考

主要 DAW での動作確認状況は [Wiki](https://github.com/novonotes/wrac-plugin-template/wiki/DAW-Compatibility-Matrix) を参照してください。

wxp クレートの使い方は [wxp の README](https://github.com/novonotes/wxp/tree/main/crates/wxp) に記載しています。

このテンプレートを元にした追加のプラグイン例は [wrac-examples](https://github.com/novonotes/wrac-examples) を参照してください。
