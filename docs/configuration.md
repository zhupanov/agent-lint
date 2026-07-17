# Configuration

Agent Lint reads an optional **`agent-lint.toml`** file from the
repository root.

## File Format

```toml
[lint]
suppress = ["M001"]                        # suppress entirely (by code)
error    = ["S033", "G005"]                # promote to error (by code or name)
warn     = ["plugin-json-invalid"]         # downgrade to warning (by name)
exclude  = ["docs/*.md", "skills/internal-*/**"]  # skip files matching globs
desc-truncated-max-chars = 200              # tighten S015 (default: 250)
skill-closure-max-lines = 700               # enable S062 budget
claude-import-max-lines = 120               # enable D004 per-import budget
claude-import-total-max-lines = 400          # enable D004 total budget
instruction-files = ["AGENTS.md", "SECURITY.md", "CLAUDE.md"]
inline-path-prefixes = ["src/", "docs/", "skills/", "scripts/"]
script-inventory = "scripts/portable-scripts.txt" # optional G009-G011 scope

[platforms]
cursor = true   # force-enable Cursor checks; false disables them
codex = false   # disable Codex checks even when Codex files exist
```

## Options

| Key | Type | Description |
|-----|------|-------------|
| `suppress` | string array | Rules to suppress completely (no output, no exit code effect) |
| `error` | string array | Rules to promote to error (overrides default severity) |
| `warn` | string array | Rules to downgrade to warning (printed, but exit 0) |
| `exclude` | string array | File glob patterns -- matching files are skipped entirely |
| `desc-truncated-max-chars` | positive integer | S015 listing threshold; defaults to 250 |
| `skill-closure-max-lines` | positive integer | Enables S062 with a transitive Markdown prompt-source line budget |
| `claude-import-max-lines` | positive integer | Enables D004 with a per-import line budget |
| `claude-import-total-max-lines` | positive integer | Enables D004 with a total recursive `@`-import closure budget |
| `instruction-files` | string array | Repository-relative Markdown files scanned by D005 |
| `inline-path-prefixes` | string array | Repository-relative prefixes, each ending in `/`, recognized by D005 |
| `script-inventory` | string | Repository-relative newline-delimited inventory used by G009-G011 |

Closure limits are disabled when omitted. The two D004 limits may be used
independently. Import and Markdown-reference traversal is recursive, bounded,
and counts each file once.

When `script-inventory` is set, blank lines and full-line `#` comments are
ignored and every other line must name an existing regular `.sh`, `.inc.bash`,
or `.awk` file beneath the repository root. Entries are sorted and deduplicated;
they do not need to be tracked by Git. The inventory becomes the authoritative
scope for G009-G011, so those rules scan every listed file on every invocation,
including pre-commit runs. Invalid, unreadable, escaping, symlinked, missing, or
unsupported entries make configuration loading fail with exit code 2.

## Platform Activation

Cursor and Codex surfaces are detected automatically. Shared `AGENTS.md` and
`.agents/skills/` surfaces are observed independently and do not imply either
platform. Platform-specific validators run in both Basic and Plugin modes when
their platform has a recognized unique surface. Basic/Plugin mode therefore
controls the existing Claude rule set; it does not disable detected platform
or shared-surface checks.

Use the optional `[platforms]` section to override detection per platform.
Each value is a boolean: `true` force-enables that platform (including in a
repository with no platform files), and `false` disables its platform-specific
validators. Omit a key to use auto-detection. Only `cursor` and `codex` are
accepted.

Cursor surfaces are `.cursorrules`, `.cursor/rules/**/*.{md,mdc}`,
`.cursor/hooks.json`, `.cursor/agents/**/*.md`, `.cursor/environment.json`,
and `.cursor/skills/*/SKILL.md`. Unique Codex surfaces are
`.codex/config.toml`, `.codex-plugin/plugin.json`, and root
`AGENTS.override.md`. Root or nested `AGENTS.md` and
`.agents/skills/*/SKILL.md` are shared surfaces. Discovery skips `.git` and
conventional dependency/build directories (`node_modules`, `vendor`, `target`,
`dist`, and `build`), and respects `[lint].exclude`.

## Rule Identifiers

Rules can be referenced by **code** (e.g., `M001`) or **human-readable
name** (e.g., `plugin-json-missing`). Priority when a rule appears in
multiple lists: `suppress` > `error` > `warn`.

## File Exclusion

The `exclude` option accepts a list of glob patterns. Files matching any
pattern are invisible to file-walking validators. The sole explicit-scope
exception is G009-G011: a path named by `script-inventory` is still scanned by
those rules. This lets repositories retain broad exclusions needed by unrelated
rules without silently weakening their portability inventory.

**Glob semantics** (matching `.gitignore` conventions):

- `*` matches any characters except `/` (single directory level)
- `**` matches across directory boundaries (recursive)
- `docs/*.md` matches `docs/readme.md` but **not** `docs/sub/nested.md`
- `docs/**/*.md` matches both `docs/readme.md` and `docs/sub/nested.md`

**Scope**: File exclusion applies to file-walking validators (skills, agents,
scripts, docs), except for explicit G009-G011 inventory entries as described
above. It does **not** apply to fixed-path structural checks (e.g.,
`plugin.json` must exist, `SECURITY.md` must exist). Use `suppress` to suppress
those rules instead.

## Default Severity

Each rule has a compiled-in default severity (**error**, **warn**, or
**suppressed**). Use `error = [...]` in `agent-lint.toml` to promote
rules to errors, or `suppress = [...]` to suppress them. See
[rules.md](rules.md) for the default severity of each rule.

## Strictness Modes

Two CLI flags override the default severity model. They are mutually
exclusive (using both exits with code 2).

**`--pedantic`**: Promotes all warnings (both `warn`-listed and
default-warning rules) to errors, except too-long rules (`name-too-long`,
`desc-too-long`, `body-too-long`, `compat-too-long`). Rules in `suppress`
stay suppressed.

**`--all`**: Forces every rule to fire as an error. The `suppress` and `warn`
lists are bypassed entirely -- all 286 rules are promoted to errors. File
exclusions (`exclude`) remain in effect. Note: `--all` applies to rules
emittable by the detected lint mode. In Basic mode (Claude, Cursor, Codex, or standalone
MCP configuration),
plugin-only rules are not dispatched regardless of `--all`.

## Behavior Without Config

If `agent-lint.toml` is absent, all rules fire at their compiled-in
default severity. See [rules.md](rules.md) for each rule's default. A
malformed config file, unknown rule code/name, or invalid glob pattern
causes exit code 2.

## Diagnostic Output

```text
error[M001/plugin-json-missing]: .claude-plugin/plugin.json is missing
warning[M002/plugin-json-invalid]: plugin.json is not valid JSON
```
