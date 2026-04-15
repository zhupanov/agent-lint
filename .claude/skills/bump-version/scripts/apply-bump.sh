#!/usr/bin/env bash
# apply-bump.sh — Apply a computed semver bump to package.json, Cargo.toml, and README.md.
#
# Contract:
#   - FIRST: verify working tree is clean (fails on any staged or unstaged changes).
#   - Validate package.json with jq.
#   - Back up package.json, Cargo.toml, and README.md (to git directory to avoid triggering dirty-tree guard on retry).
#   - Rewrite .version field in package.json atomically via jq + mv.
#   - Rewrite version field in Cargo.toml [package] section atomically via awk + mv.
#   - Replace all occurrences of old version (X.Y.Z) with new version in README.md.
#   - git add + commit with message "Bump version to <new-version>".
#   - Roll back all four files from backup if git commit fails.
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
CARGO_LOCK="$PWD/Cargo.lock"
CARGO_LOCK_BACKUP="$GIT_DIR/Cargo.lock.bump-backup"
README_FILE="$PWD/README.md"
README_BACKUP="$GIT_DIR/README.md.bump-backup"

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
  if [[ -f "$CARGO_LOCK" ]]; then
    cp "$CARGO_LOCK" "$CARGO_LOCK_BACKUP"
  fi
fi
if [[ -f "$README_FILE" ]]; then
  cp "$README_FILE" "$README_BACKUP"
fi

# Step 4a: Atomic rewrite of package.json via jq + mv.
TMP_JSON="$VERSION_FILE.tmp.$$"
if ! jq --arg v "$NEW_VERSION" '.version = $v' "$VERSION_FILE" > "$TMP_JSON"; then
  rm -f "$TMP_JSON" "$BACKUP" "$CARGO_BACKUP" "$CARGO_LOCK_BACKUP"
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
    rm -f "$CARGO_BACKUP" "$CARGO_LOCK_BACKUP"
    fail "Cargo.toml awk rewrite failed: [package] version field not found or empty output"
  fi
  # Verify the new version appears in the rewritten file.
  if ! grep -q "version = \"$NEW_VERSION\"" "$TMP_CARGO"; then
    rm -f "$TMP_CARGO"
    mv "$BACKUP" "$VERSION_FILE"
    rm -f "$CARGO_BACKUP" "$CARGO_LOCK_BACKUP"
    fail "Cargo.toml rewrite verification failed: version $NEW_VERSION not found in output"
  fi
  mv "$TMP_CARGO" "$CARGO_TOML"

  # Step 4c: Regenerate Cargo.lock to reflect the new version.
  if ! command -v cargo >/dev/null 2>&1; then
    mv "$BACKUP" "$VERSION_FILE"
    mv "$CARGO_BACKUP" "$CARGO_TOML"
    if [[ -f "$CARGO_LOCK_BACKUP" ]]; then
      mv "$CARGO_LOCK_BACKUP" "$CARGO_LOCK"
    fi
    fail "cargo not found in PATH; cannot regenerate Cargo.lock"
  fi
  if ! cargo generate-lockfile --quiet; then
    mv "$BACKUP" "$VERSION_FILE"
    mv "$CARGO_BACKUP" "$CARGO_TOML"
    if [[ -f "$CARGO_LOCK_BACKUP" ]]; then
      mv "$CARGO_LOCK_BACKUP" "$CARGO_LOCK"
    fi
    fail "cargo generate-lockfile failed after Cargo.toml rewrite"
  fi
fi

# Step 4d: Update explicit version numbers in README.md.
README_MODIFIED=false
if [[ -f "$README_FILE" ]]; then
  OLD_VERSION=$(jq -r '.version' "$BACKUP")
  if [[ -n "$OLD_VERSION" && "$OLD_VERSION" != "$NEW_VERSION" ]]; then
    OLD_VERSION_ESCAPED="${OLD_VERSION//./\\.}"
    TMP_README="$README_FILE.tmp.$$"
    sed "s/$OLD_VERSION_ESCAPED/$NEW_VERSION/g" "$README_FILE" > "$TMP_README"
    mv "$TMP_README" "$README_FILE"
    README_MODIFIED=true
  fi
fi

# Step 5: Stage and commit.
git add "$VERSION_FILE"
if [[ "$README_MODIFIED" == true ]]; then
  git add "$README_FILE"
fi
if [[ -f "$CARGO_TOML" ]]; then
  git add "$CARGO_TOML"
  if [[ -f "$CARGO_LOCK" ]]; then
    git add "$CARGO_LOCK"
  fi
fi
COMMIT_MSG="Bump version to $NEW_VERSION"
if git commit -m "$COMMIT_MSG" --quiet; then
  # Success — remove backups, emit result.
  rm -f "$BACKUP" "$CARGO_BACKUP" "$CARGO_LOCK_BACKUP" "$README_BACKUP"
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
if [[ -f "$CARGO_LOCK_BACKUP" ]]; then
  mv "$CARGO_LOCK_BACKUP" "$CARGO_LOCK"
  git reset HEAD "$CARGO_LOCK" >/dev/null 2>&1 || true
fi
if [[ -f "$README_BACKUP" ]]; then
  mv "$README_BACKUP" "$README_FILE"
  git reset HEAD "$README_FILE" >/dev/null 2>&1 || true
fi
git reset HEAD "$VERSION_FILE" >/dev/null 2>&1 || true
echo "APPLIED=false"
echo "ERROR=git commit failed; rolled back $VERSION_FILE, $CARGO_TOML, $CARGO_LOCK, and $README_FILE from backup"
exit 1
