# CLI conformance corpus

This directory contains a small checked-in regression and conformance corpus
for the released `agent-lint` CLI path. It is intentionally curated for fast,
local diagnosis; it is not an external benchmark and must not be used to claim
a detection rate, accuracy estimate, or false-positive rate.

## Layout

- `repositories/<case>/` is the repository presented to the CLI. Cases are
  classified as `clean`, `broken`, or `hard-negative` in their manifest.
- `manifests/<case>.json` is the oracle. Keeping it outside the corresponding
  repository prevents agent-lint from linting its own expected results.
- `tests/conformance.rs` copies each repository to a temporary Git repository,
  invokes the built binary offline, validates its JSON payload, and compares
  stable diagnostic identity fields rather than prose messages.

Every manifest records the detected mode and complete argument list, expected
exit code and report status, active platforms, suppression count, exact ordered
rule code/name/severity/subject-path tuples, and an explicit list of allowed
additional diagnostics. An allowed diagnostic must include a justification;
prefer an empty list. Autofix cases also record exact post-fix file contents,
which the harness validates again after an idempotent second run.

The harness enforces clean, broken, and hard-negative coverage for each initial
surface and behavior named in issue 198. To add a case, create one same-named
directory and manifest, keep the repository minimal, select only the rules the
case exercises, and add focused coverage tags. Do not put an oracle inside a
repository fixture.

JSON output is schema-validated today. SARIF is not currently a supported CLI
format; when it lands, add its payload validation to this same corpus rather
than creating a separate network-dependent harness.
