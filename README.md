# Claude Lint

Claude Lint is a configuration linter for Claude Code. It validates
`.claude/` and `.claude-plugin/` directory structures, catching
misconfigurations before they reach production.

## Features

- **Two lint modes**:
  - **Basic mode** — validates `.claude/` contents (settings hooks, private
    skill frontmatter, script references, executability)
  - **Plugin mode** — runs the full 25-validator suite when a
    `.claude-plugin/` directory is present
- **25 structural validators** covering manifests, hooks, skills, agents,
  hygiene, docs, email, user config, and Slack conventions
- **GitHub Action** for CI integration
- **Cross-platform** binaries (Linux x86_64/aarch64, macOS x86_64/aarch64)

## Usage

### GitHub Action

Add to your GitHub Actions workflow:

```yaml
- uses: zhupanov/claude-lint@v0.2.2
  with:
    path: "."
```

#### Inputs

| Input | Description | Default |
|-------|-------------|---------|
| `version` | Version of claude-lint (e.g., `0.2.0`) | Latest release |
| `path` | Path to the repository to lint | `"."` |
| `github-token` | GitHub token for API requests | `${{ github.token }}` |

> **Note:** Windows runners are not supported.

### CLI

```bash
claude-lint [PATH]
```

If `PATH` is omitted, lints the current directory. The tool detects the
repository root via `git rev-parse --show-toplevel` and selects Basic or
Plugin mode based on the presence of `.claude-plugin/`.

## Prerequisites

- [Rust](https://rustup.rs/) (toolchain version is pinned in
  `rust-toolchain.toml` and auto-installed by `rustup`)
- [pre-commit](https://pre-commit.com/) for running linters locally
- `jq` (used by the JSON lint hook)

## Local Development

### Install Rust

Install Rust via [rustup](https://rustup.rs/):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

The project pins its toolchain via `rust-toolchain.toml`. Once `rustup` is
installed, it will automatically download and use the correct Rust version
(including `clippy` and `rustfmt` components) when you run any `cargo`
command from this repository.

### Set up linters

```bash
pip install pre-commit
make setup   # runs: pre-commit install
```

### Makefile targets

| Target | Command | Description |
|--------|---------|-------------|
| `make lint` | `pre-commit run --all-files` | Run all linters (shell, markdown, JSON, YAML, Rust) |
| `make cargo-test` | `cargo test` | Run Rust unit tests |
| `make cargo-clippy` | `cargo clippy -- -D warnings` | Run Clippy with warnings as errors |
| `make clippy` | `cargo clippy --all-targets -- -D warnings` | Run Clippy on all targets |
| `make fmt` | `cargo fmt -- --check` | Check Rust formatting |
| `make shellcheck` | `pre-commit run shellcheck --all-files` | Run ShellCheck on shell scripts |
| `make markdownlint` | `pre-commit run markdownlint --all-files` | Run markdownlint |
| `make jsonlint` | `pre-commit run jsonlint --all-files` | Validate JSON files |
| `make actionlint` | `pre-commit run actionlint --all-files` | Lint GitHub Actions workflows |
| `make setup` | `pre-commit install` | Install pre-commit git hooks |

> **Note:** `make lint` also runs `cargo fmt` and `cargo clippy` via
> pre-commit hooks defined in `.pre-commit-config.yaml`.

## Project Structure

```text
src/
├── main.rs              # CLI entry point: arg parsing, repo root, mode detection
├── context.rs           # LintContext, ManifestState, LintMode
├── diagnostic.rs        # DiagnosticCollector (error accumulator)
├── frontmatter.rs       # YAML frontmatter extraction
└── validators/
    ├── mod.rs           # run_all → run_basic / run_plugin dispatch
    ├── manifest.rs      # V1, V2, V12, V13 — plugin.json & marketplace.json
    ├── hooks.rs         # V3, V4 — hooks.json & settings.json hook paths
    ├── skills.rs        # V5, V6, V15 — skills layout & frontmatter
    ├── agents.rs        # V7, V16, V21 — agent frontmatter & templates
    ├── hygiene.rs       # V8–V11, V14 — PWD hygiene, scripts, executability
    ├── docs.rs          # V22 — docs file references
    ├── email.rs         # V17 — email format
    ├── user_config.rs   # V18, V20, V23–V25 — userConfig validation
    └── slack.rs         # V19 — Slack fallback consistency
```

## Validators

### Basic Mode (`.claude/` only)

| ID | Check |
|----|-------|
| V4 | Settings.json hook command paths exist and are executable |
| V6a | Private SKILL.md frontmatter (`.claude/skills/`) |
| V9a | Private script reference integrity (`.claude/skills/`) |
| V10a | Private script executability (`.claude/skills/*/scripts/`) |

### Plugin Mode (all 25 checks)

| ID | Check |
|----|-------|
| V1 | `plugin.json` required fields and semver version |
| V2 | `marketplace.json` required fields and plugin entries |
| V3 | `hooks/hooks.json` structure and hook command paths |
| V4 | `settings.json` hook command paths |
| V5 | Skills directory layout (`skills/*/SKILL.md`) |
| V6 | SKILL.md frontmatter validation |
| V7 | Agent frontmatter (`agents/*.md`) |
| V8 | `$PWD` / hardcoded path hygiene in public skills |
| V9 | Script reference integrity (all reference patterns) |
| V10 | Shell script executability |
| V11 | Dead script detection |
| V12 | Marketplace enriched metadata |
| V13 | Plugin enriched metadata |
| V14 | `SECURITY.md` presence |
| V15 | Shared markdown reference integrity |
| V16 | Agent-template alignment |
| V17 | Email format validation |
| V18 | userConfig structure |
| V19 | Slack fallback consistency |
| V20 | userConfig → env var mapping |
| V21 | Agent-template count |
| V22 | Docs file references from CLAUDE.md |
| V23 | userConfig sensitive type |
| V24 | userConfig title field |
| V25 | userConfig type field |

## CI/CD

### CI (`.github/workflows/ci.yaml`)

Runs on pull requests to `main`:

- **lint** — pre-commit linters (shell, markdown, JSON, YAML, Rust fmt/clippy)
- **build-and-test** — `cargo build`, `cargo test`, `cargo clippy`
- **musl-build** — cross-compilation check for `x86_64-unknown-linux-musl`

### Release (`.github/workflows/release.yml`)

Triggered on push to `main` or tag push:

1. **auto-tag** — reads version from `package.json` / `Cargo.toml`, creates
   a git tag if it doesn't exist
2. **build** — cross-compiles for Linux (x86_64, aarch64 musl) and macOS
   (x86_64, aarch64)
3. **release** — creates a GitHub Release with tarballs and checksums
