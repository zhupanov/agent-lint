#!/usr/bin/env bash
# Black-box regression harness for the private upgrade-agent-lint helper.

set -euo pipefail

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
UPGRADER="$REPO_ROOT/.claude/skills/upgrade-agent-lint/scripts/upgrade-agent-lint.sh"
TEST_ROOT=$(mktemp -d)
trap 'rm -rf "$TEST_ROOT"' EXIT

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

assert_contains() {
  local output="$1"
  local expected="$2"
  [[ "$output" == *"$expected"* ]] ||
    fail "expected '$expected' in output: $output"
}

SHIM_DIR="$TEST_ROOT/shims"
ASSET_DIR="$TEST_ROOT/assets"
INSTALL_DIR="$TEST_ROOT/install"
mkdir -p "$SHIM_DIR" "$ASSET_DIR" "$INSTALL_DIR"

write_checksums() {
  (
    cd "$ASSET_DIR"
    if command -v sha256sum >/dev/null 2>&1; then
      sha256sum agent-lint-v9.8.7-aarch64-apple-darwin.tar.gz \
        >agent-lint-v9.8.7-checksums.txt
    else
      shasum -a 256 agent-lint-v9.8.7-aarch64-apple-darwin.tar.gz \
        >agent-lint-v9.8.7-checksums.txt
    fi
  )
}

printf '%s\n' \
  '#!/usr/bin/env bash' \
  'echo "agent-lint 9.8.7"' \
  >"$ASSET_DIR/agent-lint"
chmod +x "$ASSET_DIR/agent-lint"
tar -czf \
  "$ASSET_DIR/agent-lint-v9.8.7-aarch64-apple-darwin.tar.gz" \
  -C "$ASSET_DIR" agent-lint
write_checksums

cat >"$SHIM_DIR/uname" <<'EOF'
#!/usr/bin/env bash
case "${1:-}" in
  -s) printf '%s\n' "${TEST_UNAME_OS:-Darwin}" ;;
  -m) printf '%s\n' "${TEST_UNAME_ARCH:-arm64}" ;;
  *) exit 1 ;;
esac
EOF

cat >"$SHIM_DIR/gh" <<'EOF'
#!/usr/bin/env bash
if [[ "${1:-}" == "release" && "${2:-}" == "view" ]]; then
  echo "v9.8.7"
  exit 0
fi
if [[ "${1:-}" == "release" && "${2:-}" == "download" ]]; then
  shift 2
  destination=""
  patterns=()
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --dir)
        destination="$2"
        shift 2
        ;;
      --pattern)
        patterns+=("$2")
        shift 2
        ;;
      *)
        shift
        ;;
    esac
  done
  [[ -n "$destination" ]] || exit 2
  for pattern in "${patterns[@]}"; do
    cp "$TEST_ASSET_DIR/$pattern" "$destination/$pattern"
  done
  exit 0
fi
exit 2
EOF
chmod +x "$SHIM_DIR/uname" "$SHIM_DIR/gh"

upgrade_output=$(
  PATH="$SHIM_DIR:$PATH" \
    TEST_ASSET_DIR="$ASSET_DIR" \
    AGENT_LINT_INSTALL_DIR="$INSTALL_DIR" \
    "$UPGRADER"
)
assert_contains "$upgrade_output" "AGENT_LINT_VERSION=9.8.7"
assert_contains "$upgrade_output" "AGENT_LINT_BINARY=$INSTALL_DIR/agent-lint"
assert_contains "$upgrade_output" "AGENT_LINT_UPGRADED=true"
[[ "$("$INSTALL_DIR/agent-lint" --version)" == "agent-lint 9.8.7" ]] ||
  fail "installed executable did not report the downloaded version"

printf '%s\n' '#!/usr/bin/env bash' 'echo tampered' \
  >"$ASSET_DIR/agent-lint"
tar -czf \
  "$ASSET_DIR/agent-lint-v9.8.7-aarch64-apple-darwin.tar.gz" \
  -C "$ASSET_DIR" agent-lint
set +e
checksum_output=$(
  PATH="$SHIM_DIR:$PATH" \
    TEST_ASSET_DIR="$ASSET_DIR" \
    AGENT_LINT_INSTALL_DIR="$INSTALL_DIR" \
    "$UPGRADER" 2>&1
)
checksum_rc=$?
set -e
[[ "$checksum_rc" -ne 0 ]] || fail "checksum mismatch unexpectedly succeeded"
assert_contains "$checksum_output" "checksum verification failed"
[[ "$("$INSTALL_DIR/agent-lint" --version)" == "agent-lint 9.8.7" ]] ||
  fail "checksum failure replaced the existing installation"

write_checksums
set +e
version_output=$(
  PATH="$SHIM_DIR:$PATH" \
    TEST_ASSET_DIR="$ASSET_DIR" \
    AGENT_LINT_INSTALL_DIR="$INSTALL_DIR" \
    "$UPGRADER" 2>&1
)
version_rc=$?
set -e
[[ "$version_rc" -ne 0 ]] || fail "wrong-version binary unexpectedly succeeded"
assert_contains "$version_output" "downloaded binary reported 'tampered'"
[[ "$("$INSTALL_DIR/agent-lint" --version)" == "agent-lint 9.8.7" ]] ||
  fail "version failure replaced the existing installation"

set +e
platform_output=$(
  PATH="$SHIM_DIR:$PATH" \
    TEST_UNAME_OS="FreeBSD" \
    TEST_UNAME_ARCH="x86_64" \
    TEST_ASSET_DIR="$ASSET_DIR" \
    AGENT_LINT_INSTALL_DIR="$INSTALL_DIR" \
    "$UPGRADER" 2>&1
)
platform_rc=$?
set -e
[[ "$platform_rc" -ne 0 ]] || fail "unsupported platform unexpectedly succeeded"
assert_contains "$platform_output" "unsupported platform: FreeBSD x86_64"

echo "Upgrade agent-lint tests passed."
