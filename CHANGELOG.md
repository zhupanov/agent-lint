# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.2.9] - 2026-04-12

### Fixed

- Fixed release pipeline: removed deprecated `macos-13` runner that was causing all release workflow runs to fail (zero GitHub Releases were being created)
- Fixed `workflow_dispatch` version-handling bug where release job received empty version
- Added version fallback null guards and release idempotency check

### Removed

- Dropped Intel macOS (x86_64-apple-darwin) binary support (Intel Macs are EOL)

## [0.2.8] - 2026-04-12

### Added

- `--list-scripts` CLI flag that outputs all `.sh` script paths discovered in skill and script directories
- `scripts/shellcheck-scripts.sh` wrapper for piping discovered scripts to shellcheck
- `make shellcheck-skills` Makefile target for running shellcheck on skill-discovered scripts
- CI validation step for `--list-scripts` output in self-lint job
- Shared `expand_script_dirs()` helper and directory pattern constants for script discovery
- Unit tests for `expand_script_dirs`, `collect_script_paths`, and mode-scoped discovery

### Changed

- Extracted `detect_mode()` function from inline mode detection in `main.rs`
- Refactored `check_executability_in_dirs` to use shared `expand_script_dirs` helper
- CLI argument parsing now properly partitions flags and positional args with unknown flag rejection

## [0.2.7] - 2026-04-12

### Added

- Ruff-style error codes: 46 lint rules across 9 categories (M/H/S/A/G/E/U/K/D), each with a unique code (e.g., M001) and human-readable name (e.g., plugin-json-missing)
- TOML configuration file (`claude-lint.toml`) with `[lint]` section supporting `ignore` (suppress errors) and `warn` (downgrade to warnings) by code or name
- Config validation: unknown rule codes/names rejected at load time, typos in section/field names detected via `deny_unknown_fields`

### Changed

- Diagnostic output format: `error[CODE/name]: message` replaces `LINT ERROR: message`
- Exit code semantics: exit 0 when only warnings remain, exit 1 for errors, exit 2 for config errors
- `validate_userconfig_env_mapping` now reports missing env var references when `scripts/` directory is absent

## [0.2.6] - 2026-04-12

### Added

- Self-lint CI job that builds and runs `claude-lint` against the repo's own `.claude/` configuration
- Unconditional self-lint phase in `/relevant-checks` that validates Claude config on every invocation

### Changed

- `/relevant-checks` now runs in two phases: unconditional self-lint (Phase 1) followed by change-scoped pre-commit checks (Phase 2)
- Moved pre-commit availability check to gate only Phase 2, allowing self-lint to run independently
- Early exits in `run-checks.sh` now propagate self-lint exit status instead of hardcoded `exit 0`

## [0.2.5] - 2026-04-12

### Fixed

- Regenerated `Cargo.lock` to match pinned Rust 1.94.1 toolchain (lockfile version 3 → 4)

## [0.2.4] - 2026-04-12

### Changed

- Added cargo cache to musl-build CI job for faster dependency resolution
- Included `rust-toolchain.toml` in build-and-test cache key to bust cache on toolchain upgrades
- Removed unnecessary cargo cache from lint CI job (only needs pre-commit cache)

## [0.2.3] - 2026-04-12

### Added

- Comprehensive unit tests for all validator modules (manifest, hooks, skills, agents, hygiene, docs, email, user_config, slack)
- Integration-level dispatch tests for `run_all` Basic/Plugin mode selection
- RAII `CwdGuard` test helper for panic-safe working directory restoration
- `tempfile` and `serial_test` dev-dependencies for filesystem test fixtures
- `DiagnosticCollector::errors()` accessor for test assertions

### Changed

- README.md rewritten with full documentation: features, usage, local development setup, project structure, validator reference, CI/CD overview
- `run_plugin` now includes `validate_private_script_references` and `validate_private_executability` (previously only ran in Basic mode)
- `to_upper_snake_case` rewritten to be O(n) and correctly handle uppercase-after-uppercase transitions

### Fixed

- README usage example updated from stale `args` input to current `path` input, version bumped from `v0.1.4` to `v0.2.2`

## [0.2.2] - 2026-04-12

### Added

- Rust implementation of all 25 structural validators from larch's `validate-plugin-structure.sh`
- Two lint modes: basic (`.claude/` contents) and plugin (full 25-validator suite when `.claude-plugin/` exists)
- CI jobs for Rust build/test/clippy and musl cross-compilation
- `cargo-test` and `cargo-clippy` Makefile targets

### Changed

- `action.yml`: replaced free-form `args` input with typed `path` input
- `/relevant-checks` now runs `cargo test` and `cargo clippy` when Rust files are modified

### Fixed

- V22 docs reference extraction: stop at any `##` heading (bash original had `[^C]` bug)

## [0.2.1] - 2026-04-12

### Added

- Rust linters (cargo fmt, cargo clippy) to CI via pre-commit hooks in `.pre-commit-config.yaml`
- Rust toolchain setup and Cargo cache in CI workflow (`.github/workflows/ci.yaml`)
- Release-on-merge CD pipeline: auto-tag job in `release.yml` triggered on push to main
- Version sync between `package.json` and `Cargo.toml` in `/bump-version` skill
- `make clippy` and `make fmt` Makefile targets for local Rust linting

### Changed

- `apply-bump.sh` now updates both `package.json` and `Cargo.toml` atomically with rollback support
- `release.yml` supports push-to-main trigger (auto-tag + build + release) alongside existing tag-push trigger
- Aligned `Cargo.toml` version to match `package.json` (0.2.0)

## [0.2.0] - 2026-04-12

### Added

- GitHub Action boilerplate for composite shell-based distribution (`action.yml`, `scripts/install.sh`)
- Multi-platform Rust binary release workflow (`.github/workflows/release.yml`)
- Minimal Rust project scaffolding (`Cargo.toml`, `rust-toolchain.toml`, `src/main.rs`)

## [0.1.4] - 2026-04-12

### Added

- CHANGELOG.md with retroactive entries documenting all prior PRs (#1-#4)

## [0.1.3] - 2026-04-12

### Added

- GitHub Actions CI workflow running third-party linters via pre-commit on PRs to main and manual dispatch
- Makefile with `lint` target for local and CI linter execution

## [0.1.2] - 2026-04-12

### Changed

- Removed redundant explicit allow rules from `.claude/settings.json` since `defaultMode: "bypassPermissions"` already grants all permissions

## [0.1.1] - 2026-04-12

### Added

- Pre-commit linting infrastructure with shellcheck, markdownlint, jsonlint, actionlint, and standard hooks
- `/bump-version` skill for semantic version management via `package.json`
- `/relevant-checks` skill wrapping pre-commit for scoped file validation

### Changed

- Narrowed README.md scope to match actual implementation

## [0.1.0] - 2026-04-12

### Added

- Initial project setup with README
- `.claude/settings.json` with full permissions configuration
