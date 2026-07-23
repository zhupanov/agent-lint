# Architectural Invariants

These are absolute properties of Agent Lint. Unlike the aspirational guidance
in [Architectural Guidelines](ARCHITECTURAL_GUIDELINES.md), an invariant has no
exception clause. A violation is a defect. Where possible, the invariant must
also be enforced by the compiler, a test, or a lint; this file is the readable
specification, not a substitute for those checks.

## Rule identity and diagnostics

### I-Rule-1: Every diagnostic has one canonical rule identity

Every emitted lint diagnostic is associated with exactly one `LintRule`.
Every `LintRule` is current, has a reachable production validator, and has a
unique code and unique name, a compiled default severity, and an entry in the
sole live registry, `ALL_RULES`. Code and name lookup round-trips to the same
variant, and no retired or renamed identifier resolves. Validators never
invent ad hoc diagnostic identifiers.

Mechanical backing: exhaustive mappings and registry, production-owner,
positive-contract, canonical-lookup, and documentation tests.

### I-Rule-2: Rule lifecycle is explicit

Every `LintRule` is a current, reachable rule in the single live registry. Runtime lookup accepts exactly its canonical code and canonical name. When a rule is renamed or removed, obsolete identifiers do not remain as runtime aliases or compatibility-only variants; their history remains in `CHANGELOG.md`, and their codes and names are never reused for different semantics. Current documentation, configuration examples, dispatch, severity policy, and autofix metadata contain no retired identity.

Mechanical backing: the live-registry and positive-conformance coverage checks, exact lookup-cardinality tests, current-documentation identity checks, and the rule-accounting work in #589.

### I-Diag-1: Validators report; the collector decides disposition

Domain validators report a `LintRule` and message through
`DiagnosticCollector`. They do not print lint diagnostics, apply severity
overrides, or terminate the process. Suppression, promotion, downgrade,
rendering, and diagnostic accounting are centralized in
`DiagnosticCollector::report` after `LintConfig` has resolved CLI policy.

Mechanical backing: the module boundary and tests in `diagnostic.rs`; direct
output or process termination under `validators/` is a violation.

### I-Diag-2: File-scoped policy uses a structured subject path

Every file-attributable diagnostic carries an explicit repository-relative
subject path supplied through `DiagnosticCollector::report_at` or an explicit
subject scope. Severity resolution and override matching use that value before
rendering; they never infer a filename from message text. Fixed-path checks use
their logical target even when it is missing. Genuinely repository-wide or
multi-source findings remain pathless and cannot match file overrides.

Mechanical backing: the `Diagnostic::subject_path` type, collector/config
matching tests, validator dispatch tests, and per-file CLI regressions.

### I-Diag-3: Diagnostics never embed sensitive values or machine-local paths

Rendered diagnostics - message, evidence, and suggestion, in text and JSON -
never contain values the rule itself classifies as possibly sensitive (secret
literals, credential-bearing field values, server names in secret-scanning
contexts), never contain raw control characters, and never contain absolute
machine-local filesystem paths. Subjects and evidence use repository-relative
paths and neutralized text. When a finding is about a sensitive value, the
diagnostic identifies the key or location, not the value.

Mechanical backing: token-shaped and control-character regression fixtures;
conformance manifests assert redacted evidence for secret-scanning rules.

### I-Diag-4: Untrusted diagnostic data is bounded and safe

Diagnostics do not disclose secrets, uncontrolled terminal bytes, or canonical paths outside the analysis root. Evidence derived from repository content is bounded and passes through the shared sensitive-value policy before storage. Human-readable rendering escapes control characters at the output choke point. Validators do not interpolate credential-bearing values into messages or suggestions, and rejected outside-repository paths retain only safe authored or repository-relative identity.

Mechanical backing: `DiagnosticMetadata` evidence bounds and redaction, terminal-sanitization tests, repository-path containment types, and cross-surface secret and control-character regression matrices.

### I-Severity-1: Severity precedence is deterministic

For normal linting, global suppression wins over matching per-file
suppression, which wins over an explicit error, which wins over an explicit
warning, which wins over the compiled default. Pedantic mode preserves both
forms of suppression and promotes eligible warnings. All mode ignores both
forms of suppression and enables every registered rule as an error while
leaving file selection policy unchanged.

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

`DetectedSurfaces` records observed repository surfaces. `PlatformOverrides`
may explicitly enable or disable a supported platform. After configuration is
loaded, the `ValidationTargets` value used for validation is resolved from both
inputs and passed unchanged to central validator dispatch. A validator does not
independently override its platform's activation.

Mechanical backing: `DetectedSurfaces::discover`,
`DetectedSurfaces::resolve`, and `run_all_with_targets`.

### I-Discovery-1: One surface has one effective inventory

Each supported surface has one effective candidate inventory for a run. Platform detection and every validator, prompt analyzer, overlap analyzer, and autofix consumer for that surface use the same repository-relative, normalized, sorted, deduplicated, exclusion-aware set. A consumer does not independently rediscover a broader or narrower set. A disabled platform contributes no platform-only candidates, and an excluded candidate cannot activate a platform or reappear in a downstream consumer.

Mechanical backing: typed discovery inventories in `platforms.rs` and domain discovery modules, central dispatch, and surface-matrix tests that compare every consumer and autofix scope.

## Validation pipeline

### I-Dispatch-1: Mode and platform scope are owned by central dispatch

Production validators are entered through `run_all_with_targets`. Basic and
plugin scope is selected from the run's `LintContext`; Cursor and Codex scope
is selected from `ValidationTargets`. Domain validators do not call peer domains
to bypass those decisions.

Mechanical backing: the dispatch integration tests in `validators/mod.rs`.

### I-Selection-1: Focused selection is a projection

A focused rule selection is a projection of the normal lint run. Given the same resolved root, repository content, configuration, platform overrides, exclusions, and strictness, the diagnostics retained for a selected rule are identical in rule identity, severity, subject, related subjects, location, evidence, suggestion, and relative order to that rule's diagnostics in an unfocused run. `--only` does not alter discovery, platform activation, parsing, exclusions, or validator semantics; it only limits diagnostic disposition and autofix to the effective selected rules. Reordering or repeating requested identifiers does not affect validation or output order.

Mechanical backing: `RunPolicy` and `DiagnosticCollector` unit tests, focused-selection conformance axes, and CLI projection tests that compare focused output with the same rule filtered from an unfocused report.

### I-Determinism-1: Repository order cannot affect output

The same repository bytes and resolved policy produce the same ordered diagnostics and process outcome. Filesystem enumeration order, file creation order, hash iteration order, duplicate or reordered `--only` identifiers, and overlapping discovery roots do not change diagnostic identity, multiplicity, ordering, counts, or exit status. Collections crossing an architectural boundary have an explicit stable identity and ordering rule.

Mechanical backing: sorted traversal and registry-order policy, deduplicated typed inventories, and metamorphic CLI tests that build equivalent repositories in different orders and compare complete JSON reports.

### I-Parse-1: A loaded JSON manifest retains its parse state

For JSON surfaces owned by `LintContext`, the states missing, unreadable or
invalid, and parsed are distinct. Once loaded, sibling validators consume that
same `ManifestState`; an invalid existing file is never silently converted to
`Missing` or an empty JSON value.

Mechanical backing: `ManifestState`, `LintContext::new`, and their unit tests.

### I-Syntax-1: Parser adapters preserve semantic token roles

An adapter from a shared parser to a validator preserves every semantic role
and source identity that the validator uses. Executables remain distinct from
arguments and post-terminator operands; code remains distinct from prose;
mapping keys remain distinct from values; authored paths remain distinct from
decoded paths; and source spans remain attached to the token they locate. A
consumer may discard a field only when its contract cannot depend on that
field.

Mechanical backing: typed outputs from `MarkdownDocument`,
`markdown_commands`, `script_paths`, and the structured data parsers, plus
table-driven adapter tests for each consuming rule family.

### I-Validate-1: Invalid subtrees isolate dependent checks only

When a configuration subtree fails structural validation, validators skip
only semantic checks that require that subtree to be usable. They do not emit
advisory child findings for an unusable node, and they continue validating
independent sibling fields, entries, and surfaces. One malformed branch never
causes a whole-file early return when other branches remain interpretable.

Mechanical backing: mixed-validity fixtures for manifest, hook, MCP, Cursor,
Codex, and user-configuration validators assert both non-cascade behavior and
continued sibling diagnostics.

### I-Collection-1: Every element and occurrence is validated

When a validator walks a collection or scans a document, it evaluates every
element and reports every independent violation. Checking never stops at the
first element, the first occurrence, or the first violation unless the rule's
documented contract is first-match-only. Duplicate or repeated values are
each located at their own source position, never at the first textual match.
A filter that removes every declared element leaves the declaration invalid;
it does not fall back to the behavior of an absent declaration.

Mechanical backing: regression fixtures place defects in the second element
or occurrence; span assertions distinguish repeated equal values.

### I-Exclude-1: Strictness never changes file selection

Normal, pedantic, and all mode may change whether a registered rule is
suppressed, a warning, or an error. They do not remove or broaden configured
exclude patterns. Paths presented to `ExcludeSet` are normalized before glob
matching.

Mechanical backing: `LintConfig::apply_cli_mode`, `ExcludeSet::is_excluded`,
and the configuration tests.

## Mutation and process behavior

### I-Output-1: Renderers project one classified result

Text and JSON rendering consume the same post-policy diagnostic collection. Selecting an output format does not rerun validators or alter rule identity, severity, suppression, subjects, locations, evidence, suggestions, counts, active targets, or exit status. JSON mode emits one schema-valid report to stdout without incidental human-readable prose; text rendering escapes terminal control characters without changing the stored structured diagnostic.

Mechanical backing: the single collection-before-rendering pipeline, diagnostic JSON schema validation, and paired CLI tests that compare the semantic result and exit status of text and JSON runs.

### I-Fix-1: Autofix is gated by rule metadata and ends in validation

The autofix loop may dispatch a mutation only for a diagnostic whose
`LintRule::is_autofixable` value is true. It stops when no fixable diagnostic
remains, no handler makes progress, or the iteration bound is reached. After
mutation attempts, the normal lint pipeline always runs again and its result
determines the process outcome. Before each candidate write, the fixer applies
the same rule/subject suppression policy as the collector; a violation in one
file never authorizes mutation of a file where that rule is suppressed.

Mechanical backing: `run_autofix`, `autofix::apply_fix`, and autofix tests.

### I-Fix-2: Autofix metadata and dispatch are equivalent

A rule is marked autofixable if and only if it has exactly one reachable autofix handler. Every mutation is attributable to the triggering rule and an in-scope diagnostic subject, and a handler does not modify an unrelated file. The handler uses the same surface inventory, recognition contract, exclusions, and per-file suppression policy as validation. Autofix metadata, dispatch, documentation, and tests cannot describe different rule sets.

Mechanical backing: one canonical fix descriptor or an exact-set parity test across rule metadata, handler dispatch, and the documented autofix table, plus per-rule idempotency, scope, and suppression tests.

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
