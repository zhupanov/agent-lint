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
+-- diagnostic.rs        # DiagnosticCollector, structured subjects, config-aware filtering
+-- frontmatter.rs       # YAML frontmatter extraction
+-- rules.rs             # Central LintRule enum (286 rules, codes, names)
+-- test_helpers.rs      # Shared test utilities
+-- validators/
    +-- mod.rs           # run_all -> run_basic / run_plugin dispatch
    +-- manifest.rs      # M001-M017: plugin.json & marketplace.json, component paths
    +-- hooks.rs         # H001-H007, H025: hooks.json, settings.json, settings.local.json
    +-- hook_schema.rs   # H008-H024: shared hook object schema engine
    +-- skills.rs        # S001-S008: skills layout & frontmatter
    +-- skill_content/   # S009-S057, S063-S071: name, description, body, MCP, execution fields, security
    +-- contracts.rs     # A012-A013, S058-S062, G008-G011, D004-D005, L001-L006
    +-- agents.rs        # A001-A011, A014-A028: agent frontmatter, field values, templates, description quality
    +-- prompt_content.rs # Q001-Q004: shared CLAUDE.md, skill-body, and agent-body prompt checks
    +-- hygiene.rs       # G001-G007: PWD hygiene, scripts, executability, TODO detection
    +-- docs.rs          # D001-D003: docs file references, CLAUDE.md size, TODO detection
    +-- email.rs         # E001: email format
    +-- user_config.rs   # U001-U007: userConfig validation
    +-- mcp.rs           # P001, P009-P012, P017-P019, P022-P026: MCP configuration
    +-- claude_config.rs # R001-R002, O001-O006, T001-T002: .claude/ rules, output styles, settings
    +-- codex_config.rs  # CX001-CX036: .codex/config.toml validation
    +-- codex_constants.rs # Codex config key/enum tables
    +-- slack.rs         # K001: Slack fallback consistency
docs/
+-- rules.md             # Complete lint rules reference table
+-- cli.md               # CLI flags, exit codes, --autofix, --list-scripts
+-- configuration.md     # agent-lint.toml format, strictness modes
+-- github-action.md     # Action inputs, token configuration, CI setup
+-- development.md       # Local setup, Makefile targets, project structure, CI/CD
```

## Diagnostic subjects and policy

File-attributable validators report through `DiagnosticCollector::report_at`
or an explicit `with_subject_path` scope. The collector normalizes and stores
that path before resolving global and per-file policy; validators must never
recover a filename by parsing their human-readable message. Fixed-path checks
use their logical repository-relative target even when the target is missing.
Repository-wide checks use `report` without a subject. The named multi-source
prompt-budget checks remain pathless because one finding can describe several
configured roots and their shared closure.

Disposition and usage accounting stay in `diagnostic.rs`; TOML parsing and
compiled glob ownership stay in `config.rs`; candidate mutation filtering
stays in `autofix.rs`. A new file-attributable validator or fixer must test
that an exact-path override suppresses only its rule and that autofix leaves a
suppressed candidate byte-for-byte unchanged.

## JSON Schema validation pilots

Use JSON Schema only for a configuration surface's structural contract: object
shape, required properties, value types, and nested arrays or objects. Keep
cross-field semantics, security policy, filesystem checks, and product-specific
rules in explicit Rust validators.

An embedded schema must be compiled once with `LazyLock`, use `jsonschema` with
default features disabled, and contain no external `$ref`. This prevents
linting from reading local schema files or making network requests. The adapter
must map each schema error back to an existing `LintRule` and a stable,
user-facing instance path.

Migrate another surface only after a pilot preserves its accepted and rejected
fixtures and demonstrates a maintenance or production-line reduction after
counting the schema and diagnostic adapter. Do not add schema validation merely
to move hand-written checks into a different format.

## CI/CD

### CI (`.github/workflows/ci.yaml`)

Runs on pull requests to `main` and `workflow_dispatch`:

- **lint** -- pre-commit linters (shell, markdown, JSON, YAML, actionlint,
  Rust fmt); clippy is skipped here and runs in build-and-test instead
- **build-and-test** -- `cargo build`, `cargo test`, `cargo clippy`
- **musl-build** -- cross-compilation check for `x86_64-unknown-linux-musl`
- **self-lint** -- runs agent-lint against its own repo and validates
  `--list-scripts` output
- **e2e-test** -- runs the released `zhupanov/agent-lint@v2` GitHub Action
  against this repository in default, pedantic, and all modes, serving as
  both end-to-end validation and a reference model for users adding CI

### Release (`.github/workflows/release.yml`)

Triggered only by `workflow_dispatch`, normally by the repository-local
`/release-agent-lint` skill after its version PR merges:

1. **build** -- cross-compiles for Linux (x86_64, aarch64 musl) and macOS
   (aarch64)
2. **release** -- creates a GitHub Release with tarballs and checksums;
   on a new release, also moves the floating `v2` tag forward so `@v2`
   action references always resolve to the newest version
