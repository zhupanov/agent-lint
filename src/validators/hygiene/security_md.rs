use crate::diagnostic::DiagnosticCollector;
use crate::rules::LintRule;
use std::path::Path;

/// V14: SECURITY.md presence
pub fn validate_security_md(diag: &mut DiagnosticCollector) {
    if !Path::new("SECURITY.md").is_file() {
        diag.report(
            LintRule::SecurityMdMissing,
            "SECURITY.md is missing from repo root",
        );
    }
}
