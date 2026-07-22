# CLI Reference

```text
agent-lint [--pedantic | --all] [--only RULE[,RULE]...]...
           [--format text|json]
           [--autofix | --list-scripts | --closure-report] [PATH]
```

If `PATH` is omitted, the current directory is used. The tool detects the
repo root via `git rev-parse --show-toplevel` and selects Basic or Plugin
mode automatically based on the configuration it finds. Claude/MCP files,
unique Cursor or Codex surfaces, any `AGENTS.md`, and `.agents/skills/` select
Basic mode; a `.claude-plugin/` directory selects Plugin mode. Shared
`AGENTS.md` and `.agents/skills/` surfaces do not activate a platform by
themselves.

## Flags

| Flag | Description |
|------|-------------|
| `--help`, `-h` | Print help message |
| `--version`, `-V` | Print version information |
| `--list-scripts` | List discovered script paths and exit |
| `--closure-report` | Print configured prompt-source budget measurements as deterministic JSON and exit |
| `--autofix` | Fix auto-fixable violations in-place and report remaining issues |
| `--only RULE[,RULE]...` | Run only named rule codes or canonical names; repeatable |
| `--format text\|json` | Select human-readable text or versioned JSON diagnostic output |
| `--pedantic` | Promote warnings to errors (except too-long rules) |
| `--all` | Force every rule to error, ignoring config overrides |

## Exit Codes

| Code | Meaning |
|------|---------|
| `0` | Success (no errors, or only warnings) |
| `1` | Lint errors found |
| `2` | Invalid arguments or setup error (not a git repo, bad config, etc.) |

## `--only`

Use `--only` for a focused lint run. Values may be comma-delimited, repeated,
or both, and may use the same canonical rule codes and names accepted by
`agent-lint.toml`:

```bash
agent-lint --only Q002,A026 .
agent-lint --only prompt-negative-only --only agent-maxturns-invalid .
```

Duplicate identifiers are normalized. Selected rules run in deterministic
registry order, independent of the order supplied on the command line. An
unknown or empty identifier is an invalid invocation and exits 2.

Focused selection does not skip repository discovery, input parsing, or
configuration validation. Selected rules retain their configured severity,
global suppression, and matching per-file suppression. `--pedantic` promotes
selected eligible warnings only. `--all` enables selected rules as errors and
retains its normal behavior of ignoring suppression, without enabling
unselected rules. Suppressed counts and unused-override warnings consider only
selected rules.

With `--autofix`, only selected rules can trigger mutations, and the final
validation pass uses the same selection.

## `--format`

The default `--format text` behavior is unchanged. `--format json` writes one
versioned JSON document to stdout and preserves the same exit status as text
mode. It conflicts with `--list-scripts` and `--closure-report`, which own
separate stdout contracts.

See [JSON Diagnostic Output](json-output.md) for the schema, field semantics,
setup-error representation, and clean, mixed, pathless, configuration-failure,
and autofix examples.

## `--autofix`

When `--autofix` is provided, agent-lint attempts to automatically fix
violations for rules that have purely mechanical, unambiguous fixes. After
all possible fixes are applied, it runs a final validation pass and reports
any remaining issues with normal exit semantics (exit 1 if errors remain). With
`--format json`, stdout contains only the final validation document; fix
progress may be written to stderr.
Configured per-file suppressions apply to both diagnosis and mutation, so a
fixer never changes a file where that rule is suppressed. Unused-override
warnings are emitted only by the final visible pass.

See [rules.md](rules.md#auto-fixable-rules) for the list of auto-fixable
rules.

## `--list-scripts`

Outputs discovered script paths, one per line. It includes `.sh`, `.bash`,
`.inc.bash`, `.awk`, `.py`, `.js`, `.mjs`, and extensionless files in the
configured script directories. The same matrix applies to an explicit
inventory; choose a downstream tool appropriate for each path.
For shell-only repositories it remains useful for piping to external tools:

```bash
agent-lint --list-scripts . | xargs -r shellcheck
```

The wrapper script `scripts/shellcheck-scripts.sh` automates this.

When `[lint].script-inventory` is configured, this flag prints its validated,
sorted entries. Pre-commit runs the linter without filenames, so G009-G011
deterministically inspect the complete configured inventory rather than only
the files staged in an invocation.

## `--closure-report`

Outputs a JSON array ordered by group, source set, scope, and metric. Each row
contains stable `group`, `source_set`, `scope`, `metric`, `measured_value`, and
nullable `cap` fields. The command measures every configured
`prompt-source-budgets` group without running unrelated validators. Measurement
or configuration failures exit 2.
