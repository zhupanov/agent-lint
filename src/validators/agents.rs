use crate::config::ExcludeSet;
use crate::diagnostic::DiagnosticCollector;
use crate::frontmatter;
use crate::rules::LintRule;
use std::fs;
use std::path::Path;

/// V7: Validate agents/*.md frontmatter.
pub fn validate_agents(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let agents_dir = Path::new("agents");
    if !agents_dir.is_dir() {
        diag.report(LintRule::AgentsDirMissing, "agents/ directory is missing");
        return;
    }

    let mut found = 0;
    let mut excluded_count = 0;
    let re_name_invalid = regex::Regex::new(r"[^a-z0-9-]").unwrap();
    let entries = match fs::read_dir(agents_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) if n.ends_with(".md") => n.to_string(),
            _ => continue,
        };

        let agent_path = format!("agents/{name}");
        if exclude.is_excluded(&agent_path) {
            excluded_count += 1;
            continue;
        }

        found += 1;
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let fm_lines = match frontmatter::extract_frontmatter(&content) {
            Some(lines) => lines,
            None => {
                diag.report(
                    LintRule::AgentFrontmatterMalformed,
                    &format!(
                        "{agent_path}: malformed frontmatter (must start with '---' on line 1, must have closing '---')"
                    ),
                );
                continue;
            }
        };

        let fm_name = frontmatter::get_field(&fm_lines, "name");
        let fm_desc = frontmatter::get_field(&fm_lines, "description");

        if fm_name.is_none() {
            diag.report(
                LintRule::AgentFieldMissing,
                &format!("{agent_path}: missing required frontmatter field 'name'"),
            );
        }
        if fm_desc.is_none() {
            diag.report(
                LintRule::AgentFieldMissing,
                &format!("{agent_path}: missing required frontmatter field 'description'"),
            );
        }

        // A008: agent description too long
        if let Some(ref desc) = fm_desc {
            if desc.len() > 1024 {
                diag.report(
                    LintRule::AgentDescLong,
                    &format!(
                        "{agent_path}: description exceeds 1024 characters ({})",
                        desc.len()
                    ),
                );
            }
        }

        // A009: agent description too short
        if let Some(ref desc) = fm_desc {
            if desc.len() < 20 {
                diag.report(
                    LintRule::AgentDescShort,
                    &format!(
                        "{agent_path}: description is under 20 characters ({})",
                        desc.len()
                    ),
                );
            }
        }

        // A010: agent name invalid characters
        if let Some(ref n) = fm_name {
            if re_name_invalid.is_match(n) {
                diag.report(
                    LintRule::AgentNameInvalid,
                    &format!(
                        "{agent_path}: name '{}' contains characters outside [a-z0-9-]",
                        n
                    ),
                );
            }
        }
    }

    if found == 0 && excluded_count == 0 {
        diag.report(LintRule::NoAgentFiles, "agents/ has no .md files");
    }
}

/// V16: Agent-template alignment — every agents/*.md must contain
/// "Derived from" marker referencing reviewer-templates.md.
/// (Larch-specific convention check.)
pub fn validate_agent_template_alignment(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let agents_dir = Path::new("agents");
    let templates = Path::new("skills/shared/reviewer-templates.md");

    if !agents_dir.is_dir() {
        return;
    }
    if !templates.is_file() {
        diag.report(
            LintRule::TemplateFileMissing,
            &format!("reviewer-templates.md missing: {}", templates.display()),
        );
        return;
    }

    let entries = match fs::read_dir(agents_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) if n.ends_with(".md") => n.to_string(),
            _ => continue,
        };

        let agent_path = format!("agents/{name}");
        if exclude.is_excluded(&agent_path) {
            continue;
        }

        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let has_marker = content.lines().any(|line| {
            let lower = line.to_lowercase();
            lower.contains("derived from") && lower.contains("reviewer-templates.md")
        });

        if !has_marker {
            diag.report(
                LintRule::TemplateMarkerMissing,
                &format!(
                    "agents/{name} missing 'Derived from skills/shared/reviewer-templates.md' marker"
                ),
            );
        }
    }
}

/// V21: Agent-template count — number of ## Reviewer sections in
/// skills/shared/reviewer-templates.md must equal number of agents/*.md files.
/// (Larch-specific convention check.)
pub fn validate_agent_template_count(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let agents_dir = Path::new("agents");
    let templates = Path::new("skills/shared/reviewer-templates.md");

    if !agents_dir.is_dir() || !templates.is_file() {
        return; // V16 catches missing template
    }

    // Count ## Reviewer sections
    let template_content = match fs::read_to_string(templates) {
        Ok(c) => c,
        Err(_) => return,
    };
    let template_count = template_content
        .lines()
        .filter(|line| line.starts_with("## Reviewer"))
        .count();

    // Count agents/*.md files
    let mut agent_count = 0;
    if let Ok(entries) = fs::read_dir(agents_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.ends_with(".md") {
                        let agent_path = format!("agents/{name}");
                        if !exclude.is_excluded(&agent_path) {
                            agent_count += 1;
                        }
                    }
                }
            }
        }
    }

    if template_count != agent_count {
        diag.report(
            LintRule::TemplateCountMismatch,
            &format!(
                "agent-template count mismatch: {agent_count} agent file(s) but {template_count} '## Reviewer' section(s) in {}",
                templates.display()
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::DiagnosticCollector;

    // V7: validate_agents
    #[test]
    #[serial_test::serial]
    fn test_v7_valid_agents() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("agents").unwrap();
        std::fs::write(
            "agents/general.md",
            "---\nname: general\ndescription: General reviewer for code quality analysis\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v7_missing_agents_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("agents/ directory is missing"));
    }

    #[test]
    #[serial_test::serial]
    fn test_v7_empty_agents_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("agents").unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("no .md files"));
    }

    #[test]
    #[serial_test::serial]
    fn test_v7_missing_frontmatter_name() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("agents").unwrap();
        std::fs::write(
            "agents/general.md",
            "---\ndescription: General reviewer for code quality analysis\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.error_count() >= 1);
        assert!(diag.errors().iter().any(|e| e.contains("name")));
    }

    // V16: validate_agent_template_alignment
    #[test]
    #[serial_test::serial]
    fn test_v16_valid_alignment() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("agents").unwrap();
        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::write("skills/shared/reviewer-templates.md", "# Templates\n").unwrap();
        std::fs::write(
            "agents/general.md",
            "---\nname: general\ndescription: desc\n---\nDerived from skills/shared/reviewer-templates.md\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_agent_template_alignment(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v16_missing_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("agents").unwrap();
        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::write("skills/shared/reviewer-templates.md", "# Templates\n").unwrap();
        std::fs::write(
            "agents/general.md",
            "---\nname: general\ndescription: desc\n---\nNo marker here\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_agent_template_alignment(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("missing"));
    }

    // V21: validate_agent_template_count
    #[test]
    #[serial_test::serial]
    fn test_v21_matching_count() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("agents").unwrap();
        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::write(
            "skills/shared/reviewer-templates.md",
            "## Reviewer 1\nContent\n## Reviewer 2\nContent\n",
        )
        .unwrap();
        std::fs::write("agents/one.md", "---\nname: one\n---\n").unwrap();
        std::fs::write("agents/two.md", "---\nname: two\n---\n").unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_agent_template_count(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v21_mismatched_count() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("agents").unwrap();
        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::write(
            "skills/shared/reviewer-templates.md",
            "## Reviewer 1\nContent\n## Reviewer 2\nContent\n",
        )
        .unwrap();
        std::fs::write("agents/one.md", "---\nname: one\n---\n").unwrap();
        // Only 1 agent but 2 templates

        let mut diag = DiagnosticCollector::new();
        validate_agent_template_count(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("mismatch"));
    }

    // A008: agent-desc-long
    #[test]
    #[serial_test::serial]
    fn test_a008_desc_too_long() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        let long_desc = "x".repeat(1025);
        std::fs::write(
            "agents/general.md",
            format!("---\nname: general\ndescription: {long_desc}\n---\nBody\n"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("exceeds 1024")));
    }

    // A009: agent-desc-short
    #[test]
    #[serial_test::serial]
    fn test_a009_desc_too_short() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::write(
            "agents/general.md",
            "---\nname: general\ndescription: Short\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("under 20")));
    }

    // A010: agent-name-invalid
    #[test]
    #[serial_test::serial]
    fn test_a010_name_invalid() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::write(
            "agents/general.md",
            "---\nname: My_Agent\ndescription: A valid agent description here\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("outside [a-z0-9-]"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_a010_valid_name_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::write(
            "agents/general.md",
            "---\nname: general-reviewer\ndescription: A valid agent description here\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("outside [a-z0-9-]"))
        );
    }
}
