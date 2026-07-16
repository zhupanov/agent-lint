## Goal
Implement issue #108: [IMPLEMENTING] A012+: Agent frontmatter field validation (model, permissionMode, tools, skills, memory, effort, isolation, maxTurns).

## Implementation Plan
## Why

agent-lint's agent validation (A001–A011) covers name/description presence and quality plus larch-specific template alignment, but performs **no field-value validation at all**: `model: sonet` (typo), `permissionMode: yolo`, `tools: [Bsh]`, or a `skills:` entry pointing at a nonexistent skill all pass silently today. agnix validates all of these. This is the cheapest/highest-value gap from the agnix audit: spec-grounded enum/type checks with near-zero false-positive risk.

Source: agnix rule-gap audit (agnix @ `6733878`, knowledge-base v1.1.0, audited 2026-07-16).

## Rules to add (suggested codes A012+)

| agnix ID | Check | Suggested default |
|---|---|---|
| CC-AG-003 | `model` must be valid: `sonnet`/`opus`/`haiku`/`inherit` — extend the enum with full model IDs (e.g. `claude-sonnet-5`) and `[1m]` variants per current Claude Code docs | error |
| CC-AG-004 | `permissionMode` must be one of `default`, `acceptEdits`, `dontAsk`, `bypassPermissions`, `plan`, `delegate` | error |
| CC-AG-005 | Every entry in `skills:` array must exist on disk (`skills/<name>/SKILL.md` or `.claude/skills/<name>/SKILL.md`) | error |
| CC-AG-006 | No tool may appear in both `tools` and `disallowedTools` | error |
| CC-AG-008 | `memory` must be `user`, `project`, or `local` | error |
| CC-AG-009 | Tool names in `tools` must be known Claude Code tools (accept `mcp__<server>__<tool>` format) — reuse S040's tool list | error |
| CC-AG-010 | Same check for `disallowedTools` | error |
| CC-AG-012 | Warn on `permissionMode: bypassPermissions` (disables safety checks) | warn |
| CC-AG-013 | `skills:` entries must be kebab-case | warn |
| CC-AG-014 | `effort` must be `low`/`medium`/`high`/`xhigh`/`max` | error |
| CC-AG-015 | `isolation` must be `worktree` if present | error |
| CC-AG-016 | `background` must be a boolean | warn |
| CC-AG-017 | `maxTurns` must be a positive integer | error |
| CC-AG-019 | Unknown agent frontmatter field (typo catcher) | warn |

Note: CC-AG-016 is documented in agnix's VALIDATION-RULES.md but absent from its rules.json (drift on their side); validate the field name against current Claude Code docs during implementation.

## Scope extension

agnix validates `.claude/agents/*.md` in all modes; agent-lint currently only validates `agents/` in Plugin mode. Extend agent validation to `.claude/agents/*.md` in Basic mode (larch-specific rules A005–A007 should stay plugin-only).

## Implementation notes

- Extends `src/validators/agents.rs`; the known-tool list should be shared with S040 (`skill_content`) rather than duplicated.
- CC-AG-011 (hooks object in agent frontmatter) is deliberately **excluded** here — it lands with the hook-schema engine issue.

## Acceptance criteria

- [ ] New rules registered in `src/rules.rs` and documented in `docs/rules.md` (codes, names, defaults, mode column)
- [ ] Agent validation runs on `.claude/agents/*.md` in Basic mode
- [ ] Unit tests per rule (valid, invalid, missing-field cases); `make cargo-test`, `make clippy`, `make fmt` pass
- [ ] Model enum verified against current Claude Code docs before merge

## Test plan
(no test plan section in plan-file)
