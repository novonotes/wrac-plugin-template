# Setup

`wxp-gain-example` を出発点として新しい wxp プラグインを作成する手順を説明します。

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

GitHub の [wxp-gain-example](https://github.com/novonotes/wxp-gain-example) ページ右上の `Use this template` ボタンを使って新しいリポジトリを作成します。
作成後、新しいリポジトリをクローンしてサブモジュールを初期化してください。

```sh
git clone https://github.com/your-org/my-plugin.git
cd my_plugin
git submodule update --init --recursive
```

### 2. 識別子を一括置換

このリポジトリには複数種類の識別子が散在しています。
IDE の機能や `rg`、LLM エージェントなどで全ファイルを検索し、まとめて置換してください。

**置換テーブル:**

| 種別 | 現在の値 | 置換先の例 |
|------|---------|-----------|
| Rust crate 名 | `wxp_example_gain_plugin` | `my_plugin` |
| プラグイン表示名 | `WXP Example Gain` | `My Plugin` |
| プラグイン ID（逆ドメイン推奨） | `com.novo-notes.wxp-example-gain` | `com.your-company.my-plugin` |
| GUI / スクリプト内などの kebab-case 名 | `wxp-example-gain-plugin` | `my-plugin` |

> **重要:** プラグイン ID はグローバルに一意である必要があります。一度公開したら変更できません。

**手順:**

対象ファイルと残件数を確認します。

rg を用いる例:

```sh
rg --hidden "wxp_example_gain_plugin|WXP Example Gain|com\.novo-notes\.wxp-example-gain|wxp-example-gain-plugin" \
    --glob '!node_modules' --glob '!dist' --glob '!*.lock' \
    --glob '!package-lock.json' --glob '!*.zip' --glob '!docs/setup.md'
```

確認できたら、上の置換テーブルの通りに**全件置換**してください。
置換後に同じコマンドを再実行し、出力がゼロ件になれば完了です。

### 3. ビルド & インストール

リポジトリルートで以下を実行します。

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

### 4. 動作確認

デバッグビルドでは GUI リソースを Vite dev server（`localhost:5173`）から取得します。
DAW でプラグインを起動する前に、以下のコマンドで dev server を立ち上げておいてください。

```sh
cd /path/to/my_plugin/src-gui
npm install
npm run dev
```

DAW を起動して、プラグインをインサートしてみましょう。
DAW によってはプラグイン再スキャン等が必要な場合があります。
GUI はホットリロード可能です。HTML ファイルを編集してみましょう。

### 5. デバッグ

DAW はデバッガーのアタッチが難しいケースがあるので、まずはスタンドアローンアプリとしてデバッグすることをお勧めします。
VS Code で「Debug gain plugin standalone」構成を選択して実行します。

> **注意:** スタンドアローンモードでは音声フィードバックがあります。**ヘッドフォンを使用してください。**

## 次のステップ

[architecture.md](architecture.md) を読んでみましょう。
スレッドモデル・通信フロー・パラメータ変更フローの詳細を記載しています。

また、wxp の使い方は [wxp の README](https://github.com/novonotes/wxp/tree/main/crates/wxp) に記載しています。
