## Goal
Implement issue #109: [IMPLEMENTING] H008+: Hook schema validation engine — events, handler types, matchers, timeouts (settings.json, hooks.json, skill/agent frontmatter).

## Implementation Plan
## Why

This is the single biggest gap found in the agnix audit. agent-lint's H001–H007 check only that hooks files exist, parse as JSON, and that command scripts exist and are executable. The hook **schema** is never validated: an invalid event name (`PreToolUsage`), a matcher on `Stop`, a prompt-hook without `prompt`, `timeout: -5`, or an `mcp_tool` hook without `server` all pass silently. agnix has 27 hook rules; 22 are worth porting.

Source: agnix rule-gap audit (agnix @ `6733878`, audited 2026-07-16).

## Design: one hook-object validation engine, four surfaces

Build a single validator for a "hook object" and apply it everywhere hooks appear:

1. `hooks/hooks.json` (plugin)
2. `.claude/settings.json` and `.claude/settings.local.json` (`hooks` key)
3. Skill frontmatter `hooks:` (agnix CC-SK-010)
4. Agent frontmatter `hooks:` (agnix CC-AG-011)

## Rules to add (suggested codes H008+)

| agnix ID | Check | Suggested default |
|---|---|---|
| CC-HK-001 | Event name must be one of the valid events (case-sensitive; per agnix: SessionStart, UserPromptSubmit, PreToolUse, PermissionRequest, PostToolUse, PostToolUseFailure, SubagentStart, SubagentStop, Stop, PreCompact, Setup, SessionEnd, Notification, TeammateIdle, TaskCompleted — re-verify list against current docs) | error |
| CC-HK-002 | `type: prompt`/`type: agent` only on events that support them | error |
| CC-HK-004 + CC-HK-018 | Matcher on non-tool event (Stop/SubagentStop/UserPromptSubmit) — merge agnix's two variants into one rule | error |
| CC-HK-005 | Missing `type` field | error |
| CC-HK-006 | `type: command` requires `command` | error |
| CC-HK-007 | `type: prompt` requires `prompt` | error |
| CC-HK-009 | Dangerous command patterns in hook commands (`rm -rf`, `git reset --hard`, `curl \| sh`, …) | warn |
| CC-HK-010 | Timeout exceeds platform cap (600s command / 30s prompt). Only the "excessive" half; a missing-timeout nag would be noisy | warn |
| CC-HK-011 | `timeout` must be a positive integer | error |
| CC-HK-013 | `async: true` only valid on `type: command` | error |
| CC-HK-014 | `once` only valid in skill/agent frontmatter hooks, not settings.json | warn |
| CC-HK-015 | `model` only valid on prompt/agent hooks | error |
| CC-HK-016 | Unknown hook type (valid: `command`, `prompt`, `agent`, `http`, `mcp_tool`) | error |
| CC-HK-017 | prompt/agent hook should reference `$ARGUMENTS` | warn |
| CC-HK-019 | Deprecated `Setup` event → suggest `SessionStart` | warn |
| CC-HK-020 | `type: http` requires `url` | error |
| CC-HK-021 | `if` field must be a non-empty string, only on tool events | warn |
| CC-HK-022 | `shell` must be `bash`/`powershell` (share enum with S026) | warn |
| CC-HK-023 | `once` must be a boolean | error |
| CC-HK-024 | HTTP hook headers with `$VAR` interpolation need `allowedEnvVars` | warn |
| CC-HK-026 | `type: mcp_tool` requires non-empty `server` | error |
| CC-HK-027 | `type: mcp_tool` requires non-empty `tool` | error |
| CC-SK-010 | Skill frontmatter `hooks:` must follow the hooks schema (engine applied to surface 3) | error |
| CC-AG-011 | Agent frontmatter `hooks:` must follow the hooks schema (engine applied to surface 4) | error |

## Deliberately not ported

- CC-HK-003 (advisory "consider adding a matcher") — style nag, noisy.
- CC-HK-025 (matcher values vs per-event allowlists) — allowlist churns with Claude Code releases; revisit later.
- CC-HK-008/CC-HK-012 — already covered by H004 and H002/H006.

## Implementation notes

- Event list and handler-type set should live in one module; expect to update on Claude Code releases.
- `.claude/settings.local.json` is a new scan surface (currently only `settings.json` is read).
- Skill/agent frontmatter `hooks:` requires structured YAML access — coordinate with the strict-YAML-parsing issue (markdown/frontmatter robustness) if implemented first.

## Acceptance criteria

- [ ] One shared hook-object validator; all four surfaces covered, with tests per surface
- [ ] Valid event/type lists verified against current Claude Code hooks docs before merge
- [ ] Rules registered in `src/rules.rs` + `docs/rules.md`; `make cargo-test`, `make clippy`, `make fmt` pass

## Test plan
(no test plan section in plan-file)
