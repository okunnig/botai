#!/usr/bin/env bash
# scripts/build.sh - Cross-platform compilation & packaging using `cross`
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST_DIR="$ROOT_DIR/dist"
cd "$ROOT_DIR"

if ! command -v cross >/dev/null 2>&1; then
    echo "Error: 'cross' is required but not installed or not in PATH." >&2
    echo "Install it via: cargo install cross --git https://github.com/cross-rs/cross" >&2
    exit 1
fi

VERSION=$(grep -m1 '^version' Cargo.toml | sed -E 's/version[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/')
if [[ -z "$VERSION" ]]; then
    echo "Error: Could not determine version from Cargo.toml" >&2
    exit 1
fi

case "${1:-all}" in
    all|"")  TARGETS=("x86_64-unknown-linux-gnu" "aarch64-unknown-linux-gnu" "x86_64-pc-windows-gnu" "x86_64-apple-darwin" "aarch64-apple-darwin") ;;
    linux)   TARGETS=("x86_64-unknown-linux-gnu" "aarch64-unknown-linux-gnu") ;;
    windows) TARGETS=("x86_64-pc-windows-gnu") ;;
    macos)   TARGETS=("x86_64-apple-darwin" "aarch64-apple-darwin") ;;
    -h|--help)
        echo "Usage: $0 [all|linux|windows|macos|<target-triple>]"
        exit 0
        ;;
    *) TARGETS=("$1") ;;
esac

mkdir -p "$DIST_DIR"

find_binary() {
    local target="$1" bin="$2"
    for p in "${CARGO_TARGET_DIR:-}/$target/release/$bin" "/tmp/botai-build/$target/release/$bin" "$ROOT_DIR/target/$target/release/$bin"; do
        if [[ -n "$p" && -f "$p" ]]; then
            echo "$p"
            return 0
        fi
    done
    return 1
}

for target in "${TARGETS[@]}"; do
    echo "==> Building $target (v$VERSION)..."
    cross build --target "$target" --release

    bin="botai"
    [[ "$target" == *"windows"* ]] && bin="botai.exe"
    bin_path=$(find_binary "$target" "$bin") || {
        echo "Error: Binary '$bin' not found for '$target'" >&2
        exit 1
    }

    case "$target" in
        x86_64-unknown-linux-gnu) stem="botai-v${VERSION}-linux-x86_64" ;;
        aarch64-unknown-linux-gnu) stem="botai-v${VERSION}-linux-aarch64" ;;
        x86_64-pc-windows-gnu)     stem="botai-v${VERSION}-windows-x86_64" ;;
        x86_64-apple-darwin)       stem="botai-v${VERSION}-macos-x86_64" ;;
        aarch64-apple-darwin)      stem="botai-v${VERSION}-macos-aarch64" ;;
        *)                         stem="botai-v${VERSION}-${target}" ;;
    esac

    stage=$(mktemp -d)
    cp "$bin_path" "$stage/$bin"
    [[ -f README.md ]] && cp README.md "$stage/"
    [[ -f REQS.md ]] && cp REQS.md "$stage/"

    if [[ "$target" == *"windows"* ]]; then
        archive="$DIST_DIR/${stem}.zip"
        if command -v zip >/dev/null 2>&1; then
            (cd "$stage" && zip -q -r "$archive" .)
        else
            python3 -c "import shutil; shutil.make_archive('${archive%.zip}', 'zip', '$stage')"
        fi
    else
        archive="$DIST_DIR/${stem}.tar.gz"
        tar -czf "$archive" -C "$stage" .
    fi
    rm -rf "$stage"
    echo "==> Packaged: $archive"
done

echo "==> Generating SHA256SUMS..."
(
    cd "$DIST_DIR"
    archives=$(find . -maxdepth 1 -type f \( -name "*.tar.gz" -o -name "*.zip" \) -printf "%P\n" | sort)
    if [[ -n "$archives" ]]; then
        sha256sum $archives > SHA256SUMS
        echo "==> Checksums updated in $DIST_DIR/SHA256SUMS"
    fi
)

echo "==> Build and packaging completed successfully."
