#!/bin/bash
# build_wrapper_plugin.sh - 任意の CLAP plugin から VST3/AU wrapper を build する
#
# 使い方:
#   ./build_wrapper_plugin.sh <CLAP file> <output plugin name> [Debug|Release]
#
# 引数:
#   CLAP file     - CLAP plugin filename (例: "example_plugin_nih.clap")
#   Output name   - VST3/AU で使う表示名 (例: "Example Plugin NIH")
#   Debug|Release - build configuration (default: Debug)
#
# 例:
#   ./build_wrapper_plugin.sh example_plugin_nih.clap "Example Plugin NIH" Release
#   ./build_wrapper_plugin.sh "XDevice Editor.clap" "XDevice Editor" Debug

set -Eeuo pipefail

# 色付き出力用の定数
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # 色なし

# error message 出力
error() {
    echo -e "${RED}Error: $1${NC}" >&2
    exit 1
}

on_error() {
    local exit_code="$1"
    local line_no="$2"
    local command="$3"
    echo -e "${RED}Error: command failed at line ${line_no} (exit=${exit_code}): ${command}${NC}" >&2
    exit "$exit_code"
}

trap 'on_error $? $LINENO "$BASH_COMMAND"' ERR

# success message 出力
success() {
    echo -e "${GREEN}$1${NC}"
}

# warning message 出力
warning() {
    echo -e "${YELLOW}Warning: $1${NC}"
}

# この script 自身の directory を保持する
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"

# usage 表示
usage() {
    echo "Usage: $0 <CLAP file> <output plugin name> [Debug|Release]"
    echo "  If omitted, build configuration defaults to Debug"
    echo ""
    echo "Examples:"
    echo "  $0 example_plugin_nih.clap \"Example Plugin NIH\" Release"
    echo "  $0 \"XDevice Editor.clap\" \"XDevice Editor\" Debug"
    exit 1
}

# 引数 parsing
if [ $# -lt 2 ]; then
    usage
fi

CLAP_FILE="$1"
OUTPUT_NAME="$2"
BUILD_CONFIG="Debug"
BUILD_AAX="${CLAP_WRAPPER_BUILDER_BUILD_AAX:-}"
AAX_SDK_ROOT="${CLAP_WRAPPER_BUILDER_AAX_SDK_ROOT:-${AAX_SDK_ROOT:-}}"
DOWNLOAD_DEPENDENCIES="${CLAP_WRAPPER_DOWNLOAD_DEPENDENCIES:-OFF}"
AUV2_INSTRUMENT_TYPE="${CLAP_WRAPPER_AUV2_INSTRUMENT_TYPE:-aufx}"
AUV2_MANUFACTURER_NAME="${CLAP_WRAPPER_AUV2_MANUFACTURER_NAME:-Your Company}"
AUV2_MANUFACTURER_CODE="${CLAP_WRAPPER_AUV2_MANUFACTURER_CODE:-YrCo}"
AUV2_SUBTYPE_CODE="${CLAP_WRAPPER_AUV2_SUBTYPE_CODE:-WtGn}"

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
            error "Invalid build configuration: $3"
            ;;
    esac
fi

echo "CLAP file: $CLAP_FILE"
echo "Output plugin name: $OUTPUT_NAME"
echo "Build configuration: $BUILD_CONFIG"

CLAP_FULLPATH="$(cd "$(dirname "$CLAP_FILE")" && pwd)/$(basename "$CLAP_FILE")"

if [ -z "$BUILD_AAX" ]; then
    if [ -n "$AAX_SDK_ROOT" ] || [ "$DOWNLOAD_DEPENDENCIES" = "ON" ]; then
        BUILD_AAX="ON"
    else
        BUILD_AAX="OFF"
    fi
fi

echo "AAX build: $BUILD_AAX"
if [ -n "$AAX_SDK_ROOT" ]; then
    echo "AAX SDK root: $AAX_SDK_ROOT"
fi
if [[ "$OSTYPE" == darwin* ]]; then
    echo "AUv2 type/subtype/manufacturer: $AUV2_INSTRUMENT_TYPE/$AUV2_SUBTYPE_CODE/$AUV2_MANUFACTURER_CODE"
fi

# CLAP filename から拡張子を落とし、space は underscore に寄せる
# path component は落として filename だけを使う
CLAP_FILE_BASENAME=$(basename "$CLAP_FILE")
CLAP_BASE_NAME="${CLAP_FILE_BASENAME%.clap}"
CLAP_BASE_NAME="${CLAP_BASE_NAME// /_}"

# clap-wrapper directory の存在確認
if [ ! -d "$SCRIPT_DIR/clap-wrapper" ]; then
    error "clap-wrapper directory not found. Run: git clone https://github.com/free-audio/clap-wrapper.git"
fi

# clap SDK submodule を使う
CLAP_SDK_ROOT="$SCRIPT_DIR/clap"
if [ ! -d "$CLAP_SDK_ROOT" ]; then
    error "clap submodule not found. Run: git submodule update --init --recursive"
fi
success "CLAP SDK submodule found: $CLAP_SDK_ROOT"

# VST3 SDK submodule を使う
VST3_SDK_ROOT="$SCRIPT_DIR/vst3sdk"
if [ ! -d "$VST3_SDK_ROOT" ]; then
    error "vst3sdk submodule not found. Run: git submodule update --init --recursive"
fi
success "VST3 SDK submodule found: $VST3_SDK_ROOT"

# AU SDK submodule を使う
if [[ "$OSTYPE" == darwin* ]]; then
    AU_SDK_ROOT="$SCRIPT_DIR/AudioUnitSDK"
    if [[ ! -d "$AU_SDK_ROOT" ]]; then
        error "AudioUnitSDK submodule not found. Run: git submodule update --init --recursive"
    fi
    success "AU SDK submodule found: $AU_SDK_ROOT"
else
    AU_SDK_ROOT=
fi

# OS 判定と generator 選択
CMAKE_GENERATOR=""

case "$OSTYPE" in
    darwin*)
        # macOS
        if command -v xcodebuild &> /dev/null; then
            CMAKE_GENERATOR="Xcode"
            success "Detected macOS: using Xcode"
        else
            error "Xcode not found. Install Xcode or Command Line Tools."
        fi
        ;;
    linux*)
        # Linux
        CMAKE_GENERATOR="Unix Makefiles"
        success "Detected Linux: using Unix Makefiles"
        ;;
    msys*|cygwin*|mingw*)
        # Windows
        # Visual Studio は CMake に検出させる
        CMAKE_GENERATOR="Visual Studio 17 2022"
        success "Detected Windows: using Visual Studio 2022"
        ;;
    *)
        CMAKE_GENERATOR="Unix Makefiles"
        warning "Unknown OS: using Unix Makefiles"
        ;;
esac

# clap_wrapper_builder 配下に build directory を作る
BUILD_DIR="$SCRIPT_DIR/build_$CLAP_BASE_NAME"

# CMakeCache が古い source path を持つ場合は作り直す
if [ -f "$BUILD_DIR/CMakeCache.txt" ] && ! grep -Fq "$SCRIPT_DIR/clap-wrapper" "$BUILD_DIR/CMakeCache.txt"; then
    warning "Removing stale CMake cache that does not match current source directory: $BUILD_DIR"
    rm -rf "$BUILD_DIR"
fi

# CMake configure
echo "Configuring CMake..."
if [[ "$OSTYPE" == darwin* ]]; then
    # macOS では Universal Binary として build する
    cmake -S "$SCRIPT_DIR/clap-wrapper" -B "$BUILD_DIR" \
        -DCLAP_SDK_ROOT="$CLAP_SDK_ROOT" \
        -DVST3_SDK_ROOT="$VST3_SDK_ROOT" \
        -DCLAP_WRAPPER_OUTPUT_NAME="$OUTPUT_NAME" \
        -DCMAKE_BUILD_TYPE="$BUILD_CONFIG" \
        -DCMAKE_OSX_ARCHITECTURES="x86_64;arm64" \
        -DCLAP_WRAPPER_BUILD_AAX="$BUILD_AAX" \
        -DCLAP_WRAPPER_BUILD_AUV2=ON \
        -DCLAP_WRAPPER_BUILD_STANDALONE=OFF \
        -DCLAP_WRAPPER_BUILD_TESTS=OFF \
        -DCLAP_WRAPPER_DOWNLOAD_DEPENDENCIES="$DOWNLOAD_DEPENDENCIES" \
        -DAAX_SDK_ROOT="$AAX_SDK_ROOT" \
        -DAUDIOUNIT_SDK_ROOT="$AU_SDK_ROOT" \
        -DCLAP_WRAPPER_MACOS_EMBEDDED_CLAP_LOCATION="$CLAP_FULLPATH" \
        -DCLAP_WRAPPER_AUV2_INSTRUMENT_TYPE="$AUV2_INSTRUMENT_TYPE" \
        -DCLAP_WRAPPER_AUV2_MANUFACTURER_NAME="$AUV2_MANUFACTURER_NAME" \
        -DCLAP_WRAPPER_AUV2_MANUFACTURER_CODE="$AUV2_MANUFACTURER_CODE" \
        -DCLAP_WRAPPER_AUV2_SUBTYPE_CODE="$AUV2_SUBTYPE_CODE" \
        -DCLAP_WRAPPER_CXX_STANDARD=23 \
        -G "$CMAKE_GENERATOR"
else
    # macOS 以外
    cmake -S "$SCRIPT_DIR/clap-wrapper" -B "$BUILD_DIR" \
        -DCLAP_SDK_ROOT="$CLAP_SDK_ROOT" \
        -DVST3_SDK_ROOT="$VST3_SDK_ROOT" \
        -DCLAP_WRAPPER_OUTPUT_NAME="$OUTPUT_NAME" \
        -DCMAKE_BUILD_TYPE="$BUILD_CONFIG" \
        -DCLAP_WRAPPER_BUILD_AAX="$BUILD_AAX" \
        -DCLAP_WRAPPER_BUILD_STANDALONE=OFF \
        -DCLAP_WRAPPER_BUILD_TESTS=OFF \
        -DCLAP_WRAPPER_DOWNLOAD_DEPENDENCIES="$DOWNLOAD_DEPENDENCIES" \
        -DAAX_SDK_ROOT="$AAX_SDK_ROOT" \
        -G "$CMAKE_GENERATOR"
fi

success "CMake configuration complete"

# build 実行
echo "Building..."

# AudioUnitSDK header は GNU statement expression を使うため、clap-wrapper の
# -Wpedantic -Werror と衝突する。Xcode build ではその warning を抑制する。
if [[ "$CMAKE_GENERATOR" == "Xcode" ]]; then
    XCODE_FLAGS=('--' 'OTHER_CPLUSPLUSFLAGS=$(inherited) -Wno-gnu-statement-expression-from-macro-expansion -Wno-shorten-64-to-32')
    # macOS で xcbeautify がある場合だけ pipe する
    if command -v xcbeautify &> /dev/null; then
        cmake --build "$BUILD_DIR" --config "$BUILD_CONFIG" "${XCODE_FLAGS[@]}" 2>&1 | xcbeautify --quiet
    else
        cmake --build "$BUILD_DIR" --config "$BUILD_CONFIG" "${XCODE_FLAGS[@]}"
    fi
elif [[ "$CMAKE_GENERATOR" == "Visual Studio 17 2022" ]]; then
    cmake --build "$BUILD_DIR" --config "$BUILD_CONFIG"
else
    cmake --build "$BUILD_DIR"
fi
success "Build complete"

if [[ "$OSTYPE" == darwin* ]]; then
    AUV2_GENERATED_PLIST=$(find "$BUILD_DIR" -path "*/${CLAP_BASE_NAME}_as_auv2-build-helper-output/auv2_Info.plist" -type f 2>/dev/null | head -n 1 || true)
    AUV2_OUTPUT=$(find "$BUILD_DIR/$BUILD_CONFIG" -name "$OUTPUT_NAME.component" -type d 2>/dev/null | head -n 1 || true)
    if [[ -n "$AUV2_GENERATED_PLIST" && -n "$AUV2_OUTPUT" ]]; then
        # Xcode incremental build では generated Info.plist が後段で上書きされることがある。
        # auval が AudioComponents を読める状態を最後に保証してから署名し直す。
        cp "$AUV2_GENERATED_PLIST" "$AUV2_OUTPUT/Contents/Info.plist"
        if [[ -d "$AUV2_OUTPUT/Contents/PlugIns/$OUTPUT_NAME.clap" ]]; then
            codesign --force --sign - --timestamp=none "$AUV2_OUTPUT/Contents/PlugIns/$OUTPUT_NAME.clap"
        fi
        codesign --force --sign - --timestamp=none "$AUV2_OUTPUT"
    fi

    VST3_OUTPUT_FOR_SIGN=$(find "$BUILD_DIR/$BUILD_CONFIG" -name "$OUTPUT_NAME.vst3" -type d 2>/dev/null | head -n 1 || true)
    if [[ -n "$VST3_OUTPUT_FOR_SIGN" ]]; then
        if [[ -d "$VST3_OUTPUT_FOR_SIGN/Contents/PlugIns/$OUTPUT_NAME.clap" ]]; then
            codesign --force --sign - --timestamp=none "$VST3_OUTPUT_FOR_SIGN/Contents/PlugIns/$OUTPUT_NAME.clap"
        fi
        codesign --force --sign - --timestamp=none "$VST3_OUTPUT_FOR_SIGN"
    fi
fi

# build output の確認
VST3_OUTPUT=""
if [[ "$CMAKE_GENERATOR" == "Xcode" ]] || [[ "$CMAKE_GENERATOR" == "Visual Studio 17 2022" ]]; then
    # multi-configuration generator では Configuration subdirectory を見る
    if [[ "$OSTYPE" == darwin* ]]; then
        VST3_OUTPUT=$(find "$BUILD_DIR/$BUILD_CONFIG" -name "*.vst3" -type d 2>/dev/null | head -n 1)
    else
        VST3_OUTPUT=$(find "$BUILD_DIR/$BUILD_CONFIG" -name "*.vst3" -type f 2>/dev/null | head -n 1)
    fi
else
    # single-configuration generator の場合
    if [[ "$OSTYPE" == darwin* ]]; then
        VST3_OUTPUT=$(find "$BUILD_DIR" -name "*.vst3" -type d | head -n 1)
    else
        VST3_OUTPUT=$(find "$BUILD_DIR" -name "*.vst3" -type f | head -n 1)
    fi
fi

if [ -n "$VST3_OUTPUT" ]; then
    # full path に解決する
    VST3_FULLPATH="$(cd "$(dirname "$VST3_OUTPUT")" && pwd)/$(basename "$VST3_OUTPUT")"
    success "VST3 plugin generated: $VST3_FULLPATH"
else
    error "VST3 plugin not found"
fi

if [ "$BUILD_AAX" = "ON" ]; then
    AAX_OUTPUT=""
    if [[ "$CMAKE_GENERATOR" == "Xcode" ]] || [[ "$CMAKE_GENERATOR" == "Visual Studio 17 2022" ]]; then
        AAX_OUTPUT=$(find "$BUILD_DIR" -name "*.aaxplugin" \( -type d -o -type f \) 2>/dev/null | head -n 1)
    else
        AAX_OUTPUT=$(find "$BUILD_DIR" -name "*.aaxplugin" \( -type d -o -type f \) 2>/dev/null | head -n 1)
    fi

    if [ -n "$AAX_OUTPUT" ]; then
        AAX_FULLPATH="$(cd "$(dirname "$AAX_OUTPUT")" && pwd)/$(basename "$AAX_OUTPUT")"
        success "AAX plugin generated: $AAX_FULLPATH"
    else
        error "AAX plugin not found"
    fi
fi
