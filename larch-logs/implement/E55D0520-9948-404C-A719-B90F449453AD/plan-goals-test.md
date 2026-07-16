## Goal
Implement issue #115: [IMPLEMENTING] X001+: Markdown & frontmatter robustness — strict YAML parsing, unclosed fences, XML tag balance, skill size/depth limits.

## Implementation Plan
## Why

agent-lint hand-rolls frontmatter parsing (no YAML dependency — verified in `src/frontmatter.rs`), never checks code-fence balance, and only checks XML in names/descriptions (S013/S018), not bodies. agnix catches whole classes of silent breakage here: YAML that Claude Code fails to parse, unclosed fences that swallow the rest of a file, unbalanced XML tags that degrade prompts, and skill-packaging limits.

Source: agnix rule-gap audit (agnix @ `6733878`, audited 2026-07-16).

## Rules to add (suggested codes X001+ for structure, S-family for skill limits)

| agnix ID | Check | Suggested default |
|---|---|---|
| AS-016 | Skill/agent frontmatter must parse as valid YAML (catches tab indentation, bad nesting, unquoted specials that the current hand-rolled parser misreads). Also delivers CC-AG-007 parity for agents | error |
| AGM-001 (generalized) | Unclosed code fence in any linted markdown file (SKILL.md, agent .md, CLAUDE.md) — agnix scopes this to AGENTS.md; the check is universally useful | error |
| XML-001 | Unclosed XML tag in skill/agent/CLAUDE.md body | warn |
| XML-002 | Mismatched closing XML tag | warn |
| XML-003 | Closing tag with no opening tag | warn |
| AS-015 | Skill directory exceeds 8MB (platform upload limit) | warn |
| AS-013 | Skill file references nested deeper than one level (`refs/deep/nested/file.md`) — spec-recommended layout; distinct from S029's shared-md nesting | suppressed |

## Minor tweaks to existing rules (same PR series)

- S012 (`name-reserved-word`): agnix also reserves the literal name `skill` (exact match). Consider matching agnix's exact-match-plus-contains split so `claude-helper` vs a skill literally named `skill` are treated sensibly.
- S006 (`frontmatter-name-mismatch`): agnix runs this in all modes; agent-lint is Plugin-only. Run in Basic mode too.

## Implementation notes

- AS-016 introduces a YAML dependency (e.g. `serde_yaml` or `yaml-rust2`). Keep the hand-rolled parser for field extraction if desired, but add a strict parse pass for diagnostics. The hook-schema issue (skill/agent frontmatter `hooks:`) benefits from landing this first.
- XML balance must be fence-aware (reuse `src/fence.rs`) and tolerant of pseudo-tags/placeholders (`<name>`, `<1`, comparisons) — warn severity exists precisely because of this FP class. Skip inline code spans as well.

## Acceptance criteria

- [ ] YAML parse errors reported with file + line; existing valid fixtures still pass
- [ ] Fence/XML rules have FP-focused tests (fenced examples, inline code, placeholder tags, HTML in prose)
- [ ] Rules registered in `src/rules.rs` + `docs/rules.md`; `make cargo-test`, `make clippy`, `make fmt` pass


---

## Inherited from #109 (hook schema validation engine)

#109 implemented the hook-object schema engine for the **JSON surfaces only**: `hooks/hooks.json`, `.claude/settings.json`, and `.claude/settings.local.json`. The two **frontmatter** surfaces were deferred to this issue, because they need exactly the strict YAML access this issue adds. #109's acceptance criterion 1 ("all four surfaces") is therefore knowingly unmet, by operator decision.

This issue now also owns:

| agnix ID | Check | Suggested default |
|---|---|---|
| CC-SK-010 | Skill frontmatter `hooks:` must follow the hook schema (apply #109's engine to skill frontmatter) | error |
| CC-AG-011 | Agent frontmatter `hooks:` must follow the hook schema (apply #109's engine to agent frontmatter) | error |

Notes:

- #109 landed a **reusable hook-object validator**, plus its verified event list and handler-type table in one module. CC-SK-010 and CC-AG-011 should call that validator against the parsed `hooks:` value. Do not reimplement or duplicate the lists.
- The blocker is concrete: `extract_raw_value` in `src/frontmatter.rs` does `starts_with("{key}:")` and returns that single line's remainder, so a nested `hooks:` block is unreachable today. `get_field(fm, "hooks")` returns `None`; only `field_exists` is true. That is exactly what AS-016's strict parse pass unblocks.
- **Before implementing CC-AG-011**: the current Claude Code docs state that plugin subagents *ignore* the `hooks:` frontmatter field. Confirm the rule is worth applying to plugin agents before shipping it at `error`.
- See #123 for the rules #109 dropped or narrowed after verifying the proposed table against the current hooks docs. Several agnix hook rules turned out to rest on false premises. Treat the agnix rule set as unverified input here too.

Additional acceptance criteria:

- [ ] CC-SK-010 and CC-AG-011 implemented by calling #109's hook-object validator against strictly-parsed frontmatter
- [ ] Tests per surface (skill frontmatter, agent frontmatter)

## Test plan
(no test plan section in plan-file)
