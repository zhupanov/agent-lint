#!/usr/bin/env bash
set -euo pipefail

# pre-commit hook for agent-lint
# Downloads the pre-built binary from GitHub Releases (cached) and runs it.

REPO="zhupanov/agent-lint"
HOOK_DIR="$(cd "$(dirname "$0")/.." && pwd)"

# --- Version from hook repo checkout -----------------------------------------

VERSION="$(sed -n 's/.*"version": *"\([^"]*\)".*/\1/p' "$HOOK_DIR/package.json" | head -1)"
if [ -z "$VERSION" ]; then
  echo "error: could not read version from package.json" >&2
  exit 1
fi

# --- Platform detection -------------------------------------------------------

OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Linux)  TARGET_OS="unknown-linux-musl" ;;
  Darwin) TARGET_OS="apple-darwin" ;;
  *)
    echo "error: unsupported operating system: $OS" >&2
    exit 1
    ;;
esac

case "$ARCH" in
  x86_64)
    if [ "$OS" = "Darwin" ]; then
      echo "error: Intel macOS (x86_64) is not supported. Use Apple Silicon." >&2
      exit 1
    fi
    TARGET_ARCH="x86_64"
    ;;
  aarch64|arm64) TARGET_ARCH="aarch64" ;;
  *)
    echo "error: unsupported architecture: $ARCH" >&2
    exit 1
    ;;
esac

TARGET="${TARGET_ARCH}-${TARGET_OS}"

# --- Cache check --------------------------------------------------------------

CACHE_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/agent-lint-pre-commit/v${VERSION}"
BINARY="$CACHE_DIR/agent-lint"

if [ -x "$BINARY" ]; then
  exec "$BINARY" "$@"
fi

# --- Download -----------------------------------------------------------------

BASE_URL="https://github.com/${REPO}/releases/download/v${VERSION}"
TARBALL="agent-lint-v${VERSION}-${TARGET}.tar.gz"
CHECKSUMS="agent-lint-v${VERSION}-checksums.txt"

TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

echo "Downloading agent-lint v${VERSION} for ${TARGET}..." >&2
curl -fsSL -o "$TMPDIR/$TARBALL" "${BASE_URL}/${TARBALL}" || {
  echo "error: failed to download ${BASE_URL}/${TARBALL}" >&2
  exit 1
}

# --- Checksum verification ----------------------------------------------------

curl -fsSL -o "$TMPDIR/$CHECKSUMS" "${BASE_URL}/${CHECKSUMS}" || {
  echo "error: failed to download checksums" >&2
  exit 1
}

if command -v sha256sum > /dev/null 2>&1; then
  SHA_CMD="sha256sum"
elif command -v shasum > /dev/null 2>&1; then
  SHA_CMD="shasum -a 256"
else
  echo "error: no SHA-256 checksum utility found (need sha256sum or shasum)" >&2
  exit 1
fi

EXPECTED="$(grep "$TARBALL" "$TMPDIR/$CHECKSUMS" | awk '{ print $1 }')"
if [ -z "$EXPECTED" ]; then
  echo "error: checksum entry for ${TARBALL} not found" >&2
  exit 1
fi

ACTUAL="$($SHA_CMD "$TMPDIR/$TARBALL" | awk '{ print $1 }')"
if [ "$EXPECTED" != "$ACTUAL" ]; then
  echo "error: checksum mismatch. Expected: ${EXPECTED}, Got: ${ACTUAL}" >&2
  exit 1
fi

# --- Extract and cache --------------------------------------------------------

tar -xzf "$TMPDIR/$TARBALL" -C "$TMPDIR"
mkdir -p "$CACHE_DIR"
mv "$TMPDIR/agent-lint" "$BINARY"
chmod +x "$BINARY"
echo "Cached agent-lint v${VERSION} at ${BINARY}" >&2

exec "$BINARY" "$@"
