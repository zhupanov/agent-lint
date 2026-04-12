#!/usr/bin/env bash
# apply-bump.sh — Apply a computed semver bump to package.json and Cargo.toml.
#
# Contract:
#   - FIRST: verify working tree is clean (fails on any staged or unstaged changes).
#   - Validate package.json with jq.
#   - Back up package.json and Cargo.toml (to git directory to avoid triggering dirty-tree guard on retry).
#   - Rewrite .version field in package.json atomically via jq + mv.
#   - Rewrite version field in Cargo.toml [package] section atomically via awk + mv.
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
CARGO_TOML="$PWD/Cargo.toml"
GIT_DIR="$(git rev-parse --git-dir)"
BACKUP="$GIT_DIR/package.json.bump-backup"
CARGO_BACKUP="$GIT_DIR/Cargo.toml.bump-backup"

# Step 1 (FIRST): Verify clean working tree.
if [[ -n "$(git status --porcelain 2>/dev/null)" ]]; then
  fail "Working tree is not clean (staged, unstaged, or untracked changes present); refusing to bump version. Commit, stash, or clean them first."
fi

# Step 2: Validate package.json parses.
[[ -f "$VERSION_FILE" ]] || fail "$VERSION_FILE not found"
jq empty "$VERSION_FILE" 2>/dev/null || fail "$VERSION_FILE is not valid JSON"

# Step 3: Backup before mutation (stored in git directory to avoid triggering dirty-tree guard).
cp "$VERSION_FILE" "$BACKUP"
if [[ -f "$CARGO_TOML" ]]; then
  cp "$CARGO_TOML" "$CARGO_BACKUP"
fi

# Step 4a: Atomic rewrite of package.json via jq + mv.
TMP_JSON="$VERSION_FILE.tmp.$$"
if ! jq --arg v "$NEW_VERSION" '.version = $v' "$VERSION_FILE" > "$TMP_JSON"; then
  rm -f "$TMP_JSON" "$BACKUP" "$CARGO_BACKUP"
  fail "jq rewrite failed"
fi
mv "$TMP_JSON" "$VERSION_FILE"

# Step 4b: Atomic rewrite of Cargo.toml [package] version via awk + mv.
if [[ -f "$CARGO_TOML" ]]; then
  TMP_CARGO="$CARGO_TOML.tmp.$$"
  awk -v new_ver="$NEW_VERSION" '
    /^\[package\]/ { in_package=1 }
    /^\[/ && !/^\[package\]/ { in_package=0 }
    in_package && /^version *= *"/ {
      sub(/"[^"]*"/, "\"" new_ver "\"")
      substituted=1
    }
    { print }
    END { if (!substituted) exit 1 }
  ' "$CARGO_TOML" > "$TMP_CARGO"
  AWK_EXIT=$?
  if [[ $AWK_EXIT -ne 0 ]] || [[ ! -s "$TMP_CARGO" ]]; then
    rm -f "$TMP_CARGO"
    # Restore package.json from backup since we already rewrote it.
    mv "$BACKUP" "$VERSION_FILE"
    rm -f "$CARGO_BACKUP"
    fail "Cargo.toml awk rewrite failed: [package] version field not found or empty output"
  fi
  # Verify the new version appears in the rewritten file.
  if ! grep -q "version = \"$NEW_VERSION\"" "$TMP_CARGO"; then
    rm -f "$TMP_CARGO"
    mv "$BACKUP" "$VERSION_FILE"
    rm -f "$CARGO_BACKUP"
    fail "Cargo.toml rewrite verification failed: version $NEW_VERSION not found in output"
  fi
  mv "$TMP_CARGO" "$CARGO_TOML"
fi

# Step 5: Stage and commit.
git add "$VERSION_FILE"
if [[ -f "$CARGO_TOML" ]]; then
  git add "$CARGO_TOML"
fi
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
if [[ -f "$CARGO_BACKUP" ]]; then
  mv "$CARGO_BACKUP" "$CARGO_TOML"
  git reset HEAD "$CARGO_TOML" >/dev/null 2>&1 || true
fi
git reset HEAD "$VERSION_FILE" >/dev/null 2>&1 || true
echo "APPLIED=false"
echo "ERROR=git commit failed; rolled back $VERSION_FILE and $CARGO_TOML from backup"
exit 1
