# Claude Lint

- A linter for [Claude Code](https://docs.anthropic.com/en/docs/claude-code)
configuration and plugins.
- Validates `.claude/` and `.claude-plugin/`.
- Implemented in Rust, and fully configurable.

## Features

- **100 lint rules** across 9 categories (Manifest, Hooks, Skills, Agents,
  Hygiene, Email, User Config, Slack, Docs)
- **Two lint modes**:
  - **Basic mode** -- validates `.claude/` contents (settings, hooks, private
    skill frontmatter, script references, executability)
  - **Plugin mode** -- runs the full rule suite when `.claude-plugin/` is
    present
- **Configurable** -- suppress or downgrade rules via `claude-lint.toml`
- **GitHub Action** for CI integration
- **Cross-platform** binaries (Linux x86_64/aarch64, macOS aarch64)

## Quick Start

### GitHub Action

```yaml
- uses: zhupanov/claude-lint@v1
  with:
    path: "."
```

### Install on macOS

```bash
curl -fsSL "$(curl -fsSL https://api.github.com/repos/zhupanov/claude-lint/releases/latest \
  | grep -o 'https://[^"]*aarch64-apple-darwin.tar.gz')" -o /tmp/claude-lint.tar.gz
tar -xzf /tmp/claude-lint.tar.gz -C /tmp
sudo mv /tmp/claude-lint /usr/local/bin/claude-lint
```

### CLI

```bash
claude-lint [PATH]
```

If `PATH` is omitted, the current directory is used. The tool detects the
repo root via `git rev-parse --show-toplevel` and selects Basic or Plugin
mode automatically based on the presence of `.claude-plugin/`.

## GitHub Action Inputs

| Input | Description | Default |
|-------|-------------|---------|
| `version` | Version of claude-lint (e.g., `1.0.0`) | Latest release |
| `path` | Path to the repository to lint | `"."` |
| `github-token` | GitHub token for resolving latest version | `""` (see below) |

> **Note:** Windows runners are not supported.

### About `github-token`

The `github-token` input is **optional** and has a narrow purpose: it is
used solely to call the GitHub API to resolve the latest release version
when no explicit `version` is provided. If you pin `version` (e.g.,
`version: "1.0.0"`), no API call is made and the token is never used.

**When omitted**, the action automatically falls back to the built-in
`github.token` that GitHub provides to every workflow run. You do not need
to configure or pass anything -- it just works.

**What the token can access**: the token is sent in a single read-only API
request to `api.github.com/repos/zhupanov/claude-lint/releases/latest` to
fetch the latest tag name. It is never passed to the `claude-lint` binary.
The linter itself only reads local files on disk -- it makes no network
requests and has no access to your repository's GitHub API.

**When you might set it explicitly**: if you use a fine-grained PAT or a
GitHub App token with restricted permissions, and the default
`github.token` cannot reach the public releases endpoint (uncommon).

```yaml
# Minimal -- token handled automatically:
- uses: zhupanov/claude-lint@v1

# Explicit version -- no token needed at all:
- uses: zhupanov/claude-lint@v1
  with:
    version: "1.0.0"
```

## Add CI to Your Repo

Give this prompt to Claude running in your repository:

> **Add a GitHub Actions CI job called `claude-lint` that runs on pull requests
> to `main`. The job should use `ubuntu-latest`, have a 5-minute timeout,
> check out the repo with `actions/checkout@v4`, and then run
> `zhupanov/claude-lint@v1` with `path: "."`. Add it to the existing CI
> workflow if one exists, otherwise create `.github/workflows/ci.yaml` with
> `permissions: contents: read`.**

The resulting job should look like:

```yaml
  claude-lint:
    runs-on: ubuntu-latest
    timeout-minutes: 5
    steps:
      - uses: actions/checkout@v4
      - uses: zhupanov/claude-lint@v1
        with:
          path: "."
```

## CLI Reference

```text
claude-lint [--list-scripts] [PATH]
```

| Flag | Description |
|------|-------------|
| `--list-scripts` | Print all `.sh` script paths found in skill/script directories and exit |

### Exit Codes

| Code | Meaning |
|------|---------|
| `0` | Success (no errors, or only warnings) |
| `1` | Lint errors found |
| `2` | Invalid arguments or setup error (not a git repo, bad config, etc.) |

### `--list-scripts`

Outputs discovered shell scripts, one per line. Useful for piping to
external tools:

```bash
claude-lint --list-scripts . | xargs -r shellcheck
```

The wrapper script `scripts/shellcheck-scripts.sh` automates this.

## Configuration

Claude Lint reads an optional **`claude-lint.toml`** file from the
repository root.

### File Format

```toml
[lint]
ignore = ["M001", "G005"]                  # suppress entirely (by code)
warn   = ["plugin-json-invalid"]           # downgrade to warning (by name)
exclude = ["docs/*.md", "skills/internal-*/**"]  # skip files matching globs
```

### Options

| Key | Type | Description |
|-----|------|-------------|
| `ignore` | string array | Rules to suppress completely (no output, no exit code effect) |
| `warn` | string array | Rules to downgrade from error to warning (printed, but exit 0) |
| `exclude` | string array | File glob patterns -- matching files are skipped entirely |

### Rule Identifiers

Rules can be referenced by **code** (e.g., `M001`) or **human-readable
name** (e.g., `plugin-json-missing`). If a rule appears in both `ignore`
and `warn`, `ignore` takes precedence.

### File Exclusion

The `exclude` option accepts a list of glob patterns. Files matching any
pattern are completely invisible to the linter -- no rules are checked
and no diagnostics are produced for them.

**Glob semantics** (matching `.gitignore` conventions):

- `*` matches any characters except `/` (single directory level)
- `**` matches across directory boundaries (recursive)
- `docs/*.md` matches `docs/readme.md` but **not** `docs/sub/nested.md`
- `docs/**/*.md` matches both `docs/readme.md` and `docs/sub/nested.md`

**Scope**: File exclusion applies to file-walking validators (skills,
agents, scripts, docs). It does **not** apply to fixed-path structural
checks (e.g., `plugin.json` must exist, `SECURITY.md` must exist). Use
`ignore` to suppress those rules instead.

### Behavior Without Config

If `claude-lint.toml` is absent, all rules are enabled as errors and no
files are excluded. A malformed config file, unknown rule code/name, or
invalid glob pattern causes exit code 2.

### Diagnostic Output

```text
error[M001/plugin-json-missing]: .claude-plugin/plugin.json is missing
warning[G005/security-md-missing]: SECURITY.md is missing from repo root
```

## Lint Rules

Claude Lint ships **100 rules** organized into 9 categories:

| Category | Prefix | Rules | Description |
|----------|--------|-------|-------------|
| Manifest | M | 11 | `plugin.json` and `marketplace.json` validation |
| Hooks | H | 7 | `hooks.json` and `settings.json` hook paths |
| Skills | S | 53 | Skill frontmatter, naming, descriptions, body content, security |
| Agents | A | 11 | Agent frontmatter, templates, description quality, name format |
| Hygiene | G | 7 | `$PWD` hygiene, script integrity, executability, dead scripts, TODO detection |
| Email | E | 1 | Email format validation |
| User Config | U | 6 | `userConfig` structure and env var mapping |
| Slack | K | 1 | Slack fallback consistency |
| Docs | D | 3 | Docs file references, CLAUDE.md size, TODO detection |

For the complete rule table with codes, names, descriptions, and modes,
see **[docs/rules.md](docs/rules.md)**.

### Lint Modes

| Mode | Trigger | Scope |
|------|---------|-------|
| **Basic** | `.claude/` directory exists | Settings hooks, private skill frontmatter, script refs, executability, both-mode S-rules |
| **Plugin** | `.claude-plugin/` directory exists | All 100 rules including manifest, agents, hygiene, and plugin-only S-rules |

If neither directory exists, the tool prints "Nothing to lint" and exits 0.

## Local Development

### Prerequisites

- [Rust](https://rustup.rs/) (toolchain pinned in `rust-toolchain.toml`,
  auto-installed by `rustup`)
- [pre-commit](https://pre-commit.com/) for local linters
- `jq` (used by the JSON lint hook)

### Setup

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
pip install pre-commit
make setup   # runs: pre-commit install
```

### Makefile Targets

| Target | Command | Description |
|--------|---------|-------------|
| `make lint` | `pre-commit run --all-files` | Run all linters |
| `make cargo-test` | `cargo test` | Run Rust unit tests |
| `make cargo-clippy` | `cargo clippy -- -D warnings` | Run Clippy with warnings as errors |
| `make clippy` | `cargo clippy --all-targets -- -D warnings` | Run Clippy on all targets |
| `make fmt` | `cargo fmt -- --check` | Check Rust formatting |
| `make shellcheck` | `pre-commit run shellcheck --all-files` | Run ShellCheck on shell scripts |
| `make shellcheck-skills` | `scripts/shellcheck-scripts.sh` | Run ShellCheck on skill-discovered scripts |
| `make markdownlint` | `pre-commit run markdownlint --all-files` | Run markdownlint |
| `make jsonlint` | `pre-commit run jsonlint --all-files` | Validate JSON files |
| `make actionlint` | `pre-commit run actionlint --all-files` | Lint GitHub Actions workflows |
| `make setup` | `pre-commit install` | Install pre-commit git hooks |

## Project Structure

```text
src/
+-- main.rs              # CLI entry point: arg parsing, repo root, mode detection
+-- config.rs            # claude-lint.toml loading and rule resolution
+-- context.rs           # LintContext, ManifestState, LintMode
+-- diagnostic.rs        # DiagnosticCollector, Severity, config-aware filtering
+-- frontmatter.rs       # YAML frontmatter extraction
+-- rules.rs             # Central LintRule enum (100 rules, codes, names)
+-- test_helpers.rs      # Shared test utilities
+-- validators/
    +-- mod.rs           # run_all -> run_basic / run_plugin dispatch
    +-- manifest.rs      # M001-M011: plugin.json & marketplace.json
    +-- hooks.rs         # H001-H007: hooks.json & settings.json
    +-- skills.rs        # S001-S008: skills layout & frontmatter
    +-- skill_content/   # S009-S053: name, description, body, MCP, security checks
    +-- agents.rs        # A001-A011: agent frontmatter, templates, description quality
    +-- hygiene.rs       # G001-G007: PWD hygiene, scripts, executability, TODO detection
    +-- docs.rs          # D001-D003: docs file references, CLAUDE.md size, TODO detection
    +-- email.rs         # E001: email format
    +-- user_config.rs   # U001-U006: userConfig validation
    +-- slack.rs         # K001: Slack fallback consistency
docs/
+-- rules.md             # Complete lint rules reference table
```

## CI/CD

### CI (`.github/workflows/ci.yaml`)

Runs on pull requests to `main` and `workflow_dispatch`:

- **lint** -- pre-commit linters (shell, markdown, JSON, YAML, actionlint,
  Rust fmt); clippy is skipped here and runs in build-and-test instead
- **build-and-test** -- `cargo build`, `cargo test`, `cargo clippy`
- **musl-build** -- cross-compilation check for `x86_64-unknown-linux-musl`
- **self-lint** -- runs claude-lint against its own repo and validates
  `--list-scripts` output
- **e2e-test** -- uses `zhupanov/claude-lint@v1` as a GitHub Action
  (the same way clients integrate it), serving as both end-to-end
  validation and a reference model for users adding CI to their own repos

### Release (`.github/workflows/release.yml`)

Triggered on push to `main`, tag push (`v*`), or `workflow_dispatch`:

1. **auto-tag** -- reads version from `package.json` / `Cargo.toml`, creates
   a git tag if it doesn't exist
2. **build** -- cross-compiles for Linux (x86_64, aarch64 musl) and macOS
   (aarch64)
3. **release** -- creates a GitHub Release with tarballs and checksums;
   on a new release, also moves the floating `v1` tag forward so `@v1`
   action references always resolve to the newest version
