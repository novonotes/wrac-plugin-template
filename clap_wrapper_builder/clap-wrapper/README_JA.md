*「CLAP を他のプラグイン形式ですぐに実行するためのラッパー」*

# clap-wrapper プロジェクト

> English version: [README.md](README.md)

`clap-wrapper` プロジェクトは CLAP プラグインを他のプラグイン形式やスタンドアロン実行形式に変換します。CLAP 非対応のホストでプラグインを実行したり、CLAP のスタンドアロン実行版を配布したりできます。

開発者向けドキュメントの入口は [wiki](https://github.com/free-audio/clap-wrapper/wiki) です。

現在 `clap-wrapper` は CLAP を以下の形式に変換できます。

- VST3
- Audio Unit v2（AUv2）
- Audio Unit v3（AUv3、macOS と iOS）
- AAX
- シンプルなスタンドアロン

CLAP 開発者向けに、さまざまな配置、リンク、ビルド時のオプションも提供します。

## 基本的な考え方

`clap-wrapper` の基本的な考え方は次のとおりです。

1. `clap-wrapper` はさまざまな方法で CLAP を読み込む CLAP ホストの実装です。
2. `clap-wrapper` はプラグイン形式またはスタンドアロン実行ファイルの実装を提供し、ホストまたは OS にその形式を提示しながら、そのホストを使って CLAP を読み込みます。

これにより CLAP を VST3、AUv2、AUv3 ホストで使ったり、独立したスタンドアロン実行ファイルとして使ったりできます。CLAP の利用可能な機能は対象形式の同等の機能に変換されます。

## はじめに

開発者向けドキュメントの入口は [wiki](https://github.com/free-audio/clap-wrapper/wiki) です。`clap-wrapper` のビルドにはいくつかの選択肢があるため、最初に wiki を参照することを強くお勧めします。

最もシンプルな clap wrapper は、手元の VST3 SDK と CLAP SDK を使って、同名の CLAP を動的に読み込む VST3 です。以下の CMake コマンドでビルドできます。

```bash
git clone https://github.com/free-audio/clap-wrapper.git
mkdir build
cmake -B build \
      -DCLAP_SDK_ROOT={path to clap sdk} \
      -DVST3_SDK_ROOT={path to vst3 sdk} \
      -DCLAP_WRAPPER_OUTPUT_NAME="The Name of your CLAP"
```

AUv2 もビルドする場合は `-DCLAP_WRAPPER_BUILD_AUV2=ON` を指定し、`AudioUnitSDK` フォルダーに SDK を用意します。

AUv3 app extension をビルドする場合は `-DCLAP_WRAPPER_BUILD_AUV3=ON` を指定します。AUv3 のビルドには Xcode generator（`-G Xcode`）が必要です。`.appex` は app-extension 製品としてリンクされ、Xcode で署名されます。他の generator では説明付きのエラーを出して configure を中止します。clap-first の場合は代わりに `PLUGIN_FORMATS` に `AUV3` を追加します。AUv3 の現在の機能状況は wiki を参照してください。

**iOS** で利用できる形式は AUv3 app extension だけで、CLAP に次の2つの要件があります。

- appex に**静的リンク**する必要があります。iOS app extension は実行時に `.clap` を読み込めないため、clap-first 構成が必要で、1つの appex がホストするプラグインは1つです。
- GUI は `CLAP_WINDOW_API_UIKIT`（`"uikit"`）window API に対応し、渡された `UIView` に接続する必要があります。iOS に `NSView` はありません。この定数には CLAP 1.2.8 以降が必要です。

appex はコンテナーアプリに含めて配布します。インストゥルメントの場合、コンテナーアプリは配布可能な完全なスタンドアロンシンセです。

iOS の詳しい手順は [docs/ios-ja.md](docs/ios-ja.md) を参照してください。

**WebAssembly** ではラッピングを行いません。別の形式としてホストに提示する代わりに CLAP 自体を `.wclap`（WebAssembly CLAP）にコンパイルします。そのため VST3、AUv2、AUv3、AAX、スタンドアロンは使用できず、`PLUGIN_FORMATS` の既定値は `WCLAP` だけです。Emscripten または WASI-SDK を利用でき、configure に指定する toolchain ファイルで選択します。ビルドは `CMAKE_SYSTEM_NAME` または `wasm32` / `wasm64` processor から検出します。モジュールは SIMD（`-msimd128`）付きでコンパイルされ、Web ホストから呼び出す `clap_entry` と `malloc` をエクスポートします。

配置は `RESOURCE_DIRECTORY` の指定に依存します。指定した場合は、ディレクトリのコピーと `YourPlugin.wclap/module.wasm` を含むバンドル、および配布用のバンドル全体の `.tar.gz` が生成されます。指定しない場合は単一の `YourPlugin.wclap.wasm` が生成されます。

## コントリビューション

開発は `next` で行い、`main` はリリース時だけ更新します。issue や pull request を開く前に次を確認してください。

- 問題が `next` ですでに修正されていないか確認します。修正が入っても issue は自動的に閉じられないため、開いた issue は未修正の証拠にはなりません。
- pull request のベースには `main` ではなく `next` を使います。
- コミットが `.clang-format` に従って clang-format されていることを確認します。

## ライセンス

`clap-wrapper` プロジェクトは MIT ライセンスで公開されています。参照する SDK とライブラリにはそれぞれのライセンスが適用されます。

| SDK / ライブラリ | 用途 | ライセンス |
|---|---|---|
| [CLAP SDK](https://github.com/free-audio/clap) | すべてのラッパー | MIT |
| [VST3 SDK](https://github.com/steinbergmedia/vst3sdk)（Steinberg） | VST3 ラッパー | MIT（VST 商標・ロゴの使用は Steinberg のガイドラインに従う） |
| [AudioUnitSDK](https://github.com/apple/AudioUnitSDK)（Apple） | AUv2 ラッパー | Apache 2.0 |
| AAX SDK（Avid） | AAX ラッパー | GPL3 **または**商用 Avid AAX SDK License Agreement |
| [RtAudio](https://github.com/thestk/rtaudio) | Linux のスタンドアロン音声入出力。macOS / Windows では `CLAP_WRAPPER_STANDALONE_RTAUDIO=ON` または Windows の `RTAUDIO_API_ASIO=ON` 指定時のみ | MIT 相当（RtAudio license） |
| [RtMidi](https://github.com/thestk/rtmidi) | Linux のスタンドアロン MIDI 入出力。macOS / Windows では `CLAP_WRAPPER_STANDALONE_RTAUDIO=ON` 指定時のみ | MIT 相当（RtMidi license） |
| [ASIO SDK](https://www.steinberg.net/developers/)（Steinberg、RtAudio に同梱） | Windows のスタンドアロン ASIO。`RTAUDIO_API_ASIO=ON` 指定時のみ | GPL3 **または** Steinberg 独自の ASIO SDK License Agreement |
| [WIL](https://github.com/microsoft/wil)（Microsoft） | スタンドアロン、Windows のみ | MIT |
| [gulrak/filesystem](https://github.com/gulrak/filesystem) | 古いツールチェーンでの std::filesystem 代替 | MIT |
| [{fmt}](https://github.com/fmtlib/fmt)（`libs/fmt` に同梱） | 文字列整形 | MIT |
| [PreSonus plugin extensions](https://github.com/fenderdigital/presonus-plugin-extension)（`libs/psl` に同梱） | VST3 のゲインリダクション拡張 | パブリックドメイン |

macOS と Windows のスタンドアロンは各プラットフォームの音声・MIDI API（CoreAudio と CoreMIDI、WASAPI と WinMM）を使用し、RtAudio と RtMidi を取得しません。`-DCLAP_WRAPPER_STANDALONE_RTAUDIO=ON` を指定すると、Linux と同様に RtAudio と RtMidi を使ってビルドします。Windows では `-DRTAUDIO_API_ASIO=ON` で RtAudio の ASIO を WASAPI に追加でき、音声・MIDI 設定ウィンドウから選択したドライバーのコントロールパネルを開けます。ASIO は以下の AAX と同様にデュアルライセンスです。これを使ってビルドしたスタンドアロンは GPL3 で公開するか、Steinberg の ASIO SDK License Agreement に従う必要があります。

AUv3 ラッパーは Apple のシステムフレームワーク（AudioToolbox、AVFoundation、CoreAudioKit）のみを使用し、追加の SDK は必要ありません。

AAX もデュアルライセンスです。Avid AAX SDK は GPL3 または商用の [Avid AAX SDK License Agreement](https://developer.avid.com/aax) で提供されます。AAX プラグインをラップするには GPL3 で公開するか Avid の SDK ライセンスに同意する必要があります。商用 Pro Tools で AAX バイナリを読み込むには Avid/PACE ツールによる署名が必要なため、実際には Avid 開発者プログラムへの参加も必要です。
