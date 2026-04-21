#!/bin/bash
# install_wrapper_plugin.sh - Install generated VST3/AU plugins
#
# Usage:
#   ./install_wrapper_plugin.sh <CLAP file> <output plugin name> [Debug|Release]
#
# Arguments:
#   CLAP file    - CLAP plugin filename (e.g. "example_plugin_nih.clap")
#   Output name  - Display name used in VST3/AU (e.g. "Example Plugin NIH")
#   Debug|Release - Build configuration (default: Debug)
#
# Note:
#   - Run build_wrapper_plugin.sh first to generate the VST3/AU plugins

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

# Strip extension from CLAP filename and replace spaces with underscores
# Remove path component, keep filename only
CLAP_FILE_BASENAME=$(basename "$CLAP_FILE")
CLAP_BASE_NAME="${CLAP_FILE_BASENAME%.clap}"
CLAP_BASE_NAME="${CLAP_BASE_NAME// /_}"

# OS detection and installation directory setup
case "$OSTYPE" in
    darwin*)
        # macOS
        OS="macos"
        VST3_INSTALL_DIR="$HOME/Library/Audio/Plug-Ins/VST3"
        AU_INSTALL_DIR="$HOME/Library/Audio/Plug-Ins/Components"
        success "Detected macOS"
        ;;
    linux*)
        # Linux
        OS="linux"
        VST3_INSTALL_DIR="$HOME/.vst3"
        success "Detected Linux"
        ;;
    msys*|cygwin*|mingw*)
        # Windows
        OS="windows"
        # Use Windows environment variables
        if [ -n "$COMMONPROGRAMFILES" ]; then
            VST3_INSTALL_DIR="$COMMONPROGRAMFILES/VST3"
        else
            VST3_INSTALL_DIR="$LOCALAPPDATA/Programs/Common/VST3"
        fi
        success "Detected Windows"
        ;;
    *)
        error "Unsupported OS: $OSTYPE"
        ;;
esac

# Check build directory
BUILD_DIR="$SCRIPT_DIR/build_$CLAP_BASE_NAME"
if [ ! -d "$BUILD_DIR" ]; then
    error "Build directory not found. Run build_wrapper_plugin.sh first."
fi

# Search for VST3 plugin
VST3_OUTPUT=""
VST3_FILENAME="$OUTPUT_NAME.vst3"
AU_OUTPUT=""
AU_FILENAME="$OUTPUT_NAME.component"

# For multi-configuration generators
if [ -d "$BUILD_DIR/$BUILD_CONFIG" ]; then
    if [[ "$OS" == "macos" ]]; then
        VST3_OUTPUT=$(find "$BUILD_DIR/$BUILD_CONFIG" -name "$VST3_FILENAME" -type d | head -n 1 || true)
    else
        VST3_OUTPUT=$(find "$BUILD_DIR/$BUILD_CONFIG" -name "$VST3_FILENAME" -type f | head -n 1 || true)
    fi
fi

# For single-configuration generators
if [ -z "$VST3_OUTPUT" ]; then
    if [[ "$OS" == "macos" ]]; then
        VST3_OUTPUT=$(find "$BUILD_DIR" -name "$VST3_FILENAME" -type d | head -n 1 || true)
    else
        VST3_OUTPUT=$(find "$BUILD_DIR" -name "$VST3_FILENAME" -type f | head -n 1 || true)
    fi
fi

if [ -z "$VST3_OUTPUT" ]; then
    error "VST3 plugin not found. Run build_wrapper_plugin.sh first."
fi

VST3_FULLPATH="$(cd "$(dirname "$VST3_OUTPUT")" && pwd)/$(basename "$VST3_OUTPUT")"
success "VST3 plugin found: $VST3_FULLPATH"

if [[ "$OS" == "macos" ]]; then
    if [ -d "$BUILD_DIR/$BUILD_CONFIG" ]; then
        AU_OUTPUT=$(find "$BUILD_DIR/$BUILD_CONFIG" -name "$AU_FILENAME" -type d | head -n 1 || true)
    fi

    if [ -z "$AU_OUTPUT" ]; then
        AU_OUTPUT=$(find "$BUILD_DIR" -name "$AU_FILENAME" -type d | head -n 1 || true)
    fi

    if [ -n "$AU_OUTPUT" ]; then
        AU_FULLPATH="$(cd "$(dirname "$AU_OUTPUT")" && pwd)/$(basename "$AU_OUTPUT")"
        success "AU plugin found: $AU_FULLPATH"
    else
        warning "AU plugin not found. Installing VST3 only."
    fi
fi

# Check CLAP plugin installation (warning only)
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

# Defer CLAP-not-installed warning to the end

# Create VST3 installation directory
echo "Preparing VST3 installation directory..."
if [[ "$OS" == "windows" ]]; then
    # On Windows, admin privileges may be required
    mkdir -p "$VST3_INSTALL_DIR" 2>/dev/null || {
        warning "Failed to create VST3 installation directory."
        warning "Run the script with administrator privileges or install manually."
        echo ""
        echo "Manual installation:"
        echo "  cp -r \"$VST3_FULLPATH\" \"$VST3_INSTALL_DIR/\""
        exit 1
    }
else
    mkdir -p "$VST3_INSTALL_DIR" || {
        error "Failed to create VST3 installation directory: $VST3_INSTALL_DIR"
    }
fi

# Install VST3 plugin
echo "Installing VST3 plugin..."
if [[ "$OS" == "macos" ]]; then
    # On macOS, copy the entire bundle
    rm -rf "$VST3_INSTALL_DIR/$VST3_FILENAME"
    cp -r "$VST3_FULLPATH" "$VST3_INSTALL_DIR/" || {
        error "Failed to copy VST3 plugin"
    }
else
    # On other OSes, copy the file
    cp "$VST3_FULLPATH" "$VST3_INSTALL_DIR/" || {
        error "Failed to copy VST3 plugin"
    }
fi

success "Installation complete!"
echo ""
echo "VST3 plugin installed at:"
echo "  $VST3_INSTALL_DIR/$VST3_FILENAME"

if [[ "$OS" == "macos" && -n "${AU_OUTPUT:-}" ]]; then
    echo "Preparing AU installation directory..."
    mkdir -p "$AU_INSTALL_DIR" || {
        error "Failed to create AU installation directory: $AU_INSTALL_DIR"
    }

    echo "Installing AU plugin..."
    rm -rf "$AU_INSTALL_DIR/$AU_FILENAME"
    cp -r "$AU_FULLPATH" "$AU_INSTALL_DIR/" || {
        error "Failed to copy AU plugin"
    }

    echo "AU plugin installed at:"
    echo "  $AU_INSTALL_DIR/$AU_FILENAME"
fi
echo ""

if [ "$CLAP_INSTALLED" = false ]; then
    warning "Note: $CLAP_FILE must be installed for VST3 to work."
fi

# Note about DAW plugin scanning
echo ""
echo "Next steps:"
echo "1. Launch (or restart) your DAW"
echo "2. Run your DAW's plugin scan"
echo "3. $OUTPUT_NAME should appear as a VST3 plugin"
