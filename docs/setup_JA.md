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
- C++ コンパイラとビルドツール
- CMake（3.15 以上推奨）
- WebKitGTK、GTK 3、GDK X11、X11 の開発パッケージ

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

CLAP のみをビルドする場合、サブモジュールは不要です。
VST3 / AU / Standalone、または VST3 / AU の検証を行う場合は clap-wrapper 関連のサブモジュールが必要です。

### 2. プラグイン identity を設定する

プラグイン identity は `src-plugin/Cargo.toml` に集約しています。
まず `[package.metadata.wrac]` を編集してください。

```toml
[package.metadata.wrac]
company_name = "Your Company"
auv2_manufacturer_code = "YrCo"
bundle_name = "My Plugin"
standalone_name = "My Plugin Standalone"

[[package.metadata.wrac.plugins]]
plugin_id = "com.your-company.my-plugin"
plugin_name = "My Plugin"
auv2_type = "aufx"
auv2_subtype = "MyPl"
```

> **重要:** プラグイン ID はグローバルに一意である必要があります。一度公開したら変更できません。
> AUv2 の `auv2_type`、`auv2_subtype`、`auv2_manufacturer_code` はそれぞれ 4 byte の ASCII にしてください。

### 3. 残りの識別子を一括置換

このリポジトリには複数種類の識別子が散在しています。
IDE の機能や `rg`、LLM エージェントなどで全ファイルを検索し、まとめて置換してください。

**置換テーブル:**

| 種別 | 現在の値 | 置換先の例 |
|------|---------|-----------|
| Rust crate 名 | `wrac_gain_plugin` | `my_plugin` |
| GUI / スクリプト内などの kebab-case 名 | `wrac-gain-plugin` | `my-plugin` |
| `Cargo.toml` 内の repository URL | `https://github.com/novonotes/wrac-plugin-template` | `https://github.com/your-org/my-plugin` |

repository URL はデフォルトではこのテンプレートを指しています。新しいプロジェクトを作成した後、crate metadata を公開する場合は自分のリポジトリに変更してください。

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

### 4. ビルド & インストール

リポジトリルートで以下を実行します。

```sh
cd /path/to/my_plugin
cargo xtask install
```

`cargo xtask install` は選択したプラグインフォーマットをビルドしてからインストールします。
`cargo xtask install` は既定で user-local のパスにインストールします。
system-wide のみをスキャンするホスト向けには `cargo xtask install --scope=system` を使います。
`--target` オプションで `clap`、`vst3`、`au` をカンマ区切りで指定できます。

### 5. 動作確認

デバッグビルドでは GUI リソースを Vite dev server（`localhost:5173`）から取得します。
DAW でデバッグビルドのプラグインを起動する前に、以下のコマンドで dev server を立ち上げておいてください。

```sh
cd /path/to/my_plugin/src-gui
npm install
npm run dev
```

リリースビルドでは `src-plugin/build.rs` が `src-gui/dist` を zip 化して plugin バイナリに埋め込むため、dev server は不要です。

DAW を起動して、プラグインをインサートしてみましょう。
DAW によってはプラグイン再スキャン等が必要な場合があります。
GUI はホットリロード可能です。HTML ファイルを編集してみましょう。

### 6. デバッグ

DAW はデバッガーのアタッチが難しいケースがあるので、まずはスタンドアローンアプリとしてデバッグすることをお勧めします。
VS Code で「Debug gain plugin standalone」構成を選択して実行します。

> **注意:** スタンドアローンモードでは音声フィードバックがあります。**ヘッドフォンを使用してください。**

### デバッグログを見る

デバッグビルドのログは `.log/<plugin_name> Latest.log` に出ます。
追いかける場合は macOS / Linux では `tail -f ".log/<plugin_name> Latest.log"`、Windows PowerShell では `Get-Content ".log\<plugin_name> Latest.log" -Wait` などを使います。
