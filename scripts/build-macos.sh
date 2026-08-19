#!/usr/bin/env bash
# ==============================================================================
# scripts/build-macos.sh - macOS Cross-Compilation & Packaging for botai
# ==============================================================================
# Builds macOS release binaries for:
#   - x86_64-apple-darwin (Intel)
#   - aarch64-apple-darwin (Apple Silicon)
# Combines them into a Universal Binary with `lipo` if available.
#
# Packages each into dist/botai-<version>-macos-<arch>.tar.gz containing:
#   - botai (executable)
#   - README.md
#   - REQS.md
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/common.sh"

DEFAULT_TARGETS=(
    "x86_64-apple-darwin"
    "aarch64-apple-darwin"
)

TARGETS=()
GENERATE_CHECKSUM=true

usage() {
    cat <<EOF
Usage: $(basename "$0") [OPTIONS]

Builds and packages macOS release binaries for botai.

Options:
    -t, --target <TARGET>       Specific target triple to build (can be repeated)
    -e, --engine <ENGINE>       Build engine to use: 'cross' or 'cargo' (default: auto/cross)
    -d, --dist-dir <DIR>        Output directory for distribution archives (default: dist/)
        --no-checksum           Skip automatic SHA-256 checksum generation
    -h, --help                  Show this help message

Default Targets:
    - x86_64-apple-darwin   (Intel x86_64)
    - aarch64-apple-darwin  (Apple Silicon ARM64)
EOF
}

# Parse CLI arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        -t|--target)
            TARGETS+=("$2")
            shift 2
            ;;
        -e|--engine)
            BUILD_ENGINE="$2"
            shift 2
            ;;
        -d|--dist-dir)
            DIST_DIR="$2"
            shift 2
            ;;
        --no-checksum)
            GENERATE_CHECKSUM=false
            shift
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

if [[ ${#TARGETS[@]} -eq 0 ]]; then
    TARGETS=("${DEFAULT_TARGETS[@]}")
fi

VERSION=$(get_version)
ENGINE=$(detect_build_engine)

log_header "Building macOS Release Artifacts (v$VERSION)"
log_info "Build Engine: $ENGINE"
if [[ "$ENGINE" == "cross" ]]; then
    log_info "Container Engine: ${CROSS_CONTAINER_ENGINE:-default}"
fi
log_info "Output Directory: $DIST_DIR"
log_info "Targets: ${TARGETS[*]}"

get_archive_name() {
    local target="$1"
    local version="$2"
    case "$target" in
        "x86_64-apple-darwin")
            echo "botai-v${version}-macos-x86_64"
            ;;
        "aarch64-apple-darwin")
            echo "botai-v${version}-macos-aarch64"
            ;;
        *)
            echo "botai-v${version}-${target}"
            ;;
    esac
}

cd "$PROJECT_ROOT"

declare -A BUILT_BINARIES=()

for target in "${TARGETS[@]}"; do
    log_step "Compiling target: $target..."
    
    if [[ "$ENGINE" == "cross" ]]; then
        cross build --target "$target" --release --config 'build.rustflags=[]'
    else
        cargo build --target "$target" --release --config 'build.rustflags=[]'
    fi

    log_step "Locating built macOS binary for $target..."
    BINARY_PATH=$(find_built_binary "$target" "$APP_NAME" "release")
    log_info "Found binary: $BINARY_PATH"
    BUILT_BINARIES["$target"]="$BINARY_PATH"

    ARCHIVE_NAME=$(get_archive_name "$target" "$VERSION")
    log_step "Packaging $ARCHIVE_NAME.tar.gz..."
    package_tar_gz "$ARCHIVE_NAME" "$BINARY_PATH" "$APP_NAME"
done

# Check if we can create a universal binary
if [[ -n "${BUILT_BINARIES["x86_64-apple-darwin"]:-}" ]] && [[ -n "${BUILT_BINARIES["aarch64-apple-darwin"]:-}" ]]; then
    if command -v lipo >/dev/null 2>&1; then
        log_step "Creating Universal macOS Binary with lipo..."
        UNIVERSAL_DIR=$(mktemp -d -t "botai-universal-XXXXXX")
        trap 'rm -rf "$UNIVERSAL_DIR"' RETURN
        
        UNIVERSAL_BIN="$UNIVERSAL_DIR/$APP_NAME"
        lipo -create -output "$UNIVERSAL_BIN" \
            "${BUILT_BINARIES["x86_64-apple-darwin"]}" \
            "${BUILT_BINARIES["aarch64-apple-darwin"]}"
        chmod 755 "$UNIVERSAL_BIN"
        
        ARCHIVE_NAME="botai-v${VERSION}-macos-universal"
        log_step "Packaging $ARCHIVE_NAME.tar.gz..."
        package_tar_gz "$ARCHIVE_NAME" "$UNIVERSAL_BIN" "$APP_NAME"
    else
        log_info "lipo tool not found on host; skipping macOS universal binary creation."
    fi
fi

if [[ "$GENERATE_CHECKSUM" == true ]]; then
    generate_checksums "$DIST_DIR"
fi

log_success "macOS artifacts build completed successfully."
