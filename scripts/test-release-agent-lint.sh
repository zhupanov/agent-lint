#!/usr/bin/env bash
# Regression checks for the release skill's manual local-install handoff.

set -euo pipefail

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
RELEASE_SKILL="$REPO_ROOT/.claude/skills/release-agent-lint/SKILL.md"
UPGRADE_SKILL="$REPO_ROOT/.claude/skills/upgrade-agent-lint/SKILL.md"
INSTALLER_README="$REPO_ROOT/.claude/skills/upgrade-agent-lint/README.md"
INSTALL_COMMAND='sudo .claude/skills/upgrade-agent-lint/scripts/upgrade-agent-lint.sh'

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

assert_exact_line() {
  local file="$1"
  grep -F "$INSTALL_COMMAND" "$file" |
    grep -Eq '^[[:space:]]*sudo \.claude/skills/upgrade-agent-lint/scripts/upgrade-agent-lint\.sh$' ||
    fail "expected the manual install command in ${file#"$REPO_ROOT"/}"
}

[[ ! -e "$UPGRADE_SKILL" ]] || fail "the retired upgrade skill still exists"
assert_exact_line "$RELEASE_SKILL"
assert_exact_line "$INSTALLER_README"
assert_exact_line "$REPO_ROOT/README.md"
assert_exact_line "$REPO_ROOT/docs/development.md"

if grep -Fq 'Bash(.claude/skills/upgrade-agent-lint/scripts/upgrade-agent-lint.sh:*)' "$RELEASE_SKILL"; then
  fail "the release skill may not invoke the installer"
fi

echo "Release agent-lint handoff tests passed."
