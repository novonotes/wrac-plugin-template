#!/bin/bash
# build.sh - CLAP build for gain_plugin
#
# This script performs the following 3 steps:
#   1. npm build of the GUI frontend (src-gui)
#   2. cargo build of the Rust plugin (src-plugin)
#   3. Package the build artifacts into a .clap bundle format
#
# What is a .clap bundle:
#   A file/directory in a format that CLAP hosts (DAWs) can recognize as a plugin.
#   On macOS it is a bundle (directory structure similar to .app),
#   on Windows/Linux it is a single .dll/.so file.
#
# Usage:
#   ./script/build.sh [Debug|Release]
#
# Arguments:
#   Debug|Release - Build configuration (default: Debug)
#
# Output:
#   target/bundled/WXP Example Gain.clap

set -e  # Stop script on error
set -u  # Treat unset variables as an error

# ---------------------------------------------------------------------------
# OS detection
# ---------------------------------------------------------------------------
# Bundle format differs per OS, so determine it first.
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
# Build configuration
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
            echo "Error: Invalid build configuration: $1"
            exit 1
            ;;
    esac
fi

echo "Build configuration: $BUILD_CONFIG"

if [ "$BUILD_CONFIG" = "Debug" ]; then
    PROFILE_DIR="debug"
else
    PROFILE_DIR="release"
fi

# ---------------------------------------------------------------------------
# Path resolution
# ---------------------------------------------------------------------------
# Derive the absolute path of this script from BASH_SOURCE[0], then locate each directory relative to it.
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PLUGIN_ROOT="$( cd "$SCRIPT_DIR/.." && pwd )"
GUI_DIR="$PLUGIN_ROOT/src-gui"
TARGET_DIR="${CARGO_TARGET_DIR:-$PLUGIN_ROOT/target}"
BUILD_DIR="$TARGET_DIR/$PROFILE_DIR"

# ---------------------------------------------------------------------------
# Step 1: GUI frontend build
# ---------------------------------------------------------------------------
# Bundle TypeScript/CSS with Vite and output to src-gui/dist/.
# For release builds, build.rs ZIP-compresses this dist/ and embeds it in the binary.
echo "Building GUI..."
(
    cd "$GUI_DIR"
    npm install
    npm run build
)

# ---------------------------------------------------------------------------
# Step 2: Rust plugin build
# ---------------------------------------------------------------------------
# Shared libraries produced by cargo:
#   macOS:   libwxp_example_gain_plugin.dylib
#   Windows: wxp_example_gain_plugin.dll
#   Linux:   libwxp_example_gain_plugin.so
echo "Building plugin..."
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
# Step 3: Create .clap bundle
# ---------------------------------------------------------------------------
# The distribution format for CLAP plugins differs per OS:
#   macOS:   macOS bundle (directory structure + Info.plist)
#   Windows: rename .dll to .clap
#   Linux:   rename .so to .clap
PLUGIN_NAME="WXP Example Gain.clap"
BUNDLE_DIR="$TARGET_DIR/bundled/$PLUGIN_NAME"

echo "Creating bundle structure..."
rm -rf "$BUNDLE_DIR"

case "$OS" in
    macos)
        # A .clap on macOS has the same bundle structure as .app.
        # Place the executable binary under Contents/MacOS/ and metadata in Contents/Info.plist.
        mkdir -p "$BUNDLE_DIR/Contents/MacOS"

        # Info.plist: metadata file that macOS uses to identify the bundle.
        # CFBundleIdentifier must match the plugin's PLUGIN_ID.
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

        # PkgInfo: required by an old macOS convention.
        # "BNDL" is the bundle type, "????" is the creator code (generic).
        echo -n "BNDL????" > "$BUNDLE_DIR/Contents/PkgInfo"

        # Copy the .dylib under the binary name without extension.
        # The CLAP host looks for the binary by the name specified in CFBundleExecutable.
        cp "$BUILD_DIR/libwxp_example_gain_plugin.dylib" \
            "$BUNDLE_DIR/Contents/MacOS/WXP Example Gain"

        # install_name_tool: rewrite the LC_ID_DYLIB (the dylib's self-identification path).
        # Using "@loader_path/..." allows self-referencing via a relative path within the bundle,
        # making it a portable bundle that does not depend on the installation path.
        install_name_tool -id "@loader_path/WXP Example Gain" \
            "$BUNDLE_DIR/Contents/MacOS/WXP Example Gain"
        ;;
    windows)
        # On Windows, simply place the .dll as-is with the .clap extension.
        mkdir -p "$TARGET_DIR/bundled"
        cp "$BUILD_DIR/wxp_example_gain_plugin.dll" "$BUNDLE_DIR"
        ;;
    linux)
        # On Linux, similarly place the .so with the .clap extension.
        mkdir -p "$TARGET_DIR/bundled"
        cp "$BUILD_DIR/libwxp_example_gain_plugin.so" "$BUNDLE_DIR"
        ;;
esac

if [ ! -e "$BUNDLE_DIR" ]; then
    echo "Error: Build succeeded but bundled plugin not found"
    exit 1
fi

echo "Build complete!"
echo "Bundle created at: target/bundled/$PLUGIN_NAME"
