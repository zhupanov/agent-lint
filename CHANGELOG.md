# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [2.2.1] - 2026-04-15

### Changed

- Expanded e2e-test CI job from 1 step to 3 steps running agent-lint in default, pedantic, and all modes
- Increased e2e-test job timeout from 5 to 10 minutes to accommodate triple action invocation

## [2.2.0] - 2026-04-15

### Added

- `--autofix` CLI flag: automatically fixes violations for 12 rules with purely mechanical, unambiguous fixes
- New `src/autofix.rs` module with per-rule fix implementations (chmod, frontmatter edits, text replacements, bash block merging)
- `LintRule::is_autofixable()` method classifying which rules support automatic fixing
- `DiagnosticCollector::with_config_silent()` for silent re-validation during autofix loop
- `DiagnosticCollector::diagnostics()` accessor for programmatic access to collected diagnostics
- Detect-fix-revalidate loop with max 50 iterations and progress tracking
- Safety guards: S007 fix checks for `$ARGUMENTS` before removing `argument-hint`, S006 fix validates directory name before applying

## [2.1.2] - 2026-04-15

### Added

- `--pedantic` CLI flag: promotes warn-listed rules to errors (except too-long rules)
- `--all` CLI flag: forces every rule to error, bypassing ignore/warn config
- `pedantic` and `all` boolean inputs for the GitHub Actions CI action
- CI self-lint steps exercising both new flags

## [2.1.1] - 2026-04-14

### Added

- Competitive analysis document surveying 30 AI agent config linting tools and 6 adjacent tools

## [2.1.0] - 2026-04-14

### Added

- Compiled-in default severity for all 104 rules: 68 default to error, 36 style/quality/niche rules default to off
- New `[lint] error` list in `agent-lint.toml` for promoting default-off rules to errors
- `DefaultSeverity` enum and `LintRule::default_severity()` method for compile-time severity classification
- `DiagnosticCollector::new_all_enabled()` test helper for exercising default-suppressed rules
- Default severity column in `docs/rules.md` rules tables

### Changed

- Config priority cascade: user ignore > user error > user warn > compiled default severity
- Default-suppressed rules are silently skipped (no count, no output) unless promoted via config
- Updated S050 and S056 diagnostic messages to remove stale config guidance

## [2.0.0] - 2026-04-14

### Changed

- **BREAKING**: Renamed project from `claude-lint` to `agent-lint`: binary, config file (`agent-lint.toml`),
  GitHub Action (`zhupanov/agent-lint@v1`), Marketplace listing, and all documentation updated
- Legacy `claude-lint.toml` files are detected with a warning to rename

### Fixed

- Added missing S048 `ref-name-generic` and S049 `name-not-gerund` entries to `docs/rules.md`
- Removed duplicate S044 `mcp-tool-unqualified` section from `docs/rules.md`
- Fixed S044/S045 code mislabeling in CHANGELOG, code comments, and test names
- Fixed stale rule count comments in validator dispatch code

## [1.0.38] - 2026-04-14

### Added

- Added S057 `magic-number-undoc` rule: detects undocumented magic numbers in code blocks within SKILL.md files, flagging identifier assignments with numeric literals that lack a justification comment on the same or preceding line, with a well-known values allowlist for common ports, timeouts, and sizes (plugin-only)

## [1.0.37] - 2026-04-14

### Added

- Added S056 `body-no-default` rule: detects when a skill body lists multiple alternatives without stating a default recommendation, scanning prose outside code fences for "or" chains with 3+ items and suppressing when conditional framing or recommendation keywords are present (plugin-only)

## [1.0.35] - 2026-04-14

### Added

- Added S054 `desc-body-misalign` rule: detects skill descriptions whose keywords are not reflected in the body content, flagging when fewer than 50% of description keywords appear in body prose (plugin-only)

## [1.0.34] - 2026-04-14

### Added

- Added S053 `terminology-inconsistent` rule: detects when a skill body uses 3+ variants from the same synonym group (e.g., endpoint/route/url), with 8 curated synonym groups scanning prose outside code fences (plugin-only)

## [1.0.33] - 2026-04-14

### Added

- Added S050 `desc-vague-content` rule: detects vague/generic skill descriptions using two heuristics — generic verb+noun pattern and low information density (plugin-only)

## [1.0.32] - 2026-04-14

### Added

- Added S051 `script-deps-missing` rule: detects script-backed skills lacking dependency/package documentation (plugin-only)
- Added S052 `script-verify-missing` rule: detects script-backed skills lacking verification/validation steps (plugin-only)
- Added `has_scripts_dir` field to `SkillInfo` for clean separation of filesystem detection from content validation

## [1.0.28] - 2026-04-14

### Added

- Added S045 `tools-list-syntax` rule: detects when `allowed-tools` uses YAML block-list syntax instead of comma-separated scalar; suppresses S007 for the same field when list items are present

## [1.0.27] - 2026-04-14

### Changed

- Split `skill_content.rs` (2942 lines) into 8 submodules for improved maintainability
- Split `hygiene.rs` (1436 lines) into 5 submodules for improved maintainability
- Extracted shared directory-walking helpers into `walk.rs`
- Deduplicated `RE_NAME_INVALID` and `RE_TODO_MARKER` regex patterns into `common.rs`
- Updated `agents.rs`, `docs.rs`, and `skills.rs` to use shared utilities

## [1.0.26] - 2026-04-14

### Fixed

- Fixed `validate_email_format` silently passing non-string email fields (number, boolean, array, null now report E001)
- Added H007 `hooks-array-empty` rule for empty `hooks` arrays in `hooks.json`
- Changed `validate_skills_layout` to silently return when `skills/` is missing (S001 deprecated — hooks-only and agent-only plugins no longer get a false positive)
- Fixed V12/V13 enriched validators to not report "missing" for non-string email fields

## [1.0.25] - 2026-04-14

### Fixed

- Fixed S008 shared-ref regex in `validate_shared_md_references` to include `/` in the character class, enabling detection of subdirectory shared references (e.g., `skills/shared/sub/util.md`)

## [1.0.23] - 2026-04-14

### Changed

- Added `use regex::Regex` import to `agents.rs` to match all other validator files
- Added doc comment on `extract_raw_value` noting colon-suffix prevents prefix collisions
- Audited `.markdownlint.json`: removed 13 unnecessary suppressions, kept MD013 and MD024
- Fixed MD022/MD032 violations in `bump-version/SKILL.md` and `relevant-checks/SKILL.md`

## [1.0.22] - 2026-04-13

### Changed

- Refactored `LintContext` to accept explicit `base_path` instead of relying on process CWD for manifest loading
- Made `validate_dead_scripts` use pre-parsed `ManifestState` from `LintContext` instead of reading JSON files directly
- Skipped `plugin_json`/`marketplace_json` loading in Basic mode (never used by `run_basic`)
- Changed `ManifestState::load` to accept `&Path` instead of `&str`
- Consolidated `collect_json_strings` helper into `context.rs`, eliminating duplication in `hooks.rs`

## [1.0.21] - 2026-04-13

### Fixed

- Fixed `expand_script_dirs` to support multiple `*` wildcards in glob patterns (e.g., `skills/*/nested/*/scripts`) instead of silently skipping them
- Moved `#[cfg(unix)]` guard to outer executability functions so the entire directory walk is skipped on non-Unix platforms

## [1.0.20] - 2026-04-13

### Added

- Added `--help` (`-h`) and `--version` CLI flags for discoverability
- Added CWD fallback when git is unavailable or target is not a git repo, with a warning to stderr
- Single-dash flags (e.g., `-v`) are now rejected as unknown flags instead of being silently treated as paths

## [1.0.19] - 2026-04-13

### Fixed

- Fixed docs path regex to match subdirectory paths (e.g., `docs/api/reference.md`) by adding `/` to character class
- Made canonical sources heading match case-insensitive so `## Canonical Sources` variants are detected
- Extracted `shared_ref_regex` helper to build shared-reference regex from `base_dir` parameter with `regex::escape`, replacing hardcoded `skills/shared` in S029 and S036 validators

## [1.0.18] - 2026-04-13

### Changed

- Separated `DiagnosticCollector` output from collection by introducing a writer abstraction (`Box<dyn Write>`), defaulting to stderr in production and `io::sink()` in tests to eliminate stderr noise during test runs

## [1.0.17] - 2026-04-13

### Fixed

- Extracted shared `CodeFenceTracker` replacing fragile `in_fence = !in_fence` toggle in G006, G007, D003, S021, S022, S028, S038
- Code fence tracking now properly handles nested fences (4+ backtick/tilde counts), mixed fence types, and closing-fence validation per CommonMark spec
- S028 (`$ARGUMENTS` without `argument-hint`) now only checks outside code fences, fixing false positives from code examples

## [1.0.16] - 2026-04-13

### Fixed

- Fixed `strip_yaml_comments` regex stripping `#` inside quoted strings (e.g., `key: "value with # hash"` was truncated)
- Replaced naive trailing comment regex with quote-aware character parser supporting double/single quotes, backslash escapes, and doubled single-quote escapes
- Fixed potential panic on multibyte UTF-8 input by switching from char-index to byte-offset slicing via `char_indices()`

## [1.0.15] - 2026-04-13

### Fixed

- Fixed rules A008, A009, S014, S015, S034 to count Unicode characters (`chars().count()`) instead of bytes (`len()`), correcting diagnostics for non-ASCII descriptions
- Cached `chars().count()` result in local variable to avoid redundant O(n) traversals
- Added boundary tests for A008/A009 and Unicode-specific tests with multi-byte CJK characters

## [1.0.14] - 2026-04-13

### Fixed

- Fixed frontmatter `get_field` and `get_field_state` to strip single-quoted YAML values (previously only double quotes were handled)
- Deduplicated parsing logic into shared `strip_quotes` and `extract_raw_value` helpers

## [1.0.13] - 2026-04-13

### Changed

- Updated README Project Structure tree to include missing `src/test_helpers.rs`
- Fixed CI/CD section: corrected lint job description (clippy runs in build-and-test, not lint), added actionlint and workflow\_dispatch triggers, documented floating major version tag update in release job

## [1.0.12] - 2026-04-13

### Added

- Unit tests for `context.rs` (`ManifestState::load`, `LintContext::new`) and `main.rs` (`detect_mode`, `resolve_repo_root`) — previously zero test coverage
- 15 new tests covering file I/O states, mode detection precedence, and git root resolution

## [1.0.9] - 2026-04-13

### Fixed

- Fixed macOS install command in README — broken single pipe into three steps (download, extract, sudo mv) so the sudo password prompt no longer fights with curl's pipe for terminal control
- Added cleanup of temp tarball after installation

## [1.0.8] - 2026-04-13

### Added

- File glob exclusion via `[lint] exclude` in `agent-lint.toml` — matching files are
  completely invisible to the linter (no rules checked, no diagnostics produced)
- `ExcludeSet` wrapper with path normalization (`./` stripping, backslash conversion)
- Glob semantics matching `.gitignore` conventions (`*` single level, `**` recursive)
- Exclusion support in `--list-scripts` output
- 22 new unit and integration tests for exclude feature

## [1.0.5] - 2026-04-13

### Fixed

- Updated README.md and docs/rules.md rule counts from 81 to 88 to match actual code
- Added missing A008-A010, D002-D003, G006-G007 entries to docs/rules.md reference table
- Updated category counts (Agents 7→10, Hygiene 5→7, Docs 1→3) and project structure comments

## [1.0.4] - 2026-04-13

### Added

- 7 new lint rules expanding coverage beyond skill content (A008-A010, D002-D003, G006-G007)
- Agent quality: A008 description > 1024 chars, A009 description < 20 chars, A010 name charset [a-z0-9-]
- CLAUDE.md: D002 size limit (500 lines), D003 TODO/FIXME/HACK/XXX detection
- Published content: G006 TODO markers in skill bodies, G007 TODO markers in agent bodies
- Code fence exclusion: TODO detection skips content inside fenced code blocks

## [1.0.3] - 2026-04-13

### Added

- Floating major version tag (`v1`) auto-updated on each release, enabling `@v1` usage in GitHub Actions

## [1.0.2] - 2026-04-13

### Changed

- Refactored bump-version reasoning file to use temp directory instead of `.git/`, eliminating permission prompts
- Updated README summary and expanded github-token documentation with security and usage details
- Guarded empty token in install.sh to avoid sending malformed Authorization header

## [1.0.1] - 2026-04-12

### Added

- Integration tests: mode dispatch with content rules, config ignore/warn for new S* rules
- Boundary tests: S009 at 64 chars, S014 at 1024 chars, S019 at 500 lines, S034 at 20 chars
- CRLF regression test for extract_body, delimiter exact-match test
- collect_skills edge cases: empty dir, missing dir, malformed frontmatter, shared skipping
- End-to-end tests: mixed repo (public + private), valid-skill golden-path sanity check

## [0.2.12] - 2026-04-12

### Added

- Comprehensive unit tests for all 35 skill content lint rules (S009-S043)
- 36 new tests covering 12 previously untested rules plus boundary and edge cases
- S011 leading/trailing hyphen tests, S013 XML assertion tightening
- S029 nested reference pass/fail tests, S032 secret detection pattern tests
- S033 vague name with private-mode-exclusion test
- S039 inline metadata value test, boundary tests for S015/S035

### Fixed

- bump-version now regenerates Cargo.lock after updating Cargo.toml version, preventing stale lockfile drift

## [0.2.11] - 2026-04-12

### Added

- 9 remaining skill content lint rules (S035-S043) completing the full rule set
- S035: compatibility field length check (> 500 chars)
- S036: referenced .md files > 100 lines without ## headings (plugin-only)
- S037: body > 300 lines with no file references (plugin-only)
- S038: time-sensitive date/year patterns in body (plugin-only)
- S039: metadata map values that aren't strings
- S040: unrecognized tool names in allowed-tools
- S041: context: fork with no task instructions
- S042: disable-model-invocation: true with empty description
- S043: Windows-style backslash paths in frontmatter

## [0.2.10] - 2026-04-12

### Added

- 26 new skill content lint rules (S009-S034) based on Anthropic's skill spec and best practices
- Name validation: length, charset, hyphens, reserved words, XML tags, vague names
- Description quality: length limits, person check, trigger context, XML tags
- Body content: line count, empty body, consecutive bash blocks, backslash paths
- Frontmatter field types: boolean validation, context/effort/shell enums, unreachable skills
- Cross-field checks: $ARGUMENTS without argument-hint
- Structural: nested shared-md references, orphaned script files
- Security: non-HTTPS URLs, hardcoded secret detection
- New `SkillInfo` struct and `collect_skills()` shared iterator
- `FieldState` enum and `get_field_state()` for three-state frontmatter extraction
- `field_exists()` and `extract_body()` frontmatter helpers
- New `skill_content.rs` validator module with mode-aware dispatch

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
- TOML configuration file (`agent-lint.toml`) with `[lint]` section supporting `ignore` (suppress errors) and `warn` (downgrade to warnings) by code or name
- Config validation: unknown rule codes/names rejected at load time, typos in section/field names detected via `deny_unknown_fields`

### Changed

- Diagnostic output format: `error[CODE/name]: message` replaces `LINT ERROR: message`
- Exit code semantics: exit 0 when only warnings remain, exit 1 for errors, exit 2 for config errors
- `validate_userconfig_env_mapping` now reports missing env var references when `scripts/` directory is absent

## [0.2.6] - 2026-04-12

### Added

- Self-lint CI job that builds and runs `agent-lint` against the repo's own `.claude/` configuration
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
