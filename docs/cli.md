# CLI Reference

```text
agent-lint [--pedantic | --all] [--autofix] [--list-scripts] [PATH]
```

If `PATH` is omitted, the current directory is used. The tool detects the
repo root via `git rev-parse --show-toplevel` and selects Basic or Plugin
mode automatically based on the presence of `.claude-plugin/`.

## Flags

| Flag | Description |
|------|-------------|
| `--help`, `-h` | Print help message |
| `--version` | Print version information |
| `--list-scripts` | List discovered script paths and exit |
| `--autofix` | Fix auto-fixable violations in-place and report remaining issues |
| `--pedantic` | Promote warnings to errors (except too-long rules) |
| `--all` | Force every rule to error, ignoring config overrides |

## Exit Codes

| Code | Meaning |
|------|---------|
| `0` | Success (no errors, or only warnings) |
| `1` | Lint errors found |
| `2` | Invalid arguments or setup error (not a git repo, bad config, etc.) |

## `--autofix`

When `--autofix` is provided, agent-lint attempts to automatically fix
violations for rules that have purely mechanical, unambiguous fixes. After
all possible fixes are applied, it runs a final validation pass and reports
any remaining issues with normal exit semantics (exit 1 if errors remain).

See [rules.md](rules.md#auto-fixable-rules) for the list of auto-fixable
rules.

## `--list-scripts`

Outputs discovered shell scripts, one per line. Useful for piping to
external tools:

```bash
agent-lint --list-scripts . | xargs -r shellcheck
```

The wrapper script `scripts/shellcheck-scripts.sh` automates this.
