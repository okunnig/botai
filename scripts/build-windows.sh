#!/usr/bin/env bash
# ==============================================================================
# scripts/build-windows.sh - Windows Cross-Compilation & Packaging for botai
# ==============================================================================
# Builds Windows release binary (x86_64-pc-windows-gnu) using containerized cross.
#
# Packages into dist/botai-<version>-windows-x86_64.zip containing:
#   - botai.exe (Windows executable)
#   - README.md
#   - REQS.md
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/common.sh"

DEFAULT_TARGET="x86_64-pc-windows-gnu"
TARGETS=()
GENERATE_CHECKSUM=true

usage() {
    cat <<EOF
Usage: $(basename "$0") [OPTIONS]

Builds and packages Windows release binaries for botai.

Options:
    -t, --target <TARGET>       Target triple (default: x86_64-pc-windows-gnu)
    -e, --engine <ENGINE>       Build engine to use: 'cross' or 'cargo' (default: auto/cross)
    -d, --dist-dir <DIR>        Output directory for distribution archives (default: dist/)
        --no-checksum           Skip automatic SHA-256 checksum generation
    -h, --help                  Show this help message
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
    TARGETS=("$DEFAULT_TARGET")
fi

VERSION=$(get_version)
ENGINE=$(detect_build_engine)

log_header "Building Windows Release Artifacts (v$VERSION)"
log_info "Build Engine: $ENGINE"
if [[ "$ENGINE" == "cross" ]]; then
    log_info "Container Engine: ${CROSS_CONTAINER_ENGINE:-default}"
fi
log_info "Output Directory: $DIST_DIR"
log_info "Targets: ${TARGETS[*]}"

cd "$PROJECT_ROOT"

SUCCESS_COUNT=0
FAIL_COUNT=0

for target in "${TARGETS[@]}"; do
    log_step "Compiling target: $target..."
    
    BUILD_SUCCESS=true
    if [[ "$ENGINE" == "cross" ]]; then
        cross build --target "$target" --release --config 'build.rustflags=[]' || BUILD_SUCCESS=false
    else
        cargo build --target "$target" --release --config 'build.rustflags=[]' || BUILD_SUCCESS=false
    fi

    if [[ "$BUILD_SUCCESS" == false ]]; then
        log_warn "Failed to compile for target '$target'. If using cargo, ensure target is installed via 'rustup target add $target' and mingw-w64 is present, or use '--engine cross'."
        FAIL_COUNT=$((FAIL_COUNT + 1))
        continue
    fi

    log_step "Locating built Windows binary for $target..."
    BINARY_PATH=$(find_built_binary "$target" "${APP_NAME}.exe" "release")
    log_info "Found binary: $BINARY_PATH"

    ARCHIVE_NAME="botai-v${VERSION}-windows-x86_64"
    log_step "Packaging $ARCHIVE_NAME.zip..."
    package_zip "$ARCHIVE_NAME" "$BINARY_PATH" "${APP_NAME}.exe"

    if [[ "$target" != "x86_64-pc-windows-gnu" ]]; then
        package_zip "botai-v${VERSION}-${target}" "$BINARY_PATH" "${APP_NAME}.exe"
    fi
    SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
done

if [[ "$GENERATE_CHECKSUM" == true && $SUCCESS_COUNT -gt 0 ]]; then
    generate_checksums "$DIST_DIR"
fi

if [[ $SUCCESS_COUNT -gt 0 ]]; then
    log_success "Windows artifacts build completed ($SUCCESS_COUNT succeeded, $FAIL_COUNT failed)."
else
    log_warn "Windows target builds were not completed."
fi

