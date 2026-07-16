### [Code Review] Self-review: H021 does not implement the docs-supported "`if` only on tool events" half

- **File**: `src/validators/hook_schema.rs` (H021)
- **Disposition**: rejected — borderline; deliberate approved narrowing, no false positives.
- **Detail**: H021 checks only that `if` is a non-empty string. The hooks
  reference does support the other half of the audited rule: "Only evaluated on
  tool events: `PreToolUse`, `PostToolUse`, `PostToolUseFailure`,
  `PermissionRequest`, and `PermissionDenied`. On other events, a hook with `if`
  set never runs." So `if` on, say, `Stop` is a silently dead filter that H021
  will not warn about.
- **Why rejected**: this is one of the narrowings the operator approved for this
  run, it is `warn`-severity, and the gap is a false *negative* only — no valid
  config is blocked. Unlike H019/H013 (which were fixed because they produce
  `error`-severity false positives against configs the docs explicitly bless),
  nothing here misfires. Widening the rule would be unrequested scope.

### [Code Review] Self-review: H024 does not check that the interpolated var is actually listed in allowedEnvVars

- **File**: `src/validators/hook_schema.rs` (`check_http_headers`)
- **Disposition**: rejected — low priority; rule as specified is satisfied.
- **Detail**: H024 returns clean as soon as `allowedEnvVars` is any non-empty
  array, without checking membership. Per the reference, "References to unlisted
  variables are replaced with empty strings", so
  `{"headers": {"A": "$FOO"}, "allowedEnvVars": ["BAR"]}` still silently
  resolves `$FOO` to empty and H024 stays quiet.
- **Why rejected**: the audited rule (CC-HK-024) is "HTTP hook headers with
  `$VAR` interpolation need `allowedEnvVars`", which the current check
  implements. Per-variable membership checking is a `warn`-level false negative
  and an extension beyond the requested rule.

### [Code Review] Self-review: H023 misses `rm -r -f` written as separate flags

- **File**: `src/validators/hook_schema.rs` (`DANGEROUS_COMMAND_PATTERNS`)
- **Disposition**: rejected — low priority; heuristic warn rule.
- **Detail**: the recursive+force regex requires the flags in one cluster
  (`-rf`, `-fr`, `-Rf`, `-vrf` all match), so `rm -r -f /x` is not flagged.
- **Why rejected**: H023 is an intentionally heuristic `warn`-level pattern
  matcher, not an exhaustive shell parser; exhaustive coverage is explicitly not
  its goal, and the miss blocks nobody. The benign-command regression test
  confirms the patterns do not overreach, which is the higher-risk direction.

### [Code Review] Self-review: implementation commit message rule-count narrative is stale post-rebase

- **File**: commit `2414bf0` message body ("Adds 18 rules (104 -> 122)")
- **Disposition**: rejected — not fixable within this subagent's contract.
- **Detail**: the rebase that landed #125 (13 unrelated rules) underneath this
  work moved the baseline, so the registry goes 117 -> 135, not 104 -> 122. The
  "18 rules added" figure is still correct; only the before/after pair is stale.
  Every in-tree count claim (README.md, docs/rules.md, docs/configuration.md,
  docs/development.md, src/rules.rs, src/config.rs) was verified to agree at 135
  (89 error / 41 warn / 5 suppressed), and the per-prefix README table matches
  the registry exactly, so this is confined to the commit message prose.
- **Why rejected**: correcting it requires amending the commit, which the
  self-reviewer contract forbids (never `git commit`). Surfaced to the
  orchestrator, which owns the commit route, in case it wants to reword during
  the checks-commit composite or squash.
