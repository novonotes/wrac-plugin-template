#!/bin/bash
# build_and_install.sh - Build and install gain_plugin (all-in-one)
#
# Builds and installs the CLAP, and also handles VST3 / AU and standalone.
#
# Usage:
#   ./script/build_and_install.sh [Debug|Release]
#
# Arguments:
#   Debug|Release - Build configuration (default: Debug)

set -e  # Stop script on error
set -u

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PLUGIN_ROOT="$( cd "$SCRIPT_DIR/.." && pwd )"
TARGET_DIR="${CARGO_TARGET_DIR:-$PLUGIN_ROOT/target}"
DEFAULT_WRAPPER_DIR="$( cd "$PLUGIN_ROOT/clap_wrapper_builder" 2>/dev/null && pwd || true )"
WRAPPER_DIR="${CLAP_WRAPPER_DIR:-$DEFAULT_WRAPPER_DIR}"

# Escape codes for terminal color output
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'  # No Color (reset)

# Set BUILD_CONFIG from the first argument, defaulting to "Debug".
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
        echo "Error: Unsupported OS $(uname -s)"
        exit 1
        ;;
esac

echo -e "${BLUE}Installing gain_plugin (CLAP + wrapper)...${NC}"
echo "Build configuration: $BUILD_CONFIG"
echo "Wrapper build: ${CLAP_ONLY:+skip}(CLAP_ONLY=${CLAP_ONLY:-0})"
echo ""

echo "1. Building CLAP plugin..."
"$SCRIPT_DIR/build.sh" "$BUILD_CONFIG"

echo ""
echo "2. Installing CLAP plugin..."
"$SCRIPT_DIR/install.sh"

if [[ "$OS" != "linux" && "${CLAP_ONLY:-0}" != "1" ]]; then
    if [[ -z "$WRAPPER_DIR" || ! -d "$WRAPPER_DIR" ]]; then
        echo "Error: clap_wrapper_builder not found"
        echo "Set the CLAP_WRAPPER_DIR environment variable to the path of clap_wrapper_builder"
        exit 1
    fi

    echo ""
    echo "3. Building VST3 / AU wrapper..."
    SKIP_CLAP_BUILD=1 "$SCRIPT_DIR/build_wrapper.sh" "$BUILD_CONFIG"

    echo ""
    echo "4. Installing VST3 / AU wrapper..."
    (
        cd "$WRAPPER_DIR"
        ./install_wrapper_plugin.sh "$TARGET_DIR/bundled/WXP Example Gain.clap" "WXP Example Gain" "$BUILD_CONFIG"
    )

    if [[ "${BUILD_STANDALONE:-1}" == "1" ]]; then
        echo ""
        echo "5. Building standalone app..."
        SKIP_CLAP_BUILD=1 "$SCRIPT_DIR/build_standalone.sh" "$BUILD_CONFIG"
    else
        echo ""
        echo "5. BUILD_STANDALONE=0: skipping standalone app build"
    fi
else
    echo ""
    if [[ "${CLAP_ONLY:-0}" == "1" ]]; then
        echo "3. CLAP_ONLY=1: skipping VST3 / AU / standalone"
    else
        echo "3. Skipping VST3 / AU / standalone on Linux"
    fi
fi

echo ""
echo -e "${GREEN}gain_plugin installation complete!${NC}"
echo "Installed plugins:"
echo "  - WXP Example Gain.clap (CLAP)"
if [[ "$OS" != "linux" && "${CLAP_ONLY:-0}" != "1" ]]; then
    echo "  - WXP Example Gain.vst3 (VST3)"
    if [[ "$OS" == "macos" ]]; then
        echo "  - WXP Example Gain.component (AU)"
    fi
    if [[ "${BUILD_STANDALONE:-1}" == "1" ]]; then
        echo "  - WXP Example Gain Standalone (build only)"
    fi
fi
