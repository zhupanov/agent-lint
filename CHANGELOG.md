# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Added U009 (`userconfig-default-secret`, default warning) for `userConfig`
  option `default` values that commit a secret in the world-readable
  `.claude-plugin/plugin.json`: a `default` declared on a `sensitive: true`
  option, or a string/string-array literal matching the shared possible-secret
  heuristic on any option, on top-level and channel surfaces. It is an agent-lint
  security convention stricter than the manifest schema, is non-autofixable, and
  never echoes the default value in the message, evidence, or suggestion

- Added full T001 (`pr-template-invalid`) URL-template validation and T002
  (`channels-enabled-unsupported`) repository-scope validation. T002 retains
  `channels-enabled-invalid` as a compatibility selector alias; both rules
  report JSON locations, category-only evidence, and safe fixed suggestions.
- Extended S031 (`non-https-url`) and S032 (`hardcoded-secret`) to
  platform-gated `.agents/skills/` and `.cursor/skills/` surfaces when those
  targets are active. Autofix for S031 remains scoped to `skills/` and
  `.claude/skills/`
- Added U008 (`userconfig-option-invalid`) for non-object `userConfig` option
  entries, unknown option fields, invalid optional field shapes, and type
  semantic combinations on top-level and channel surfaces

### Changed

- Made D001 (`docs-ref-missing`) and D002 (`claudemd-too-large`) Always-mode
  checks with structured Markdown Canonical-sources parsing, exact `docs/`
  left-boundary matching, shared repository-safe path probing, and file-level
  advisory size metadata so Basic CLAUDE.md-only repos and fenced/parent-path
  examples no longer false-positive or stay silent
- Retired O005 (`style-name-long`): Claude Code has no output-style name-length
  cap, so O005 is now inert in normal, pedantic, all, and focused runs. Existing
  O005 / `style-name-long` selectors remain accepted for compatibility but emit
  no findings; output-style names remain optional and unconstrained.
- Cursor `.mdc` rule frontmatter (CU002-CU008) is now modeled on Cursor's
  four value-derived activation states (Always, Auto Attached, Agent
  Requested, Manual) instead of key presence. Null, empty, and blank `globs`
  values — including Cursor's own documented empty-`globs` Manual shape — are
  valid unset values and no longer report CU004; CU007 fires only when an
  Always rule declares at least one effective, structurally valid glob; a
  present non-string, non-null `description` is now a CU003 error; a UTF-8 BOM
  no longer hides a valid opening delimiter; near-openers such as `----` and
  `---suffix` report CU002 instead of CU003; and CU002-CU008 rule diagnostics
  carry structured locations, field-scoped evidence, and mechanical
  suggestions, including a targeted quote-the-pattern suggestion when strict
  YAML rejects an unquoted glob (`globs: *.ts`) as an unknown anchor/alias on
  a `globs:` line
- Aligned S028 (`args-no-hint`) and S069 (`hint-no-args`) with the merged
  command-argument contract. S028's compiled default is now a warning (still an
  error under `--pedantic`, suppressible, and per-file overridable) to match its
  advisory optional-metadata nature and its inverse smell S069. S069 no longer
  false-positives on positional-argument skills: a body now counts as
  referencing its arguments when it contains `$ARGUMENTS`/`${ARGUMENTS}`
  anywhere, or a positional reference `$1`–`$9` / `${1}`–`${9}` on a line
  outside code fences (`$10` and `$1x` do not count; fenced positional refs such
  as `awk '{print $1}'` still do not suppress the smell). S028's trigger set is
  unchanged (`$ARGUMENTS` forms only).
- Agent discovery is now recursive across `.claude/agents/`, the plugin
  `agents/` default, and manifest-declared `plugin.json` `agents` roots, matching
  Claude Code: agents nested in subdirectories are seen by every agent rule
  (A002-A004, A008-A030) and the A030 overlap pool, and carry their full
  repository-relative path in diagnostics and `related_subjects`. A001
  (`agents-dir-missing`) is narrowed to a manifest-declared agent path that does
  not exist — an absent implicit `agents/` is now clean — and A004
  (`no-agent-files`) fires only for a present default or declared root that holds
  no agent files (all-excluded roots stay silent). Template rules A005-A007 stay
  scoped to the flat top-level `agents/*.md` convention. Exclusions and per-file
  overrides match the full nested path, and a symlinked root is never followed
  out of the repository
- **BREAKING**: G010 (`bash32-incompatible`) and G011 (`awk-regex-nonascii`) are
  now compiled-default warnings instead of errors, so a run that only trips them
  no longer exits non-zero; a repository targeting Bash 3.2 or ASCII-only
  portable awk restores the failure with `error = ["G010", "G011"]`. G009
  (`bash-replacement-unsafe`) stays a default error. G009-G011 were rebuilt on a
  shared shell/awk lexical layer (`validators/shell.rs`) that masks comments,
  quoting, and inert text rather than matching raw lines: G009 now covers
  positional/special/braced/command/arithmetic replacements and the single-slash
  `${v/pat/$rep}` form while sparing quoted, ANSI-C, escaped, and literal
  replacements; G010 flags a probe-verified Bash-3.2 matrix (adding `;&`, `;;&`,
  `|&`, `declare -g`, `shopt -s globstar`, `wait -n`), gates the `if command
  <cmd>` errexit hazard on lexical `set -e`/`.inc.bash` and the empty-array
  hazard on `set -u`/`.inc.bash` with conservative function-scoped analysis, and
  no longer flags Bash-3.2-supported forms; G011 analyzes the actual awk regex
  operand (regex literals, `~`/`match`/`sub`/`gsub`/`split` operands, `-F`/`FS`
  values, and `-v` variables traced to a regex use), sparing display-only text
  and ASCII regexes. All three now emit structured source locations, evidence,
  and suggestions
- Narrowed D003/G006/G007 unfinished-work detection to a shared syntactic
  marker grammar (`TODO:` / `FIXME(owner):` / comment and unchecked-task
  forms) with Markdown context exclusions, structured span/evidence/suggestion
  metadata, once-per-file reporting, and Basic-mode dispatch for D003 so
  prose about marker words no longer false-positives
- Unified S008/S029/S036 shared-Markdown reference recognition into one
  scanner: brace-less `$CLAUDE_PLUGIN_ROOT/...` forms, exact `.md` token
  boundaries (no `.md.backup` / `.mdx` / `.md/child` prefix truncation), and
  HTML-comment dormancy. S008 dedupes per normalized target and attaches the
  first live occurrence's line
- X001 messages no longer embed YAML-relative `at line N, column M` parser
  coordinates; structured file location remains authoritative (with column
  when the parser supplies one)
- S004 names UTF-8 BOM-caused malformed frontmatter and always reports line 1;
  S005–S007 attach structured lines for locatable simple top-level keys
- Soft-retired S001 registry wording and `docs/development.md` module note to
  match the S049 convention (`S002-S008` for the skills layout module)

- **BREAKING**: S055 (`script-errhand-missing`) recursively inspects
  shell/Python scripts under each public skill's `scripts/` tree (including
  `.bash` and extensionless shebang files), requires both statement-level
  `try:` and `except` for Python, and attributes each finding to the offending
  script path instead of `SKILL.md`. Per-file overrides that suppressed S055
  via `SKILL.md` paths must retarget `scripts/` globs or individual script
  paths; skill-level `suppress = ["S055"]` is unchanged.
- Q001 (`prompt-generic-filler`), Q002 (`prompt-negative-only`), and Q003
  (`prompt-weak-critical`) now share one operative live-directive classifier: a
  phrase counts only when, within its sentence, it opens the instruction, an
  agent subject (`you`/`the agent`/`agents`/`assistant`/`model`) uses a modal
  before it, or it follows an `if`/`when`/`before`/`after`/`unless`/`while`
  setup clause and comma. Descriptive, historical, and interrogative prose and
  every example scope (including `# Important examples`) are inert. Q002 now
  associates a positive `instead`/`rather`/`prefer` alternative only within the
  same Markdown instruction scope (same paragraph or list item, within three
  source lines) instead of a global prose-line window, and Q003 treats a
  sentence-leading `Should` conditional inversion as non-weak. All three report
  every violating line in source order with the matched range, bounded masked
  evidence, and a suggestion; existing safety/integrity exemptions and
  severities are unchanged
- **BREAKING**: Removed I005 (`instruction-file-structure`) and its legacy
  CX044 / `codex-agents-structure` aliases. Syntax and byte-length heuristics
  cannot soundly decide whether inherited instruction files are
  project-specific; existing `agent-lint.toml` references and `--only`
  selections using those identifiers now fail as invalid rule identifiers.
- Narrowed I004 (`instruction-file-generic`) to exact generic-only Markdown
  prose (`be helpful`, `be accurate`, `write good code`, `follow best
  practices`, and conjunctions of those complete phrases) after shared
  Markdown exclusions, and re-severitized it to a default warning with
  structured location, bounded evidence, and suggestion metadata
- Broadened body-content recognizers for S041/S046/S047/S055: wider S041 task
  verbs, numbered-list continuations and `1)` markers for S046, plural example
  headings for S047, and shell `|| { …; exit|return; }` / `if !` idioms for
  S055. S041 (`fork-no-task`) now defaults to warning instead of error
- **BREAKING**: Removed repository-specific Slack rule K001
  (`slack-fallback-mismatch`) and its validator. The rule hardcoded
  `LARCH_SLACK_*` fallback names that belong to the separate Larch consumer,
  not to Agent Lint's portable Claude/Slack contract. Existing
  `agent-lint.toml` references and `--only` selections using `K001` or
  `slack-fallback-mismatch` now fail as invalid rule identifiers; remove those
  entries. Downstream repositories that need the three-variable invariant must
  enforce it in their own tests or lint configuration.
- S030 (`orphaned-skill-files`) treats a scripts file as referenced when its
  name appears in any skill-local `.md` (not only `SKILL.md`), with a leading
  character-boundary check so `dry-run.sh` no longer shadows `run.sh`
- S036 (`ref-no-toc`) uses `MarkdownDocument` headings (levels 1–6 outside
  fences) and honors `ExcludeSet` for the shared-file subject
- S072 (`skill-dir-oversized`) counts files under conventional build/dependency
  directories (`dist`, `node_modules`, …), still skipping `.git` and directory
  symlinks
- S073 (`skill-ref-nested`) flags only skill-relative `.md` links, skips URI
  schemes, strips `#fragment`/`?query` before depth checks, counts `..`
  components, and attaches link line metadata
- **BREAKING**: Removed U003 (`userconfig-env-missing`). Existing
  `agent-lint.toml` references and `--only` selections using `U003` /
  `userconfig-env-missing` now fail as invalid rule identifiers; remove those
  entries. Option use is no longer inferred from `scripts/**/*.sh` text.
- **BREAKING**: U007 (`userconfig-key-invalid`) is now a default error and
  accepts only `^[A-Za-z_][A-Za-z0-9_]*$` (hyphen and dot keys are rejected)
- Broadened U001/U002/U004/U005/U006 to validate every
  `channels[].userConfig` / `channels.<name>.userConfig` with the same schema
  as top-level `userConfig`
- U006 now accepts exactly `string`, `number`, `boolean`, `directory`, and
  `file`
- U002/U005 reject empty and Unicode-whitespace-only labels (stricter than the
  upstream schema, documented in `docs/rules.md`)
- User-config diagnostics carry JSON-pointer evidence and actionable
  suggestions without exposing configured default values
- **BREAKING**: Removed unsupported skill-name rules S012
  (`name-reserved-word`) and S013 (`name-has-xml`). Existing `agent-lint.toml`
  references and `--only` selections using either retired code or name now
  fail as invalid rule identifiers; remove those entries. S010
  (`name-invalid-chars`) remains the single diagnostic for angle brackets and
  other invalid name characters. S013 no longer participates in `--autofix`.
- Narrowed S033 (`name-vague`) to exact domainless implementation labels
  (`helper`, `helpers`, `util`, `utilities`, `utility`, `utils`, `tool`,
  `tools`) for published plugin skills, with a domain/task-focused message
  and suggestion; S033 diagnostics now carry the same name-field location
  and canonical-name evidence as S009-S011
- Stopped flagging broad subject nouns (`data`, `files`, `documents`) and
  compounds that contain otherwise generic tokens under S033
- **BREAKING**: Renamed rule G005 from `security-md-missing` to
  `security-policy-missing`. The stable code `G005` is unchanged; the
  pre-rename name `security-md-missing` remains accepted as a silent legacy
  selector alias (canonical identity in diagnostics and JSON stays
  `G005`/`security-policy-missing`). G005 now recognizes a repository-local
  `SECURITY.md` in the repository root, `.github/`, or `docs/` (previously
  repo root only), matching GitHub's supported community-health file locations
- Skill frontmatter field-type rules S023-S027, S035, S039, S043, S063, S064,
  S066, S070, and S071, plus the cross-field rules S028/S069, now read
  frontmatter through canonical YAML instead of the legacy line-oriented
  helpers. Trailing YAML comments, YAML 1.2 boolean spellings (`True`/`TRUE`),
  quoting, and multiline scalars are interpreted as the platform's parser would;
  each rule skips a file whose frontmatter is invalid YAML or not a mapping
  (X001/S004/S005 own those states). S035 counts Unicode scalar values rather
  than bytes; S039 flags every non-string `metadata` entry and reports a single
  shape diagnostic for a present-but-non-mapping `metadata`; S064 fires only for
  a non-empty string `agent`; S070 reads canonical keys so quoted (`"name"`) and
  spaced (`name :`) spellings are no longer reported as unknown fields; and S043
  scans only path-configuration values, exempting the `description`,
  `compatibility`, `when_to_use`, and `metadata` fields, with a single-line-safe
  autofix
- `frontmatter::parse_yaml_strict` restores the trailing newline dropped by line
  extraction, so a frontmatter block whose final line is a bare `key:` (a valid
  null value) no longer reports X001 (`frontmatter-yaml-invalid`)

### Deprecated

- Soft-retired S042 (`dmi-empty-desc`): the rule no longer emits in any mode
  (including `--all`) because it was a strict subset of
  S005 (`frontmatter-field-missing`), while `S042` / `dmi-empty-desc` remain
  recognized in configuration for compatibility
- Soft-retired S049 (`name-not-gerund`): the rule no longer emits in any mode
  (including `--all`), while `S049` / `name-not-gerund` remain recognized in
  configuration for compatibility

### Removed

- **BREAKING**: Removed CU009 (`cursor-description-missing`). Description
  presence is what selects Cursor's Agent Requested mode, so a
  missing-description diagnostic has no sound positive case and falsely warned
  on valid Manual rules (including empty frontmatter). Configurations
  referencing `CU009` or `cursor-description-missing` — in `agent-lint.toml`
  lists, per-file overrides, or `--only` selections — must delete that
  identifier; it is not aliased to any other rule and now fails as an invalid
  rule identifier

## [3.0.1] - 2026-07-17

### Added

- Added config-only per-file rule suppression overrides with structured
  diagnostic subjects and suppression-aware autofix behavior

### Changed

- Documented the administrator squash-merge procedure for release pull requests

## [3.0.0] - 2026-07-17

### Added

- Added a version-bump verification harness
- Added JSON Schema validation for Cursor cloud environment structure

### Changed

- Replaced manual CLI parsing with `clap` and derived rule-registry metadata
  with `strum`
- Centralized filesystem traversal and exclusion handling
- Replaced the deprecated YAML parser with `noyalib`
- Replaced custom Markdown parsing with a shared Comrak document model
- Centralized HTTP(S) URL validation with the `url` crate

## [2.7.0] - 2026-07-17

### Added

- Added normalized per-path D004 import caps with explicit precedence over the
  compatible global per-import cap
- Added named S062 prompt-source budgets for root, transitive closure, token,
  blank-line-neutral content-token, and conditional-source measurements
- Added `--closure-report` deterministic JSON output for configured prompt
  source groups
- Ported the remaining focused S021 and G009 downstream regression cases

### Changed

- Dispatch configured prompt and script contracts in Basic mode as well as
  Plugin mode
- Report all S021 pairs with stable file locations and deterministic file order

## [2.6.0] - 2026-07-16

### Added

- Added an explicit, validated script inventory for deterministic G009-G011
  scans of `.sh`, `.inc.bash`, and standalone `.awk` files
- Ported Bash 3.2 empty-array flow analysis and awk command/body parsing for
  continuations, heredocs, multiline programs, and regex-only non-ASCII checks

### Changed

- Enabled G010 and G011 as compiled-default errors
- Made explicit portability inventories authoritative even when unrelated
  global exclusion globs match an inventoried file

## [2.4.0] - 2026-07-16

### Added

- Migrated general-purpose agent evidence, skill invocation, fenced-shell,
  prompt-closure, document-pointer, and shipped-script safety checks
- Added configurable skill-description, skill-closure, and `CLAUDE.md`
  import-closure limits
- Added optional Bash 3.2 and portable awk regex checks

### Changed

- Extended consecutive Bash detection to robust fences, reference files,
  short breadcrumb/comment separators, and reason-bearing exceptions

## [2.3.5] - 2026-04-18

### Changed

- Final update to `PROPOSED_AGNIX_CHANGES.txt` candidate issues list

## [2.3.4] - 2026-04-18

### Changed

- Updated `PROPOSED_AGNIX_CHANGES.txt` candidate issues list

## [2.3.3] - 2026-04-18

### Added

- Added `PROPOSED_AGNIX_CHANGES.txt` with a list of yet-to-be-verified candidate issues to file

## [2.3.2] - 2026-04-15

### Added

- Added `COMPETITOR-FEATURES.md` with competitive feature gap analysis: 26 features agent-lint lacks, 8 with inferior implementation

## [2.3.1] - 2026-04-15

### Changed

- Refactored documentation: extracted CLI, GitHub Action, Configuration, and Development sections into dedicated docs under `docs/`
- Moved auto-fixable rules table from README to `docs/rules.md`
- Renamed "Both" lint mode to "Always" and "off" default severity to "suppressed" in `docs/rules.md`

## [2.3.0] - 2026-04-15

### Changed

- **BREAKING**: Renamed `ignore` config field to `suppress` in `agent-lint.toml` — users must update `[lint] ignore = [...]` to `[lint] suppress = [...]`
- Using the old `ignore` field now produces a clear error listing the valid field names

## [2.2.7] - 2026-04-15

### Changed

- Suppressed `body-too-long` (S019) by default — reduced default-warning count from 34 to 33, default-suppressed count now 3

## [2.2.6] - 2026-04-15

### Changed

- Suppressed `body-no-examples` (S047) by default — reduced default-warning count from 35 to 34, default-suppressed count now 2

## [2.2.5] - 2026-04-15

### Changed

- 35 previously suppressed rules now fire as warnings by default (only `name-not-gerund` stays suppressed)
- `--pedantic` mode now promotes default-warning rules to errors (in addition to user-warn-listed rules)

## [2.2.4] - 2026-04-15

### Changed

- Updated all documentation references from `@v1` to `@v2`
- Added exact version pinning with advisories in CI and pre-commit examples
- `/bump-version` now auto-updates explicit version numbers in README.md

## [2.2.3] - 2026-04-15

### Added

- Pre-commit hook support: users can now use agent-lint via the pre-commit framework without manual installation
- `scripts/pre-commit-hook.sh` downloads and caches the pre-built binary from GitHub Releases with SHA-256 verification

## [2.2.2] - 2026-04-15

### Changed

- `/relevant-checks` self-lint phase now calls `agent-lint --all` for strictest severity-level validation

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
- `--all` CLI flag: forces every rule to error, bypassing suppress/warn config
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

- Config priority cascade: user suppress > user error > user warn > compiled default severity
- Default-suppressed rules are silently skipped (no count, no output) unless promoted via config
- Updated S050 and S056 diagnostic messages to remove stale config guidance

## [2.0.0] - 2026-04-14

### Changed

- **BREAKING**: Renamed project from `claude-lint` to `agent-lint`: binary, config file (`agent-lint.toml`),
  GitHub Action (`zhupanov/agent-lint@v2`), Marketplace listing, and all documentation updated
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

- Added E002 `email-type-invalid` for non-string plugin and marketplace email metadata.

### Changed

- Hardened E001 email contact-metadata validation, made its quality convention explicit, and changed its default severity to warning; diagnostics now redact email values and include structured field locations.

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

- Integration tests: mode dispatch with content rules, config suppress/warn for new S* rules
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
- TOML configuration file (`agent-lint.toml`) with `[lint]` section supporting `suppress` (suppress errors) and `warn` (downgrade to warnings) by code or name
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
