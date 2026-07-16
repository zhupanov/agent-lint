## Goal
Implement issue #113: [IMPLEMENTING] M012+: plugin.json manifest extras (component paths, lspServers, homepage, author, userConfig keys, channels).

## Implementation Plan
## Why

agent-lint's M-rules cover plugin.json/marketplace.json presence, JSON validity, required fields, semver, and enriched metadata — and its marketplace + userConfig depth already exceeds agnix. But agnix validates several manifest aspects agent-lint misses: component-path safety, plugin layout constraints, and newer manifest sections (`lspServers`, `channels`).

Source: agnix rule-gap audit (agnix @ `6733878`, audited 2026-07-16).

## Rules to add (suggested codes M012+)

| agnix ID | Check | Suggested default |
|---|---|---|
| CC-PL-002 + CC-PL-008 | Components (skills/agents/hooks/commands) must not live inside `.claude-plugin/` — check both the physical layout and manifest path fields pointing there | error |
| CC-PL-007 | Manifest component paths (`commands`, `agents`, `skills`, `hooks`) must be relative — no absolute paths (`/`, `C:\`), no `..` traversal | error |
| CC-PL-005 | `name` present but empty/whitespace-only — first check whether M003 already fires on `""`; if not, fix M003 rather than adding a rule | fold into M003 (verify) |
| CC-PL-009 | If `author` object present, `author.name` must be a non-empty string (complements M011's `author.email` check) | warn |
| CC-PL-010 | `homepage`, if present, must be a valid http(s) URL | warn |
| CC-PL-011 | `lspServers` entries require `command` and `extensionToLanguage` | error |
| CC-PL-012 | `userConfig` keys must be valid identifiers (completes U001–U006, which validate entry shape but not key format) | warn |
| CC-PL-013 | `channels` entries must reference a `server` | warn |
| CC-PL-014 | Plugin agents must not use `hooks`, `mcpServers`, or `permissionMode` frontmatter (unsupported in plugin context) — plugin mode only; interacts with the agent-frontmatter issue | warn |

## Implementation notes

- Extends `src/validators/manifest.rs` (and `agents.rs` for CC-PL-014).
- CC-PL-014's unsupported-field list should be verified against current plugin docs; it changes as plugin support matures.

## Acceptance criteria

- [ ] Rules registered in `src/rules.rs` + `docs/rules.md`; unit tests per rule (including traversal edge cases like `foo/../../etc`)
- [ ] M003 empty-string behavior tested and documented either way
- [ ] `make cargo-test`, `make clippy`, `make fmt` pass

## Test plan
(no test plan section in plan-file)
