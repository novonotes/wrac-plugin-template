#!/bin/bash
# install.sh - CLAP installation for gain_plugin
#
# Copies the .clap bundle created by build.sh to the OS-specific CLAP plugin directory.
# DAWs automatically scan this directory to detect plugins.
#
# Installation directories:
#   macOS:   ~/Library/Audio/Plug-Ins/CLAP/
#   Windows: %LOCALAPPDATA%/Programs/Common/CLAP/
#   Linux:   ~/.clap/
#
# Usage:
#   ./script/install.sh
#
# Note: The bundle must be created with build.sh before running this script.

set -e  # Stop script on error
set -u  # Treat unset variables as an error

# ---------------------------------------------------------------------------
# OS detection
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
        echo "Error: Unsupported OS $(uname -s)"
        exit 1
        ;;
esac

echo "Detected OS: $OS"

# ---------------------------------------------------------------------------
# Path resolution
# ---------------------------------------------------------------------------
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PLUGIN_ROOT="$( cd "$SCRIPT_DIR/.." && pwd )"
TARGET_DIR="${CARGO_TARGET_DIR:-$PLUGIN_ROOT/target}"

BUNDLE_NAME="WXP Example Gain.clap"
BUNDLE_PATH="$TARGET_DIR/bundled/${BUNDLE_NAME}"

# ---------------------------------------------------------------------------
# Bundle existence check
# ---------------------------------------------------------------------------
# Verify that build.sh has completed successfully before installing.
if [ ! -e "$BUNDLE_PATH" ]; then
    echo "Error: Bundle not found: $BUNDLE_PATH"
    echo "Please run build.sh first"
    exit 1
fi

# ---------------------------------------------------------------------------
# OS-specific installation
# ---------------------------------------------------------------------------
case "$OS" in
    macos)
        # Standard CLAP directory on macOS.
        # Most DAWs including Logic Pro and Ableton Live scan this directory.
        echo "Preparing installation directory..."
        mkdir -p ~/Library/Audio/Plug-Ins/CLAP || {
            echo "Error: Failed to create CLAP plugin directory"
            exit 1
        }

        # Remove existing version before overwriting (macOS bundles are directories).
        if [ -e ~/Library/Audio/Plug-Ins/CLAP/"${BUNDLE_NAME}" ]; then
            rm -rf ~/Library/Audio/Plug-Ins/CLAP/"${BUNDLE_NAME}"
        fi

        echo "Installing plugin..."
        # macOS bundles are directory structures, so -r (recursive copy) is required.
        cp -r "$BUNDLE_PATH" ~/Library/Audio/Plug-Ins/CLAP/ || {
            echo "Error: Failed to copy plugin"
            exit 1
        }

        echo "Installation complete!"
        echo "Plugin installed at: ~/Library/Audio/Plug-Ins/CLAP/${BUNDLE_NAME}"
        ;;
    windows)
        # Standard CLAP directory on Windows (user-local).
        # %PROGRAMFILES%/Common Files/CLAP/ is also common, but this location does not require admin rights.
        CLAP_DIR="$LOCALAPPDATA/Programs/Common/CLAP"

        echo "Note: Installing to Program Files may require administrator privileges"
        echo "Preparing installation directory..."

        mkdir -p "$CLAP_DIR" || {
            echo "Error: Failed to create CLAP plugin directory"
            echo "Please run the following command manually:"
            echo "cp \"$BUNDLE_PATH\" \"$CLAP_DIR/\""
            exit 1
        }

        if [ -e "$CLAP_DIR/${BUNDLE_NAME}" ]; then
            rm -rf "$CLAP_DIR/${BUNDLE_NAME}"
        fi

        echo "Installing plugin..."
        cp "$BUNDLE_PATH" "$CLAP_DIR/" || {
            echo "Error: Failed to copy plugin"
            echo "Please run the following command manually:"
            echo "cp \"$BUNDLE_PATH\" \"$CLAP_DIR/\""
            exit 1
        }

        echo "Installation complete!"
        echo "Plugin installed at: $CLAP_DIR/${BUNDLE_NAME}"
        ;;
    linux)
        # Standard CLAP directory on Linux (user-local).
        # Use /usr/lib/clap/ for system-wide installation (requires root),
        # but ~/.clap/ is used here to avoid requiring root privileges.
        CLAP_DIR="$HOME/.clap"

        echo "Preparing installation directory..."
        mkdir -p "$CLAP_DIR" || {
            echo "Error: Failed to create CLAP plugin directory"
            exit 1
        }

        if [ -e "$CLAP_DIR/${BUNDLE_NAME}" ]; then
            rm -rf "$CLAP_DIR/${BUNDLE_NAME}"
        fi

        echo "Installing plugin..."
        cp -r "$BUNDLE_PATH" "$CLAP_DIR/" || {
            echo "Error: Failed to copy plugin"
            exit 1
        }

        echo "Installation complete!"
        echo "Plugin installed at: $CLAP_DIR/${BUNDLE_NAME}"
        ;;
esac
