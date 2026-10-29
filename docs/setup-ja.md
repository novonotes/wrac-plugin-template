# Setup

> English version: [setup.md](setup.md)

`wrac-plugin-template` を出発点として、新しい wxp プラグインを作成する手順を説明します。

## 前提条件

### CLAP のみをビルドする場合

- Rust（最新の stable）
- Node.js（npm）

### VST3 / AU / AAX、または開発用 standalone app もビルドする場合

clap-wrapper を使って VST3 / AU / AAX を生成する場合や、開発用 standalone app をビルドする場合は、追加で以下が必要です。

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
VST3 / AU、開発用 standalone app、または VST3 / AU の検証を行う場合は、clap-wrapper が利用する SDK サブモジュールが必要です。
AAX ビルドには、加えて private な AAX SDK が必要です。ローカルの AAX path は `.env` に書いてください。詳細は [AAX Build and Validation](aax-ja.md) を参照してください。

### 2. プラグインの識別情報を設定する

プラグインの識別情報は `plugins/wrac-gain/src-plugin/wrac-plugin.toml` に集約しています。
host-visible ID を Rust code や Cargo metadata に重複して書かず、この manifest を編集してください。

各プラグインのマニフェストは `<plugin-root>/src-plugin/Cargo.toml` と `<plugin-root>/src-plugin/wrac-plugin.toml` に置いてください。xtask と CI は WRAC マニフェストの欠落や別配置を拒否します。
設定項目と注意事項は [wrac_manifest の README](../crates/wrac_manifest/README_JA.md) を参照してください。

### 3. 残りの識別子を一括置換

このリポジトリには複数種類の識別子が散在しています。
IDE の機能や `rg`、LLM エージェントなどで全ファイルを検索し、まとめて置換してください。

**置換テーブル:**

| 種別 | 現在の値 | 置換先の例 |
|------|---------|-----------|
| WRAC plugin package 名（Cargo package） | `wrac_gain_plugin` | `my_plugin` |
| GUI / スクリプト内などの kebab-case 名 | `wrac-gain-plugin` | `my-plugin` |
| `Cargo.toml` 内の repository URL | `https://github.com/novonotes/wrac-plugin-template` | `https://github.com/your-org/my-plugin` |

例えば、次のコマンドで残っているテンプレート名とリポジトリ URL を検索できます。

```sh
rg --hidden 'wrac_gain_plugin|WRAC Gain|com\.your-company\.wrac-gain|wrac-gain-plugin' \
  --glob '!.git/**' --glob '!node_modules/**' --glob '!dist/**' \
  --glob '!target/**' --glob '!clap_wrapper_builder/clap-wrapper/**' \
  --glob '!*.lock' --glob '!docs/setup*.md'
rg --hidden 'repository = "https://github.com/novonotes/wrac-plugin-template"' \
  --glob '!.git/**' --glob 'Cargo.toml'
```

検索結果を確認してプラグインに関係する識別子を置き換えたら、もう一度検索して漏れが
ないか確認してください。テンプレート自体への参照が検索結果に含まれる場合もあります。

### 4. ビルド & インストール

リポジトリルートで以下を実行します。

```sh
cd /path/to/my_plugin
cargo xtask install
```

`cargo xtask install` は選択したプラグインフォーマットをビルドしてインストールします。
各 xtask コマンドの詳しい使い方は `cargo xtask --help` を参照してください。

### 5. 動作確認

デバッグビルドでは、GUI リソースを Vite の開発サーバー（`localhost:5173`）から取得します。
DAW でデバッグビルドのプラグインを起動する前に、以下のコマンドで開発サーバーを立ち上げておいてください。

```sh
cd /path/to/my_plugin/plugins/wrac-gain/src-gui
npm install
npm run dev
```

リリースビルドでは、`src-plugin/build.rs` が隣接する `src-gui/dist` を zip 化してプラグインバイナリに埋め込むため、開発サーバーは不要です。

DAW を起動して、プラグインをインサートしてみましょう。
DAW によってはプラグイン再スキャン等が必要な場合があります。
GUI はホットリロード可能です。HTML ファイルを編集してみましょう。

### 6. デバッグ

DAW はデバッガーのアタッチが難しいケースがあるので、まずは開発用 standalone app でデバッグすることをお勧めします。
VS Code で「Debug gain plugin standalone」構成を選択して実行します。

> **注意:** スタンドアローンモードでは音声フィードバックがあります。**ヘッドフォンを使用してください。**

### デバッグログを見る

デバッグビルドのログは `.log/<plugin_name> Latest.log` に出ます。
追いかける場合は macOS / Linux では `tail -f ".log/<plugin_name> Latest.log"`、Windows PowerShell では `Get-Content ".log\<plugin_name> Latest.log" -Wait` などを使います。
ログについての詳細は `crates/wrac_log` のディレクトリを参照してください。
