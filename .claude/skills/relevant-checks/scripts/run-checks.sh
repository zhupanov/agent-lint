#!/usr/bin/env bash
# Run validation checks relevant to modified files on the current branch.
# Delegates to pre-commit for file-type routing and linting.
# This script is private to the /relevant-checks skill.
set -uo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)" || { echo "ERROR: not inside a git repository"; exit 1; }
cd "$REPO_ROOT" || exit 1

PC_EXIT=0

# ---------------------------------------------------------------------------
# Self-lint: always validate the repo's own Claude configuration
# ---------------------------------------------------------------------------
if command -v cargo >/dev/null 2>&1; then
    echo "=== Running claude-lint (self-lint) ==="
    cargo run --locked --quiet -- . || PC_EXIT=1
else
    echo "WARN: cargo not available, skipping claude-lint self-lint"
fi

# ---------------------------------------------------------------------------
# Determine changed files (union of branch diff + staged + unstaged + untracked)
# ---------------------------------------------------------------------------
if git rev-parse --verify main >/dev/null 2>&1; then
    branch_diff="$(git diff --name-only main...HEAD 2>/dev/null || true)"
elif git rev-parse --verify origin/main >/dev/null 2>&1; then
    branch_diff="$(git diff --name-only origin/main...HEAD 2>/dev/null || true)"
else
    branch_diff=""
fi

# Staged changes (files added to index but not yet committed)
staged_diff="$(git diff --cached --name-only 2>/dev/null || true)"

# Unstaged changes (modified but not yet staged)
unstaged_diff="$(git diff --name-only 2>/dev/null || true)"

# Untracked files (newly created, not yet staged — e.g., files written by Claude)
untracked="$(git ls-files --others --exclude-standard 2>/dev/null || true)"

# Union and deduplicate
MODIFIED_FILES="$(printf '%s\n%s\n%s\n%s' "$branch_diff" "$staged_diff" "$unstaged_diff" "$untracked" | sort -u | grep -v '^$' || true)"

if [ -z "$MODIFIED_FILES" ]; then
    echo "No modified files detected — no further checks to run."
    exit "$PC_EXIT"
fi

# ---------------------------------------------------------------------------
# Build file array, filtering to files that exist on disk (deleted files from
# branch diff would cause pre-commit to fail with file-not-found errors).
# Uses a portable while-read loop instead of mapfile for macOS Bash 3.2 compat.
# ---------------------------------------------------------------------------
files=()
while IFS= read -r f; do
    if [ -f "$f" ]; then
        files+=("$f")
    fi
done <<< "$MODIFIED_FILES"

# ---------------------------------------------------------------------------
# If all changes are deletions (files[] empty but MODIFIED_FILES non-empty),
# there are no files to lint. Exit 0 with a message.
# ---------------------------------------------------------------------------
if [ ${#files[@]} -eq 0 ]; then
    echo "No existing modified files to check (all changes are deletions)."
    exit "$PC_EXIT"
fi

# ---------------------------------------------------------------------------
# Pre-flight: ensure pre-commit is installed (gates Phase 2 only)
# ---------------------------------------------------------------------------
command -v pre-commit >/dev/null 2>&1 || {
    echo "ERROR: pre-commit not found. Run: pip install pre-commit"
    exit 1
}

# ---------------------------------------------------------------------------
# Run pre-commit on changed files. Pre-commit handles file-type routing via
# the types/files fields in .pre-commit-config.yaml — no manual gating needed.
# ---------------------------------------------------------------------------
echo "=== Running pre-commit on ${#files[@]} changed file(s) ==="
pre-commit run --files "${files[@]}" || PC_EXIT=1

# ---------------------------------------------------------------------------
# Run Rust checks when .rs or Cargo.toml files are among the changes.
# ---------------------------------------------------------------------------
RUST_CHANGED=false
for f in "${files[@]}"; do
    case "$f" in
        *.rs|Cargo.toml|Cargo.lock) RUST_CHANGED=true; break ;;
    esac
done

if [ "$RUST_CHANGED" = true ] && command -v cargo >/dev/null 2>&1; then
    echo "=== Running cargo test ==="
    cargo test || PC_EXIT=1
    echo "=== Running cargo clippy ==="
    cargo clippy -- -D warnings || PC_EXIT=1
fi

exit "$PC_EXIT"
