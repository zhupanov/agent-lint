---
name: bump-version
description: Classify and apply a semantic version bump based on the current branch diff. Updates package.json and Cargo.toml, and commits exactly one version-only commit.
allowed-tools: Bash, Read
---

# Bump Version

Classify and apply a semantic version bump for this PR. Produces exactly ONE commit: a version-only edit of `package.json` and `Cargo.toml`.

## Classification rules

The classifier inspects the diff between the branch and main. Since this repo currently has no defined public surface directories, all changes default to PATCH. The MAJOR/MINOR rules below are documented for when the repo grows directories designated as public surface — update the `git diff` scope in `classify-bump.sh` at that time.

Severity hierarchy: **MAJOR > MINOR > PATCH** (highest wins).

### MAJOR — backward-incompatible changes
Any of the following in the designated public surface:
- A deleted public-facing file
- A renamed public-facing file (git status `R`)
- A changed `name:` frontmatter field in an existing SKILL.md
- A `--<flag>` token removed from a SKILL.md's `argument-hint:` frontmatter field

### MINOR — backward-compatible additions
Any of the following in the designated public surface (only if not MAJOR):
- A newly added public-facing file
- A `--<flag>` token added to a SKILL.md's `argument-hint:` frontmatter field

### PATCH — everything else
Default for all other changes. Every PR must bump at least PATCH per policy.

## Caveat — escalation-only clause

After `classify-bump.sh` computes its deterministic baseline, the main agent (you) reviews the full diff for **behavioral** changes that a reasonable client would judge as unexpectedly backward-incompatible — even when no signature changed.

**You may ONLY escalate severity (PATCH → MINOR → MAJOR). Never downgrade.**

If you escalate, append a paragraph to the reasoning log file explaining why.

## How it works

1. The caller invokes this skill.
2. The skill runs `classify-bump.sh`, which:
   - Fetches `origin/main` (best-effort, non-fatal on failure)
   - Resolves `BASE` via `main` → `origin/main` fallback
   - Validates `package.json` via `jq`
   - Detects an **already-bumped branch** by checking whether HEAD itself is a commit with subject `^Bump version to [0-9]+\.[0-9]+\.[0-9]+$`. If HEAD is such a commit, emits `BUMP_TYPE=NONE` and exits 0 (no-op).
   - Computes `git diff -M --name-status $BASE HEAD -- skills agents` for file-level classification
   - Writes evidence to `${IMPLEMENT_TMPDIR:-$(mktemp -d)}/bump-version-reasoning.md`
   - Emits `KEY=VALUE` lines on stdout: `CURRENT_VERSION`, `NEW_VERSION`, `BUMP_TYPE`, `REASONING_FILE`
3. You (main agent) parse the output, read the reasoning log, review the diff, and apply the **escalation-only** caveat review. If you escalate, update `NEW_VERSION` accordingly and append reasoning to the log.
4. You invoke `apply-bump.sh --new-version <NEW_VERSION>`, which:
   - First verifies the working tree is clean (fails on any staged or unstaged changes)
   - Backs up `package.json`, `Cargo.toml`, and `Cargo.lock` (to git directory to avoid triggering dirty-tree guard on retry)
   - Rewrites `package.json` `.version` field via `jq` (atomic via tmp + mv)
   - Rewrites `Cargo.toml` `[package]` version via `awk` (atomic via tmp + mv)
   - Regenerates `Cargo.lock` via `cargo generate-lockfile` to reflect the new version
   - `git add` (package.json, Cargo.toml, Cargo.lock) + `git commit -m "Bump version to <NEW_VERSION>"`
   - Rolls back all three files from backup on commit failure
5. If `BUMP_TYPE=NONE`, skip the apply step and report "already bumped".

## Usage

```bash
$PWD/.claude/skills/bump-version/scripts/classify-bump.sh
```

Parse the output for `CURRENT_VERSION`, `NEW_VERSION`, `BUMP_TYPE`, `REASONING_FILE`.

If `BUMP_TYPE=NONE`, report the no-op and exit.

Otherwise, review the reasoning log and the branch diff. Decide whether to escalate. If escalating, compute the new version from `CURRENT_VERSION` + your escalated bump type and append your reasoning to the log file.

Then apply:

```bash
$PWD/.claude/skills/bump-version/scripts/apply-bump.sh --new-version <NEW_VERSION>
```

## Output contract

The reasoning log at `${IMPLEMENT_TMPDIR:-$(mktemp -d)}/bump-version-reasoning.md` may be embedded into the PR body for documentation purposes.

## Exit codes
- `classify-bump.sh` — 0 on success (including `BUMP_TYPE=NONE`), non-zero on parse/validation failure
- `apply-bump.sh` — 0 on successful commit, non-zero on dirty worktree or commit failure (rollback performed)
