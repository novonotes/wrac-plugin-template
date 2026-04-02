#!/bin/bash
# build_wrapper.sh - gain_plugin の VST3 / AU ラッパーをビルド
#
# 使い方:
#   ./script/build_wrapper.sh [Debug|Release]
#
# 環境変数:
#   SKIP_CLAP_BUILD=1 を指定すると、事前の CLAP ビルドをスキップする。

set -e
set -u

case "$(uname -s)" in
    Darwin*)
        OS="macos"
        ;;
    Linux*)
        OS="linux"
        ;;
    MINGW*|MSYS*|CYGWIN*)
        OS="windows"
        ;;
    *)
        echo "エラー: 未対応のOS $(uname -s)"
        exit 1
        ;;
esac

BUILD_CONFIG="${1:-Debug}"

case "$BUILD_CONFIG" in
    Debug|debug|DEBUG)
        BUILD_CONFIG="Debug"
        ;;
    Release|release|RELEASE)
        BUILD_CONFIG="Release"
        ;;
    *)
        echo "エラー: 無効なビルド構成: $BUILD_CONFIG"
        exit 1
        ;;
esac

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PLUGIN_ROOT="$( cd "$SCRIPT_DIR/.." && pwd )"
TARGET_DIR="${CARGO_TARGET_DIR:-$PLUGIN_ROOT/target}"
DEFAULT_WRAPPER_DIR="$( cd "$PLUGIN_ROOT/../../clap_wrapper_builder" 2>/dev/null && pwd || true )"
WRAPPER_DIR="${CLAP_WRAPPER_DIR:-$DEFAULT_WRAPPER_DIR}"

if [[ -z "$WRAPPER_DIR" || ! -d "$WRAPPER_DIR" ]]; then
    echo "エラー: clap_wrapper_builder が見つかりません"
    echo "CLAP_WRAPPER_DIR 環境変数で clap_wrapper_builder のパスを指定してください"
    exit 1
fi

if [[ "${SKIP_CLAP_BUILD:-0}" != "1" ]]; then
    echo "CLAPプラグインを先にビルドします..."
    "$SCRIPT_DIR/build.sh" "$BUILD_CONFIG"
fi

if [[ "$OS" == "linux" ]]; then
    echo "Linux では VST3 / AU ラッパーのビルドをスキップします"
    exit 0
fi

echo "VST3 / AU ラッパーをビルドしています..."
(
    cd "$WRAPPER_DIR"
    ./build_wrapper_plugin.sh "$TARGET_DIR/bundled/WXP Example Gain.clap" "WXP Example Gain" "$BUILD_CONFIG"
)

echo "VST3 / AU ラッパーのビルドが完了しました"
