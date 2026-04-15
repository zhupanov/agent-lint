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
```

## Options

| Key | Type | Description |
|-----|------|-------------|
| `suppress` | string array | Rules to suppress completely (no output, no exit code effect) |
| `error` | string array | Rules to promote to error (overrides default severity) |
| `warn` | string array | Rules to downgrade to warning (printed, but exit 0) |
| `exclude` | string array | File glob patterns -- matching files are skipped entirely |

## Rule Identifiers

Rules can be referenced by **code** (e.g., `M001`) or **human-readable
name** (e.g., `plugin-json-missing`). Priority when a rule appears in
multiple lists: `suppress` > `error` > `warn`.

## File Exclusion

The `exclude` option accepts a list of glob patterns. Files matching any
pattern are completely invisible to the linter -- no rules are checked
and no diagnostics are produced for them.

**Glob semantics** (matching `.gitignore` conventions):

- `*` matches any characters except `/` (single directory level)
- `**` matches across directory boundaries (recursive)
- `docs/*.md` matches `docs/readme.md` but **not** `docs/sub/nested.md`
- `docs/**/*.md` matches both `docs/readme.md` and `docs/sub/nested.md`

**Scope**: File exclusion applies to file-walking validators (skills,
agents, scripts, docs). It does **not** apply to fixed-path structural
checks (e.g., `plugin.json` must exist, `SECURITY.md` must exist). Use
`suppress` to suppress those rules instead.

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
`desc-too-long`, `compat-too-long`). Rules in `suppress` stay suppressed.
Suppressed rules stay suppressed unless explicitly enabled. Too-long
rules keep their current severity.

**`--all`**: Forces every rule to fire as an error. The `suppress` and `warn`
lists are bypassed entirely -- all 104 rules are promoted to errors. File
exclusions (`exclude`) remain in effect. Note: `--all` applies to rules
emittable by the detected lint mode. In Basic mode (`.claude/` only),
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
