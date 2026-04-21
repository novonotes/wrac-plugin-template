#!/bin/bash
# build_wrapper.sh - Build VST3 / AU wrapper for gain_plugin
#
# Usage:
#   ./script/build_wrapper.sh [Debug|Release]
#
# Environment variables:
#   Set SKIP_CLAP_BUILD=1 to skip the preceding CLAP build.

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
        ;;
    Release|release|RELEASE)
        BUILD_CONFIG="Release"
        ;;
    *)
        echo "Error: Invalid build configuration: $BUILD_CONFIG"
        exit 1
        ;;
esac

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PLUGIN_ROOT="$( cd "$SCRIPT_DIR/.." && pwd )"
TARGET_DIR="${CARGO_TARGET_DIR:-$PLUGIN_ROOT/target}"
DEFAULT_WRAPPER_DIR="$( cd "$PLUGIN_ROOT/clap_wrapper_builder" 2>/dev/null && pwd || true )"
WRAPPER_DIR="${CLAP_WRAPPER_DIR:-$DEFAULT_WRAPPER_DIR}"

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
    echo "Skipping VST3 / AU wrapper build on Linux"
    exit 0
fi

echo "Building VST3 / AU wrapper..."
(
    cd "$WRAPPER_DIR"
    ./build_wrapper_plugin.sh "$TARGET_DIR/bundled/WXP Example Gain.clap" "WXP Example Gain" "$BUILD_CONFIG"
)

echo "VST3 / AU wrapper build complete"
