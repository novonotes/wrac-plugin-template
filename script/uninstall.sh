#!/bin/bash
# uninstall.sh - Remove installed WRAC Gain plugins
#
# Removes the WRAC Gain plugin files installed by this repository's
# build/install scripts.
#
# Usage:
#   ./script/uninstall.sh
#   ./script/uninstall.sh --dry-run

set -Eeuo pipefail

on_error() {
    local exit_code="$1"
    local line_no="$2"
    local command="$3"
    echo "Error: command failed at line ${line_no} (exit=${exit_code}): ${command}" >&2
    exit "$exit_code"
}

trap 'on_error $? $LINENO "$BASH_COMMAND"' ERR

DRY_RUN=false

usage() {
    echo "Usage: $0 [--dry-run]"
    exit 1
}

while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run)
            DRY_RUN=true
            ;;
        -h|--help)
            usage
            ;;
        *)
            echo "Error: unknown argument: $1" >&2
            usage
            ;;
    esac
    shift
done

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
        echo "Error: unsupported OS $(uname -s)" >&2
        exit 1
        ;;
esac

removed_count=0
missing_count=0
forgot_receipt_count=0

remove_path() {
    local path="$1"
    local needs_admin="${2:-false}"
    local parent_dir

    if [ ! -e "$path" ]; then
        echo "Not found: $path"
        missing_count=$((missing_count + 1))
        return
    fi

    removed_count=$((removed_count + 1))
    if [ "$DRY_RUN" = true ]; then
        echo "Would remove: $path"
        return
    fi

    echo "Removing: $path"
    if [ "$needs_admin" != true ]; then
        rm -rf "$path"
        return
    fi

    parent_dir="$(dirname "$path")"
    if [ -w "$parent_dir" ]; then
        rm -rf "$path"
        return
    fi

    sudo rm -rf "$path"
}

forget_macos_receipt() {
    local package_id="$1"

    if ! command -v pkgutil >/dev/null 2>&1; then
        return
    fi

    if ! pkgutil --pkg-info "$package_id" >/dev/null 2>&1; then
        echo "Receipt not found: $package_id"
        return
    fi

    forgot_receipt_count=$((forgot_receipt_count + 1))
    if [ "$DRY_RUN" = true ]; then
        echo "Would forget receipt: $package_id"
        return
    fi

    echo "Forgetting receipt: $package_id"
    sudo pkgutil --forget "$package_id" >/dev/null
}

echo "Uninstalling WRAC Gain plugins"
if [ "$DRY_RUN" = true ]; then
    echo "dry-run: no files will be removed"
fi
echo ""

case "$OS" in
    macos)
        remove_path "$HOME/Library/Audio/Plug-Ins/CLAP/WRAC Gain.clap"
        remove_path "$HOME/Library/Audio/Plug-Ins/VST3/WRAC Gain.vst3"
        remove_path "$HOME/Library/Audio/Plug-Ins/Components/WRAC Gain.component"

        remove_path "/Library/Audio/Plug-Ins/VST3/WRAC Gain.vst3" true
        remove_path "/Library/Audio/Plug-Ins/Components/WRAC Gain.component" true

        forget_macos_receipt "WRAC_GAIN_VST3"
        forget_macos_receipt "WRAC_GAIN_AU"
        ;;
    windows)
        remove_path "${LOCALAPPDATA}/Programs/Common/CLAP/WRAC Gain.clap"

        if [ -n "${COMMONPROGRAMFILES:-}" ]; then
            remove_path "${COMMONPROGRAMFILES}/VST3/WRAC Gain.vst3"
        fi
        remove_path "${LOCALAPPDATA}/Programs/Common/VST3/WRAC Gain.vst3"
        ;;
    linux)
        remove_path "$HOME/.clap/WRAC Gain.clap"
        ;;
esac

echo ""
echo "Uninstall complete: ${removed_count} removed, ${missing_count} not found, ${forgot_receipt_count} receipts forgotten"
