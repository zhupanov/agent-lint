# Development

## Prerequisites

- [Rust](https://rustup.rs/) (toolchain pinned in `rust-toolchain.toml`,
  auto-installed by `rustup`)
- [pre-commit](https://pre-commit.com/) for local linters
- `jq` (used by the JSON lint hook)

## Setup

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
pip install pre-commit
make setup   # runs: pre-commit install
```

## Makefile Targets

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
- **e2e-test** -- uses `zhupanov/agent-lint@v2` as a GitHub Action
  (the same way clients integrate it), serving as both end-to-end
  validation and a reference model for users adding CI to their own repos

### Release (`.github/workflows/release.yml`)

Triggered on push to `main`, tag push (`v*`), or `workflow_dispatch`:

1. **auto-tag** -- reads version from `package.json` / `Cargo.toml`, creates
   a git tag if it doesn't exist
2. **build** -- cross-compiles for Linux (x86_64, aarch64 musl) and macOS
   (aarch64)
3. **release** -- creates a GitHub Release with tarballs and checksums;
   on a new release, also moves the floating `v2` tag forward so `@v2`
   action references always resolve to the newest version
