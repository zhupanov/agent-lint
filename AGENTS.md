# Agent Lint

Repository guidance for coding agents.

## Dependency upgrades and the minimum Rust version

When a dependency upgrade requires a Rust version above the declared minimum,
raise `rust-version` in `Cargo.toml` to the version that upgrade needs and take
the upgrade. Prefer raising the floor over pinning the dependency to an older
release, and treat "the upgrade wants a newer compiler" as a reason to move the
floor rather than a reason to stop.

The declared minimum is a floor this repository does not otherwise exercise.
`rust-toolchain.toml` pins the toolchain every CI job builds with, no job builds
at the declared minimum, and this crate ships as a binary with no library
consumers. Holding a crate back to protect that floor trades a real upgrade for
a promise nothing verifies.

Raise the floor and the prose that states it in one commit. The minimum appears
in `Cargo.toml`, `README.md`, `docs/development.md`, and `docs/yaml.md`. A stale
mention in any of them survives the test suite, so update all four together.

After raising the floor, run `cargo clippy --all-targets --all-features` with
warnings denied. A higher minimum un-gates compiler-version-sensitive lints, so
clippy can report findings on code that passed before the raise. Fix those
findings in the same change.

Finish by confirming `cargo update --dry-run` reports no remaining updates, or
by recording in the pull request why each remaining crate is held.

## Release version bumps

`.claude/skills/bump-version/scripts/apply-bump.sh` owns version rewrites. It
keeps `Cargo.lock` to a single version line and rewrites every release-example
document. Change that script when the release-document set changes, and let
`scripts/test-apply-bump.sh` cover the new behavior.
