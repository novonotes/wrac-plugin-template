#!/bin/bash
# build_and_install.sh - gain_plugin のビルド＆インストール（一括）
#
# CLAP をビルドしてインストールし、VST3 / AU と standalone も処理する。
#
# 使い方:
#   ./script/build_and_install.sh [Debug|Release]
#
# 引数:
#   Debug|Release - ビルド構成（省略時は Debug）

set -e  # エラー発生時にスクリプトを停止
set -u

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PLUGIN_ROOT="$( cd "$SCRIPT_DIR/.." && pwd )"
TARGET_DIR="${CARGO_TARGET_DIR:-$PLUGIN_ROOT/target}"
DEFAULT_WRAPPER_DIR="$( cd "$PLUGIN_ROOT/clap_wrapper_builder" 2>/dev/null && pwd || true )"
WRAPPER_DIR="${CLAP_WRAPPER_DIR:-$DEFAULT_WRAPPER_DIR}"

# ターミナル出力の色付け用エスケープコード
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'  # No Color（色のリセット）

# 第 1 引数を BUILD_CONFIG に設定。省略時は "Debug"。
BUILD_CONFIG="${1:-Debug}"

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

echo -e "${BLUE}gain_plugin (CLAP + wrapper) をインストール中...${NC}"
echo "ビルド構成: $BUILD_CONFIG"
echo "Wrapper ビルド: ${CLAP_ONLY:+スキップする}(CLAP_ONLY=${CLAP_ONLY:-0})"
echo ""

echo "1. CLAPプラグインをビルドしています..."
"$SCRIPT_DIR/build.sh" "$BUILD_CONFIG"

echo ""
echo "2. CLAPプラグインをインストールしています..."
"$SCRIPT_DIR/install.sh"

if [[ "$OS" != "linux" && "${CLAP_ONLY:-0}" != "1" ]]; then
    if [[ -z "$WRAPPER_DIR" || ! -d "$WRAPPER_DIR" ]]; then
        echo "エラー: clap_wrapper_builder が見つかりません"
        echo "CLAP_WRAPPER_DIR 環境変数で clap_wrapper_builder のパスを指定してください"
        exit 1
    fi

    echo ""
    echo "3. VST3 / AU ラッパーをビルドしています..."
    SKIP_CLAP_BUILD=1 "$SCRIPT_DIR/build_wrapper.sh" "$BUILD_CONFIG"

    echo ""
    echo "4. VST3 / AU ラッパーをインストールしています..."
    (
        cd "$WRAPPER_DIR"
        ./install_wrapper_plugin.sh "$TARGET_DIR/bundled/WXP Example Gain.clap" "WXP Example Gain" "$BUILD_CONFIG"
    )

    if [[ "${BUILD_STANDALONE:-1}" == "1" ]]; then
        echo ""
        echo "5. standalone アプリをビルドしています..."
        SKIP_CLAP_BUILD=1 "$SCRIPT_DIR/build_standalone.sh" "$BUILD_CONFIG"
    else
        echo ""
        echo "5. BUILD_STANDALONE=0 のため standalone アプリのビルドをスキップします"
    fi
else
    echo ""
    if [[ "${CLAP_ONLY:-0}" == "1" ]]; then
        echo "3. CLAP_ONLY=1 のため VST3 / AU / standalone の処理をスキップします"
    else
        echo "3. Linux では VST3 / AU / standalone の処理をスキップします"
    fi
fi

echo ""
echo -e "${GREEN}gain_plugin のインストールが完了しました！${NC}"
echo "インストールされたプラグイン:"
echo "  - WXP Example Gain.clap (CLAP形式)"
if [[ "$OS" != "linux" && "${CLAP_ONLY:-0}" != "1" ]]; then
    echo "  - WXP Example Gain.vst3 (VST3形式)"
    if [[ "$OS" == "macos" ]]; then
        echo "  - WXP Example Gain.component (AU形式)"
    fi
    if [[ "${BUILD_STANDALONE:-1}" == "1" ]]; then
        echo "  - WXP Example Gain Standalone (ビルドのみ)"
    fi
fi
