# Agent Lint

- A linter for [Claude Code](https://docs.anthropic.com/en/docs/claude-code)
configuration and plugins.
- Validates `.claude/` and `.claude-plugin/`.
- Implemented in Rust, and fully configurable.

## Features

- **104 lint rules** across 9 categories (Manifest, Hooks, Skills, Agents,
  Hygiene, Email, User Config, Slack, Docs)
- **Two lint modes**:
  - **Basic mode** -- validates `.claude/` contents (settings, hooks, private
    skill frontmatter, script references, executability)
  - **Plugin mode** -- runs the full rule suite when `.claude-plugin/` is
    present
- **Configurable** -- suppress or downgrade rules via `agent-lint.toml`
- **GitHub Action** for CI integration
- **Cross-platform** binaries (Linux x86_64/aarch64, macOS aarch64)

## Quick Start

### GitHub Action

```yaml
- uses: zhupanov/agent-lint@v1
  with:
    path: "."
```

### Pre-commit

Add to your `.pre-commit-config.yaml`:

```yaml
repos:
  - repo: https://github.com/zhupanov/agent-lint
    rev: v2.2.2  # or any release tag
    hooks:
      - id: agent-lint
```

The hook automatically downloads the pre-built binary for your platform
and caches it. Pass CLI flags via `args`:

```yaml
      - id: agent-lint
        args: [--pedantic]
```

### Install on macOS

```bash
curl -fsSL "$(curl -fsSL https://api.github.com/repos/zhupanov/agent-lint/releases/latest \
  | grep -o 'https://[^"]*aarch64-apple-darwin.tar.gz')" -o /tmp/agent-lint.tar.gz
tar -xzf /tmp/agent-lint.tar.gz -C /tmp
sudo mv /tmp/agent-lint /usr/local/bin/agent-lint
```

### CLI

```bash
agent-lint [OPTIONS] [PATH]
```

If `PATH` is omitted, the current directory is used. The tool detects the
repo root via `git rev-parse --show-toplevel` and selects Basic or Plugin
mode automatically based on the presence of `.claude-plugin/`.

#### Flags

| Flag | Description |
|------|-------------|
| `--help`, `-h` | Print help message |
| `--version` | Print version information |
| `--list-scripts` | List discovered script paths and exit |
| `--autofix` | Fix auto-fixable violations in-place and report remaining issues |

#### `--autofix`

When `--autofix` is provided, agent-lint attempts to automatically fix
violations for rules that have purely mechanical, unambiguous fixes. After
all possible fixes are applied, it runs a final validation pass and reports
any remaining issues with normal exit semantics (exit 1 if errors remain).

**Auto-fixable rules (12 of 104):**

| Rule | Code | Fix |
|------|------|-----|
| hook-not-executable | H005 | `chmod +x` on script |
| script-not-executable | G003 | `chmod +x` on script |
| frontmatter-name-mismatch | S006 | Set `name:` to match directory |
| frontmatter-field-empty | S007 | Remove empty optional field |
| name-has-xml | S013 | Strip XML tags from name |
| desc-has-xml | S018 | Strip XML tags from description |
| consecutive-bash | S021 | Merge adjacent bash blocks |
| backslash-path | S022 | Replace `\` with `/` in body |
| non-https-url | S031 | `http://` → `https://` |
| frontmatter-backslash | S043 | Replace `\` with `/` in frontmatter |
| tools-list-syntax | S045 | YAML list → comma-separated scalar |
| pwd-in-skill | G001 | `$PWD/` → `${CLAUDE_PLUGIN_ROOT}/` |

Each fix is logged to stderr. Default-suppressed rules (S021, S045, etc.)
must be enabled via `agent-lint.toml` to be detected and fixed.

## GitHub Action Inputs

| Input | Description | Default |
|-------|-------------|---------|
| `version` | Version of agent-lint (e.g., `1.0.0`) | Latest release |
| `path` | Path to the repository to lint | `"."` |
| `github-token` | GitHub token for resolving latest version | `""` (see below) |
| `pedantic` | Enable pedantic mode (promote warn-listed rules to errors, except too-long) | `"false"` |
| `all` | Enable all mode (every rule fires as an error) | `"false"` |

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
request to `api.github.com/repos/zhupanov/agent-lint/releases/latest` to
fetch the latest tag name. It is never passed to the `agent-lint` binary.
The linter itself only reads local files on disk -- it makes no network
requests and has no access to your repository's GitHub API.

**When you might set it explicitly**: if you use a fine-grained PAT or a
GitHub App token with restricted permissions, and the default
`github.token` cannot reach the public releases endpoint (uncommon).

```yaml
# Minimal -- token handled automatically:
- uses: zhupanov/agent-lint@v1

# Explicit version -- no token needed at all:
- uses: zhupanov/agent-lint@v1
  with:
    version: "1.0.0"
```

## Add CI to Your Repo

Give this prompt to Claude running in your repository:

> **Add a GitHub Actions CI job called `agent-lint` that runs on pull requests
> to `main`. The job should use `ubuntu-latest`, have a 5-minute timeout,
> check out the repo with `actions/checkout@v4`, and then run
> `zhupanov/agent-lint@v1` with `path: "."`. Add it to the existing CI
> workflow if one exists, otherwise create `.github/workflows/ci.yaml` with
> `permissions: contents: read`.**

The resulting job should look like:

```yaml
  agent-lint:
    runs-on: ubuntu-latest
    timeout-minutes: 5
    steps:
      - uses: actions/checkout@v4
      - uses: zhupanov/agent-lint@v1
        with:
          path: "."
```

## CLI Reference

```text
agent-lint [--pedantic | --all] [--list-scripts] [PATH]
```

| Flag | Description |
|------|-------------|
| `--list-scripts` | Print all `.sh` script paths found in skill/script directories and exit |
| `--pedantic` | Promote all enabled rules to errors (except too-long rules) |
| `--all` | Force every rule to error, ignoring config overrides |

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
agent-lint --list-scripts . | xargs -r shellcheck
```

The wrapper script `scripts/shellcheck-scripts.sh` automates this.

## Configuration

Agent Lint reads an optional **`agent-lint.toml`** file from the
repository root.

### File Format

```toml
[lint]
ignore = ["M001"]                          # suppress entirely (by code)
error  = ["S033", "G005"]                  # promote to error (by code or name)
warn   = ["plugin-json-invalid"]           # downgrade to warning (by name)
exclude = ["docs/*.md", "skills/internal-*/**"]  # skip files matching globs
```

### Options

| Key | Type | Description |
|-----|------|-------------|
| `ignore` | string array | Rules to suppress completely (no output, no exit code effect) |
| `error` | string array | Rules to promote to error (overrides default severity) |
| `warn` | string array | Rules to downgrade to warning (printed, but exit 0) |
| `exclude` | string array | File glob patterns -- matching files are skipped entirely |

### Rule Identifiers

Rules can be referenced by **code** (e.g., `M001`) or **human-readable
name** (e.g., `plugin-json-missing`). Priority when a rule appears in
multiple lists: `ignore` > `error` > `warn`.

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

### Default Severity

Each rule has a compiled-in default severity: **error** (68 rules) or
**off** (36 rules). Style, quality, and niche rules are off by default.
Use `error = [...]` in `agent-lint.toml` to enable them. See
[docs/rules.md](docs/rules.md) for the default severity of each rule.

### Strictness Modes

Two CLI flags override the default severity model. They are mutually
exclusive (using both exits with code 2).

**`--pedantic`**: Promotes all rules that are in the `warn` list to errors,
except rules whose name ends in `-too-long` (`name-too-long`,
`desc-too-long`, `body-too-long`, `compat-too-long`). Rules in `ignore`
stay ignored. Default-suppressed rules stay suppressed unless explicitly
enabled via `error` in `agent-lint.toml`. Too-long rules keep their
current severity.

**`--all`**: Forces every rule to fire as an error. The `ignore` and `warn`
lists are bypassed entirely — all 104 rules are promoted to errors. File
exclusions (`exclude`) remain in effect. Note: `--all` applies to rules
emittable by the detected lint mode. In Basic mode (`.claude/` only),
plugin-only rules are not dispatched regardless of `--all`.

### Behavior Without Config

If `agent-lint.toml` is absent, 68 rules are enabled as errors and 36
style/quality/niche rules are off. A malformed config file, unknown rule
code/name, or invalid glob pattern causes exit code 2.

### Diagnostic Output

```text
error[M001/plugin-json-missing]: .claude-plugin/plugin.json is missing
warning[M002/plugin-json-invalid]: plugin.json is not valid JSON
```

## Lint Rules

Agent Lint ships **104 rules** organized into 9 categories:

| Category | Prefix | Rules | Description |
|----------|--------|-------|-------------|
| Manifest | M | 11 | `plugin.json` and `marketplace.json` validation |
| Hooks | H | 7 | `hooks.json` and `settings.json` hook paths |
| Skills | S | 57 | Skill frontmatter, naming, descriptions, body content, security |
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
| **Plugin** | `.claude-plugin/` directory exists | All 104 rules including manifest, agents, hygiene, and plugin-only S-rules |

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
+-- config.rs            # agent-lint.toml loading and rule resolution
+-- context.rs           # LintContext, ManifestState, LintMode
+-- diagnostic.rs        # DiagnosticCollector, Severity, config-aware filtering
+-- frontmatter.rs       # YAML frontmatter extraction
+-- rules.rs             # Central LintRule enum (104 rules, codes, names)
+-- test_helpers.rs      # Shared test utilities
+-- validators/
    +-- mod.rs           # run_all -> run_basic / run_plugin dispatch
    +-- manifest.rs      # M001-M011: plugin.json & marketplace.json
    +-- hooks.rs         # H001-H007: hooks.json & settings.json
    +-- skills.rs        # S001-S008: skills layout & frontmatter
    +-- skill_content/   # S009-S057: name, description, body, MCP, security checks
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
- **self-lint** -- runs agent-lint against its own repo and validates
  `--list-scripts` output
- **e2e-test** -- uses `zhupanov/agent-lint@v1` as a GitHub Action
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
