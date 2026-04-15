use crate::config::ExcludeSet;
use crate::diagnostic::DiagnosticCollector;
use crate::rules::LintRule;
use regex::Regex;
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

use super::common::RE_TODO_MARKER;

static RE_DOCS_REF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"docs/[a-zA-Z0-9._/-]+\.md").unwrap());

/// V22: Docs file references from CLAUDE.md.
pub fn validate_docs_references(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    if exclude.is_excluded("CLAUDE.md") {
        return;
    }
    let claude_md = Path::new("CLAUDE.md");
    if !claude_md.is_file() {
        return;
    }

    let content = match fs::read_to_string(claude_md) {
        Ok(c) => c,
        Err(_) => return,
    };

    let section = extract_canonical_sources_section(&content);

    let mut seen = std::collections::HashSet::new();

    for cap in RE_DOCS_REF.find_iter(&section) {
        let doc_path = cap.as_str();
        if seen.insert(doc_path.to_string()) && !Path::new(doc_path).is_file() {
            diag.report(
                LintRule::DocsRefMissing,
                &format!(
                    "docs reference in CLAUDE.md canonical sources not found on disk: {doc_path}"
                ),
            );
        }
    }
}

/// D002: CLAUDE.md size limit (500 lines).
pub fn validate_claudemd_size(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    if exclude.is_excluded("CLAUDE.md") {
        return;
    }
    let claude_md = Path::new("CLAUDE.md");
    if !claude_md.is_file() {
        return;
    }

    let content = match fs::read_to_string(claude_md) {
        Ok(c) => c,
        Err(_) => return,
    };

    let line_count = content.lines().count();
    if line_count > 500 {
        diag.report(
            LintRule::ClaudemdTooLarge,
            &format!(
                "CLAUDE.md exceeds 500 lines ({} lines); consider splitting into docs/ files",
                line_count
            ),
        );
    }
}

/// D003: TODO/FIXME/HACK/XXX markers in CLAUDE.md.
pub fn validate_claudemd_todos(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    if exclude.is_excluded("CLAUDE.md") {
        return;
    }
    let claude_md = Path::new("CLAUDE.md");
    if !claude_md.is_file() {
        return;
    }

    let content = match fs::read_to_string(claude_md) {
        Ok(c) => c,
        Err(_) => return,
    };

    for line in crate::fence::lines_outside_fences(&content) {
        if let Some(m) = RE_TODO_MARKER.find(line) {
            diag.report(
                LintRule::TodoInDocs,
                &format!(
                    "CLAUDE.md contains {} marker; remove before publishing",
                    m.as_str()
                ),
            );
            break; // Report once per file (matches G006/G007 pattern)
        }
    }
}

fn extract_canonical_sources_section(content: &str) -> String {
    let mut in_section = false;
    let mut result = Vec::new();

    for line in content.lines() {
        if line
            .to_ascii_lowercase()
            .starts_with("## canonical sources")
        {
            in_section = true;
            result.push(line);
            continue;
        }
        if in_section {
            if line.starts_with("## ") {
                break;
            }
            result.push(line);
        }
    }

    result.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_section_stops_at_any_heading() {
        let content = "\
## Canonical sources\n\
- docs/foo.md\n\
- docs/bar.md\n\
## Contributing\n\
This should not be included\n\
";
        let section = extract_canonical_sources_section(content);
        assert!(section.contains("docs/foo.md"));
        assert!(section.contains("docs/bar.md"));
        assert!(!section.contains("Contributing"));
    }

    #[test]
    fn test_extract_section_case_insensitive() {
        let content = "\
## Canonical Sources\n\
- docs/foo.md\n\
## Other\n\
";
        let section = extract_canonical_sources_section(content);
        assert!(section.contains("docs/foo.md"));
        assert!(!section.contains("Other"));
    }

    #[test]
    #[serial_test::serial]
    fn test_v22_subdirectory_docs_reference() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("docs/sub").unwrap();
        std::fs::write("docs/sub/architecture.md", "# Arch\n").unwrap();
        std::fs::write(
            "CLAUDE.md",
            "# Claude\n## Canonical sources\n- docs/sub/architecture.md\n## Other\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_docs_references(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_extract_section_stops_at_c_heading() {
        let content = "\
## Canonical sources\n\
- docs/foo.md\n\
## Configuration\n\
Should not be here\n\
";
        let section = extract_canonical_sources_section(content);
        assert!(section.contains("docs/foo.md"));
        assert!(!section.contains("Configuration"));
    }

    #[test]
    #[serial_test::serial]
    fn test_v22_valid_docs_reference() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("docs").unwrap();
        std::fs::write("docs/architecture.md", "# Arch\n").unwrap();
        std::fs::write(
            "CLAUDE.md",
            "# Claude\n## Canonical sources\n- docs/architecture.md\n## Other\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_docs_references(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v22_missing_docs_reference() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::write(
            "CLAUDE.md",
            "# Claude\n## Canonical sources\n- docs/nonexistent.md\n## Other\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_docs_references(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("not found on disk"));
    }

    #[test]
    #[serial_test::serial]
    fn test_v22_no_claude_md_silent() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_docs_references(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    // D002: claudemd-too-large
    #[test]
    #[serial_test::serial]
    fn test_d002_claudemd_too_large() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let content = "line\n".repeat(501);
        std::fs::write("CLAUDE.md", &content).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_claudemd_size(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("exceeds 500")));
    }

    #[test]
    #[serial_test::serial]
    fn test_d002_claudemd_500_lines_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let content = "line\n".repeat(500);
        std::fs::write("CLAUDE.md", &content).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_claudemd_size(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("exceeds 500")));
    }

    // D003: todo-in-docs
    #[test]
    #[serial_test::serial]
    fn test_d003_todo_outside_fence() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::write("CLAUDE.md", "# Docs\nTODO: finish this section\n").unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_claudemd_todos(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("TODO")));
    }

    #[test]
    #[serial_test::serial]
    fn test_d003_todo_inside_fence_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::write(
            "CLAUDE.md",
            "# Docs\n\n```bash\n# TODO: this is in a code block\n```\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_claudemd_todos(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("TODO")));
    }

    #[test]
    #[serial_test::serial]
    fn test_d003_todo_in_nested_fence_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        // 4-backtick fence containing 3-backtick line with TODO — should not trigger
        std::fs::write(
            "CLAUDE.md",
            "# Docs\n\n````\n```\n# TODO: nested fence content\n```\n````\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_claudemd_todos(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag.errors().iter().any(|e| e.contains("TODO")),
            "TODO inside nested 4-backtick fence should not trigger D003"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_d003_no_claudemd_silent() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_claudemd_todos(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }
}
