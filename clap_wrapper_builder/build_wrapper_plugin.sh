#!/bin/bash
# build_wrapper_plugin.sh - 任意のCLAPプラグインからVST3/AUラッパーをビルド
#
# 使い方:
#   ./build_wrapper_plugin.sh <CLAPファイル名> <出力プラグイン名> [Debug|Release]
#
# 引数:
#   CLAPファイル名 - CLAPプラグインのファイル名 (例: "example_plugin_nih.clap")
#   出力プラグイン名 - VST3/AUで使用する表示名 (例: "Example Plugin NIH")
#   Debug|Release - ビルド構成（省略時は Debug）
#
# 例:
#   ./build_wrapper_plugin.sh example_plugin_nih.clap "Example Plugin NIH" Release
#   ./build_wrapper_plugin.sh "XDevice Editor.clap" "XDevice Editor" Debug

set -Eeuo pipefail

# カラー出力用の定数
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# エラーメッセージ表示関数
error() {
    echo -e "${RED}エラー: $1${NC}" >&2
    exit 1
}

on_error() {
    local exit_code="$1"
    local line_no="$2"
    local command="$3"
    echo -e "${RED}エラー: ${line_no} 行目のコマンドが失敗しました (exit=${exit_code}): ${command}${NC}" >&2
    exit "$exit_code"
}

trap 'on_error $? $LINENO "$BASH_COMMAND"' ERR

# 成功メッセージ表示関数
success() {
    echo -e "${GREEN}$1${NC}"
}

# 警告メッセージ表示関数
warning() {
    echo -e "${YELLOW}警告: $1${NC}"
}

# 現在のディレクトリを保存
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"

# 使用方法を表示する関数
usage() {
    echo "使用方法: $0 <CLAPファイル名> <出力プラグイン名> [Debug|Release]"
    echo "  引数を指定しない場合、ビルド構成のデフォルトは Debug です"
    echo ""
    echo "例:"
    echo "  $0 example_plugin_nih.clap \"Example Plugin NIH\" Release"
    echo "  $0 \"XDevice Editor.clap\" \"XDevice Editor\" Debug"
    exit 1
}

# 引数の処理
if [ $# -lt 2 ]; then
    usage
fi

CLAP_FILE="$1"
OUTPUT_NAME="$2"
BUILD_CONFIG="Debug"

if [ $# -ge 3 ]; then
    case "$3" in
        Debug|debug|DEBUG)
            BUILD_CONFIG="Debug"
            ;;
        Release|release|RELEASE)
            BUILD_CONFIG="Release"
            ;;
        -h|--help)
            usage
            ;;
        *)
            error "無効なビルド構成: $3"
            ;;
    esac
fi

echo "CLAPファイル: $CLAP_FILE"
echo "出力プラグイン名: $OUTPUT_NAME"
echo "ビルド構成: $BUILD_CONFIG"

# CLAPファイル名から拡張子を除いた名前を取得し、スペースをアンダースコアに置換
# パス部分を除去してファイル名のみを取得
CLAP_FILE_BASENAME=$(basename "$CLAP_FILE")
CLAP_BASE_NAME="${CLAP_FILE_BASENAME%.clap}"
CLAP_BASE_NAME="${CLAP_BASE_NAME// /_}"

# clap-wrapper ディレクトリの確認
if [ ! -d "$SCRIPT_DIR/clap-wrapper" ]; then
    error "clap-wrapper ディレクトリが見つかりません。先に git clone https://github.com/free-audio/clap-wrapper.git を実行してください。"
fi

# サブモジュールの clap SDK を使用
CLAP_SDK_ROOT="$SCRIPT_DIR/clap"
if [ ! -d "$CLAP_SDK_ROOT" ]; then
    error "clap サブモジュールが見つかりません。git submodule update --init --recursive を実行してください。"
fi
success "CLAP SDK サブモジュールを検出: $CLAP_SDK_ROOT"

# サブモジュールの VST3 SDK を使用
VST3_SDK_ROOT="$SCRIPT_DIR/vst3sdk"
if [ ! -d "$VST3_SDK_ROOT" ]; then
    error "vst3sdk サブモジュールが見つかりません。git submodule update --init --recursive を実行してください。"
fi
success "VST3 SDK サブモジュールを検出: $VST3_SDK_ROOT"

# サブモジュールの AU SDK を使用
if [[ "$OSTYPE" == darwin* ]]; then
    AU_SDK_ROOT="$SCRIPT_DIR/AudioUnitSDK"
    if [[ ! -d "$AU_SDK_ROOT" ]]; then
        error "AudioUnitSDK サブモジュールが見つかりません。git submodule update --init --recursive を実行してください。"
    fi
    success "AU SDK サブモジュールを検出: $AU_SDK_ROOT"
else
    AU_SDK_ROOT=
fi

# OS の検出とジェネレータの選択
CMAKE_GENERATOR=""

case "$OSTYPE" in
    darwin*)
        # macOS
        if command -v xcodebuild &> /dev/null; then
            CMAKE_GENERATOR="Xcode"
            success "macOS を検出: Xcode を使用します"
        else
            error "Xcode が見つかりません。Xcode または Command Line Tools をインストールしてください。"
        fi
        ;;
    linux*)
        # Linux
        CMAKE_GENERATOR="Unix Makefiles"
        success "Linux を検出: Unix Makefiles を使用します"
        ;;
    msys*|cygwin*|mingw*)
        # Windows
        # CMakeが自動的にVisual Studioを検出する
        CMAKE_GENERATOR="Visual Studio 17 2022"
        success "Windows を検出: Visual Studio 2022 を使用します"
        ;;
    *)
        CMAKE_GENERATOR="Unix Makefiles"
        warning "不明な OS: Unix Makefiles を使用します"
        ;;
esac

# ビルドディレクトリをclap_wrapper_builderに作成
BUILD_DIR="$SCRIPT_DIR/build_$CLAP_BASE_NAME"

# CMake の設定
echo "CMake を設定中..."
if [[ "$OSTYPE" == darwin* ]]; then
    # macOS の場合、Universal Binary を作成
    cmake -S "$SCRIPT_DIR/clap-wrapper" -B "$BUILD_DIR" \
        -DCLAP_SDK_ROOT="$CLAP_SDK_ROOT" \
        -DVST3_SDK_ROOT="$VST3_SDK_ROOT" \
        -DCLAP_WRAPPER_OUTPUT_NAME="$OUTPUT_NAME" \
        -DCMAKE_BUILD_TYPE="$BUILD_CONFIG" \
        -DCMAKE_OSX_ARCHITECTURES="x86_64;arm64" \
        -DCLAP_WRAPPER_BUILD_AUV2=ON \
        -DAUDIOUNIT_SDK_ROOT="$AU_SDK_ROOT" \
        -DCLAP_WRAPPER_CXX_STANDARD=23 \
        -G "$CMAKE_GENERATOR"
else
    # その他のプラットフォーム
    cmake -S "$SCRIPT_DIR/clap-wrapper" -B "$BUILD_DIR" \
        -DCLAP_SDK_ROOT="$CLAP_SDK_ROOT" \
        -DVST3_SDK_ROOT="$VST3_SDK_ROOT" \
        -DCLAP_WRAPPER_OUTPUT_NAME="$OUTPUT_NAME" \
        -DCMAKE_BUILD_TYPE="$BUILD_CONFIG" \
        -G "$CMAKE_GENERATOR"
fi

success "CMake の設定が完了しました"

# ビルドの実行
echo "ビルド中..."

# AudioUnitSDK のヘッダーが GNU statement expression を使用しており、
# clap-wrapper の -Wpedantic -Werror と衝突するため、Xcode ビルド時に抑制する
if [[ "$CMAKE_GENERATOR" == "Xcode" ]]; then
    XCODE_FLAGS=('--' 'OTHER_CPLUSPLUSFLAGS=$(inherited) -Wno-gnu-statement-expression-from-macro-expansion -Wno-shorten-64-to-32')
    XCODE_BUILD_ARGS=(--clean-first)
    # macOS かつ xcbeautify がある場合のみパイプを追加
    if command -v xcbeautify &> /dev/null; then
        cmake --build "$BUILD_DIR" --config "$BUILD_CONFIG" "${XCODE_BUILD_ARGS[@]}" "${XCODE_FLAGS[@]}" 2>&1 | xcbeautify --quiet
    else
        cmake --build "$BUILD_DIR" --config "$BUILD_CONFIG" "${XCODE_BUILD_ARGS[@]}" "${XCODE_FLAGS[@]}"
    fi
elif [[ "$CMAKE_GENERATOR" == "Visual Studio 17 2022" ]]; then
    cmake --build "$BUILD_DIR" --config "$BUILD_CONFIG"
else
    cmake --build "$BUILD_DIR"
fi
success "ビルドが完了しました"

# ビルド結果の確認
VST3_OUTPUT=""
if [[ "$CMAKE_GENERATOR" == "Xcode" ]] || [[ "$CMAKE_GENERATOR" == "Visual Studio 17 2022" ]]; then
    # マルチコンフィギュレーションジェネレータの場合、Configuration サブディレクトリを探す
    if [[ "$OSTYPE" == darwin* ]]; then
        VST3_OUTPUT=$(find "$BUILD_DIR/$BUILD_CONFIG" -name "*.vst3" -type d 2>/dev/null | head -n 1)
    else
        VST3_OUTPUT=$(find "$BUILD_DIR/$BUILD_CONFIG" -name "*.vst3" -type f 2>/dev/null | head -n 1)
    fi
else
    # シングルコンフィギュレーションジェネレータの場合
    if [[ "$OSTYPE" == darwin* ]]; then
        VST3_OUTPUT=$(find "$BUILD_DIR" -name "*.vst3" -type d | head -n 1)
    else
        VST3_OUTPUT=$(find "$BUILD_DIR" -name "*.vst3" -type f | head -n 1)
    fi
fi

if [ -n "$VST3_OUTPUT" ]; then
    # フルパスを取得
    VST3_FULLPATH="$(cd "$(dirname "$VST3_OUTPUT")" && pwd)/$(basename "$VST3_OUTPUT")"
    success "VST3 プラグインが生成されました: $VST3_FULLPATH"
else
    error "VST3 プラグインが見つかりません"
fi
