# Competitive Analysis: AI Agent Configuration Linters

> Research conducted 2026-04-14. Star counts retrieved via GitHub API on that date.
> Codebase context: `zhupanov/agent-lint` — 104 rules, Rust, Claude Code-focused.

## Top 30 Dedicated AI Agent Config Linters

Ranked by relevance to agent/AI config linting (descending), with star count as tiebreaker.

| # | Repo | Stars | Description | Key Differentiator |
|---|------|-------|-------------|-------------------|
| 1 | [agent-sh/agnix](https://github.com/agent-sh/agnix) | 176 | LSP + CLI linter for CLAUDE.md, AGENTS.md, SKILL.md, hooks, MCP. 399 rules. Rust. | Most comprehensive; 9+ AI platforms; IDE plugins; auto-fix |
| 2 | [alexfazio/plankton](https://github.com/alexfazio/plankton) | 277 | Write-time code quality enforcement for Claude Code via Rust-based linters | Enforcement layer (triggers on file edit), not pure config linter |
| 3 | [thedaviddias/skill-check](https://github.com/thedaviddias/skill-check) | 66 | Linter for agent skill files | SKILL.md focused |
| 4 | [10xChengTu/harness-engineering](https://github.com/10xChengTu/harness-engineering) | 66 | AGENTS.md, lint rules, eval systems, prompt engineering setup | Harness bootstrapping + validation |
| 5 | [seojoonkim/agentlinter](https://github.com/seojoonkim/agentlinter) | 48 | ESLint for AI Agents — AGENTS.md/CLAUDE.md scoring, diagnostics, auto-fix | 8-dimension weighted scoring (0-100) |
| 6 | [Camil-H/cli-agent-lint](https://github.com/Camil-H/cli-agent-lint) | 28 | CLI readiness checker for AI agents: token waste, blocking, safety | Different angle: CLI tool readiness for agents |
| 7 | [WhitehatD/crag](https://github.com/WhitehatD/crag) | 26 | One governance.md compiles to 14 agent formats. Drift detection. | Cross-agent compilation + validation |
| 8 | [netresearch/agent-rules-skill](https://github.com/netresearch/agent-rules-skill) | 26 | Agent Skill for generating/validating AGENTS.md structure | Validator as an agent skill |
| 9 | [roli-lpci/lintlang](https://github.com/roli-lpci/lintlang) | 24 | Static linter for AI agent configs, tool descriptions, system prompts | HERM v1.1 scoring; zero LLM; H1-H7 structural patterns |
| 10 | [samilozturk/agentlint](https://github.com/samilozturk/agentlint) | 22 | ESLint for coding agents. AGENTS.md, rules, skills structured/codebase-aware | npm @agent-lint/cli ecosystem |
| 11 | [xiaolai/nlpm-for-claude](https://github.com/xiaolai/nlpm-for-claude) | 17 | NL Programming Manager — scan, lint, score NL artifacts for Claude | NL-artifact scoring (broader than single file type) |
| 12 | [carlrannaberg/cclint](https://github.com/carlrannaberg/cclint) | 16 | Linter for Claude Code project files. Zod schema validation. | TypeScript; modular; agents/commands/settings |
| 13 | [dotcommander/cclint](https://github.com/dotcommander/cclint) | 13 | Linter for Claude Code agents, commands, and skills | Another cclint variant |
| 14 | [0xmariowu/AgentLint](https://github.com/0xmariowu/AgentLint) | 13 | Fix repo context issues causing AI hallucinations | Repo-level fixer for agent context |
| 15 | [TheStack-ai/pulser](https://github.com/TheStack-ai/pulser) | 12 | SKILL.md linter for Claude Code. Auto-diagnose/fix. GH Actions. | SKILL.md focused; has Marketplace action |
| 16 | [nulone/claude-rules-doctor](https://github.com/nulone/claude-rules-doctor) | 10 | Catch dead .claude/rules/ files. Glob pattern validation. | Narrow focus: dead rule detection |
| 17 | [tilework-tech/nori-lint](https://github.com/tilework-tech/nori-lint) | 8 | Opinionated linter for SKILL.md files | SKILL.md focused |
| 18 | [stbenjam/claudelint](https://github.com/stbenjam/claudelint) | 8 | Linter for Claude Marketplaces and Plugins. Rule-based, extensible. | Python; `.claudelint.yaml` config; Docker |
| 19 | [giacomo/agents-lint](https://github.com/giacomo/agents-lint) | 7 | Detect stale paths, dead npm scripts, context rot in AGENTS.md | Semantic grounding: cross-refs AGENTS.md vs codebase |
| 20 | [voodootikigod/skills-check](https://github.com/voodootikigod/skills-check) | 7 | Quality toolkit for Agent Skills: lint, audits, budgets | GH Actions: "Skills Check CI" |
| 21 | [mraml/nod](https://github.com/mraml/nod) | 7 | Rule-based linter for AI/LLM specs: security + compliance elements | Security-first; platform-agnostic |
| 22 | [felixgeelhaar/cclint](https://github.com/felixgeelhaar/cclint) | 6 | Fast CLAUDE.md context file linter. Anthropic best-practices alignment. | TypeScript; CLAUDE.md focused |
| 23 | [jed1978/instrlint](https://github.com/jed1978/instrlint) | 6 | Lint/optimize CLAUDE.md/AGENTS.md: dead rules, token waste, structural | Dual CLAUDE.md + AGENTS.md targeting |
| 24 | [ofershap/ai-context-kit](https://github.com/ofershap/ai-context-kit) | 6 | Multi-platform context file linter: Cursor, CLAUDE.md, AGENTS.md, Copilot | Rare multi-platform scope |
| 25 | [HaasStefan/skills-lint](https://github.com/HaasStefan/skills-lint) | 6 | Linter for Agent Skills with Token Budget Rules | Rust; token budget focus |
| 26 | [himself65/skill-lint](https://github.com/himself65/skill-lint) | 5 | Validate Agent Skills (SKILL.md) for Claude.ai and Claude Code | SKILL.md validation |
| 27 | [nedcodes-ok/cursor-doctor](https://github.com/nedcodes-ok/cursor-doctor) | 5 | Cursor rules linter and auto-fixer. 100+ checks, zero deps. | Cursor-specific; conflict detection |
| 28 | [vtemian/lint-claude](https://github.com/vtemian/lint-claude) | 5 | Enforce CLAUDE.md guidelines using Claude AI | LLM-powered (uses Claude for enforcement) |
| 29 | [skill-tools/skill-tools](https://github.com/skill-tools/skill-tools) | 5 | Lint, score, route SKILL.md per agentskills.io spec. BM25 routing. | Spec-driven; includes routing |
| 30 | [bitflight-devops/skilllint](https://github.com/bitflight-devops/skilllint) | 4 | Platform-agnostic linter for AI agent plugins/skills: Claude, Cursor, Codex | Multi-platform skills linting |

## Notable Adjacent Tools

| Repo | Stars | Description |
|------|-------|-------------|
| [intellectronica/ruler](https://github.com/intellectronica/ruler) | 2,627 | Apply same rules to all coding agents. Config propagation + drift detection. |
| [open-gitagent/gitagent](https://github.com/open-gitagent/gitagent) | 2,674 | Git-native standard for defining AI agents. Includes `gitagent validate`. |
| [botingw/rulebook-ai](https://github.com/botingw/rulebook-ai) | 590 | Universal managed template for consistent AI agent rules across 10+ platforms. |
| [lifedever/claude-rules](https://github.com/lifedever/claude-rules) | 156 | Coding standards for AI assistants. Auto-detect stack, generate rules. |
| [promptfoo/promptfoo](https://github.com/promptfoo/promptfoo) | 20,097 | Test prompts, agents, RAGs. Red teaming. (Runtime eval, not static linting.) |

## Sub-Niche Clusters

The 30 repos cluster into these sub-niches:

- **Comprehensive multi-file linters**: agnix, agentlinter, cclint variants, instrlint
- **SKILL.md focused**: skill-check, pulser, nori-lint, skills-lint, skill-lint, skill-tools, skills-check
- **AGENTS.md focused**: agents-lint, agent-rules-skill, samilozturk/agentlinter
- **Cursor focused**: cursor-doctor
- **Multi-platform**: ai-context-kit, skilllint, crag
- **Security/compliance**: nod, akz4ol/agentlinter (3 stars, just below cutoff)
- **LLM-powered**: lint-claude

## Key Insights

1. **agnix (176 stars) is the most comprehensive competitor** — Rust-based CLI + LSP with 399 rules covering 9+ AI platforms. IDE plugins for VS Code, JetBrains, Neovim, Zed. Auto-fix capabilities.

2. **The space is nascent** — most repos created Q1-Q2 2026, 176 stars is the ceiling for dedicated tools. The market is pre-consolidation with no dominant player.

3. **Most tools use deterministic static analysis** — a few exceptions exist (vtemian/lint-claude uses Claude AI for enforcement), but the overwhelming design choice is zero LLM dependency for CI/CD compatibility.

4. **Cross-platform coverage is rare** — only 2-3 tools (agnix, ai-context-kit, skilllint) attempt to lint configs for multiple AI coding platforms simultaneously. Most are Claude-only or Cursor-only.

5. **GitHub Actions integration exists in a few tools** (agent-lint, Pulser, Skills Check CI) but the majority of competitors do not ship published Actions.

6. **SKILL.md linting is an active sub-niche** with 5+ dedicated tools — agent-lint itself has 57 dedicated SKILL.md rules (S001-S057).

7. **Semantic/grounding validation is partially addressed** — agent-lint (dead scripts, path validation), agents-lint, and ctxlint cross-reference context files against the live codebase, but most tools remain purely structural.

8. **None of the 30 identified tools target Gemini agent config specifically** — noting Gemini lacks a standardized config directory format analogous to `.claude/`.

9. **Naming collision is severe** — 4+ repos called "cclint", 3+ called "agentlinter", 3+ called "agent-lint", creating discoverability challenges.

## Risk Assessment

**Medium**. Primary risks:

- **Platform spec instability** — if Anthropic/Cursor/OpenAI change config formats, all linters break
- **First-party tooling** — platform vendors may release built-in validation that obsoletes third-party linters
- **Fragmentation** — 20+ competing tools all sub-200 stars, poor discoverability

Mitigating factors: no dominant player has emerged, agent-lint's 104 rules + GitHub Actions + Rust performance provide a solid foundation.

## Open Questions

1. **What specific rules do competitors implement that agent-lint doesn't?** Agnix claims 399 rules vs agent-lint's 104 — a deep rule comparison would identify borrowable ideas.
2. **Which tools have GitHub Actions marketplace entries?** Pulser, Skills Check CI, and agent-lint do; most others don't. Distribution advantage.
3. **How stable are these repos?** Most are <4 months old with single contributors. Monitoring survival rates would inform competitive strategy.
4. **Would Anthropic/OpenAI/Cursor ship first-party linting?** Existential risk for all third-party tools.

## Architectural Patterns Observed

The tools divide into three architectural camps:

1. **Monorepo multi-surface** (agnix only) — single Rust core distributed via CLI, LSP, MCP, and WASM. Most sophisticated but most complex to maintain.
2. **Monolithic CLI** (agentlinter, agents-lint, lintlang, cursor-doctor, cclint variants) — single executable with embedded rules. Optimizes for simplicity and zero-config.
3. **Extensible framework** (claudelint/stbenjam, ctxlint) — plugin mechanisms allowing external rule authoring.

Key architectural divide: **structural/syntactic linters** (checking file format, schema, frontmatter) vs **semantic/grounding linters** (checking whether context claims match the actual codebase). agent-lint bridges both with its dead-script detection, path validation, and cross-field consistency checks.
