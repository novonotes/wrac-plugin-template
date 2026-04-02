#!/bin/bash
# install.sh - gain_plugin の CLAP インストール
#
# build.sh で作成した .clap バンドルを、OS ごとの CLAP プラグインディレクトリにコピーする。
# DAW はこのディレクトリを自動スキャンしてプラグインを検出する。
#
# インストール先:
#   macOS:   ~/Library/Audio/Plug-Ins/CLAP/
#   Windows: %LOCALAPPDATA%/Programs/Common/CLAP/
#   Linux:   ~/.clap/
#
# 使い方:
#   ./script/install.sh
#
# 注意: 実行前に build.sh でバンドルを作成しておく必要がある。

set -e  # エラー発生時にスクリプトを停止
set -u  # 未定義変数の参照をエラーにする

# ---------------------------------------------------------------------------
# OS 検出
# ---------------------------------------------------------------------------
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

echo "検出されたOS: $OS"

# ---------------------------------------------------------------------------
# パスの解決
# ---------------------------------------------------------------------------
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PLUGIN_ROOT="$( cd "$SCRIPT_DIR/.." && pwd )"
TARGET_DIR="${CARGO_TARGET_DIR:-$PLUGIN_ROOT/target}"

BUNDLE_NAME="WXP Example Gain.clap"
BUNDLE_PATH="$TARGET_DIR/bundled/${BUNDLE_NAME}"

# ---------------------------------------------------------------------------
# バンドルの存在確認
# ---------------------------------------------------------------------------
# インストール前に build.sh が正常完了しているかを確認する。
if [ ! -e "$BUNDLE_PATH" ]; then
    echo "エラー: バンドルが見つかりません: $BUNDLE_PATH"
    echo "先に build.sh を実行してください"
    exit 1
fi

# ---------------------------------------------------------------------------
# OS 別インストール
# ---------------------------------------------------------------------------
case "$OS" in
    macos)
        # macOS の CLAP 標準ディレクトリ。
        # Logic Pro、Ableton Live 等の多くの DAW がここをスキャンする。
        echo "インストールディレクトリを準備しています..."
        mkdir -p ~/Library/Audio/Plug-Ins/CLAP || {
            echo "エラー: CLAPプラグインディレクトリを作成できませんでした"
            exit 1
        }

        # 既存バージョンを削除してから上書きする（macOS バンドルはディレクトリのため）。
        if [ -e ~/Library/Audio/Plug-Ins/CLAP/"${BUNDLE_NAME}" ]; then
            rm -rf ~/Library/Audio/Plug-Ins/CLAP/"${BUNDLE_NAME}"
        fi

        echo "プラグインをインストールしています..."
        # macOS バンドルはディレクトリ構造のため -r（再帰コピー）が必要。
        cp -r "$BUNDLE_PATH" ~/Library/Audio/Plug-Ins/CLAP/ || {
            echo "エラー: プラグインのコピーに失敗しました"
            exit 1
        }

        echo "インストールが完了しました！"
        echo "プラグインは以下の場所にインストールされました: ~/Library/Audio/Plug-Ins/CLAP/${BUNDLE_NAME}"
        ;;
    windows)
        # Windows の CLAP 標準ディレクトリ（ユーザーローカル）。
        # %PROGRAMFILES%/Common Files/CLAP/ もよく使われるが、管理者権限が不要なこちらを使う。
        CLAP_DIR="$LOCALAPPDATA/Programs/Common/CLAP"

        echo "注意: Program Files へのインストールには管理者権限が必要な場合があります"
        echo "インストールディレクトリを準備しています..."

        mkdir -p "$CLAP_DIR" || {
            echo "エラー: CLAPプラグインディレクトリを作成できませんでした"
            echo "手動で以下のコマンドを実行してください:"
            echo "cp \"$BUNDLE_PATH\" \"$CLAP_DIR/\""
            exit 1
        }

        if [ -e "$CLAP_DIR/${BUNDLE_NAME}" ]; then
            rm -rf "$CLAP_DIR/${BUNDLE_NAME}"
        fi

        echo "プラグインをインストールしています..."
        cp "$BUNDLE_PATH" "$CLAP_DIR/" || {
            echo "エラー: プラグインのコピーに失敗しました"
            echo "手動で以下のコマンドを実行してください:"
            echo "cp \"$BUNDLE_PATH\" \"$CLAP_DIR/\""
            exit 1
        }

        echo "インストールが完了しました！"
        echo "プラグインは以下の場所にインストールされました: $CLAP_DIR/${BUNDLE_NAME}"
        ;;
    linux)
        # Linux の CLAP 標準ディレクトリ（ユーザーローカル）。
        # システム全体にインストールする場合は /usr/lib/clap/ を使うが、
        # root 権限が不要な ~/.clap/ を使う。
        CLAP_DIR="$HOME/.clap"

        echo "インストールディレクトリを準備しています..."
        mkdir -p "$CLAP_DIR" || {
            echo "エラー: CLAPプラグインディレクトリを作成できませんでした"
            exit 1
        }

        if [ -e "$CLAP_DIR/${BUNDLE_NAME}" ]; then
            rm -rf "$CLAP_DIR/${BUNDLE_NAME}"
        fi

        echo "プラグインをインストールしています..."
        cp -r "$BUNDLE_PATH" "$CLAP_DIR/" || {
            echo "エラー: プラグインのコピーに失敗しました"
            exit 1
        }

        echo "インストールが完了しました！"
        echo "プラグインは以下の場所にインストールされました: $CLAP_DIR/${BUNDLE_NAME}"
        ;;
esac
