# Architectural Guidelines

These guidelines describe the preferred shape of Agent Lint. They are
aspirational review criteria, not absolute rules. Meaningful deviations should
be explained in the change that introduces them. Recurring, deterministic
requirements should graduate to tests or lints instead of relying on this
document.

The absolute rules are in [Architectural Invariants](ARCHITECTURAL_INVARIANTS.md).

## System shape

The normal lint path has one direction of travel:

```text
CLI and repository root (main.rs)
  -> configuration and platform activation (config.rs, platforms.rs)
  -> parsed shared inputs (context.rs)
  -> mode and platform dispatch (validators/mod.rs)
  -> domain validators (validators/**)
  -> rule policy and output (rules.rs, diagnostic.rs)
```

`autofix.rs` is a controlled side path. It consumes diagnostics from that same
pipeline, mutates only supported rule violations, and then enters the normal
validation path again.

## Change discipline

### G-Fix-1: Fix the class, not only the observed instance

- Why: validators commonly have sibling surfaces for public plugins, private
  Claude configuration, Cursor, Codex, and MCP. A local fix can leave the same
  defect in another surface.
- Guidance: search for sibling parsers, validators, dispatch sites, and autofix
  handlers when fixing a defect. Fix every equivalent site or record why a
  sibling has intentionally different semantics.
- Deviate when: the sibling consumes a different external contract or cannot
  reach the failing state; identify that boundary in the review notes.

### G-Test-1: A behavior change ships with an executable regression

- Why: examples in prose do not protect rule behavior, severity resolution,
  mode dispatch, or autofix convergence.
- Guidance: reproduce the old failure in a unit or integration test, then
  verify the canonical diagnostic identity and, where relevant, severity,
  subject, message, or file content. Message text or counts alone do not prove
  that the intended rule emitted.
  Prefer a small temporary repository over mocks of filesystem behavior.
- Deviate when: the change is documentation-only or the behavior depends on an
  external service that cannot be represented locally; describe the manual
  verification.

### G-Review-1: Review public behavior across every CLI strictness mode

- Why: normal, pedantic, and all mode intentionally transform the same rule
  registry differently.
- Guidance: when changing default severity, suppression, or promotion logic,
  check all three modes and preserve the documented precedence in
  `LintConfig::apply_cli_mode` and `DiagnosticCollector::report`.
- Deviate when: the change cannot affect rule disposition, such as an internal
  parser refactor with byte-for-byte equivalent diagnostics.

### G-Close-1: Verify binding comments before closing a work issue

- Why: acceptance criteria added as issue comments were repeatedly not
  implemented by the PR authored from the body: #508, #537, #525, #509,
  #521, #546. One binding extension was orphaned entirely when its host
  issue closed (#387).
- Guidance: before closing a work issue, re-read its comments. For each
  binding addendum, either verify the implementation covers it or merge it
  into a follow-up issue body. Combined issues should fold binding comments
  into the body before implementation starts.
- Deviate when: a comment is explicitly informational or superseded; note
  that in the closing comment.

## Ownership and layering

### G-Own-1: Keep one canonical owner for each concept

- Why: duplicated policy drifts and makes a rule behave differently depending
  on its call path.
- Guidance: preserve the current ownership boundaries:
  `rules.rs` owns rule identity and compiled defaults; `config.rs` owns user
  policy and exclusions; `platforms.rs` owns discovery and activation;
  `validators/mod.rs` owns dispatch; validator modules own domain facts;
  `diagnostic.rs` owns disposition and rendering; `autofix.rs` owns mutation.
- Deviate when: an external format needs an isolated adapter; keep policy in
  the canonical owner and make the adapter translate only.

### G-Layer-1: Keep validators leaf-like

- Why: a validator is easiest to test when it reads an explicit input, reports
  facts, and does not control the process.
- Guidance: validators should not parse CLI arguments, select lint modes,
  decide exit codes, print diagnostics directly, or apply fixes. Share lexical
  and traversal behavior through focused helpers such as `frontmatter.rs`,
  `fence.rs`, `validators/common.rs`, and `traversal.rs`.
- Deviate when: a validator needs a small domain-specific parser that has no
  second consumer; keep it private to that validator.

### G-Dispatch-1: Add surfaces through the central dispatch

- Why: calling a validator opportunistically from another validator hides its
  mode and platform requirements.
- Guidance: register new validation at the appropriate basic, plugin, or
  platform branch in `validators/mod.rs`. Keep surface detection in
  `platforms.rs` and configuration overrides in `config.rs`.
- Deviate when: the check is an inseparable sub-check of an already-dispatched
  domain validator and shares exactly the same inputs and scope.

### G-Dep-1: Point dependencies toward shared primitives, not across domains

- Why: imports between peer validator domains couple unrelated external
  formats and invite cycles.
- Guidance: move genuinely shared parsing, constants, or traversal into a
  narrowly named common module. Keep platform-specific constants with their
  platform unless another domain actually consumes the same contract.
- Deviate when: one domain explicitly embeds another domain's public format;
  document that contract at the import.

## Rule evolution

### G-Rule-1: Treat the full rule lifecycle as a registry-wide change

- Why: a `LintRule` participates in lookup, configuration, default severity,
  strictness modes, validator ownership, positive and negative contracts,
  documentation, and possibly autofix.
- Guidance: add, change, or remove the enum variant, canonical code/name,
  compiled default, `ALL_RULES`, reachable validator, explicit positive
  identity test, negative boundary test, documentation, and autofix decision
  together. An autofix mapping exists only when the transformation is
  deterministic and safe.
- Deviate when: an implementation refactor leaves the observable rule contract
  unchanged; preserve the existing lifecycle evidence.

### G-Compat-1: Retire rule identities completely and never reuse them

- Why: compatibility-only identities create registry entries with no reachable
  validator and make public accounting untrustworthy.
- Guidance: removed and renamed rules retain no runtime lookup aliases. Never
  reuse a retired code or name for new semantics; keep migration history in the
  changelog and update every first-party selector in the same change. Preserve
  exit-code meanings and stable diagnostic prefixes for current rules.
- Deviate when: none for runtime rule aliases or code reuse.

### G-Diag-1: Report actionable facts with stable identity

- Why: a diagnostic is both user guidance and machine-observable output.
- Guidance: report the path or surface, the violated condition, and enough
  context to fix it without embedding secrets. Supply file identity as a
  structured subject, never only inside prose. Keep wording deterministic and
  assert the rule identity and subject in tests instead of matching only prose.
- Deviate when: the rule is repository-wide and no single path owns the
  violation.

### G-Sev-1: Derive default severity from platform impact

- Why: severity drifted both ways. Load-blocking Codex failures defaulted to
  warning (#325) while conventions and heuristics defaulted to error (#289,
  #333, #319, #355, #360, #276, #384), failing valid repositories in CI.
- Guidance: default to error only when the platform rejects or ignores the
  configuration at load or run time, or the defect is a security exposure.
  Default heuristics, conventions, and style checks to warning. Record the
  platform behavior that justifies each error-severity default in
  docs/rules.md next to the rule's provenance.
- Deviate when: an agent-lint-specific convention is deliberately strict;
  say so explicitly in the rule's documentation instead of implying a
  platform requirement.

### G-Spec-1: Record provenance for external contracts

- Why: platform event names, field vocabularies, schemas, limits, and accepted shapes change independently of Agent Lint, and an unversioned copied table can silently become a false-positive source.
- Guidance: keep each external vocabulary in one canonical owner and record its authoritative URL or artifact, verification date or version, and a reproducible refresh procedure nearby. Vendored schemas and hand-maintained constant tables follow the same provenance rule. Reverify sibling adapters and fixtures when refreshing a contract.
- Deviate when: the contract is defined entirely by this repository; identify it as an Agent Lint convention instead of attaching external provenance.

## Input and filesystem handling

### G-Parse-1: Preserve missing, invalid, and valid as distinct states

- Why: treating an unreadable or malformed file as absent can suppress the
  diagnostic that explains the real problem.
- Guidance: use a typed state like `ManifestState` when absence is allowed but
  malformed content is not. Parse once at the owning boundary and pass the
  result down instead of re-reading the same file in sibling validators.
- Deviate when: the rule is explicitly presence-only or the external format
  defines malformed content as equivalent to absence.

### G-Path-1: Make repository-relative path policy explicit

- Why: Agent Lint changes to the resolved repository root and configuration
  globs are matched against normalized repository-relative paths.
- Guidance: construct paths from the repository root, normalize paths before
  exclusion checks, skip known generated or dependency trees during recursive
  discovery, and apply exclusions before expensive reads.
- Deviate when: an API provides an absolute path; convert it at the boundary or
  explain why retaining the absolute identity is required.

### G-Input-1: Treat linted repository content as untrusted data

- Why: configuration, Markdown, JSON, TOML, YAML, command strings, and paths
  are controlled by the repository being analyzed.
- Guidance: parse rather than execute, validate path containment before a
  mutation, avoid shell interpretation, bound recursive or content-heavy work,
  and do not echo likely secret values in diagnostics.
- Deviate when: none for executing linted content. A test fixture may invoke a
  controlled helper that is part of the test itself.

### G-Root-2: Do not expand ambient working-directory dependence

- Why: process working directory is mutable global state, obscures which repository a helper reads, and forces otherwise independent tests to serialize.
- Guidance: new parsing, discovery, and validation APIs accept an explicit repository root or an explicit repository-relative input. Restrict `set_current_dir` to CLI startup and controlled test infrastructure, and prefer root-carrying types when a path will cross module boundaries.
- Deviate when: an external library requires ambient current-directory semantics; isolate that call behind a narrow adapter and restore process state with a guard in tests.

### G-I/O-1: Preserve discovery errors when a rule owns them

- Why: silently treating unreadable or malformed owned input as absent can hide the only diagnostic that explains why dependent checks did not run.
- Guidance: best-effort optional discovery may skip an unreadable candidate only when no rule promises to diagnose that state. When a surface has an invalid-file, unsafe-path, or unreadable-input rule, retain the error or typed rejected state through discovery and report it before skipping dependent semantic checks. Continue with independent siblings.
- Deviate when: the external contract explicitly treats the input as optional and indistinguishable from absence, and no Agent Lint rule claims that failure state.

### G-Inventory-1: Discover once, then pass inventories

- Why: repeated filesystem discovery lets detection, validation, prompt analysis, overlap analysis, and autofix disagree about which files belong to one runtime surface.
- Guidance: expose a typed inventory from the owning discovery module and pass or reuse it across consumers. Define normalization, exclusion, symlink, pruning, deduplication, and ordering policy at that boundary. A consumer may derive a documented subset from the inventory but should not repeat the repository walk.
- Deviate when: the consumer intentionally owns a different external scope, such as upload-size accounting that includes generated directories; name the differing policy and test the boundary.

### G-Order-1: Normalize before deduplication and sorting

- Why: equivalent authored paths and overlapping roots can otherwise produce duplicate findings or platform-dependent order.
- Guidance: define the semantic identity of each collected item, normalize to that identity before deduplication, and sort before returning a collection across a module boundary. Preserve source order only when source order is itself part of the diagnostic contract, such as duplicate keys or token spans.
- Deviate when: the collection is internal, single-pass, and never affects externally observable output; keep that locality evident.

### G-Classify-1: Classify prose by positive grammar, not substring or denylist

- Why: substring gates and negation denylists misclassify open-ended text.
  'Because' contains 'use' (#345), descriptive history repaired a negative
  directive (#536, #557), a denial passed as a provenance marker (#528), and
  denylist families stayed incomplete (#377). Fixed keyword lists also missed
  ordinary inputs (#328, #390, #300, #424).
- Guidance: state what the classifier accepts as a positive grammar over
  word-boundary tokens, and treat everything else as non-matching. Prefer
  parsed structure (canonical YAML, MarkdownDocument masked prose, argv
  roles) over raw text. Ship hard-negative cases with every classifier.
- Deviate when: the vocabulary is genuinely closed and exact-match, such as a
  platform enum; then an exact allowlist is the positive grammar.

## Autofix

### G-FixSafe-1: Make every autofix conservative and idempotent

- Why: `--autofix` writes to user repositories, so a plausible but incorrect
  rewrite is worse than leaving a diagnostic.
- Guidance: fix only a syntax or metadata transformation with one clear result.
  Preserve unrelated bytes where practical, honor the validator's scope and
  exclusions and per-file suppressions, return whether a write actually changed
  content, and test that a second run makes no change.
- Deviate when: none for idempotency. If a safe deterministic repair is not
  available, leave the rule diagnostic-only.

### G-FixSafe-2: Validator and fixer share one contract

- Why: duplicated recognition logic can make a fixer rewrite content that its
  validator would accept or fail to repair content it rejects.
- Guidance: share constants and parsers, or pin equivalent behavior with paired
  tests. A rule marked auto-fixable must have a reachable handler and a test
  that validates after mutation.
- Deviate when: platform-specific implementations must use different system
  APIs while producing the same postcondition.

## Rust and tests

### G-Rust-1: Encode domain states in types and exhaustive matches

- Why: enums such as `LintMode`, `CliMode`, `ManifestState`, `Severity`, and
  `LintRule` make illegal fall-through visible to the compiler.
- Guidance: prefer a small enum or struct over boolean combinations and magic
  strings. Match exhaustively when adding a state should require every consumer
  to make a decision.
- Deviate when: values are an open-ended external namespace that must remain
  forward-compatible.

### G-Rust-2: Keep failure handling proportional to the boundary

- Why: invalid user input should become a diagnostic or a CLI error, while a
  violated internal assumption should be loud.
- Guidance: use `Result` for recoverable I/O and parse failures. Reserve
  `unwrap` and `expect` in production for construction-time constants or
  assumptions already checked in the same path, with a reason visible nearby.
- Deviate when: test setup should fail immediately and the panic already names
  the failed operation.

### G-Test-2: Isolate tests that mutate process-global state

- Why: the working directory and environment are shared by parallel tests.
- Guidance: use `CwdGuard`, temporary directories, and `serial_test` for tests
  that change the current directory. Restore environment variables and other
  process-global state through drop guards even when assertions panic.
- Deviate when: the test never mutates process-global state.

### G-Test-3: Put regression defects in the second occurrence

- Why: first-only defects passed every existing test: only the first raw map
  was shape-checked (#546), repeated values took the first span (#510), late
  array defects were skipped (#539), and only the first violation in a
  document was reported (#359, #335).
- Guidance: when testing collection or multi-occurrence behavior, place the
  defect in the second element, second occurrence, or second file, and assert
  every expected diagnostic, not just one. Pair with a first-element case
  only when ordering matters.
- Deviate when: the contract is genuinely first-match-only; cite that
  contract in the test.

### G-Test-4: Test contracts by class and axis

- Why: one positive fixture does not protect mode, platform, suppression, exclusion, structured metadata, hard-negative precision, or autofix behavior.
- Guidance: for a new or materially changed rule family, identify applicable clean, broken, and hard-negative cases and the relevant contract axes: Basic or Plugin mode, strictness, focused selection, platform activation, suppression, exclusion, diagnostic subject and metadata, and autofix or no-autofix. Add the tuples to the checked-in contract matrix when the rule crosses surfaces or policy boundaries.
- Deviate when: an axis provably cannot affect the rule; leave it out rather than adding a ceremonial fixture, and record the boundary in the test or review notes when it is not obvious.

## Documentation and enforcement

### G-Doc-1: Keep drift-prone facts out of prose

- Why: rule totals, source line numbers, and duplicated defaults become stale
  without a compiler error.
- Guidance: refer to code by symbol or module, derive exact counts from the
  sole live `ALL_RULES` registry, and keep exact defaults in one code owner.
  Reserve approximate wording such as `~300` for marketing prose; generate or
  mechanically check exact totals, prefix tables, rule rows, and autofix
  denominators. Sweep README and `docs/` when renaming a rule, flag, config key,
  module, or script.
- Deviate when: a literal is itself part of the public contract and tests pin
  the documentation to the implementation.

### G-Enf-1: Prefer mechanical enforcement for deterministic rules

- Why: review guidance is weak protection for a condition a test or lint can
  decide exactly.
- Guidance: add a focused test, compiler-enforced type, pre-commit hook, or
  lint when a violation is deterministic and likely to recur. Keep this file
  for judgment calls and architectural direction.
- Deviate when: enforcement would duplicate an upstream compiler or linter
  check without improving its signal.
