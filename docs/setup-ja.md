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
AAX ビルドには、加えて private な AAX SDK が必要です。展開した SDK root directory を `AAX_SDK_ROOT` で指定してください。詳細は [AAX Build and Validation](aax-ja.md) を参照してください。

### 2. プラグインの識別情報を設定する

プラグインの識別情報は、プラグインパッケージの manifest に集約しています。初期状態では `plugins/wrac-gain/src-plugin/Cargo.toml` です。
この guide に別の manifest sample を複製するのではなく、そこにあるコメント付きの `[package.metadata.wrac]` と `[[package.metadata.wrac.plugins]]` を直接編集してください。

> **重要:** プラグイン ID はグローバルに一意である必要があります。一度公開したら変更できません。
> AUv2 の `auv2_type`、`auv2_subtype`、`auv2_manufacturer_code` は、それぞれ 4 byte の ASCII にしてください。
> `clap_features` は実際の audio/MIDI 挙動と一致させてください。CLAP host が直接読みます。
> `vst3_subcategories` は VST3 host browser category を制御します。`Fx|Dynamics` のような Steinberg 形式の `|` 区切り値を指定してください。
> `vst3_component_id` は安定した UUID にしてください。release 前に一度生成し、同じ製品では変更しないでください。
> `aax_manufacturer_id`、`aax_product_id`、各 AAX stem config の `plugin_id` は、安定した 4 byte ASCII ID にしてください。
> AAX stem config には、製品が実際に対応する channel layout だけを列挙してください。

### 3. 残りの識別子を一括置換

このリポジトリには複数種類の識別子が散在しています。
IDE の機能や `rg`、LLM エージェントなどで全ファイルを検索し、まとめて置換してください。

**置換テーブル:**

| 種別 | 現在の値 | 置換先の例 |
|------|---------|-----------|
| WRAC plugin package 名（Cargo package） | `wrac_gain_plugin` | `my_plugin` |
| GUI / スクリプト内などの kebab-case 名 | `wrac-gain-plugin` | `my-plugin` |
| `Cargo.toml` 内の repository URL | `https://github.com/novonotes/wrac-plugin-template` | `https://github.com/your-org/my-plugin` |

repository URL は、デフォルトではこのテンプレートを指しています。新しいプロジェクトを作成した後、crate metadata を公開する場合は自分のリポジトリに変更してください。

**手順:**

対象ファイルと残件数を確認します。

rg を用いる例:

```sh
rg --hidden "wrac_gain_plugin|WRAC Gain|com\.your-company\.wrac-gain|wrac-gain-plugin" \
    --glob '!node_modules' --glob '!dist' --glob '!*.lock' \
    --glob '!package-lock.json' --glob '!*.zip' \
    --glob '!docs/setup.md' --glob '!docs/setup-ja.md'

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
workspace に複数の WRAC plugin package がある場合は、Cargo package 名を `-p/--package` で指定してください。
`cargo xtask install` は、既定ではユーザー単位のパスにインストールします。
system-wide の plugin folder だけをスキャンするホスト向けには、`cargo xtask install --scope=system` を使います。
`--target` オプションで `clap`、`vst3`、`au`、`aax` をカンマ区切りで指定できます。
AAX は既定の target set には含まれないため、`--target=aax` で明示的に指定してください。
macOS / Windows では AAX は system-wide の Avid plugin folder にインストールされるため、`--scope=system` を使ってください。

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

Standalone app は軽量な開発用 host であり、リリース用のプラグインフォーマットや出荷物ではありません。

> **注意:** スタンドアローンモードでは音声フィードバックがあります。**ヘッドフォンを使用してください。**

### デバッグログを見る

デバッグビルドのログは `.log/<plugin_name> Latest.log` に出ます。
追いかける場合は macOS / Linux では `tail -f ".log/<plugin_name> Latest.log"`、Windows PowerShell では `Get-Content ".log\<plugin_name> Latest.log" -Wait` などを使います。
