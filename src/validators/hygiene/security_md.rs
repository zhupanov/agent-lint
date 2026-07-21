// This module, its file, and `validate_security_md` keep the historical
// `security_md` name. Only the public rule identity is a stable contract: code
// `G005`, name `security-policy-missing` (see `LintRule::SecurityMdMissing`).
use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata};
use crate::rules::LintRule;
use std::path::Path;

/// Repository-relative directories GitHub recognizes for a community-health
/// `SECURITY.md`. GitHub's documented precedence is `.github`, the repository
/// root, then `docs`; for a presence check any of the three satisfies the rule.
///
/// Source: <https://docs.github.com/en/communities/setting-up-your-project-for-healthy-contributions/creating-a-default-community-health-file#supported-file-types>
/// Retrieved 2026-07-21. Supported community-health locations can change, so
/// this list and the matching row in `docs/rules.md` must be revisited together.
const SECURITY_POLICY_DIRS: [&str; 3] = [".github", ".", "docs"];

/// The exact, case-sensitive name a repository-local security policy must use.
const SECURITY_POLICY_FILE: &str = "SECURITY.md";

/// G005: a repository-local security policy is present in a GitHub-supported
/// location. A regular `SECURITY.md` in the repository root, `.github/`, or
/// `docs/` satisfies the rule. Only an exact-case, regular, non-symlink file
/// counts; a directory, a wrong-case name, or a symlink does not, matching a
/// committed repository-owned policy and avoiding platform-dependent traversal.
///
/// The subject is the logical root `SECURITY.md` even though the policy may live
/// elsewhere, so global suppression and a per-file override on `SECURITY.md`
/// keep working. An organization default served from a public `.github`
/// repository cannot be observed locally, so this stays a warning that normal
/// suppression can silence.
pub fn validate_security_md(diag: &mut DiagnosticCollector) {
    if SECURITY_POLICY_DIRS
        .iter()
        .any(|dir| has_security_policy(Path::new(dir)))
    {
        return;
    }

    diag.report_at_with(
        LintRule::SecurityMdMissing,
        SECURITY_POLICY_FILE,
        "no SECURITY.md security policy found in the repository",
        DiagnosticMetadata::default()
            .with_suggestion("add a SECURITY.md at the repository root, .github/, or docs/"),
    );
}

/// Whether `dir` directly contains an exact-case, regular, non-symlink file
/// named `SECURITY.md`.
///
/// The directory is enumerated rather than probed by path so the exact on-disk
/// spelling is honored even on case-insensitive filesystems, and the entry's own
/// type is inspected (`DirEntry::file_type` does not follow symlinks) so a
/// symlinked `SECURITY.md` is rejected instead of being traversed.
fn has_security_policy(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        if entry.file_name() != SECURITY_POLICY_FILE {
            continue;
        }
        return entry
            .file_type()
            .map(|file_type| file_type.is_file())
            .unwrap_or(false);
    }
    false
}
