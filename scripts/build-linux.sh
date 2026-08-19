#!/usr/bin/env bash
# ==============================================================================
# scripts/build-linux.sh - Linux Cross-Compilation & Packaging for botai
# ==============================================================================
# Builds Linux binaries for:
#   - x86_64-unknown-linux-gnu
#   - x86_64-unknown-linux-musl
#   - aarch64-unknown-linux-gnu
#   - aarch64-unknown-linux-musl
#
# Packages each into dist/botai-<version>-linux-<arch>.tar.gz containing:
#   - botai (executable)
#   - README.md
#   - REQS.md
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/common.sh"

# Default targets
DEFAULT_TARGETS=(
    "x86_64-unknown-linux-gnu"
    "x86_64-unknown-linux-musl"
    "aarch64-unknown-linux-gnu"
    "aarch64-unknown-linux-musl"
)

TARGETS=()
GENERATE_CHECKSUM=true

usage() {
    cat <<EOF
Usage: $(basename "$0") [OPTIONS]

Builds and packages Linux release binaries for botai.

Options:
    -t, --target <TARGET>       Specific target triple to build (can be repeated)
    -e, --engine <ENGINE>       Build engine to use: 'cross' or 'cargo' (default: auto/cross)
    -d, --dist-dir <DIR>        Output directory for distribution archives (default: dist/)
        --no-checksum           Skip automatic SHA-256 checksum generation
    -h, --help                  Show this help message

Default Targets:
    - x86_64-unknown-linux-gnu   (glibc x86_64)
    - x86_64-unknown-linux-musl  (static musl x86_64)
    - aarch64-unknown-linux-gnu  (glibc ARM64)
    - aarch64-unknown-linux-musl (static musl ARM64)
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

log_header "Building Linux Release Artifacts (v$VERSION)"
log_info "Build Engine: $ENGINE"
if [[ "$ENGINE" == "cross" ]]; then
    log_info "Container Engine: ${CROSS_CONTAINER_ENGINE:-default}"
fi
log_info "Output Directory: $DIST_DIR"
log_info "Targets: ${TARGETS[*]}"

# Map target triple to descriptive archive suffix
get_archive_name() {
    local target="$1"
    local version="$2"
    case "$target" in
        "x86_64-unknown-linux-gnu")
            echo "botai-v${version}-linux-x86_64-gnu"
            ;;
        "x86_64-unknown-linux-musl")
            echo "botai-v${version}-linux-x86_64-musl"
            ;;
        "aarch64-unknown-linux-gnu")
            echo "botai-v${version}-linux-aarch64-gnu"
            ;;
        "aarch64-unknown-linux-musl")
            echo "botai-v${version}-linux-aarch64-musl"
            ;;
        *)
            echo "botai-v${version}-${target}"
            ;;
    esac
}

cd "$PROJECT_ROOT"

for target in "${TARGETS[@]}"; do
    log_step "Compiling target: $target..."
    
    if [[ "$ENGINE" == "cross" ]]; then
        cross build --target "$target" --release --config 'build.rustflags=[]'
    else
        cargo build --target "$target" --release --config 'build.rustflags=[]'
    fi

    log_step "Locating built binary for $target..."
    BINARY_PATH=$(find_built_binary "$target" "$APP_NAME" "release")
    log_info "Found binary: $BINARY_PATH"

    ARCHIVE_NAME=$(get_archive_name "$target" "$VERSION")
    log_step "Packaging $ARCHIVE_NAME.tar.gz..."
    package_tar_gz "$ARCHIVE_NAME" "$BINARY_PATH" "$APP_NAME"

    # Also create standard alias for the default x86_64 GNU target if built
    if [[ "$target" == "x86_64-unknown-linux-gnu" ]]; then
        package_tar_gz "botai-v${VERSION}-linux-x86_64" "$BINARY_PATH" "$APP_NAME"
    elif [[ "$target" == "aarch64-unknown-linux-gnu" ]]; then
        package_tar_gz "botai-v${VERSION}-linux-aarch64" "$BINARY_PATH" "$APP_NAME"
    fi
done

if [[ "$GENERATE_CHECKSUM" == true ]]; then
    generate_checksums "$DIST_DIR"
fi

log_success "Linux artifacts build completed successfully."
