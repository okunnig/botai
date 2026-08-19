#!/usr/bin/env bash
# scripts/build.sh - Native compilation & packaging for host machine
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST_DIR="$ROOT_DIR/dist"
cd "$ROOT_DIR"

VERSION=$(grep -m1 '^version' Cargo.toml | sed -E 's/version[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/')
if [[ -z "$VERSION" ]]; then
    echo "Error: Could not determine version from Cargo.toml" >&2
    exit 1
fi

# Detect host operating system
OS_UNAME="$(uname -s)"
case "$OS_UNAME" in
    Linux*)   OS="linux" ;;
    Darwin*)  OS="macos" ;;
    CYGWIN*|MINGW*|MSYS*|Windows*) OS="windows" ;;
    *)        OS="$(echo "$OS_UNAME" | tr '[:upper:]' '[:lower:]')" ;;
esac

# Detect host architecture
ARCH_UNAME="$(uname -m)"
case "$ARCH_UNAME" in
    x86_64|amd64)   ARCH="x86_64" ;;
    aarch64|arm64)  ARCH="aarch64" ;;
    *)              ARCH="$ARCH_UNAME" ;;
esac

PLATFORM="${OS}-${ARCH}"
STEM="botai-v${VERSION}-${PLATFORM}"

if [[ "$OS" == "windows" ]]; then
    BIN="botai.exe"
else
    BIN="botai"
fi

mkdir -p "$DIST_DIR"

echo "==> Building botai v${VERSION} for native host (${PLATFORM})..."
cargo build --release

find_binary() {
    local bin="$1"
    for p in "${CARGO_TARGET_DIR:-}/release/$bin" "/tmp/botai-build/release/$bin" "$ROOT_DIR/target/release/$bin"; do
        if [[ -n "$p" && -f "$p" ]]; then
            echo "$p"
            return 0
        fi
    done
    return 1
}

BIN_PATH=$(find_binary "$BIN") || {
    echo "Error: Release binary '$BIN' not found." >&2
    exit 1
}

STAGE_DIR=$(mktemp -d)
cleanup() {
    rm -rf "$STAGE_DIR"
}
trap cleanup EXIT

cp "$BIN_PATH" "$STAGE_DIR/$BIN"
[[ -f README.md ]] && cp README.md "$STAGE_DIR/"
[[ -f REQS.md ]] && cp REQS.md "$STAGE_DIR/"

if [[ "$OS" == "windows" ]]; then
    ARCHIVE="$DIST_DIR/${STEM}.zip"
    if command -v zip >/dev/null 2>&1; then
        (cd "$STAGE_DIR" && zip -q -r "$ARCHIVE" .)
    elif command -v python3 >/dev/null 2>&1; then
        python3 -c "import shutil; shutil.make_archive('${ARCHIVE%.zip}', 'zip', '$STAGE_DIR')"
    else
        echo "Error: Neither zip nor python3 is available to create .zip archive" >&2
        exit 1
    fi
else
    ARCHIVE="$DIST_DIR/${STEM}.tar.gz"
    tar -czf "$ARCHIVE" -C "$STAGE_DIR" .
fi

echo "==> Packaged: $ARCHIVE"

echo ""
echo "==> Generating SHA256SUMS..."
(
    cd "$DIST_DIR"
    shopt -s nullglob
    archives=(*.tar.gz *.zip)
    shopt -u nullglob
    if [[ ${#archives[@]} -gt 0 ]]; then
        if command -v sha256sum >/dev/null 2>&1; then
            sha256sum "${archives[@]}" > SHA256SUMS
        elif command -v shasum >/dev/null 2>&1; then
            shasum -a 256 "${archives[@]}" > SHA256SUMS
        else
            python3 -c "
import hashlib, glob
with open('SHA256SUMS', 'w') as out:
    for fname in sorted(glob.glob('*.tar.gz') + glob.glob('*.zip')):
        with open(fname, 'rb') as f:
            digest = hashlib.sha256(f.read()).hexdigest()
        out.write(f'{digest}  {fname}\n')
"
        fi
        echo "==> Checksums updated in $DIST_DIR/SHA256SUMS"
    fi
)

echo ""
echo "==> Build and packaging completed successfully."
echo "==> Distributable artifacts available in: $DIST_DIR"


