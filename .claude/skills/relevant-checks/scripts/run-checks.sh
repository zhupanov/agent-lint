#!/usr/bin/env bash
# Run validation checks relevant to modified files on the current branch.
# Delegates to pre-commit for file-type routing and linting.
# This script is private to the /relevant-checks skill.
set -uo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)" || { echo "ERROR: not inside a git repository"; exit 1; }
cd "$REPO_ROOT" || exit 1

PC_EXIT=0

# ---------------------------------------------------------------------------
# CI owns the repository-wide self-lint. Keep this ordinary local helper
# change-scoped so it does not add a second broad compilation or scan.
# ---------------------------------------------------------------------------
# Determine changed files (union of branch diff + staged + unstaged + untracked).
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
# Determine whether the repository-owned Rust check is needed. Keep this
# pattern aligned with the Rust hooks in .pre-commit-config.yaml.
# ---------------------------------------------------------------------------
RUST_CHANGED=false
for f in "${files[@]}"; do
    case "$f" in
        *.rs|Cargo.toml|*/Cargo.toml|Cargo.lock|*/Cargo.lock|rust-toolchain|rust-toolchain.toml|*/rust-toolchain|*/rust-toolchain.toml|.cargo/config|.cargo/config.toml|*/.cargo/config|*/.cargo/config.toml)
            RUST_CHANGED=true
            break
            ;;
    esac
done

# ---------------------------------------------------------------------------
# Run pre-commit on changed files. Its Rust Clippy hook is skipped here so the
# same driver runs exactly once through the bounded make entry point below.
# ---------------------------------------------------------------------------
echo "=== Running pre-commit on ${#files[@]} changed file(s) ==="
if [ "$RUST_CHANGED" = true ]; then
    PRE_COMMIT_SKIP="${SKIP:-}"
    case ",$PRE_COMMIT_SKIP," in
        *,cargo-clippy,*) ;;
        *) PRE_COMMIT_SKIP="${PRE_COMMIT_SKIP:+$PRE_COMMIT_SKIP,}cargo-clippy" ;;
    esac
    SKIP="$PRE_COMMIT_SKIP" pre-commit run --files "${files[@]}" || PC_EXIT=1
else
    pre-commit run --files "${files[@]}" || PC_EXIT=1
fi

if [ "$RUST_CHANGED" = true ]; then
    echo "=== Running make rust-check ==="
    make rust-check || PC_EXIT=1
fi

exit "$PC_EXIT"
