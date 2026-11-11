#!/usr/bin/env bash
# apply-bump.sh — Apply a computed semver bump to package.json, Cargo.toml, Cargo.lock,
# and every release-example document.
#
# Contract:
#   - FIRST: verify working tree is clean (fails on any staged or unstaged changes).
#   - Validate package.json with jq.
#   - Back up package.json, Cargo.toml, Cargo.lock, and each release-example document
#     (to the git directory to avoid triggering the dirty-tree guard on retry).
#   - Rewrite .version field in package.json atomically via jq + mv.
#   - Rewrite version field in Cargo.toml [package] section atomically via awk + mv.
#   - Rewrite ONLY this package's version line in Cargo.lock atomically via awk + mv,
#     then prove manifest/lockfile agreement with `cargo metadata --locked`. The whole
#     dependency graph is deliberately left untouched: a version bump is not a
#     dependency upgrade.
#   - Replace all occurrences of old version (X.Y.Z) with new version in each
#     release-example document.
#   - git add + commit with message "Bump version to <new-version>".
#   - Roll back every touched file from backup if any step fails. An ERR trap
#     covers steps that run unguarded under `set -e` (for example `git add`
#     losing a race for .git/index.lock), so a caller parsing stdout always
#     receives APPLIED=false / ERROR= rather than a silent non-zero exit.
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

set -Eeuo pipefail

# Release-example documents whose pinned versions must track the package version.
#
# MUST stay in sync with RELEASE_EXAMPLE_DOCUMENTS in src/documentation_consistency.rs.
# The Rust test `release_example_documents_match_bump_script` parses this very array
# and fails the build if the two lists ever drift, so a document added there without
# being added here (or vice versa) cannot reach main.
RELEASE_EXAMPLE_DOCS=(
  "README.md"
  "docs/github-action.md"
  "docs/development.md"
)

# Temp files created during mutation. Registered as they are created so a
# failure between creation and `mv` cannot strand one inside the repository.
TEMP_FILES=()

# cleanup_temps — remove any temp file this run created and did not move.
cleanup_temps() {
  local tmp
  for tmp in ${TEMP_FILES[@]+"${TEMP_FILES[@]}"}; do
    rm -f "$tmp"
  done
}

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

# Release-example documents that exist in this tree, with their backups (parallel arrays).
DOC_FILES=()
DOC_BACKUPS=()
# Release-example documents whose rewrite actually changed the file.
DOC_MODIFIED=()

# restore_manifests — move every existing manifest backup back over its file.
# Restoring a file the run never modified is a harmless no-op, so this is safe on
# every failure path regardless of how far the run got.
restore_manifests() {
  if [[ -f "$BACKUP" ]]; then
    mv "$BACKUP" "$VERSION_FILE"
  fi
  if [[ -f "$CARGO_BACKUP" ]]; then
    mv "$CARGO_BACKUP" "$CARGO_TOML"
  fi
  if [[ -f "$CARGO_LOCK_BACKUP" ]]; then
    mv "$CARGO_LOCK_BACKUP" "$CARGO_LOCK"
  fi
}

# restore_docs — move every existing release-example backup back over its document.
restore_docs() {
  local index
  for ((index = 0; index < ${#DOC_FILES[@]}; index++)); do
    if [[ -f "${DOC_BACKUPS[index]}" ]]; then
      mv "${DOC_BACKUPS[index]}" "${DOC_FILES[index]}"
    fi
  done
}

# rollback_all — restore every backup taken by this run.
rollback_all() {
  # Disarm first: a restore that itself fails must not re-enter this handler.
  trap - ERR
  restore_manifests
  restore_docs
  cleanup_temps
}

# unexpected_failure — ERR trap, armed once backups exist. Any mutating step that
# fails outside an explicit guard lands here, so the tree is restored and the
# caller still gets a parseable verdict instead of a bare non-zero exit.
# Invoked indirectly by `trap unexpected_failure ERR`, which shellcheck cannot
# see: SC2329 (newer builds) and SC2317 (0.10.x) both call it unreachable.
# shellcheck disable=SC2329,SC2317
unexpected_failure() {
  local status=$?
  local failed_command=$BASH_COMMAND
  rollback_all
  echo "APPLIED=false"
  echo "ERROR=unexpected failure (exit $status) while running: ${failed_command//$'\n'/ }; rolled back every touched file from backup"
  exit 1
}

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
for doc in "${RELEASE_EXAMPLE_DOCS[@]}"; do
  doc_file="$PWD/$doc"
  # Absent documents are skipped entirely: nothing to back up, rewrite, or stage.
  if [[ -f "$doc_file" ]]; then
    doc_backup="$GIT_DIR/${doc//\//__}.bump-backup"
    cp "$doc_file" "$doc_backup"
    DOC_FILES+=("$doc_file")
    DOC_BACKUPS+=("$doc_backup")
  fi
done

# Backups now exist, so recovery is possible: arm the ERR trap for every
# mutating step below that is not already wrapped in an explicit guard.
trap unexpected_failure ERR

# Step 4a: Atomic rewrite of package.json via jq + mv.
TMP_JSON="$VERSION_FILE.tmp.$$"
TEMP_FILES+=("$TMP_JSON")
if ! jq --arg v "$NEW_VERSION" '.version = $v' "$VERSION_FILE" > "$TMP_JSON"; then
  rm -f "$TMP_JSON"
  rollback_all
  fail "jq rewrite failed"
fi
mv "$TMP_JSON" "$VERSION_FILE"

# Step 4b: Atomic rewrite of Cargo.toml [package] version via awk + mv.
if [[ -f "$CARGO_TOML" ]]; then
  TMP_CARGO="$CARGO_TOML.tmp.$$"
  TEMP_FILES+=("$TMP_CARGO")
  if ! awk -v new_ver="$NEW_VERSION" '
    /^\[package\]/ { in_package=1 }
    /^\[/ && !/^\[package\]/ { in_package=0 }
    in_package && /^version *= *"/ {
      sub(/"[^"]*"/, "\"" new_ver "\"")
      substituted=1
    }
    { print }
    END { if (!substituted) exit 1 }
  ' "$CARGO_TOML" > "$TMP_CARGO" || [[ ! -s "$TMP_CARGO" ]]; then
    rm -f "$TMP_CARGO"
    rollback_all
    fail "Cargo.toml awk rewrite failed: [package] version field not found or empty output"
  fi
  # Verify the new version appears in the rewritten file.
  if ! grep -q "version = \"$NEW_VERSION\"" "$TMP_CARGO"; then
    rm -f "$TMP_CARGO"
    rollback_all
    fail "Cargo.toml rewrite verification failed: version $NEW_VERSION not found in output"
  fi
  mv "$TMP_CARGO" "$CARGO_TOML"

  # Step 4c: Sync this package's own entry in Cargo.lock to the new version.
  #
  # `cargo generate-lockfile` must NOT be used here: it re-resolves the entire
  # dependency graph, so a version-only release commit would silently also ship
  # every semver-compatible upstream upgrade published since the lockfile was last
  # written. Rewrite the single local version line instead and let
  # `cargo metadata --locked` prove the lockfile still matches the manifest.
  if [[ -f "$CARGO_LOCK" ]]; then
    if ! command -v cargo >/dev/null 2>&1; then
      rollback_all
      fail "cargo not found in PATH; cannot verify Cargo.lock after the version bump"
    fi

    PACKAGE_NAME=$(awk '
      /^\[package\]/ { in_package=1; next }
      /^\[/ { in_package=0 }
      in_package && /^name *= *"/ {
        line = $0
        sub(/^name *= *"/, "", line)
        sub(/".*$/, "", line)
        print line
        exit
      }
    ' "$CARGO_TOML")
    if [[ -z "$PACKAGE_NAME" ]]; then
      rollback_all
      fail "Cargo.toml [package] name not found; cannot locate the package entry in Cargo.lock"
    fi

    TMP_LOCK="$CARGO_LOCK.tmp.$$"
    TEMP_FILES+=("$TMP_LOCK")
    if ! awk -v pkg="$PACKAGE_NAME" -v new_ver="$NEW_VERSION" '
      /^\[\[package\]\]/ { in_target=0 }
      /^name *= *"/ {
        line = $0
        sub(/^name *= *"/, "", line)
        sub(/".*$/, "", line)
        if (line == pkg) { in_target=1 }
      }
      in_target && /^version *= *"/ {
        sub(/"[^"]*"/, "\"" new_ver "\"")
        in_target=0
        substituted=1
      }
      { print }
      END { if (!substituted) exit 1 }
    ' "$CARGO_LOCK" > "$TMP_LOCK" || [[ ! -s "$TMP_LOCK" ]]; then
      rm -f "$TMP_LOCK"
      rollback_all
      fail "Cargo.lock awk rewrite failed: no version line for package $PACKAGE_NAME"
    fi
    mv "$TMP_LOCK" "$CARGO_LOCK"

    # 2>&1 >/dev/null keeps stderr only. This check fails for causes it cannot
    # tell apart (a cold registry cache with no network, a toolchain older than
    # rust-version, a malformed [patch]/[replace]), so report what cargo said
    # rather than asserting the lockfile and manifest disagree.
    if ! CARGO_METADATA_ERR=$(cargo metadata --locked --format-version 1 2>&1 >/dev/null); then
      rollback_all
      fail "cargo metadata --locked failed after the Cargo.lock version sync; cargo reported: ${CARGO_METADATA_ERR//$'\n'/ }"
    fi
  fi
fi

# Step 4d: Update explicit version numbers in every release-example document.
OLD_VERSION=$(jq -r '.version' "$BACKUP")
if [[ -n "$OLD_VERSION" && "$OLD_VERSION" != "null" && "$OLD_VERSION" != "$NEW_VERSION" ]]; then
  OLD_VERSION_ESCAPED="${OLD_VERSION//./\\.}"
  for ((i = 0; i < ${#DOC_FILES[@]}; i++)); do
    doc_file="${DOC_FILES[i]}"
    tmp_doc="$doc_file.tmp.$$"
    TEMP_FILES+=("$tmp_doc")
    sed "s/$OLD_VERSION_ESCAPED/$NEW_VERSION/g" "$doc_file" > "$tmp_doc"
    if cmp -s "$tmp_doc" "$doc_file"; then
      # The rewrite was a no-op: leave the file (and the index) untouched.
      rm -f "$tmp_doc"
    else
      mv "$tmp_doc" "$doc_file"
      DOC_MODIFIED+=("$doc_file")
    fi
  done
fi

# Step 5: Stage and commit.
git add "$VERSION_FILE"
for ((i = 0; i < ${#DOC_MODIFIED[@]}; i++)); do
  git add "${DOC_MODIFIED[i]}"
done
if [[ -f "$CARGO_TOML" ]]; then
  git add "$CARGO_TOML"
  if [[ -f "$CARGO_LOCK" ]]; then
    git add "$CARGO_LOCK"
  fi
fi
COMMIT_MSG="Bump version to $NEW_VERSION"
if git commit -m "$COMMIT_MSG" --quiet; then
  # Success — remove backups, emit result.
  rm -f "$BACKUP" "$CARGO_BACKUP" "$CARGO_LOCK_BACKUP"
  for ((i = 0; i < ${#DOC_BACKUPS[@]}; i++)); do
    rm -f "${DOC_BACKUPS[i]}"
  done
  COMMIT_SHA=$(git rev-parse HEAD)
  echo "APPLIED=true"
  echo "COMMIT_SHA=$COMMIT_SHA"
  exit 0
fi

# Step 6: Rollback on commit failure.
rollback_all
git reset HEAD "$VERSION_FILE" >/dev/null 2>&1 || true
if [[ -f "$CARGO_TOML" ]]; then
  git reset HEAD "$CARGO_TOML" >/dev/null 2>&1 || true
fi
if [[ -f "$CARGO_LOCK" ]]; then
  git reset HEAD "$CARGO_LOCK" >/dev/null 2>&1 || true
fi
for ((i = 0; i < ${#DOC_FILES[@]}; i++)); do
  git reset HEAD "${DOC_FILES[i]}" >/dev/null 2>&1 || true
done
echo "APPLIED=false"
echo "ERROR=git commit failed; rolled back $VERSION_FILE, $CARGO_TOML, $CARGO_LOCK, and every release-example document from backup"
exit 1
