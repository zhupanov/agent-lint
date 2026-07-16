## Goal
Implement issue #106: [IMPLEMENTING] G004/S030: also scan Makefile (and assess python/) so indirectly-invoked scripts aren't false-flagged dead/orphaned.

## Implementation Plan
## Summary

`G004` (dead-script) and `S030` (orphaned-skill-files) decide whether a script is "used" by pattern-matching qualified script-path literals across a **fixed set of surfaces**. That surface set omits the `Makefile` and `python/`, so a script invoked **only** from a Make target or from Python is false-flagged as dead/orphaned. Downstream plugins must then hand-maintain reachability inventories or `exclude` entries purely to suppress those false positives.

This proposes adding the `Makefile` (and assessing `python/`) to the scanned surfaces so real invocations are detected automatically.

## Current behavior (observed on HEAD = 2.3.5)

`validate_dead_scripts` (`src/validators/hygiene/dead_scripts.rs`) collects "references" by walking:

- `skills/`, `.claude/skills/`, `hooks/`, `.github/workflows/`, `scripts/` (the dir list at lines 38-44)
- parsed `settings.json` / `hooks.json`
- `skills/shared/*.md` code fences

and matching qualified path literals (`${CLAUDE_PLUGIN_ROOT}/scripts/foo.sh`, bare `scripts/foo.sh`, `$SCRIPT_DIR/foo.sh`). Any top-level `scripts/*.sh` whose path is not found is reported dead (the final loop, lines 192-215). `S030`'s reference collector (`src/validators/hygiene/scripts.rs:40`) walks `skills/` and `.claude/skills/` similarly for skill-local scripts.

**Not scanned:** the `Makefile`, `python/`, bare-basename prose mentions, and variable-built paths.

## Why it matters

A plugin that has moved invocation into a CLI dispatch (Python) and runs harness/lint scripts from Make targets ends up with many genuinely-live scripts that have no qualified-path literal in any scanned surface.

Motivating downstream (larch): roughly 85 scripts are invoked from the `Makefile` (`bash scripts/foo.sh`; ~89 such references) or via `python3 cli.py ...` dispatch, and are kept "visible" to the linter only by ~140 lines of inline reachability inventory parked in `SKILL.md`. `SKILL.md` is loaded into the agent's context on every run, so this is pure linter-bookkeeping sitting in the prompt surface.

## Proposed change

1. Add the `Makefile` (and any `*.mk`) to the walked surfaces in both `validate_dead_scripts` and the S030 reference collector; extract `bash scripts/foo.sh` and `${CLAUDE_PLUGIN_ROOT}/scripts/foo.sh` literals. Strip `#` comments, mirroring the existing YAML-comment stripping (`strip_yaml_comments`). High value, low risk: Make targets are real invocations.
2. Assess adding `python/`: extract literal `scripts/...sh` / `${CLAUDE_PLUGIN_ROOT}/scripts/...sh` references. Expected yield is lower where logic was ported into Python rather than shelled out, so measure before committing.

## Coverage caveat (explicit non-goal)

Bare-basename prose mentions (for example `design-step3-entry.sh` in running text) and variable-built paths are structurally un-followable by a literal scan and will still need an explicit `exclude`. The goal is to shrink the manual surface to its true minimum, not to zero.

## Benefit

Downstream plugins can delete large always-loaded reachability inventories and most `exclude` entries while keeping real, automatic dead-script detection.

## Test plan
(no test plan section in plan-file)
