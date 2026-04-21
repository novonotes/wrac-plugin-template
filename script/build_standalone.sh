#!/bin/bash
# build_standalone.sh - Build standalone app for gain_plugin
#
# Usage:
#   ./script/build_standalone.sh [Debug|Release]
#
# Environment variables:
#   Set SKIP_CLAP_BUILD=1 to skip the preceding CLAP build.
#   Override the standalone Plugin ID with WXP_EXAMPLE_GAIN_STANDALONE_PLUGIN_ID.

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
        echo "Error: Unsupported OS $(uname -s)"
        exit 1
        ;;
esac

BUILD_CONFIG="${1:-Debug}"

case "$BUILD_CONFIG" in
    Debug|debug|DEBUG)
        BUILD_CONFIG="Debug"
        PROFILE_DIR="debug"
        ;;
    Release|release|RELEASE)
        BUILD_CONFIG="Release"
        PROFILE_DIR="release"
        ;;
    *)
        echo "Error: Invalid build configuration: $BUILD_CONFIG"
        exit 1
        ;;
esac

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PLUGIN_ROOT="$( cd "$SCRIPT_DIR/.." && pwd )"
TARGET_DIR="${CARGO_TARGET_DIR:-$PLUGIN_ROOT/target}"
BUILD_DIR="$TARGET_DIR/$PROFILE_DIR"
DEFAULT_WRAPPER_DIR="$( cd "$PLUGIN_ROOT/clap_wrapper_builder" 2>/dev/null && pwd || true )"
WRAPPER_DIR="${CLAP_WRAPPER_DIR:-$DEFAULT_WRAPPER_DIR}"
STANDALONE_PLUGIN_ID="${WXP_EXAMPLE_GAIN_STANDALONE_PLUGIN_ID:-com.novo-notes.wxp-example-gain}"
STANDALONE_OUTPUT_NAME="${WXP_EXAMPLE_GAIN_STANDALONE_OUTPUT_NAME:-WXP Example Gain Standalone}"

if [[ -z "$WRAPPER_DIR" || ! -d "$WRAPPER_DIR" ]]; then
    echo "Error: clap_wrapper_builder not found"
    echo "Set the CLAP_WRAPPER_DIR environment variable to the path of clap_wrapper_builder"
    exit 1
fi

if [[ "${SKIP_CLAP_BUILD:-0}" != "1" ]]; then
    echo "Building CLAP plugin first..."
    "$SCRIPT_DIR/build.sh" "$BUILD_CONFIG"
fi

if [[ "$OS" == "linux" ]]; then
    echo "Skipping standalone wrapper build on Linux"
    exit 0
fi

if [[ "$OSTYPE" =~ ^(msys|cygwin|mingw).* ]]; then
    LIB_FILE_NAME="wxp_example_gain_plugin.lib"
else
    LIB_FILE_NAME="libwxp_example_gain_plugin.a"
fi

echo "Building standalone wrapper..."
(
    cd "$WRAPPER_DIR"
    CLAP_WRAPPER_BUILDER_BUILD_VST3=OFF \
    CLAP_WRAPPER_BUILDER_BUILD_AUV2=OFF \
    CLAP_WRAPPER_STANDALONE_PLUGIN_ID="$STANDALONE_PLUGIN_ID" \
    CLAP_WRAPPER_STANDALONE_OUTPUT_NAME="$STANDALONE_OUTPUT_NAME" \
    ./build_wrapper_plugin_static.sh "$BUILD_DIR/$LIB_FILE_NAME" "WXP Example Gain Static" "$BUILD_CONFIG"
)

WRAPPER_BUILD_BASE="${LIB_FILE_NAME%.a}"
WRAPPER_BUILD_BASE="${WRAPPER_BUILD_BASE%.lib}"
WRAPPER_BUILD_DIR="$WRAPPER_DIR/build_${WRAPPER_BUILD_BASE}_static"
STANDALONE_TARGET_DIR="$TARGET_DIR/standalone/$BUILD_CONFIG"
mkdir -p "$STANDALONE_TARGET_DIR"

if [[ "$OS" == "macos" ]]; then
    STANDALONE_SOURCE=$(find "$WRAPPER_BUILD_DIR" -path "*/$BUILD_CONFIG/${STANDALONE_OUTPUT_NAME}.app" -type d 2>/dev/null | head -n 1 || true)
    if [[ -z "$STANDALONE_SOURCE" ]]; then
        STANDALONE_SOURCE=$(find "$WRAPPER_BUILD_DIR" -path "*/${STANDALONE_OUTPUT_NAME}.app" -type d 2>/dev/null | head -n 1 || true)
    fi
    if [[ -z "$STANDALONE_SOURCE" ]]; then
        echo "Error: Standalone app not found"
        exit 1
    fi

    rm -rf "$STANDALONE_TARGET_DIR/${STANDALONE_OUTPUT_NAME}.app"
    ln -s "$STANDALONE_SOURCE" "$STANDALONE_TARGET_DIR/${STANDALONE_OUTPUT_NAME}.app"
    echo "Created symlink to standalone app: $STANDALONE_TARGET_DIR/${STANDALONE_OUTPUT_NAME}.app"
elif [[ "$OS" == "windows" ]]; then
    STANDALONE_SOURCE=$(find "$WRAPPER_BUILD_DIR" -path "*/$BUILD_CONFIG/${STANDALONE_OUTPUT_NAME}.exe" -type f 2>/dev/null | head -n 1 || true)
    if [[ -z "$STANDALONE_SOURCE" ]]; then
        STANDALONE_SOURCE=$(find "$WRAPPER_BUILD_DIR" -path "*/${STANDALONE_OUTPUT_NAME}.exe" -type f 2>/dev/null | head -n 1 || true)
    fi
    if [[ -z "$STANDALONE_SOURCE" ]]; then
        echo "Error: Standalone app not found"
        exit 1
    fi

    cp "$STANDALONE_SOURCE" "$STANDALONE_TARGET_DIR/${STANDALONE_OUTPUT_NAME}.exe"
    echo "Copied standalone app: $STANDALONE_TARGET_DIR/${STANDALONE_OUTPUT_NAME}.exe"
fi
