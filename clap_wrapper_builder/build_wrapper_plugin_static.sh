#!/bin/bash
# build_wrapper_plugin_static.sh - Build VST3 wrapper from a CLAP plugin static library
#
# Usage:
#   ./build_wrapper_plugin_static.sh <static library file> <output plugin name> [Debug|Release] [standalone plugin ID]
#
# Arguments:
#   Library file   - CLAP plugin static library filename (e.g. "libxdevice_plugin.a")
#   Output name    - Display name used in VST3/AU (e.g. "Example Plugin NIH")
#   Debug|Release  - Build configuration (default: Debug)
#   Standalone ID  - When specified, also builds standalone (falls back to CLAP_WRAPPER_STANDALONE_PLUGIN_ID env var)
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
    echo "Usage: $0 <library file> <output plugin name> [Debug|Release] [standalone plugin ID]"
    echo "  If omitted, build configuration defaults to Debug"
    echo ""
    echo "Examples:"
    echo "  $0 example_plugin_nih.clap \"Example Plugin NIH\" Release"
    echo "  $0 \"XDevice Editor.clap\" \"XDevice Editor\" Debug"
    echo "  $0 libexample_plugin_clack.a \"Example Plugin Clack Static\" Debug com.novo-notes.pulsus-gain-clack"
    exit 1
}

# Argument parsing
if [ $# -lt 2 ]; then
    usage
fi

CLAP_LIBRARY_FILE="$1"
OUTPUT_NAME="$2"
BUILD_CONFIG="Debug"
STANDALONE_PLUGIN_ID="${CLAP_WRAPPER_STANDALONE_PLUGIN_ID:-}"
STANDALONE_OUTPUT_NAME="${CLAP_WRAPPER_STANDALONE_OUTPUT_NAME:-}"
BUILD_VST3="${CLAP_WRAPPER_BUILDER_BUILD_VST3:-}"
BUILD_AUV2="${CLAP_WRAPPER_BUILDER_BUILD_AUV2:-}"
BUILD_DIR_SUFFIX="${CLAP_WRAPPER_BUILDER_BUILD_DIR_SUFFIX:-}"

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

if [ $# -ge 4 ]; then
    STANDALONE_PLUGIN_ID="$4"
fi

if [ -z "$BUILD_VST3" ]; then
    BUILD_VST3="ON"
fi

if [ -z "$BUILD_AUV2" ]; then
    if [[ "$OSTYPE" == darwin* ]]; then
        BUILD_AUV2="ON"
    else
        BUILD_AUV2="OFF"
    fi
fi

echo "CLAP library file: $CLAP_LIBRARY_FILE"
echo "Output plugin name: $OUTPUT_NAME"
echo "Build configuration: $BUILD_CONFIG"
echo "VST3 build: $BUILD_VST3"
echo "AU build: $BUILD_AUV2"
if [ -n "$STANDALONE_PLUGIN_ID" ]; then
    if [ -z "$STANDALONE_OUTPUT_NAME" ]; then
        STANDALONE_OUTPUT_NAME="${OUTPUT_NAME} Standalone"
    fi
    echo "Standalone build: enabled"
    echo "Standalone output name: $STANDALONE_OUTPUT_NAME"
    echo "Standalone Plugin ID: $STANDALONE_PLUGIN_ID"
else
    echo "Standalone build: disabled"
fi

# Strip extension from CLAP library filename and replace spaces with underscores
# Remove path component, keep filename only
CLAP_FILE_BASENAME=$(basename "$CLAP_LIBRARY_FILE")
CLAP_BASE_NAME="${CLAP_FILE_BASENAME%.a}"
CLAP_BASE_NAME="${CLAP_BASE_NAME%.lib}"
CLAP_BASE_NAME="${CLAP_BASE_NAME// /_}_static"

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
# On Windows, use a short hash to avoid MAX_PATH limits
if [[ "$OSTYPE" =~ ^(msys|cygwin|mingw).* ]]; then
    CLAP_BASE_HASH=$(printf '%s' "$CLAP_BASE_NAME" | cksum | awk '{print $1}')
    BUILD_DIR="$SCRIPT_DIR/bw_$CLAP_BASE_HASH"
else
    BUILD_DIR="$SCRIPT_DIR/build_$CLAP_BASE_NAME"
fi

if [ -n "$BUILD_DIR_SUFFIX" ]; then
    SAFE_BUILD_DIR_SUFFIX="${BUILD_DIR_SUFFIX//[^A-Za-z0-9_-]/_}"
    BUILD_DIR="${BUILD_DIR}_${SAFE_BUILD_DIR_SUFFIX}"
fi

# Rebuild if the CMakeCache has a stale source path (e.g. after repo rename)
if [ -f "$BUILD_DIR/CMakeCache.txt" ] && ! grep -Fq "$SCRIPT_DIR" "$BUILD_DIR/CMakeCache.txt"; then
    warning "Removing stale CMake cache that does not match current source directory: $BUILD_DIR"
    rm -rf "$BUILD_DIR"
fi

CMAKE_STANDALONE_ARGS=()
if [ -n "$STANDALONE_PLUGIN_ID" ]; then
    CMAKE_STANDALONE_ARGS+=(
        -DCLAP_WRAPPER_BUILDER_BUILD_STANDALONE=ON
        -DCLAP_WRAPPER_BUILDER_STANDALONE_PLUGIN_ID="$STANDALONE_PLUGIN_ID"
        -DCLAP_WRAPPER_BUILDER_STANDALONE_OUTPUT_NAME="$STANDALONE_OUTPUT_NAME"
        -DCLAP_WRAPPER_DOWNLOAD_DEPENDENCIES=TRUE
    )
fi

CMAKE_FORMAT_ARGS=()
if [ -n "${CLAP_WRAPPER_BUILDER_BUILD_VST3:-}" ]; then
    CMAKE_FORMAT_ARGS+=(-DCLAP_WRAPPER_BUILDER_BUILD_VST3="${CLAP_WRAPPER_BUILDER_BUILD_VST3}")
fi
if [ -n "${CLAP_WRAPPER_BUILDER_BUILD_AUV2:-}" ]; then
    CMAKE_FORMAT_ARGS+=(-DCLAP_WRAPPER_BUILDER_BUILD_AUV2="${CLAP_WRAPPER_BUILDER_BUILD_AUV2}")
fi

CMAKE_CONFIGURE_ARGS=(
    -S "."
    -B "$BUILD_DIR"
    -DCLAP_SDK_ROOT="$CLAP_SDK_ROOT"
    -DVST3_SDK_ROOT="$VST3_SDK_ROOT"
    -DCLAP_WRAPPER_OUTPUT_NAME="$OUTPUT_NAME"
    -DCMAKE_BUILD_TYPE="$BUILD_CONFIG"
    -DCLAP_WRAPPER_BUILDER_TARGET_LIB="$CLAP_LIBRARY_FILE"
    -DCLAP_WRAPPER_BUILD_AUV2=ON
    -DAUDIOUNIT_SDK_ROOT="$AU_SDK_ROOT"
    -DCLAP_WRAPPER_CXX_STANDARD=23
    -G "$CMAKE_GENERATOR"
)
if [ "${#CMAKE_FORMAT_ARGS[@]}" -gt 0 ]; then
    CMAKE_CONFIGURE_ARGS+=("${CMAKE_FORMAT_ARGS[@]}")
fi
if [ "${#CMAKE_STANDALONE_ARGS[@]}" -gt 0 ]; then
    CMAKE_CONFIGURE_ARGS+=("${CMAKE_STANDALONE_ARGS[@]}")
fi

cmake "${CMAKE_CONFIGURE_ARGS[@]}"

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

if [[ "$BUILD_VST3" != "OFF" ]]; then
    if [ -n "$VST3_OUTPUT" ]; then
        # Resolve full path
        VST3_FULLPATH="$(cd "$(dirname "$VST3_OUTPUT")" && pwd)/$(basename "$VST3_OUTPUT")"
        success "VST3 plugin generated: $VST3_FULLPATH"
    else
        error "VST3 plugin not found"
    fi
fi

if [ -n "$STANDALONE_PLUGIN_ID" ]; then
    STANDALONE_OUTPUT=""
    if [[ "$CMAKE_GENERATOR" == "Xcode" ]] || [[ "$CMAKE_GENERATOR" == "Visual Studio 17 2022" ]]; then
        if [[ "$OSTYPE" == darwin* ]]; then
            STANDALONE_OUTPUT=$(find "$BUILD_DIR/$BUILD_CONFIG" -name "${STANDALONE_OUTPUT_NAME}.app" -type d 2>/dev/null | head -n 1)
        elif [[ "$OSTYPE" =~ ^(msys|cygwin|mingw).* ]]; then
            STANDALONE_OUTPUT=$(find "$BUILD_DIR/$BUILD_CONFIG" -name "${STANDALONE_OUTPUT_NAME}.exe" -type f 2>/dev/null | head -n 1)
        fi
    else
        if [[ "$OSTYPE" == darwin* ]]; then
            STANDALONE_OUTPUT=$(find "$BUILD_DIR" -name "${STANDALONE_OUTPUT_NAME}.app" -type d 2>/dev/null | head -n 1)
        elif [[ "$OSTYPE" =~ ^(msys|cygwin|mingw).* ]]; then
            STANDALONE_OUTPUT=$(find "$BUILD_DIR" -name "${STANDALONE_OUTPUT_NAME}.exe" -type f 2>/dev/null | head -n 1)
        fi
    fi

    if [ -n "$STANDALONE_OUTPUT" ]; then
        STANDALONE_FULLPATH="$(cd "$(dirname "$STANDALONE_OUTPUT")" && pwd)/$(basename "$STANDALONE_OUTPUT")"
        success "Standalone app generated: $STANDALONE_FULLPATH"
    else
        error "Standalone app not found"
    fi
fi
