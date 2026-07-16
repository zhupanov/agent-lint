# Architectural Invariants

These are absolute properties of Agent Lint. Unlike the aspirational guidance
in [Architectural Guidelines](ARCHITECTURAL_GUIDELINES.md), an invariant has no
exception clause. A violation is a defect. Where possible, the invariant must
also be enforced by the compiler, a test, or a lint; this file is the readable
specification, not a substitute for those checks.

## Rule identity and diagnostics

### I-Rule-1: Every diagnostic has one canonical rule identity

Every emitted lint diagnostic is associated with exactly one `LintRule`.
Every `LintRule` has a unique code and unique name, a compiled default
severity, and an entry in `ALL_RULES`. Code and name lookup round-trips to the
same variant. Validators never invent ad hoc diagnostic identifiers.

Mechanical backing: exhaustive mappings and the registry tests in `rules.rs`.

### I-Diag-1: Validators report; the collector decides disposition

Domain validators report a `LintRule` and message through
`DiagnosticCollector`. They do not print lint diagnostics, apply severity
overrides, or terminate the process. Suppression, promotion, downgrade,
rendering, and diagnostic accounting are centralized in
`DiagnosticCollector::report` after `LintConfig` has resolved CLI policy.

Mechanical backing: the module boundary and tests in `diagnostic.rs`; direct
output or process termination under `validators/` is a violation.

### I-Severity-1: Severity precedence is deterministic

For normal linting, an explicit suppression wins over an explicit error,
which wins over an explicit warning, which wins over the compiled default.
Pedantic mode preserves explicit suppressions and promotes eligible warnings.
All mode enables every registered rule as an error while leaving file
selection policy unchanged.

Mechanical backing: `LintConfig::apply_cli_mode`,
`DiagnosticCollector::report`, and their unit tests.

## Repository and configuration boundaries

### I-Root-1: A run has one resolved analysis root

Before configuration, discovery, validation, or autofix begins, the CLI
resolves the Git top level when available, otherwise the canonical target
directory, and makes it the working root. All repository-relative surfaces,
configuration, exclusions, diagnostics, and mutations in that run refer to
that same root. If the root cannot be resolved or entered, validation does not
proceed.

Mechanical backing: `resolve_repo_root` and the startup sequence in `main.rs`.

### I-Config-1: Invalid explicit configuration never degrades to defaults

A missing `agent-lint.toml` selects defaults. An existing but unreadable or
malformed configuration, an unknown field or rule identifier, an invalid glob,
an unsafe configured path, or an invalid threshold stops the run as a usage
error. It is never treated as if configuration were absent.

Mechanical backing: deny-unknown deserialization and validation in
`LintConfig::load`, plus its unit tests.

### I-Platform-1: Detection and activation are separate decisions

`PlatformDetection` records observed repository surfaces. `PlatformOverrides`
may explicitly enable or disable a supported platform. After configuration is
loaded, the `ActivePlatforms` value used for validation is resolved from both
inputs and passed unchanged to central validator dispatch. A validator does not
independently override its platform's activation.

Mechanical backing: `PlatformDetection::discover`,
`PlatformDetection::activate`, and `run_all_with_platforms`.

## Validation pipeline

### I-Dispatch-1: Mode and platform scope are owned by central dispatch

Production validators are entered through `run_all_with_platforms`. Basic and
plugin scope is selected from the run's `LintContext`; Cursor and Codex scope
is selected from `ActivePlatforms`. Domain validators do not call peer domains
to bypass those decisions.

Mechanical backing: the dispatch integration tests in `validators/mod.rs`.

### I-Parse-1: A loaded JSON manifest retains its parse state

For JSON surfaces owned by `LintContext`, the states missing, unreadable or
invalid, and parsed are distinct. Once loaded, sibling validators consume that
same `ManifestState`; an invalid existing file is never silently converted to
`Missing` or an empty JSON value.

Mechanical backing: `ManifestState`, `LintContext::new`, and their unit tests.

### I-Exclude-1: Strictness never changes file selection

Normal, pedantic, and all mode may change whether a registered rule is
suppressed, a warning, or an error. They do not remove or broaden configured
exclude patterns. Paths presented to `ExcludeSet` are normalized before glob
matching.

Mechanical backing: `LintConfig::apply_cli_mode`, `ExcludeSet::is_excluded`,
and the configuration tests.

## Mutation and process behavior

### I-Fix-1: Autofix is gated by rule metadata and ends in validation

The autofix loop may dispatch a mutation only for a diagnostic whose
`LintRule::is_autofixable` value is true. It stops when no fixable diagnostic
remains, no handler makes progress, or the iteration bound is reached. After
mutation attempts, the normal lint pipeline always runs again and its result
determines the process outcome.

Mechanical backing: `run_autofix`, `autofix::apply_fix`, and autofix tests.

### I-Exit-1: Exit status reflects the final classified outcome

Invalid invocation, unresolved paths, and invalid explicit configuration exit
with the usage-error status. A completed validation exits nonzero exactly when
at least one final diagnostic is an error. Warnings and suppressed diagnostics
alone do not fail the run. Autofix uses the post-fix validation result, never
the pre-fix diagnostic count.

Mechanical backing: the control flow in `main.rs` and diagnostic count tests.

## Test integrity

### I-Test-1: Tests restore process-global state

A test that changes the current working directory or another process-global
setting restores it even if the test panics. Tests that mutate the working
directory are serialized so parallel execution cannot redirect another test's
filesystem operations.

Mechanical backing: `CwdGuard`, `serial_test`, and repository-wide test review.
