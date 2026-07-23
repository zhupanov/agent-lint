#!/usr/bin/env bash
# Download, verify, and install the latest stable agent-lint release.

set -euo pipefail

REPO="zhupanov/agent-lint"
INSTALL_DIR="${AGENT_LINT_INSTALL_DIR:-/usr/local/bin}"

fail() {
  echo "ERROR: $*" >&2
  exit 1
}

[[ $# -eq 0 ]] || fail "upgrade-agent-lint does not accept arguments"

for required_command in gh tar awk install mktemp mv uname; do
  command -v "$required_command" >/dev/null 2>&1 ||
    fail "required command not found: $required_command"
done

case "$INSTALL_DIR" in
  /*) ;;
  *) fail "AGENT_LINT_INSTALL_DIR must be an absolute path" ;;
esac
[[ -d "$INSTALL_DIR" ]] ||
  fail "installation directory does not exist: $INSTALL_DIR"

OS=$(uname -s)
ARCH=$(uname -m)

case "$OS:$ARCH" in
  Darwin:arm64 | Darwin:aarch64)
    TARGET="aarch64-apple-darwin"
    ;;
  Linux:x86_64)
    TARGET="x86_64-unknown-linux-musl"
    ;;
  Linux:arm64 | Linux:aarch64)
    TARGET="aarch64-unknown-linux-musl"
    ;;
  *)
    fail "unsupported platform: $OS $ARCH"
    ;;
esac

TAG=$(gh release view --repo "$REPO" --json tagName --jq '.tagName')
[[ "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] ||
  fail "latest release has an invalid tag: $TAG"
VERSION="${TAG#v}"
ARCHIVE="agent-lint-v${VERSION}-${TARGET}.tar.gz"
CHECKSUMS="agent-lint-v${VERSION}-checksums.txt"

DOWNLOAD_DIR=$(mktemp -d)
STAGED_DESTINATION=""
STAGED_WITH_SUDO=false
cleanup() {
  rm -f \
    "$DOWNLOAD_DIR/$ARCHIVE" \
    "$DOWNLOAD_DIR/$CHECKSUMS" \
    "$DOWNLOAD_DIR/agent-lint"
  rmdir "$DOWNLOAD_DIR" 2>/dev/null || true
  if [[ -n "$STAGED_DESTINATION" ]]; then
    if [[ "$STAGED_WITH_SUDO" == true ]]; then
      sudo rm -f "$STAGED_DESTINATION" >/dev/null 2>&1 || true
    else
      rm -f "$STAGED_DESTINATION"
    fi
  fi
}
trap cleanup EXIT

gh release download "$TAG" \
  --repo "$REPO" \
  --dir "$DOWNLOAD_DIR" \
  --pattern "$ARCHIVE" \
  --pattern "$CHECKSUMS"

EXPECTED=$(
  awk -v archive="$ARCHIVE" '
    $2 == archive {
      count++
      checksum = $1
    }
    END {
      if (count == 1) {
        print checksum
      }
    }
  ' "$DOWNLOAD_DIR/$CHECKSUMS"
)
[[ "$EXPECTED" =~ ^[0-9a-fA-F]{64}$ ]] ||
  fail "checksum manifest does not contain exactly one valid entry for $ARCHIVE"

if command -v sha256sum >/dev/null 2>&1; then
  ACTUAL=$(sha256sum "$DOWNLOAD_DIR/$ARCHIVE" | awk '{print $1}')
elif command -v shasum >/dev/null 2>&1; then
  ACTUAL=$(shasum -a 256 "$DOWNLOAD_DIR/$ARCHIVE" | awk '{print $1}')
else
  fail "no SHA-256 checksum utility found"
fi

[[ "$ACTUAL" == "$EXPECTED" ]] ||
  fail "checksum verification failed for $ARCHIVE"

ARCHIVE_ENTRIES=$(tar -tzf "$DOWNLOAD_DIR/$ARCHIVE")
[[ "$ARCHIVE_ENTRIES" == "agent-lint" ||
  "$ARCHIVE_ENTRIES" == "./agent-lint" ]] ||
  fail "release archive must contain only the agent-lint executable"

tar -xzf "$DOWNLOAD_DIR/$ARCHIVE" -C "$DOWNLOAD_DIR"
[[ -f "$DOWNLOAD_DIR/agent-lint" && ! -L "$DOWNLOAD_DIR/agent-lint" ]] ||
  fail "release archive did not extract a regular agent-lint executable"

if ! SOURCE_VERSION_OUTPUT=$("$DOWNLOAD_DIR/agent-lint" --version); then
  fail "downloaded agent-lint executable failed its version check"
fi
[[ "$SOURCE_VERSION_OUTPUT" == "agent-lint $VERSION" ]] ||
  fail "downloaded binary reported '$SOURCE_VERSION_OUTPUT', expected 'agent-lint $VERSION'"

DESTINATION="$INSTALL_DIR/agent-lint"
STAGED_DESTINATION="$INSTALL_DIR/.agent-lint.upgrade.$$"
if [[ -w "$INSTALL_DIR" ]]; then
  install -m 0755 "$DOWNLOAD_DIR/agent-lint" "$STAGED_DESTINATION"
else
  command -v sudo >/dev/null 2>&1 ||
    fail "installation directory is not writable and sudo is unavailable: $INSTALL_DIR"
  STAGED_WITH_SUDO=true
  sudo install -m 0755 "$DOWNLOAD_DIR/agent-lint" "$STAGED_DESTINATION"
fi

if ! STAGED_VERSION_OUTPUT=$("$STAGED_DESTINATION" --version); then
  fail "staged agent-lint executable failed its version check"
fi
[[ "$STAGED_VERSION_OUTPUT" == "agent-lint $VERSION" ]] ||
  fail "staged binary reported '$STAGED_VERSION_OUTPUT', expected 'agent-lint $VERSION'"

if [[ "$STAGED_WITH_SUDO" == true ]]; then
  sudo mv -f "$STAGED_DESTINATION" "$DESTINATION"
else
  mv -f "$STAGED_DESTINATION" "$DESTINATION"
fi
STAGED_DESTINATION=""

if ! INSTALLED_VERSION_OUTPUT=$("$DESTINATION" --version); then
  fail "installed agent-lint executable failed its version check"
fi
[[ "$INSTALLED_VERSION_OUTPUT" == "agent-lint $VERSION" ]] ||
  fail "installed binary reported '$INSTALLED_VERSION_OUTPUT', expected 'agent-lint $VERSION'"

echo "AGENT_LINT_VERSION=$VERSION"
echo "AGENT_LINT_BINARY=$DESTINATION"
echo "AGENT_LINT_UPGRADED=true"
