#!/bin/bash
# install_wrapper_plugin.sh - 生成されたVST3/AUプラグインをインストール
#
# 使い方:
#   ./install_wrapper_plugin.sh <CLAPファイル名> <出力プラグイン名> [Debug|Release]
#
# 引数:
#   CLAPファイル名 - CLAPプラグインのファイル名 (例: "example_plugin_nih.clap")
#   出力プラグイン名 - VST3/AUで使用する表示名 (例: "Example Plugin NIH")
#   Debug|Release - ビルド構成（省略時は Debug）
#
# 注意:
#   - build_wrapper_plugin.sh を先に実行してVST3/AUを生成する必要があります

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

# OS判定とインストール先ディレクトリの設定
case "$OSTYPE" in
    darwin*)
        # macOS
        OS="macos"
        VST3_INSTALL_DIR="$HOME/Library/Audio/Plug-Ins/VST3"
        AU_INSTALL_DIR="$HOME/Library/Audio/Plug-Ins/Components"
        success "macOS を検出"
        ;;
    linux*)
        # Linux
        OS="linux"
        VST3_INSTALL_DIR="$HOME/.vst3"
        success "Linux を検出"
        ;;
    msys*|cygwin*|mingw*)
        # Windows
        OS="windows"
        # Windows環境変数を使用
        if [ -n "$COMMONPROGRAMFILES" ]; then
            VST3_INSTALL_DIR="$COMMONPROGRAMFILES/VST3"
        else
            VST3_INSTALL_DIR="$LOCALAPPDATA/Programs/Common/VST3"
        fi
        success "Windows を検出"
        ;;
    *)
        error "未対応のOS: $OSTYPE"
        ;;
esac

# ビルドディレクトリの確認
BUILD_DIR="$SCRIPT_DIR/build_$CLAP_BASE_NAME"
if [ ! -d "$BUILD_DIR" ]; then
    error "ビルドディレクトリが見つかりません。先に build_wrapper_plugin.sh を実行してください。"
fi

# VST3プラグインの検索
VST3_OUTPUT=""
VST3_FILENAME="$OUTPUT_NAME.vst3"
AU_OUTPUT=""
AU_FILENAME="$OUTPUT_NAME.component"

# マルチコンフィギュレーションジェネレータの場合
if [ -d "$BUILD_DIR/$BUILD_CONFIG" ]; then
    if [[ "$OS" == "macos" ]]; then
        VST3_OUTPUT=$(find "$BUILD_DIR/$BUILD_CONFIG" -name "$VST3_FILENAME" -type d | head -n 1 || true)
    else
        VST3_OUTPUT=$(find "$BUILD_DIR/$BUILD_CONFIG" -name "$VST3_FILENAME" -type f | head -n 1 || true)
    fi
fi

# シングルコンフィギュレーションジェネレータの場合
if [ -z "$VST3_OUTPUT" ]; then
    if [[ "$OS" == "macos" ]]; then
        VST3_OUTPUT=$(find "$BUILD_DIR" -name "$VST3_FILENAME" -type d | head -n 1 || true)
    else
        VST3_OUTPUT=$(find "$BUILD_DIR" -name "$VST3_FILENAME" -type f | head -n 1 || true)
    fi
fi

if [ -z "$VST3_OUTPUT" ]; then
    error "VST3プラグインが見つかりません。先に build_wrapper_plugin.sh を実行してください。"
fi

VST3_FULLPATH="$(cd "$(dirname "$VST3_OUTPUT")" && pwd)/$(basename "$VST3_OUTPUT")"
success "VST3プラグインを検出: $VST3_FULLPATH"

if [[ "$OS" == "macos" ]]; then
    if [ -d "$BUILD_DIR/$BUILD_CONFIG" ]; then
        AU_OUTPUT=$(find "$BUILD_DIR/$BUILD_CONFIG" -name "$AU_FILENAME" -type d | head -n 1 || true)
    fi

    if [ -z "$AU_OUTPUT" ]; then
        AU_OUTPUT=$(find "$BUILD_DIR" -name "$AU_FILENAME" -type d | head -n 1 || true)
    fi

    if [ -n "$AU_OUTPUT" ]; then
        AU_FULLPATH="$(cd "$(dirname "$AU_OUTPUT")" && pwd)/$(basename "$AU_OUTPUT")"
        success "AUプラグインを検出: $AU_FULLPATH"
    else
        warning "AUプラグインが見つかりませんでした。VST3 のみインストールします。"
    fi
fi

# CLAPプラグインの確認（警告のみ）
CLAP_INSTALLED=false

if [[ "$OS" == "macos" ]]; then
    if [ -e "$HOME/Library/Audio/Plug-Ins/CLAP/$CLAP_FILE" ] || \
       [ -e "/Library/Audio/Plug-Ins/CLAP/$CLAP_FILE" ]; then
        CLAP_INSTALLED=true
    fi
elif [[ "$OS" == "linux" ]]; then
    if [ -e "$HOME/.clap/$CLAP_FILE" ] || \
       [ -e "/usr/lib/clap/$CLAP_FILE" ]; then
        CLAP_INSTALLED=true
    fi
elif [[ "$OS" == "windows" ]]; then
    if [ -e "$LOCALAPPDATA/Programs/Common/CLAP/$CLAP_FILE" ]; then
        CLAP_INSTALLED=true
    fi
fi

# CLAP未インストール警告は最後にまとめて表示

# VST3インストールディレクトリの作成
echo "VST3インストールディレクトリを準備しています..."
if [[ "$OS" == "windows" ]]; then
    # Windowsの場合、管理者権限が必要な可能性がある
    mkdir -p "$VST3_INSTALL_DIR" 2>/dev/null || {
        warning "VST3インストールディレクトリの作成に失敗しました。"
        warning "管理者権限でスクリプトを実行するか、手動でインストールしてください。"
        echo ""
        echo "手動インストール方法:"
        echo "  cp -r \"$VST3_FULLPATH\" \"$VST3_INSTALL_DIR/\""
        exit 1
    }
else
    mkdir -p "$VST3_INSTALL_DIR" || {
        error "VST3インストールディレクトリを作成できませんでした: $VST3_INSTALL_DIR"
    }
fi

# VST3プラグインのインストール
echo "VST3プラグインをインストールしています..."
if [[ "$OS" == "macos" ]]; then
    # macOSではバンドル全体をコピー
    rm -rf "$VST3_INSTALL_DIR/$VST3_FILENAME"
    cp -r "$VST3_FULLPATH" "$VST3_INSTALL_DIR/" || {
        error "VST3プラグインのコピーに失敗しました"
    }
else
    # その他のOSではファイルをコピー
    cp "$VST3_FULLPATH" "$VST3_INSTALL_DIR/" || {
        error "VST3プラグインのコピーに失敗しました"
    }
fi

success "インストールが完了しました！"
echo ""
echo "VST3プラグインは以下の場所にインストールされました:"
echo "  $VST3_INSTALL_DIR/$VST3_FILENAME"

if [[ "$OS" == "macos" && -n "${AU_OUTPUT:-}" ]]; then
    echo "AUインストールディレクトリを準備しています..."
    mkdir -p "$AU_INSTALL_DIR" || {
        error "AUインストールディレクトリを作成できませんでした: $AU_INSTALL_DIR"
    }

    echo "AUプラグインをインストールしています..."
    rm -rf "$AU_INSTALL_DIR/$AU_FILENAME"
    cp -r "$AU_FULLPATH" "$AU_INSTALL_DIR/" || {
        error "AUプラグインのコピーに失敗しました"
    }

    echo "AUプラグインは以下の場所にインストールされました:"
    echo "  $AU_INSTALL_DIR/$AU_FILENAME"
fi
echo ""

if [ "$CLAP_INSTALLED" = false ]; then
    warning "注意: VST3を使用するには、$CLAP_FILE がインストールされている必要があります。"
fi

# DAWのスキャンに関する注意
echo ""
echo "次のステップ:"
echo "1. DAWを起動（または再起動）してください"
echo "2. DAWのプラグインスキャンを実行してください"
echo "3. $OUTPUT_NAME がVST3プラグインとして表示されるはずです"
