#!/usr/bin/env bash
# apply-bump.sh — Apply a computed semver bump to package.json.
#
# Contract:
#   - FIRST: verify working tree is clean (fails on any staged or unstaged changes).
#   - Validate package.json with jq.
#   - Back up package.json (to .git/ to avoid triggering dirty-tree guard on retry).
#   - Rewrite .version field atomically via jq + mv.
#   - git add + commit with message "Bump version to <new-version>".
#   - Roll back from backup if git commit fails.
#
# Usage:
#   apply-bump.sh --new-version <x.y.z>
#
# Output (stdout):
#   APPLIED=true|false
#   COMMIT_SHA=<sha>             (if APPLIED=true)
#   ERROR=<message>              (if APPLIED=false)
#
# Exit codes: 0 on success, 1 on invalid args / validation / dirty worktree / commit failure.

set -euo pipefail

# fail MESSAGE — emit APPLIED=false / ERROR=MESSAGE on stdout and exit 1.
fail() {
  echo "APPLIED=false"
  echo "ERROR=$1"
  exit 1
}

NEW_VERSION=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --new-version)
      if [[ $# -lt 2 || -z "${2:-}" ]]; then
        fail "Missing value for --new-version"
      fi
      NEW_VERSION="$2"
      shift 2
      ;;
    *) fail "Unknown argument: $1" ;;
  esac
done

if [[ -z "$NEW_VERSION" ]]; then
  fail "Missing required argument: --new-version"
fi

if ! [[ "$NEW_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  fail "--new-version '$NEW_VERSION' is not semver (expected X.Y.Z)"
fi

VERSION_FILE="$PWD/package.json"
BACKUP="$PWD/.git/package.json.bump-backup"

# Step 1 (FIRST): Verify clean working tree.
if [[ -n "$(git status --porcelain 2>/dev/null)" ]]; then
  fail "Working tree is not clean (staged, unstaged, or untracked changes present); refusing to bump version. Commit, stash, or clean them first."
fi

# Step 2: Validate package.json parses.
[[ -f "$VERSION_FILE" ]] || fail "$VERSION_FILE not found"
jq empty "$VERSION_FILE" 2>/dev/null || fail "$VERSION_FILE is not valid JSON"

# Step 3: Backup before mutation (stored in .git/ to avoid triggering dirty-tree guard).
cp "$VERSION_FILE" "$BACKUP"

# Step 4: Atomic rewrite via jq + mv.
TMP_JSON="$VERSION_FILE.tmp.$$"
if ! jq --arg v "$NEW_VERSION" '.version = $v' "$VERSION_FILE" > "$TMP_JSON"; then
  rm -f "$TMP_JSON" "$BACKUP"
  fail "jq rewrite failed"
fi
mv "$TMP_JSON" "$VERSION_FILE"

# Step 5: Stage and commit.
git add "$VERSION_FILE"
COMMIT_MSG="Bump version to $NEW_VERSION"
if git commit -m "$COMMIT_MSG" --quiet; then
  # Success — remove backup, emit result.
  rm -f "$BACKUP"
  COMMIT_SHA=$(git rev-parse HEAD)
  echo "APPLIED=true"
  echo "COMMIT_SHA=$COMMIT_SHA"
  exit 0
fi

# Step 6: Rollback on commit failure.
mv "$BACKUP" "$VERSION_FILE"
git reset HEAD "$VERSION_FILE" >/dev/null 2>&1 || true
echo "APPLIED=false"
echo "ERROR=git commit failed; rolled back $VERSION_FILE from backup"
exit 1
