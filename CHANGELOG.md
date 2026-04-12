# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

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
