#!/bin/bash
# build.sh - gain_plugin の CLAP ビルド
#
# このスクリプトは以下の 3 ステップを実行します:
#   1. GUI フロントエンド（src-gui）の npm ビルド
#   2. Rust プラグイン（src-plugin）の cargo ビルド
#   3. ビルド成果物を .clap バンドル形式にパッケージング
#
# .clap バンドルとは:
#   CLAP ホスト（DAW）がプラグインとして認識できる形式のファイル/ディレクトリ。
#   macOS ではバンドル（.app に似たディレクトリ構造）、
#   Windows/Linux では単一の .dll/.so ファイルになる。
#
# 使い方:
#   ./script/build.sh [Debug|Release]
#
# 引数:
#   Debug|Release - ビルド構成（省略時は Debug）
#
# 出力:
#   target/bundled/WXP Example Gain.clap

set -e  # エラー発生時にスクリプトを停止
set -u  # 未定義変数の参照をエラーにする

# ---------------------------------------------------------------------------
# OS 検出
# ---------------------------------------------------------------------------
# バンドル形式が OS ごとに異なるため、最初に判定しておく。
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
# ビルド構成の決定
# ---------------------------------------------------------------------------
BUILD_CONFIG="Debug"
CARGO_BUILD_FLAG=""
if [ $# -eq 1 ]; then
    case "$1" in
        Debug|debug|DEBUG)
            BUILD_CONFIG="Debug"
            ;;
        Release|release|RELEASE)
            BUILD_CONFIG="Release"
            CARGO_BUILD_FLAG="--release"
            ;;
        *)
            echo "エラー: 無効なビルド構成: $1"
            exit 1
            ;;
    esac
fi

echo "ビルド構成: $BUILD_CONFIG"

if [ "$BUILD_CONFIG" = "Debug" ]; then
    PROFILE_DIR="debug"
else
    PROFILE_DIR="release"
fi

# ---------------------------------------------------------------------------
# パスの解決
# ---------------------------------------------------------------------------
# BASH_SOURCE[0] からスクリプト自身の絶対パスを求め、そこから相対的に各ディレクトリを特定する。
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PLUGIN_ROOT="$( cd "$SCRIPT_DIR/.." && pwd )"
GUI_DIR="$PLUGIN_ROOT/src-gui"
TARGET_DIR="${CARGO_TARGET_DIR:-$PLUGIN_ROOT/target}"
BUILD_DIR="$TARGET_DIR/$PROFILE_DIR"

# ---------------------------------------------------------------------------
# ステップ 1: GUI フロントエンドのビルド
# ---------------------------------------------------------------------------
# Vite で TypeScript/CSS をバンドルし、src-gui/dist/ に出力する。
# リリースビルドでは build.rs がこの dist/ を ZIP 化してバイナリに埋め込む。
echo "GUI をビルドしています..."
(
    cd "$GUI_DIR"
    npm install
    npm run build
)

# ---------------------------------------------------------------------------
# ステップ 2: Rust プラグインのビルド
# ---------------------------------------------------------------------------
# cargo が生成する共有ライブラリ:
#   macOS:   libwxp_example_gain_plugin.dylib
#   Windows: wxp_example_gain_plugin.dll
#   Linux:   libwxp_example_gain_plugin.so
echo "プラグインをビルドしています..."
(
    if [ "$OS" = "macos" ]; then
        MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}" \
        WRY_OBJC_SUFFIX="${WRY_OBJC_SUFFIX:-WxpExampleGainPlugin}" \
        cargo build --target-dir "$TARGET_DIR" --manifest-path "$PLUGIN_ROOT/src-plugin/Cargo.toml" $CARGO_BUILD_FLAG
    else
        cargo build --target-dir "$TARGET_DIR" --manifest-path "$PLUGIN_ROOT/src-plugin/Cargo.toml" $CARGO_BUILD_FLAG
    fi
)

# ---------------------------------------------------------------------------
# ステップ 3: .clap バンドルの作成
# ---------------------------------------------------------------------------
# CLAP プラグインの配布形式は OS ごとに異なる:
#   macOS:   macOS バンドル（ディレクトリ構造 + Info.plist）
#   Windows: .dll を .clap にリネーム
#   Linux:   .so を .clap にリネーム
PLUGIN_NAME="WXP Example Gain.clap"
BUNDLE_DIR="$TARGET_DIR/bundled/$PLUGIN_NAME"

echo "バンドル構造を作成しています..."
rm -rf "$BUNDLE_DIR"

case "$OS" in
    macos)
        # macOS の .clap は .app と同様のバンドル構造を持つ。
        # Contents/MacOS/ に実行可能バイナリ、Contents/Info.plist にメタデータを配置する。
        mkdir -p "$BUNDLE_DIR/Contents/MacOS"

        # Info.plist: macOS がバンドルを識別するためのメタデータファイル。
        # CFBundleIdentifier はプラグインの PLUGIN_ID と一致させる。
        cat > "$BUNDLE_DIR/Contents/Info.plist" << EOF
<?xml version="1.0" encoding="UTF-8"?>

<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist>
  <dict>
    <key>CFBundleExecutable</key>
    <string>WXP Example Gain</string>
    <key>CFBundleIconFile</key>
    <string></string>
    <key>CFBundleIdentifier</key>
    <string>com.novo-notes.wxp-example-gain</string>
    <key>CFBundleName</key>
    <string>WXP Example Gain</string>
    <key>CFBundleDisplayName</key>
    <string>WXP Example Gain</string>
    <key>CFBundlePackageType</key>
    <string>BNDL</string>
    <key>CFBundleSignature</key>
    <string>????</string>
    <key>CFBundleShortVersionString</key>
    <string>1.0.0</string>
    <key>CFBundleVersion</key>
    <string>1.0.0</string>
    <key>NSHumanReadableCopyright</key>
    <string></string>
    <key>NSHighResolutionCapable</key>
    <true/>
  </dict>
</plist>
EOF

        # PkgInfo: macOS の古い慣習で必要なファイル。
        # "BNDL" はバンドルタイプ、"????" はクリエータコード（汎用）。
        echo -n "BNDL????" > "$BUNDLE_DIR/Contents/PkgInfo"

        # .dylib を拡張子なしのバイナリ名でコピーする。
        # CLAP ホストは CFBundleExecutable に指定した名前でバイナリを探す。
        cp "$BUILD_DIR/libwxp_example_gain_plugin.dylib" \
            "$BUNDLE_DIR/Contents/MacOS/WXP Example Gain"

        # install_name_tool: dylib の LC_ID_DYLIB（ライブラリの自己識別パス）を書き換える。
        # "@loader_path/..." にすることで、バンドル内の相対パスで自己参照できるようになり、
        # インストール先のパスに依存しないポータブルなバンドルになる。
        install_name_tool -id "@loader_path/WXP Example Gain" \
            "$BUNDLE_DIR/Contents/MacOS/WXP Example Gain"
        ;;
    windows)
        # Windows では .dll をそのまま .clap として配置するだけでよい。
        mkdir -p "$TARGET_DIR/bundled"
        cp "$BUILD_DIR/wxp_example_gain_plugin.dll" "$BUNDLE_DIR"
        ;;
    linux)
        # Linux も同様に .so を .clap として配置する。
        mkdir -p "$TARGET_DIR/bundled"
        cp "$BUILD_DIR/libwxp_example_gain_plugin.so" "$BUNDLE_DIR"
        ;;
esac

if [ ! -e "$BUNDLE_DIR" ]; then
    echo "エラー: ビルドは成功しましたが、バンドルされたプラグインが見つかりません"
    exit 1
fi

echo "ビルドが完了しました！"
echo "バンドルは以下の場所に作成されました: target/bundled/$PLUGIN_NAME"
