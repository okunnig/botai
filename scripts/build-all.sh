#!/usr/bin/env bash
# ==============================================================================
# scripts/build-all.sh - Master Multi-Platform Build & Packaging Script for botai
# ==============================================================================
# Orchestrates building and packaging of release artifacts across:
#   - Linux   (x86_64 gnu/musl, aarch64 gnu/musl)
#   - Windows (x86_64-pc-windows-gnu)
#   - macOS   (x86_64, aarch64, universal)
#
# Generates dist/SHA256SUMS checksum manifest and displays a formatted summary table.
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/common.sh"

BUILD_LINUX=false
BUILD_WINDOWS=false
BUILD_MACOS=false
CUSTOM_DIST_DIR=""
EXPLICIT_ENGINE=""

usage() {
    cat <<EOF
Usage: $(basename "$0") [OPTIONS]

Master build orchestrator for compiling and packaging botai artifacts.

Options:
    -l, --linux                 Build Linux distributable packages
    -w, --windows               Build Windows distributable packages
    -m, --macos                 Build macOS distributable packages
    -a, --all                   Build all platforms (Linux, Windows, macOS) [default if none set]
    -e, --engine <ENGINE>       Build engine to use: 'cross' or 'cargo' (default: cross)
    -d, --dist-dir <DIR>        Output directory for distribution archives (default: dist/)
    -h, --help                  Show this help message

Examples:
    $(basename "$0") --linux
    $(basename "$0") --windows --dist-dir /custom/dist
    $(basename "$0") --all
EOF
}

# Parse CLI arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        -l|--linux)
            BUILD_LINUX=true
            shift
            ;;
        -w|--windows)
            BUILD_WINDOWS=true
            shift
            ;;
        -m|--macos)
            BUILD_MACOS=true
            shift
            ;;
        -a|--all)
            BUILD_LINUX=true
            BUILD_WINDOWS=true
            BUILD_MACOS=true
            shift
            ;;
        -e|--engine)
            EXPLICIT_ENGINE="$2"
            shift 2
            ;;
        -d|--dist-dir)
            CUSTOM_DIST_DIR="$2"
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            log_error "Unknown option: $1"
            usage
            exit 1
            ;;
    esac
done

# Default to building all platforms if no specific platform was requested
if [[ "$BUILD_LINUX" == false && "$BUILD_WINDOWS" == false && "$BUILD_MACOS" == false ]]; then
    BUILD_LINUX=true
    BUILD_WINDOWS=true
    BUILD_MACOS=true
fi

if [[ -n "$CUSTOM_DIST_DIR" ]]; then
    DIST_DIR="$CUSTOM_DIST_DIR"
fi

if [[ -n "$EXPLICIT_ENGINE" ]]; then
    BUILD_ENGINE="$EXPLICIT_ENGINE"
fi

VERSION=$(get_version)
ENGINE=$(detect_build_engine)

log_header "botai v$VERSION - Multi-Platform Distribution Build"
log_info "Build Engine: $ENGINE"
if [[ "$ENGINE" == "cross" ]]; then
    log_info "Container Engine: ${CROSS_CONTAINER_ENGINE:-default}"
fi
log_info "Target Distribution Directory: $DIST_DIR"
log_info "Platforms Selected: Linux=$BUILD_LINUX, Windows=$BUILD_WINDOWS, macOS=$BUILD_MACOS"

ensure_dist_dir

COMMON_ARGS=(--dist-dir "$DIST_DIR" --no-checksum)
if [[ -n "${BUILD_ENGINE:-}" ]]; then
    COMMON_ARGS+=(--engine "$BUILD_ENGINE")
fi

if [[ "$BUILD_LINUX" == true ]]; then
    log_step "Invoking Linux build script..."
    "$SCRIPT_DIR/build-linux.sh" "${COMMON_ARGS[@]}"
fi

if [[ "$BUILD_WINDOWS" == true ]]; then
    log_step "Invoking Windows build script..."
    "$SCRIPT_DIR/build-windows.sh" "${COMMON_ARGS[@]}"
fi

if [[ "$BUILD_MACOS" == true ]]; then
    log_step "Invoking macOS build script..."
    "$SCRIPT_DIR/build-macos.sh" "${COMMON_ARGS[@]}"
fi

# Generate unified SHA256SUMS for all packages
generate_checksums "$DIST_DIR"

# Print pretty formatted artifact summary table
print_artifact_table "$DIST_DIR"

log_success "All requested builds and distribution packages completed successfully."
