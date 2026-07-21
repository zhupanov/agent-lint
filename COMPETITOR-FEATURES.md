# Competitor Feature Gap Analysis

> Research conducted 2026-04-15. Codebase context: branch `main`, commit
> `34bb8a1`. 5 research agents + 5 validation reviewers.

Comprehensive list of major features (not specific lint rules) that
competitors listed in `COMPETITIVE-ANALYSIS.md` have and agent-lint
either lacks entirely or has inferior implementation.

## Features Agent-Lint Lacks Entirely

| # | Feature | Description | Competitors |
|---|---------|-------------|-------------|
| 1 | **LSP Server** | Real-time IDE diagnostics, code actions, hover info. Agent-lint's product surface is CLI + GitHub Action + pre-commit; no editor feedback loop. | agnix, cursor-doctor, seojoonkim/agentlinter |
| 2 | **IDE Extensions/Plugins** | First-party editor extensions for VS Code, JetBrains, Neovim, Zed. | agnix (4 editors), cursor-doctor (VS Code), seojoonkim/agentlinter (VS Code) |
| 3 | **MCP Server Mode** | Lint exposed as a tool for AI agents via Model Context Protocol. | agnix (agnix-mcp), samilozturk/agentlint (read-only MCP) |
| 4 | **WASM Distribution** | Browser/edge deployment, browser playground. | agnix (agnix-wasm + playground) |
| 5 | **Structured Output Formats** | Machine-parseable output (JSON, SARIF, etc.) for CI pipelines, SonarQube, etc. Agent-lint outputs only human-readable `error[CODE/name]: msg` to stderr; summary to stdout. No `--format` flag. | cli-agent-lint (JSON), ai-context-kit (structured reports), seojoonkim/agentlinter (`--format github`), skills-check (JSON report) |
| 6 | **Scoring/Grading Systems** | Numeric quality scores (0-100) across weighted dimensions. | seojoonkim/agentlinter (8-dimension weighted 0-100), lintlang (HERM v1.1 scoring, H1-H7), xiaolai/nlpm-for-claude (NL-artifact scoring) |
| 7 | **Token Counting/Budget Analysis** | Token measurement, waste detection, budget limits. | skills-lint (Rust, token budgets), skills-check (token budgets), cli-agent-lint (token waste), ai-context-kit (token measurement + task-budget `select()`) |
| 8 | **Plugin/Extension System** | User-authored rules without recompiling. Agent-lint's 225 rules are a closed Rust enum in `src/rules.rs`. | claudelint (custom Python rule files), ctxlint (extensible framework), seojoonkim/agentlinter (`.agentlinterrc` custom rules) |
| 9 | **Multi-Platform Linting** | Lint configs for Cursor, Copilot, Codex, Gemini, Kiro, Cline, etc. Agent-lint is Claude Code-only (`.claude/` and `.claude-plugin/`). | agnix (9+ platforms), ai-context-kit (Cursor+Claude+Copilot), skilllint (Claude+Cursor+Codex), crag (14 agent formats) |
| 10 | **Init/Scaffolding Commands** | Generate starter config files, templates. | seojoonkim/agentlinter (`init` templates), ai-context-kit (`init`), samilozturk/agentlint (`init`/`scan`/`score`), claudelint (`--init`) |
| 11 | **Watch Mode** | File system watchers for incremental re-lint on save. LSP-based tools provide this inherently. | agnix (via LSP), plankton (Claude Code hooks on each edit) |
| 12 | **Config Inheritance/Presets** | Base configs, `extends` directive, shareable presets. Agent-lint has flat single-file `agent-lint.toml`. | seojoonkim/agentlinter (`.agentlinterrc` with `extends`), ruler (centralized rules, nested `.ruler/` dirs, MCP propagation) |
| 13 | **Docker Distribution** | Container image for enterprise CI/CD. | claudelint (Docker on GHCR) |
| 14 | **npm Distribution** | `npm install` / `npx` usage. Agent-lint distributes via GitHub Releases + GH Action + pre-commit but not via language package registries. | agnix (npm), samilozturk/agentlint (`@agent-lint/cli`), skills-lint (npm) |
| 15 | **pip/uvx Distribution** | Python package manager distribution. | claudelint (pip/uvx installable) |
| 16 | **Homebrew Distribution** | macOS package manager distribution. | agnix (Homebrew formula) |
| 17 | **Cargo Install (crates.io)** | Rust package registry distribution. | agnix (`cargo install`) |
| 18 | **Cross-Agent Compilation/Sync** | One source config compiled to multiple agent formats with sync/export. | crag (governance.md to 14 formats), ai-context-kit (`sync`), seojoonkim/agentlinter (multi-framework `export`) |
| 19 | **Drift Detection** | Detect config staleness/divergence from codebase state over time. | crag (cross-format drift), agents-lint (stale paths, context rot) |
| 20 | **Web Reports/Dashboards** | Web-based quality reports with percentile tracking. | seojoonkim/agentlinter (web reports, percentiles) |
| 21 | **Lint Result Caching** | Cache lint results for unchanged files to speed up re-runs. (Agent-lint's pre-commit hook caches the binary download, but no analysis result caching exists.) | skills-check (persistent disk caching) |
| 22 | **Browser Playground** | Try rules in a web browser without installing. | agnix (browser playground) |
| 23 | **Windows Binaries** | Pre-built Windows binaries and GH Action Windows runner support. (Agent-lint's Rust code compiles on Windows; the gap is binary distribution. Also no Intel macOS binary.) | skills-lint (Windows binaries) |
| 24 | **Cross-Platform Conflict Detection** | Detect conflicts between Cursor, Claude, and Copilot configs in the same repo. | ai-context-kit |
| 25 | **Staleness/Freshness Scoring** | Freshness metrics for config files. | agents-lint, seojoonkim/agentlinter |
| 26 | **Rename-Aware Fixes** | Auto-fixes that track and update cross-file references. | seojoonkim/agentlinter |

## Features With Inferior Implementation

| # | Feature | Agent-Lint Status | Competitor Advantage |
|---|---------|-------------------|---------------------|
| 27 | **Autofix Coverage** | 12/225 rules, intentionally limited to purely mechanical fixes (chmod, string replacement). Autofix infrastructure is sound (iterative loop, re-validation). | agnix claims broader auto-fix across 399 rules (unverified). seojoonkim/agentlinter has rename-aware fixes. |
| 28 | **Semantic/Codebase Grounding** | Substantial: dead script detection (G004), path validation (G002), script executability (G003), shared markdown refs (S008, S029), agent/template count alignment (A005-A007), CLAUDE.md docs refs (D001), orphaned skill files (S030). | ctxlint, agents-lint, seojoonkim/agentlinter go deeper: npm script verification against `package.json`, cross-file conflict analysis, maintenance workflows driven by local change signals. |
| 29 | **CI Annotations** | GH Action runs `agent-lint` and passes stderr through. The Diagnostic struct has no file/line/column fields, so there are no GitHub workflow annotations (`::error file=...,line=...::`), no inline PR comments. | seojoonkim/agentlinter has `--format github` for native CI annotations. Pulser has GitHub Actions Marketplace entry. |
| 30 | **Inline Suppression** | Global suppression via `agent-lint.toml` (`suppress = [...]`) and file-level exclusion via `exclude` globs. No inline comment-based suppression within linted files (e.g., `<!-- agent-lint-disable S001 -->`). | ESLint-style per-line/block suppression is a standard pattern. |
| 31 | **Monorepo Support** | Single git root detection via `git rev-parse --show-toplevel` (with non-git fallback). Users can invoke `agent-lint path-a && agent-lint path-b`, but no single invocation supports multiple roots, no config aggregation, no root-relative path reporting. | crag has workspace/monorepo commands. ruler has nested `.ruler/` directories. |
| 32 | **Diff-Only Linting** | Always lints the full directory tree. No `--diff`, `--changed-only`, or `--staged` mode. | Standard in mature linters for large repo CI performance. |
| 33 | **Rule Doc Generation** | `docs/rules.md` exists as a comprehensive rule reference but appears manually maintained. No generation from `src/rules.rs` metadata, creating potential drift risk. | Some extensible frameworks auto-generate rule docs from rule metadata. |
| 34 | **CI Integration Breadth** | Integrates with GitHub Actions (composite action) and pre-commit (with binary caching). No templates or documentation for GitLab CI, CircleCI, Azure DevOps, Bitbucket Pipelines, or Jenkins. | Several competitors document multi-CI usage. |

## Risk Assessment

**Medium**. The largest competitive risk is agnix's comprehensive
multi-surface distribution (CLI + LSP + MCP + WASM + IDE plugins + 9
platforms + 399 rules). However, most competitor tools are less than 4
months old with single contributors. Two existential risks apply to all
tools in this space: platform vendors may release first-party validation,
and platform spec formats may change. Agent-lint's strength (focused,
fast, deterministic, zero-dependency Rust CLI) remains valuable for
CI/CD use cases.

## Key Files Relevant to Feature Gaps

- `src/main.rs` -- CLI entry point, flag parsing, mode detection
- `src/diagnostic.rs` -- Diagnostic struct (no file/line fields),
  stderr-only output formatting
- `src/rules.rs` -- Closed LintRule enum (225 variants), severity
  defaults, autofix registry
- `src/config.rs` -- Flat TOML loading, suppress/error/warn/exclude, no
  inheritance
- `src/autofix.rs` -- Iterative autofix loop, 12 fixable rules,
  Unix-only chmod
- `src/validators/` -- 24 validator modules, mode-gated dispatch (Basic
  vs Plugin)
- `action.yml` -- GitHub Action composite, no annotation formatting
- `.pre-commit-hooks.yaml` -- Pre-commit integration with binary caching
- `Cargo.toml` -- Not published to crates.io

## Open Questions

1. Which competitor feature claims are verified? Most competitor
   assertions (agnix's 399 rules, agentlinter's scoring dimensions)
   are from READMEs/descriptions and have not been verified against
   source code.
2. How stable are the competitor tools? Most are less than 4 months old.
   Survival rates will determine which features become market
   expectations vs. niche experiments.
3. Would Anthropic ship first-party CLAUDE.md/SKILL.md validation? This
   is an existential risk for all tools.
4. The `.pre-commit-hooks.yaml` description claims to lint `.cursorrules`
   but agent-lint does not lint Cursor files -- this is a pre-existing
   documentation inaccuracy.
