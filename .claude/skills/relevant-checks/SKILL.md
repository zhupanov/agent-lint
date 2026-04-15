---
name: relevant-checks
description: Run repo-specific validation checks based on modified files. Use when you need to validate code quality after implementation, after code review fixes, or when fixing CI failures.
allowed-tools: Bash
---

# Relevant Checks

Run validation checks on the current branch. This is a repo-specific skill — each repository defines its own `/relevant-checks` with checks appropriate for that repo.

## How it works

The script runs two phases:

### Phase 1: Unconditional self-lint

Regardless of which files changed, the script builds and runs `agent-lint --all` against the repository root to validate the repo's own Claude configuration (`.claude/` directory). The `--all` flag forces every lint rule to error severity, ignoring config overrides — ensuring the strictest possible validation. This is a repo-wide invariant check that always executes. Requires `cargo` — if unavailable, this phase is skipped with a warning.

### Phase 2: Change-scoped linting

Changed files are collected from the branch diff, staged changes, unstaged changes, and untracked files. The union is passed to `pre-commit run --files`, which routes each file to the appropriate linter hooks based on file type. Deleted files are filtered out automatically.

The following linters are configured in `.pre-commit-config.yaml`:

- **Whitespace/formatting**: trailing-whitespace, end-of-file-fixer
- **YAML files (`.yml`, `.yaml`)**: check-yaml
- **Large files**: check-added-large-files
- **Shell scripts (`.sh`)**: shellcheck
- **Markdown files (`.md`)**: markdownlint (using `.markdownlint.json` config)
- **JSON files (`.json`)**: jq validation
- **GitHub Actions workflows (`.yml`, `.yaml`)**: actionlint
- **Rust files (`.rs`, `Cargo.toml`, `Cargo.lock`)**: cargo fmt (format check), cargo clippy (lint)

When Rust source files (`.rs`, `Cargo.toml`, `Cargo.lock`) are among the changes and `cargo` is available, the script also runs `cargo test` and `cargo clippy -- -D warnings` after pre-commit.

If all changed files are deletions (no existing files to lint), the change-scoped phase exits early — but the self-lint phase still runs.

## Usage

Run the private check script:

```bash
$PWD/.claude/skills/relevant-checks/scripts/run-checks.sh
```

The script automatically detects which files were modified on the current branch, filters to existing files, and runs `pre-commit run --files` on them. Pre-commit handles file-type routing internally — only hooks whose file patterns match the changed files will execute.

## Retry semantics

If the script exits non-zero, one or more checks failed. The caller should:

1. Diagnose the failure from the script output
2. Fix the issue
3. Re-invoke `/relevant-checks` to confirm the fix

Pre-commit runs all applicable hooks even if earlier ones fail, so you can see all failures at once.
