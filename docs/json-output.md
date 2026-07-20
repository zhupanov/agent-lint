# JSON Diagnostic Output

Use `--format json` to emit the final lint result as one versioned JSON
document on stdout:

```bash
agent-lint --format json .
```

The public v1 contract is defined by
[`schemas/diagnostic-output-v1.schema.json`](../schemas/diagnostic-output-v1.schema.json).
The payload includes both a stable schema identifier and `schema_version: 1`.
New optional fields may be added compatibly, but existing v1 fields keep their
documented meaning.

JSON output contains no ANSI escapes or human summary. Normal linting writes
nothing to stderr in JSON mode. `--autofix` may write fix progress to stderr;
stdout still contains only the final post-fix validation document. Exit codes
remain 0 for no lint errors, 1 for lint errors, and 2 for invocation, setup, or
configuration errors.

`--format` conflicts with `--list-scripts` and `--closure-report`. Those
commands retain their existing line-oriented and JSON output contracts.

## Top-Level Fields

- `analysis_root` is always `.`. Paths are repository-relative, and the
  resolved absolute root is not disclosed.
- `mode` is `basic`, `plugin`, or `null` when no lint mode was selected.
- `strictness` is `normal`, `pedantic`, or `all`.
- `selected_rules` is `null` for a full run. A focused `--only` run lists its
  canonical rules in deterministic registry order, making incomplete scans
  explicit to machine consumers.
- `active_platforms` is deterministically ordered as Claude, Cursor, and
  Codex. Claude is active whenever Basic or Plugin mode selects the central
  Claude validation pipeline; Cursor and Codex appear when their platform
  validators are active. Shared surfaces do not create additional platform
  names.
- `status` is `clean`, `warnings`, `errors`, or `usage-error`.
- `counts.errors` and `counts.warnings` count emitted rule diagnostics;
  `counts.suppressed` counts policy-suppressed diagnostics, and
  `counts.notices` counts non-rule notices.
- `diagnostics` preserves validator emission order. Each item contains its
  canonical rule code and name, resolved severity, message, and optional
  structured path, location, evidence, and suggestion.
- `notices` contains non-rule conditions such as unused configuration
  overrides, repository-root fallback warnings, and setup/configuration
  errors. Notices do not pretend to be lint rules.

Locations use one-based lines and Unicode-scalar columns. Range starts are
inclusive and ends are exclusive. Unknown paths, locations, columns,
evidence, and suggestions are omitted; renderers never recover them from the
human message.

## Examples

### Clean Run

```json
{
  "$schema": "https://raw.githubusercontent.com/zhupanov/agent-lint/main/schemas/diagnostic-output-v1.schema.json",
  "schema_version": 1,
  "agent_lint_version": "installed-version",
  "analysis_root": ".",
  "mode": "basic",
  "strictness": "normal",
  "selected_rules": null,
  "active_platforms": ["claude"],
  "status": "clean",
  "counts": { "errors": 0, "warnings": 0, "suppressed": 0, "notices": 0 },
  "diagnostics": [],
  "notices": []
}
```

### Mixed Warning and Error Run

This excerpt shows a path-only error and a warning with a structured line. A
path-only finding has no fabricated line number.

```json
{
  "status": "errors",
  "counts": { "errors": 1, "warnings": 1, "suppressed": 0, "notices": 0 },
  "diagnostics": [
    {
      "code": "M001",
      "name": "plugin-json-missing",
      "severity": "error",
      "subject_path": ".claude-plugin/plugin.json",
      "message": ".claude-plugin/plugin.json is missing"
    },
    {
      "code": "X002",
      "name": "unclosed-code-fence",
      "severity": "warning",
      "subject_path": ".claude/skills/example/SKILL.md",
      "location": { "start": { "line": 5 } },
      "message": ".claude/skills/example/SKILL.md:5: unclosed code fence"
    }
  ]
}
```

### Repository-Wide Finding

A finding that belongs to several prompt sources has neither `subject_path`
nor `location`:

```json
{
  "code": "S062",
  "name": "skill-closure-large",
  "severity": "warning",
  "message": "prompt-source group 'agents': roots root lines is 12 (configured maximum 10)"
}
```

### Configuration Failure

When JSON was successfully selected, configuration failures still produce one
document and exit 2. The following excerpt shows its classification:

```json
{
  "mode": null,
  "status": "usage-error",
  "counts": { "errors": 0, "warnings": 0, "suppressed": 0, "notices": 1 },
  "diagnostics": [],
  "notices": [
    {
      "kind": "configuration",
      "severity": "error",
      "message": "agent-lint.toml: invalid TOML"
    }
  ]
}
```

### Autofix

```bash
agent-lint --autofix --format json .
```

Fix progress such as the following may appear on stderr:

```text
fixed[S031/non-https-url]: .claude/skills/example/SKILL.md: replaced http:// with https://
```

The JSON document on stdout describes only the final validation. If the fix
removed the last diagnostic, its status is `clean` and `diagnostics` is empty.
