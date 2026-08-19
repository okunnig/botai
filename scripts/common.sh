#!/usr/bin/env bash
# ==============================================================================
# scripts/common.sh - Common Utilities for botai Build & Packaging
# ==============================================================================
# Provides shared variables, version parsing, build engine detection (cross/cargo),
# target directory resolution, archive packaging (tar.gz, zip), and checksum tools.
# ==============================================================================

set -euo pipefail

# ------------------------------------------------------------------------------
# Directory & Project Context
# ------------------------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DEFAULT_DIST_DIR="$PROJECT_ROOT/dist"
DIST_DIR="${DIST_DIR:-$DEFAULT_DIST_DIR}"
APP_NAME="botai"
CARGO_TOML="$PROJECT_ROOT/Cargo.toml"
README_MD="$PROJECT_ROOT/README.md"
REQS_MD="$PROJECT_ROOT/REQS.md"

# ------------------------------------------------------------------------------
# Terminal Colors & Styling
# ------------------------------------------------------------------------------
if [[ -t 1 ]] && [[ -z "${NO_COLOR:-}" ]]; then
    COLOR_RESET="\033[0m"
    COLOR_BOLD="\033[1m"
    COLOR_DIM="\033[2m"
    COLOR_RED="\033[31m"
    COLOR_GREEN="\033[32m"
    COLOR_YELLOW="\033[33m"
    COLOR_BLUE="\033[34m"
    COLOR_MAGENTA="\033[35m"
    COLOR_CYAN="\033[36m"
    COLOR_WHITE="\033[37m"
else
    COLOR_RESET=""
    COLOR_BOLD=""
    COLOR_DIM=""
    COLOR_RED=""
    COLOR_GREEN=""
    COLOR_YELLOW=""
    COLOR_BLUE=""
    COLOR_MAGENTA=""
    COLOR_CYAN=""
    COLOR_WHITE=""
fi

# ------------------------------------------------------------------------------
# Logging Helpers
# ------------------------------------------------------------------------------
log_info() {
    printf "${COLOR_BLUE}${COLOR_BOLD}[INFO]${COLOR_RESET} %s\n" "$*"
}

log_step() {
    printf "${COLOR_CYAN}${COLOR_BOLD}[STEP]${COLOR_RESET} %s\n" "$*"
}

log_success() {
    printf "${COLOR_GREEN}${COLOR_BOLD}[SUCCESS]${COLOR_RESET} %s\n" "$*"
}

log_warn() {
    printf "${COLOR_YELLOW}${COLOR_BOLD}[WARN]${COLOR_RESET} %s\n" "$*" >&2
}

log_error() {
    printf "${COLOR_RED}${COLOR_BOLD}[ERROR]${COLOR_RESET} %s\n" "$*" >&2
}

log_header() {
    printf "\n${COLOR_MAGENTA}${COLOR_BOLD}==============================================================================${COLOR_RESET}\n"
    printf "${COLOR_MAGENTA}${COLOR_BOLD} %s${COLOR_RESET}\n" "$*"
    printf "${COLOR_MAGENTA}${COLOR_BOLD}==============================================================================${COLOR_RESET}\n\n"
}

# ------------------------------------------------------------------------------
# Version Extraction
# ------------------------------------------------------------------------------
get_version() {
    if [[ ! -f "$CARGO_TOML" ]]; then
        log_error "Cargo.toml not found at $CARGO_TOML"
        return 1
    fi
    local version
    version=$(grep -m1 '^version[[:space:]]*=' "$CARGO_TOML" | sed -E 's/version[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/')
    if [[ -z "$version" ]]; then
        log_error "Could not parse version from $CARGO_TOML"
        return 1
    fi
    echo "$version"
}

# ------------------------------------------------------------------------------
# Build Engine Detection (cross / cargo)
# ------------------------------------------------------------------------------
detect_build_engine() {
    local engine="${BUILD_ENGINE:-auto}"

    if [[ "$engine" == "cross" ]]; then
        if ! command -v cross >/dev/null 2>&1; then
            log_error "'cross' requested but not installed or not in PATH."
            return 1
        fi
        _configure_container_engine
        echo "cross"
        return 0
    elif [[ "$engine" == "cargo" ]]; then
        if ! command -v cargo >/dev/null 2>&1; then
            log_error "'cargo' requested but not installed or not in PATH."
            return 1
        fi
        echo "cargo"
        return 0
    fi

    # Auto-detection: prioritize `cross` for isolated containerized builds
    if command -v cross >/dev/null 2>&1; then
        _configure_container_engine
        echo "cross"
        return 0
    elif command -v cargo >/dev/null 2>&1; then
        echo "cargo"
        return 0
    else
        log_error "Neither 'cross' nor 'cargo' was found in PATH."
        return 1
    fi
}

_configure_container_engine() {
    if [[ -z "${CROSS_CONTAINER_ENGINE:-}" ]]; then
        if command -v podman >/dev/null 2>&1; then
            export CROSS_CONTAINER_ENGINE="podman"
        elif command -v docker >/dev/null 2>&1; then
            export CROSS_CONTAINER_ENGINE="docker"
        else
            log_warn "Neither podman nor docker detected; cross may fail if container runtime is needed."
        fi
    fi
}

# ------------------------------------------------------------------------------
# Target Directory & Binary Resolution
# ------------------------------------------------------------------------------
get_cargo_target_dir() {
    if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
        echo "$CARGO_TARGET_DIR"
        return 0
    fi

    local cargo_config="$PROJECT_ROOT/.cargo/config.toml"
    if [[ -f "$cargo_config" ]]; then
        local configured_dir
        configured_dir=$(grep -m1 '^[[:space:]]*target-dir[[:space:]]*=' "$cargo_config" | sed -E 's/^[[:space:]]*target-dir[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/' || true)
        if [[ -n "$configured_dir" ]]; then
            echo "$configured_dir"
            return 0
        fi
    fi

    if [[ -d "/tmp/botai-build" ]]; then
        echo "/tmp/botai-build"
        return 0
    fi

    echo "$PROJECT_ROOT/target"
}

find_built_binary() {
    local target="$1"
    local bin_name="${2:-$APP_NAME}"
    local profile="${3:-release}"

    local cargo_target_dir
    cargo_target_dir="$(get_cargo_target_dir)"

    local search_paths=(
        "$PROJECT_ROOT/target/$target/$profile/$bin_name"
        "$cargo_target_dir/$target/$profile/$bin_name"
        "/tmp/botai-build/$target/$profile/$bin_name"
        "$PROJECT_ROOT/target/$profile/$bin_name"
        "$cargo_target_dir/$profile/$bin_name"
        "/tmp/botai-build/$profile/$bin_name"
    )

    for path in "${search_paths[@]}"; do
        if [[ -f "$path" ]]; then
            echo "$path"
            return 0
        fi
    done

    log_error "Could not find built binary '$bin_name' for target '$target' in any target directory."
    return 1
}

# ------------------------------------------------------------------------------
# Archive Packaging Helpers
# ------------------------------------------------------------------------------
ensure_dist_dir() {
    mkdir -p "$DIST_DIR"
}

package_tar_gz() {
    local archive_name="$1"
    local binary_src="$2"
    local binary_dest_name="$3"
    shift 3
    local extra_files=("$@")

    ensure_dist_dir
    local staging_dir
    staging_dir=$(mktemp -d -t "botai-stage-XXXXXX")
    trap 'rm -rf "$staging_dir"' RETURN

    cp "$binary_src" "$staging_dir/$binary_dest_name"
    chmod 755 "$staging_dir/$binary_dest_name"

    # Copy documentation files
    if [[ -f "$README_MD" ]]; then
        cp "$README_MD" "$staging_dir/README.md"
    fi
    if [[ -f "$REQS_MD" ]]; then
        cp "$REQS_MD" "$staging_dir/REQS.md"
    fi

    for file in "${extra_files[@]}"; do
        if [[ -f "$file" ]]; then
            cp "$file" "$staging_dir/"
        fi
    done

    local archive_path="$DIST_DIR/${archive_name}.tar.gz"
    tar -czf "$archive_path" -C "$staging_dir" .

    local size
    size=$(du -h "$archive_path" | cut -f1)
    log_success "Packaged: $archive_path ($size)"
}

package_zip() {
    local archive_name="$1"
    local binary_src="$2"
    local binary_dest_name="$3"
    shift 3
    local extra_files=("$@")

    ensure_dist_dir
    local staging_dir
    staging_dir=$(mktemp -d -t "botai-stage-zip-XXXXXX")
    trap 'rm -rf "$staging_dir"' RETURN

    cp "$binary_src" "$staging_dir/$binary_dest_name"

    # Copy documentation files
    if [[ -f "$README_MD" ]]; then
        cp "$README_MD" "$staging_dir/README.md"
    fi
    if [[ -f "$REQS_MD" ]]; then
        cp "$REQS_MD" "$staging_dir/REQS.md"
    fi

    for file in "${extra_files[@]}"; do
        if [[ -f "$file" ]]; then
            cp "$file" "$staging_dir/"
        fi
    done

    local archive_path="$DIST_DIR/${archive_name}.zip"
    if command -v zip >/dev/null 2>&1; then
        (cd "$staging_dir" && zip -q -r "$archive_path" .)
    else
        python3 -c "import shutil; shutil.make_archive('${archive_path%.zip}', 'zip', '$staging_dir')"
    fi

    local size
    size=$(du -h "$archive_path" | cut -f1)
    log_success "Packaged: $archive_path ($size)"
}

# ------------------------------------------------------------------------------
# Checksum Manifest & Formatted Output
# ------------------------------------------------------------------------------
generate_checksums() {
    local target_dist="${1:-$DIST_DIR}"
    if [[ ! -d "$target_dist" ]]; then
        log_warn "Dist directory $target_dist does not exist."
        return 0
    fi

    local checksum_file="$target_dist/SHA256SUMS"
    log_step "Generating SHA-256 checksums in $checksum_file..."

    (
        cd "$target_dist"
        # Find all .tar.gz and .zip files, excluding previous checksum files
        local archives
        archives=$(find . -maxdepth 1 -type f \( -name "*.tar.gz" -o -name "*.zip" \) -printf "%P\n" | sort)
        if [[ -n "$archives" ]]; then
            sha256sum $archives > "SHA256SUMS"
            log_success "Generated SHA256SUMS for $(echo "$archives" | wc -l) artifact(s)."
        else
            log_warn "No archive artifacts found to generate checksums."
        fi
    )
}

print_artifact_table() {
    local target_dist="${1:-$DIST_DIR}"
    local checksum_file="$target_dist/SHA256SUMS"

    log_header "Distributable Artifacts Summary"

    if [[ ! -f "$checksum_file" ]] || [[ ! -s "$checksum_file" ]]; then
        log_warn "No checksum manifest found at $checksum_file."
        return 0
    fi

    printf "${COLOR_BOLD}%-46s %-10s %-64s${COLOR_RESET}\n" "Artifact File" "Size" "SHA-256 Checksum"
    printf "%s\n" "----------------------------------------------------------------------------------------------------------------------------------"

    while read -r hash file; do
        if [[ -n "$file" && -f "$target_dist/$file" ]]; then
            local size
            size=$(du -h "$target_dist/$file" | cut -f1)
            printf "%-46s %-10s ${COLOR_DIM}%s${COLOR_RESET}\n" "$file" "$size" "$hash"
        fi
    done < "$checksum_file"

    printf "%s\n\n" "----------------------------------------------------------------------------------------------------------------------------------"
    log_info "Artifacts location: $target_dist"
}
