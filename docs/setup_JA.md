# Setup

> English version: [setup.md](setup.md)

`wrac-plugin-template` を出発点として新しい wxp プラグインを作成する手順を説明します。

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

GitHub の [wrac-plugin-template](https://github.com/novonotes/wrac-plugin-template) ページ右上の `Use this template` ボタンを使って新しいリポジトリを作成します。
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
| Rust crate 名 | `wrac_gain_plugin` | `my_plugin` |
| プラグイン表示名 | `WRAC Gain` | `My Plugin` |
| プラグイン ID（逆ドメイン推奨） | `com.your-company.wrac-gain` | `com.your-company.my-plugin` |
| GUI / スクリプト内などの kebab-case 名 | `wrac-gain-plugin` | `my-plugin` |
| `Cargo.toml` 内の repository URL | `https://github.com/novonotes/wrac-plugin-template` | `https://github.com/your-org/my-plugin` |

> **重要:** プラグイン ID はグローバルに一意である必要があります。一度公開したら変更できません。
> repository URL はデフォルトではこのテンプレートを指しています。新しいプロジェクトを作成した後、crate metadata を公開する場合は自分のリポジトリに変更してください。

**手順:**

対象ファイルと残件数を確認します。

rg を用いる例:

```sh
rg --hidden "wrac_gain_plugin|WRAC Gain|com\.your-company\.wrac-gain|wrac-gain-plugin" \
    --glob '!node_modules' --glob '!dist' --glob '!*.lock' \
    --glob '!package-lock.json' --glob '!*.zip' \
    --glob '!docs/setup.md' --glob '!docs/setup_JA.md'

rg --hidden 'repository = "https://github.com/novonotes/wrac-plugin-template"' --glob 'Cargo.toml'
```

確認できたら、上の置換テーブルの通りに**全件置換**してください。
置換後に同じコマンド群を再実行し、出力がゼロ件になれば完了です。

### 3. ビルド & インストール

リポジトリルートで以下を実行します。

```sh
cd /path/to/my_plugin
cargo xtask build --install
```

以下のディレクトリにビルド済みプラグインがインストールされます:

| OS | インストール先 |
|----|--------------|
| macOS | `~/Library/Audio/Plug-Ins/CLAP/` |
| Windows | `%LOCALAPPDATA%/Programs/Common/CLAP/` および `%LOCALAPPDATA%/Programs/Common/VST3/` |
| Linux | `~/.clap/` |

VST3 / AU / standalone は現在の OS で対応している場合に同時にビルドされます。
スタンドアローンアプリには plugin install 先がないため、xtask は artifact path を表示します。

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

### デバッグログを見る

デバッグビルドのログは `.log/WRAC Gain Latest.log` に出ます。
追いかける場合は macOS / Linux では `tail -f ".log/WRAC Gain Latest.log"`、Windows PowerShell では `Get-Content ".log\WRAC Gain Latest.log" -Wait` を使います。
ログレベルは `RUST_LOG=trace` のように環境変数で指定できます。
