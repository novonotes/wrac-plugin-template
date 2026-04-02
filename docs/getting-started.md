# Getting Started

`wxp-gain-example` をベースとして新しい wxp プラグインを作成する手順を説明します。

## 前提条件

### CLAP のみをビルドする場合

- Rust（最新の stable）
- Node.js（npm）

### VST3 / AU / Standalone もビルドする場合

clap-wrapper を用いて VST3 / AU / Standalone を生成するには、追加で以下が必要です。

**macOS:**
- Xcode または Xcode Command Line Tools
- CMake（3.15 以上推奨）

**Windows:**
- Visual Studio 2022（C++ ビルドツール付き）
- CMake（3.15 以上推奨）

**Linux:**
- 現在 CLAP のみのサポートです。

### デバッグ

VS Code のデバッグ設定を用意しています。
利用するには [CodeLLDB](https://marketplace.visualstudio.com/items?itemName=vadimcn.vscode-lldb) の拡張が必要です。

## 最初のプラグインを作成する

### 1. リポジトリのセットアップ

```sh
git clone https://github.com/novonotes/wxp-gain-example.git
```

### 2. テンプレートをコピー

リポジトリ全体をそのままコピーします。

```sh
cp -r wxp-gain-example /path/to/my_plugin
```

### 3. テンプレート識別子を一括置換

テンプレートには複数種類の識別子が散在しています。
IDE の機能や `rg`、LLM エージェントなどで全ファイルを検索、まとめて置換してください。

**置換テーブル:**

| 種別 | テンプレート値 | 置換先の例 |
|------|---------------|-----------|
| Rust crate 名 | `wxp_example_gain_plugin` | `my_plugin` |
| プラグイン表示名 | `WXP Example Gain` | `My Plugin` |
| プラグイン ID（逆ドメイン推奨） | `com.novo-notes.wxp-example-gain` | `com.your-company.my-plugin` |
| GUI / スクリプト内などの kebab-case 名 | `wxp-example-gain-plugin` | `my-plugin` |

> **重要:** プラグイン ID はグローバルに一意である必要があります。一度公開したら変更できません。

**手順:**

対象ファイルと残件数を確認します。

rg を用いる例:

```sh
rg "wxp_example_gain_plugin|WXP Example Gain|com\.novo-notes\.wxp-example-gain|wxp-example-gain-plugin" \
    --glob '!node_modules' --glob '!dist' --glob '!*.lock' --glob '!*.zip'
```

確認できたら、上の置換テーブルの通りに**全件置換**してください。
置換後に同じコマンドを再実行し、出力がゼロ件になれば完了です。

### 4. 依存バージョンの設定

`src-plugin/Cargo.toml` の `wxp` / `wxp_clack` / `wry` / `novonotes_run_loop` の `rev` を、
利用したいバージョンに合わせて更新してください。

```sh
cargo check --manifest-path src-plugin/Cargo.toml --all-targets
```

### 5. ビルド & インストール

```sh
cd /path/to/my_plugin
./script/build_and_install.sh
```

以下のディレクトリにビルド済みプラグインがインストールされます:

| OS | インストール先 |
|----|--------------|
| macOS | `~/Library/Audio/Plug-Ins/CLAP/` |
| Windows | `%LOCALAPPDATA%/Programs/Common/CLAP/` |
| Linux | `~/.clap/` |

VST3 / AU も同時にインストールされます。

### 6. 動作確認

デバッグビルドでは Vite dev server（`localhost:5173`）から GUI リソースを取得します。
以下のように起動してください。

```sh
cd /path/to/my_plugin/src-gui
npm install
npm run dev
```

DAW を起動して、プラグインをインサートしてみましょう。
DAW によってはプラグイン再スキャン等が必要な場合があります。
GUI はホットリロード可能です。HTML ファイルを編集してみましょう。

### 7. デバッグ

DAW はデバッガーのアタッチが難しいケースがあるので、まずはスタンドアローンアプリとしてデバッグすることをお勧めします。
VS Code で「Debug gain plugin standalone」構成を選択して実行します。

> **注意:** スタンドアローンモードでは音声フィードバックがあります。**ヘッドフォンを使用してください。**

## 次のステップ

ルートの [README.md](../README.md) を読んでみましょう。
スレッドモデル・通信フロー・パラメータ変更フローの詳細等を記載しています。
