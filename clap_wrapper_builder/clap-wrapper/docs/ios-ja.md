# iOS での clap-wrapper の使い方

> English version: [ios.md](ios.md)

iOS でラッパーが生成するプラグイン形式は **AUv3 app extension**（`.appex`）の1種類で、これを含む**コンテナーアプリ**も生成します。インストゥルメントの場合、このアプリは単なるテスト用ではなく、音声出力、MIDI 入出力、プラグイン UI を備えた、配布可能な完全なスタンドアロンシンセアプリです。

VST3、AUv2、AAX、RtAudio スタンドアロンは iOS には存在しません。`PLUGIN_FORMATS` に含めないでください。

## 要件

- Xcode をインストールした macOS（コマンドラインツールだけでは不足）
- CMake 3.21 以降
- **Xcode generator**（`-G Xcode`）。Ninja / Makefiles では読み込み可能な appex を作れないため、ラッパーは configure 時に処理を中止します。
- clap-first 構成: CLAP を**静的ライブラリ**と小さなエントリーソースとして用意します（`tests/clap-first-example` を参照）。iOS ではこれ以外の選択肢はありません（以下の静的リンクの説明を参照）。
- 実機で動かすには Apple Developer のチーム ID が必要です。シミュレーターでは不要です。

## macOS / Windows との5つの違い

**1. 動的ロードは行いません。**

macOS / Windows ではラッパーが実行時に検索パスから `.clap` を開きます（`dlopen` / `LoadLibrary`）。iOS では app extension が自分のバンドルに署名されていないコードをロードすることは禁止されており、ユーザーが書き込めるプラグインフォルダーもありません。そのため CLAP エントリーポイントを **appex バイナリに静的リンク**します。ラッパーは `STATICALLY_LINKED_CLAP_ENTRY=1` でコンパイルされ、ファイルシステム検索を省いて `clap_entry` グローバルを直接呼び出します。

その結果、1つの appex がホストする CLAP は、コンパイル時に組み込んだ1つだけです。ユーザーはアプリにプラグインをインストールできません。

**2. プラグインはコピーするファイルではなくアプリです。**

`~/Library/Audio/Plug-Ins` や `C:\Program Files\Common Files\VST3` はありません。`.appex` は単体ではインストールできず、App Store、Xcode、TestFlight からインストールするコンテナー `.app` の中に置く必要があります。アプリのインストール時にシステムが拡張を登録します。iOS でプラグインを配布するにはアプリを配布します。

iOS のバンドルはフラットで、配置も異なります。拡張は `Contents/PlugIns/` ではなく `MyApp.app/PlugIns/MyPlugin.appex` に置きます。

**3. バンドル ID と署名には必須の要件があります。**

- appex のバンドル ID は、ホストアプリ ID をプレフィックスとする子 ID でなければなりません。例えばホストが `com.you.myplug`、appex が `com.you.myplug.auv3` です。そうでなければ `installd` が拒否します。ID を書き換えると署名済み entitlement が無効になるため、ラッパーは書き換えません。
- ホストと appex は同じ開発チームを使い、同じデバイスファミリーを対象にする必要があります。ラッパーは両方に iPhone+iPad（`1,2`）を設定します。
- 実機ビルドには有効な署名 ID が必要です。シミュレータービルドは自動的に ad-hoc 署名を使います。
- app-sandbox entitlement の設定は不要です。macOS の AUv3 appex は明示的なサンドボックス化が必要ですが、iOS では暗黙的に適用されます。

**4. GUI は Cocoa ではなく UIKit です。**

ラッパーは `CLAP_WINDOW_API_COCOA` の代わりに CLAP window API 文字列 `"uikit"`（`CLAP_WINDOW_API_UIKIT`）でプラグインにビューを要求し、`NSView` ではなく `UIView` の親を渡します。

プラグインには次の対応が必要です。

- `is_api_supported(CLAP_WINDOW_API_UIKIT)` が true を返す。
- `gui->create()` で `CLAP_WINDOW_API_UIKIT` を受け付ける。
- `clap_window.ptr` を `UIView` として扱い、自分の `UIView` を追加する。

`CLAP_WINDOW_API_UIKIT` は CLAP 1.2.8 の `clap/ext/gui.h` に追加されたため、iOS ビルドにはそれ以降の CLAP SDK が必要です。ラッパーは 1.2.10 を固定しています（`cmake/base_sdks.cmake`）。`CLAP_SDK_ROOT` に古い CLAP を指定すると iOS ビルドはコンパイルできません。

iOS にフローティングウィンドウはありません。エディターは常に埋め込まれ、ホストがリサイズします。

**5. AU identity を CMake で指定する必要があります。**

macOS ではラッパーが CLAP を開いて AUv2 / AUv3 identity を読み取る小さな helper ツールをビルドします。この helper は macOS バイナリであり、Xcode は一度の configure で macOS ツールと iOS appex をビルドできません。そのため iOS では CMake 変数から `Info.plist` と factory class を直接生成します。manufacturer / subtype / type を渡さない場合は警告なく `errr` が既定値となり、不正な identity でプラグインが登録されます。

## Configure とビルド

シミュレーター:

```
cmake -B build-ios -G Xcode \
      -DCMAKE_SYSTEM_NAME=iOS \
      -DCMAKE_OSX_SYSROOT=iphonesimulator \
      -DCMAKE_OSX_DEPLOYMENT_TARGET=15.0

cmake --build build-ios --config Debug
```

実機:

```
cmake -B build-ios-dev -G Xcode \
      -DCMAKE_SYSTEM_NAME=iOS \
      -DCMAKE_OSX_SYSROOT=iphoneos \
      -DCMAKE_OSX_DEPLOYMENT_TARGET=15.0 \
      -DCMAKE_XCODE_ATTRIBUTE_DEVELOPMENT_TEAM=ABCDE12345

cmake --build build-ios-dev --config Release
```

`CMAKE_OSX_DEPLOYMENT_TARGET` は常に明示的に設定してください。未定義の場合、ラッパーのトップレベル CMakeLists は macOS の既定値（10.13）を設定し、iOS のバージョンとして意味のない値が appex の `Info.plist` に入ります。

## CMake の設定

### ステップ1: appex（clap-first）

```cmake
add_library(myplug-impl STATIC my_plugin.cpp)
target_link_libraries(myplug-impl PUBLIC clap clap-wrapper-extensions)

make_clapfirst_plugins(
        TARGET_NAME myplug
        IMPL_TARGET myplug-impl
        OUTPUT_NAME "My Plug"
        ENTRY_SOURCE "my_plugin_entry.cpp"

        BUNDLE_IDENTIFIER "com.you.myplug"
        BUNDLE_VERSION ${PROJECT_VERSION}

        # On iOS only these two are meaningful
        PLUGIN_FORMATS CLAP AUV3

        # Required on iOS - there is no CLAP probing here
        AUV2_MANUFACTURER_NAME "You Audio"
        AUV2_MANUFACTURER_CODE "YoAu"
        AUV2_SUBTYPE_CODE "MyPl"
        AUV2_INSTRUMENT_TYPE "aumu"   # aumu instrument, aufx effect
)
```

これによりバンドル ID が `com.you.myplug.auv3` の `myplug_auv3` ターゲットが作成されます。iOS では `.clap` ファイルを使えませんが、clap-first が要求するため `CLAP` はリストに残す必要があります。

### ステップ2: アプリ（スタンドアロンと appex コンテナー）

macOS の clap-first では AUv3 スタンドアロンホストもビルドします。iOS では署名とブランドに関する選択が必要なため、アプリを自分で設定します。

```cmake
add_executable(myplug_ios_host)

# The host instantiates the plugin in-process, so it needs the CLAP
# entry symbol too - add the same entry source and impl library.
target_sources(myplug_ios_host PRIVATE my_plugin_entry.cpp)
target_link_libraries(myplug_ios_host PRIVATE myplug-impl)

target_add_auv3_standalone_ios_wrapper(
        TARGET myplug_ios_host
        AUV3_TARGET myplug_auv3
        OUTPUT_NAME "My Plug"

        # Host ID: the appex ID above must be this plus ".auv3"
        BUNDLE_IDENTIFIER "com.you.myplug"
        BUNDLE_VERSION ${PROJECT_VERSION}

        # Must match the appex identity exactly - the factory class name
        # is derived from manufacturer + subtype
        AU_TYPE "aumu"
        AU_SUBTYPE "MyPl"
        AU_MANUFACTURER "YoAu"

        DEVELOPMENT_TEAM "ABCDE12345"          # optional
        ICON_ASSET_CATALOG "${CMAKE_CURRENT_SOURCE_DIR}/Assets.xcassets"
        LAUNCH_SCREEN_IMAGE "LaunchImage"      # optional
)
```

呼び出し順序に注意してください。AUv3 ターゲットに対し、`target_add_auv3_standalone_ios_wrapper` より前に `target_add_auv3_wrapper` を実行する必要があります。後者は `make_clapfirst_plugins` の内部で実行されます。

オプションとして、独自の `Info.plist.in` を指定する `CUSTOM_INFO_PLIST_TEMPLATE`、個別の plist キー用の `EXTRA_INFO_PLIST_ENTRIES`、アイコンセットの名前が `AppIcon` ではない場合の `APP_ICON_NAME` があります。

ホストは appex を `PlugIns/` に埋め込み、ビルド後に自身を再署名します。

## 実行

シミュレーター:

```
xcrun simctl boot "iPhone 15"
xcrun simctl install booted "<path to My Plug.app>"
xcrun simctl launch booted com.you.myplug
```
Xcode generator はアプリを構成とプラットフォームごとのフォルダー（例: `build-ios/Debug-iphonesimulator/`）に出力します。プロジェクトのサブディレクトリや `ASSET_OUTPUT_DIRECTORY` を設定した場合はそれも加わります。

実機では、生成した Xcode プロジェクトを開き、アプリターゲットを選択して実行します。

アプリを起動するとスタンドアロンが動作します。アプリのインストール時に AUv3 がシステムに登録され、AUM、Cubasis、Loopy Pro、GarageBand など他のホストにも表示されます。

### アプリの機能

インストゥルメント（`aumu` / `aufg`）の場合、アプリ自体が利用可能なスタンドアロンシンセであり、その形での配布を想定しています。

- プラグイン UI が安全領域内の画面全体に表示されます。
- プラグインのすべての出力バスがシステム出力にミックスされます。
- 起動後に接続されたデバイスを含む、検出したすべての MIDI ソースに接続します。プラグインに MIDI 出力ポートがある場合は仮想 MIDI 出力を公開します。
- 通話、Siri、AirPods、AirPlay、USB オーディオなどによる中断や経路変更に対応し、ハードウェアのサンプルレートが変わると音声グラフを再構築します。
- バックグラウンドオーディオが有効で、バックグラウンドでも再生を続けます。
- アプリアイコン、起動画面、追加の `Info.plist` キーは CMake から設定できます（上記のオプションを参照）。

音声**入力**バスは接続されません。入力ソースを選ぶ UI がないためです。エフェクトプラグインには無音が入力され、スタンドアロンアプリはインストゥルメント向けです。実際の AUv3 ホストではエフェクトも通常どおり動作します。

### アプリがプラグインをプロセス内でインスタンス化する理由

iOS 18 以降、プロセスローカルの AudioComponent registry はサードパーティの AUv3 extension を通常のホストプロセスから隠します。自分自身に埋め込まれた appex を探すアプリも例外ではありません。そのためアプリは registry を検索せず、ラッパー runtime と生成された factory class を自身にリンクしてプラグインを直接作成します。appex 自体は引き続きビルド・インストールされ、外部ホストで通常どおり動作します。

## トラブルシューティング

| 症状 | 原因 |
|---|---|
| configure が「AUv3 requires the Xcode generator」で失敗する | `-G Xcode` を使っていない |
| プラグインが「errr」として登録される | AU identity を CMake に渡していない |
| 実機への appex インストールが失敗する（OSStatus -3000） | appex のバンドル ID が `<host-id>.auv3` ではない、またはチームが一致しない |
| 「no code-sign identity for a device build」 | `DEVELOPMENT_TEAM` を設定するかシミュレーター向けにビルドする |
| ホストに「Factory class ... is not linked」と表示される | ホストターゲットにエントリーソース / 実装ライブラリがない、またはホストと appex の AU コードが異なる |
| iPhone には表示されるが iPad には表示されない | デバイスファミリーが一致しない。両バンドルを `1,2` にする |
| GUI が空のまま | プラグインが `CLAP_WINDOW_API_UIKIT` window API を実装していない |
| `clap_proxy.cpp` や `preset_discovery.cpp` で `std::filesystem` または `if` 初期化子のエラー | C++17 未満でビルドしている。現在のラッパーは `cxx_std_17` を最低要件にしているため、古いラッパーでのみ発生する。自身のプロジェクトの `CMAKE_CXX_STANDARD` を 17 以上に設定する |

## 現在の制限

- 1つの appex に CLAP は1つ、プラグインも1つのみ。
- AUv3 では `RESOURCE_DIRECTORY` に非対応。
- スタンドアロンアプリに音声入力がないため、エフェクトプラグインは AUv3 のみ。
- スタンドアロンアプリは短い MIDI メッセージを送受信し、SysEx には非対応。
