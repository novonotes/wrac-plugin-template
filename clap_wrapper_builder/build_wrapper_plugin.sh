#!/bin/bash
# build_wrapper_plugin.sh - Build VST3/AU wrapper from any CLAP plugin
#
# Usage:
#   ./build_wrapper_plugin.sh <CLAP file> <output plugin name> [Debug|Release]
#
# Arguments:
#   CLAP file    - CLAP plugin filename (e.g. "example_plugin_nih.clap")
#   Output name  - Display name used in VST3/AU (e.g. "Example Plugin NIH")
#   Debug|Release - Build configuration (default: Debug)
#
# Examples:
#   ./build_wrapper_plugin.sh example_plugin_nih.clap "Example Plugin NIH" Release
#   ./build_wrapper_plugin.sh "XDevice Editor.clap" "XDevice Editor" Debug

set -Eeuo pipefail

# Constants for colored output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Error message function
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

# Success message function
success() {
    echo -e "${GREEN}$1${NC}"
}

# Warning message function
warning() {
    echo -e "${YELLOW}Warning: $1${NC}"
}

# Save the directory of this script
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"

# Usage display function
usage() {
    echo "Usage: $0 <CLAP file> <output plugin name> [Debug|Release]"
    echo "  If omitted, build configuration defaults to Debug"
    echo ""
    echo "Examples:"
    echo "  $0 example_plugin_nih.clap \"Example Plugin NIH\" Release"
    echo "  $0 \"XDevice Editor.clap\" \"XDevice Editor\" Debug"
    exit 1
}

# Argument parsing
if [ $# -lt 2 ]; then
    usage
fi

CLAP_FILE="$1"
OUTPUT_NAME="$2"
BUILD_CONFIG="Debug"
BUILD_AAX="${CLAP_WRAPPER_BUILDER_BUILD_AAX:-}"
AAX_SDK_ROOT="${CLAP_WRAPPER_BUILDER_AAX_SDK_ROOT:-${AAX_SDK_ROOT:-}}"
DOWNLOAD_DEPENDENCIES="${CLAP_WRAPPER_DOWNLOAD_DEPENDENCIES:-OFF}"

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

# Strip extension from CLAP filename and replace spaces with underscores
# Remove path component, keep filename only
CLAP_FILE_BASENAME=$(basename "$CLAP_FILE")
CLAP_BASE_NAME="${CLAP_FILE_BASENAME%.clap}"
CLAP_BASE_NAME="${CLAP_BASE_NAME// /_}"

# Check for clap-wrapper directory
if [ ! -d "$SCRIPT_DIR/clap-wrapper" ]; then
    error "clap-wrapper directory not found. Run: git clone https://github.com/free-audio/clap-wrapper.git"
fi

# Use the clap SDK submodule
CLAP_SDK_ROOT="$SCRIPT_DIR/clap"
if [ ! -d "$CLAP_SDK_ROOT" ]; then
    error "clap submodule not found. Run: git submodule update --init --recursive"
fi
success "CLAP SDK submodule found: $CLAP_SDK_ROOT"

# Use the VST3 SDK submodule
VST3_SDK_ROOT="$SCRIPT_DIR/vst3sdk"
if [ ! -d "$VST3_SDK_ROOT" ]; then
    error "vst3sdk submodule not found. Run: git submodule update --init --recursive"
fi
success "VST3 SDK submodule found: $VST3_SDK_ROOT"

# Use the AU SDK submodule
if [[ "$OSTYPE" == darwin* ]]; then
    AU_SDK_ROOT="$SCRIPT_DIR/AudioUnitSDK"
    if [[ ! -d "$AU_SDK_ROOT" ]]; then
        error "AudioUnitSDK submodule not found. Run: git submodule update --init --recursive"
    fi
    success "AU SDK submodule found: $AU_SDK_ROOT"
else
    AU_SDK_ROOT=
fi

# OS detection and generator selection
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
        # CMake automatically detects Visual Studio
        CMAKE_GENERATOR="Visual Studio 17 2022"
        success "Detected Windows: using Visual Studio 2022"
        ;;
    *)
        CMAKE_GENERATOR="Unix Makefiles"
        warning "Unknown OS: using Unix Makefiles"
        ;;
esac

# Create build directory inside clap_wrapper_builder
BUILD_DIR="$SCRIPT_DIR/build_$CLAP_BASE_NAME"

# Rebuild if the CMakeCache has a stale source path (e.g. after repo rename)
if [ -f "$BUILD_DIR/CMakeCache.txt" ] && ! grep -Fq "$SCRIPT_DIR/clap-wrapper" "$BUILD_DIR/CMakeCache.txt"; then
    warning "Removing stale CMake cache that does not match current source directory: $BUILD_DIR"
    rm -rf "$BUILD_DIR"
fi

# CMake configuration
echo "Configuring CMake..."
if [[ "$OSTYPE" == darwin* ]]; then
    # On macOS, build a Universal Binary
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
        -DCLAP_WRAPPER_CXX_STANDARD=23 \
        -G "$CMAKE_GENERATOR"
else
    # Other platforms
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

# Run the build
echo "Building..."

# AudioUnitSDK headers use GNU statement expressions which conflict with
# clap-wrapper's -Wpedantic -Werror; suppress the warning during Xcode builds
if [[ "$CMAKE_GENERATOR" == "Xcode" ]]; then
    XCODE_FLAGS=('--' 'OTHER_CPLUSPLUSFLAGS=$(inherited) -Wno-gnu-statement-expression-from-macro-expansion -Wno-shorten-64-to-32')
    # Pipe through xcbeautify only when on macOS and xcbeautify is available
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

# Verify build output
VST3_OUTPUT=""
if [[ "$CMAKE_GENERATOR" == "Xcode" ]] || [[ "$CMAKE_GENERATOR" == "Visual Studio 17 2022" ]]; then
    # For multi-configuration generators, look in the Configuration subdirectory
    if [[ "$OSTYPE" == darwin* ]]; then
        VST3_OUTPUT=$(find "$BUILD_DIR/$BUILD_CONFIG" -name "*.vst3" -type d 2>/dev/null | head -n 1)
    else
        VST3_OUTPUT=$(find "$BUILD_DIR/$BUILD_CONFIG" -name "*.vst3" -type f 2>/dev/null | head -n 1)
    fi
else
    # For single-configuration generators
    if [[ "$OSTYPE" == darwin* ]]; then
        VST3_OUTPUT=$(find "$BUILD_DIR" -name "*.vst3" -type d | head -n 1)
    else
        VST3_OUTPUT=$(find "$BUILD_DIR" -name "*.vst3" -type f | head -n 1)
    fi
fi

if [ -n "$VST3_OUTPUT" ]; then
    # Resolve full path
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
