#!/usr/bin/env bash
set -euo pipefail

# claude-lint installer for GitHub Actions
# Downloads a pre-built binary from GitHub Releases and adds it to PATH.

REPO="zhupanov/claude-lint"

# --- Platform guard -----------------------------------------------------------

if [ "${RUNNER_OS:-}" = "Windows" ]; then
  echo "::error::claude-lint GitHub Action does not support Windows runners."
  exit 1
fi

# --- Platform detection -------------------------------------------------------

OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Linux)  TARGET_OS="unknown-linux-musl" ;;
  Darwin) TARGET_OS="apple-darwin" ;;
  *)
    echo "::error::Unsupported operating system: $OS"
    exit 1
    ;;
esac

case "$ARCH" in
  x86_64)        TARGET_ARCH="x86_64" ;;
  aarch64|arm64) TARGET_ARCH="aarch64" ;;
  *)
    echo "::error::Unsupported architecture: $ARCH"
    exit 1
    ;;
esac

TARGET="${TARGET_ARCH}-${TARGET_OS}"

# --- Version resolution -------------------------------------------------------

# Strip optional leading 'v' to prevent vv-doubling in URLs
VERSION="${VERSION#v}"

if [ -z "$VERSION" ]; then
  echo "Resolving latest release..."
  VERSION="$(
    curl -fsSL \
      -H "Authorization: Bearer ${GITHUB_TOKEN}" \
      -H "Accept: application/vnd.github+json" \
      "https://api.github.com/repos/${REPO}/releases/latest" \
    | grep '"tag_name"' \
    | sed -E 's/.*"tag_name": *"v?([^"]+)".*/\1/'
  )"
  if [ -z "$VERSION" ]; then
    echo "::error::Failed to resolve latest release version."
    exit 1
  fi
  echo "Resolved latest version: ${VERSION}"
fi

# --- Download -----------------------------------------------------------------

BASE_URL="https://github.com/${REPO}/releases/download/v${VERSION}"
TARBALL="claude-lint-v${VERSION}-${TARGET}.tar.gz"
CHECKSUMS="claude-lint-v${VERSION}-checksums.txt"

INSTALL_DIR="$(mktemp -d)"
cd "$INSTALL_DIR"

echo "Downloading ${TARBALL}..."
curl -fsSL -o "$TARBALL" "${BASE_URL}/${TARBALL}" || {
  echo "::error::Failed to download ${BASE_URL}/${TARBALL}"
  exit 1
}

# --- Checksum verification ----------------------------------------------------

echo "Downloading checksums..."
curl -fsSL -o "$CHECKSUMS" "${BASE_URL}/${CHECKSUMS}" || {
  echo "::error::Failed to download ${BASE_URL}/${CHECKSUMS}"
  exit 1
}

# Cross-platform checksum command: sha256sum on Linux, shasum -a 256 on macOS
if command -v sha256sum > /dev/null 2>&1; then
  SHA_CMD="sha256sum"
elif command -v shasum > /dev/null 2>&1; then
  SHA_CMD="shasum -a 256"
else
  echo "::error::No SHA-256 checksum utility found (need sha256sum or shasum)."
  exit 1
fi

EXPECTED="$(grep "$TARBALL" "$CHECKSUMS" | awk '{ print $1 }')"
if [ -z "$EXPECTED" ]; then
  echo "::error::Checksum entry for ${TARBALL} not found in ${CHECKSUMS}."
  exit 1
fi

ACTUAL="$($SHA_CMD "$TARBALL" | awk '{ print $1 }')"
if [ "$EXPECTED" != "$ACTUAL" ]; then
  echo "::error::Checksum mismatch for ${TARBALL}. Expected: ${EXPECTED}, Got: ${ACTUAL}"
  exit 1
fi
echo "Checksum verified."

# --- Extract and install ------------------------------------------------------

tar -xzf "$TARBALL"
echo "${INSTALL_DIR}" >> "$GITHUB_PATH"
echo "claude-lint ${VERSION} installed for ${TARGET}."
