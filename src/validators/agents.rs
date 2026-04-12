use crate::diagnostic::DiagnosticCollector;
use crate::frontmatter;
use std::fs;
use std::path::Path;

/// V7: Validate agents/*.md frontmatter.
pub fn validate_agents(diag: &mut DiagnosticCollector) {
    let agents_dir = Path::new("agents");
    if !agents_dir.is_dir() {
        diag.fail("agents/ directory is missing");
        return;
    }

    let mut found = 0;
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

        found += 1;
        let agent_path = format!("agents/{name}");
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let fm_lines = match frontmatter::extract_frontmatter(&content) {
            Some(lines) => lines,
            None => {
                diag.fail(&format!(
                    "{agent_path}: malformed frontmatter (must start with '---' on line 1, must have closing '---')"
                ));
                continue;
            }
        };

        let fm_name = frontmatter::get_field(&fm_lines, "name");
        let fm_desc = frontmatter::get_field(&fm_lines, "description");

        if fm_name.is_none() {
            diag.fail(&format!(
                "{agent_path}: missing required frontmatter field 'name'"
            ));
        }
        if fm_desc.is_none() {
            diag.fail(&format!(
                "{agent_path}: missing required frontmatter field 'description'"
            ));
        }
    }

    if found == 0 {
        diag.fail("agents/ has no .md files");
    }
}

/// V16: Agent-template alignment — every agents/*.md must contain
/// "Derived from" marker referencing reviewer-templates.md.
/// (Larch-specific convention check.)
pub fn validate_agent_template_alignment(diag: &mut DiagnosticCollector) {
    let agents_dir = Path::new("agents");
    let templates = Path::new("skills/shared/reviewer-templates.md");

    if !agents_dir.is_dir() {
        return;
    }
    if !templates.is_file() {
        diag.fail(&format!(
            "reviewer-templates.md missing: {}",
            templates.display()
        ));
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

        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let has_marker = content.lines().any(|line| {
            let lower = line.to_lowercase();
            lower.contains("derived from") && lower.contains("reviewer-templates.md")
        });

        if !has_marker {
            diag.fail(&format!(
                "agents/{name} missing 'Derived from skills/shared/reviewer-templates.md' marker"
            ));
        }
    }
}

/// V21: Agent-template count — number of ## Reviewer sections in
/// skills/shared/reviewer-templates.md must equal number of agents/*.md files.
/// (Larch-specific convention check.)
pub fn validate_agent_template_count(diag: &mut DiagnosticCollector) {
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
                        agent_count += 1;
                    }
                }
            }
        }
    }

    if template_count != agent_count {
        diag.fail(&format!(
            "agent-template count mismatch: {agent_count} agent file(s) but {template_count} '## Reviewer' section(s) in {}",
            templates.display()
        ));
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
        let saved = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("agents").unwrap();
        std::fs::write(
            "agents/general.md",
            "---\nname: general\ndescription: General reviewer\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_agents(&mut diag);
        assert_eq!(diag.error_count(), 0);

        std::env::set_current_dir(saved).unwrap();
    }

    #[test]
    #[serial_test::serial]
    fn test_v7_missing_agents_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let saved = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_agents(&mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("agents/ directory is missing"));

        std::env::set_current_dir(saved).unwrap();
    }

    #[test]
    #[serial_test::serial]
    fn test_v7_empty_agents_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let saved = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("agents").unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_agents(&mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("no .md files"));

        std::env::set_current_dir(saved).unwrap();
    }

    #[test]
    #[serial_test::serial]
    fn test_v7_missing_frontmatter_name() {
        let tmp = tempfile::tempdir().unwrap();
        let saved = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("agents").unwrap();
        std::fs::write(
            "agents/general.md",
            "---\ndescription: General reviewer\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_agents(&mut diag);
        assert!(diag.error_count() >= 1);
        assert!(diag.errors().iter().any(|e| e.contains("name")));

        std::env::set_current_dir(saved).unwrap();
    }

    // V16: validate_agent_template_alignment
    #[test]
    #[serial_test::serial]
    fn test_v16_valid_alignment() {
        let tmp = tempfile::tempdir().unwrap();
        let saved = std::env::current_dir().unwrap();
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
        validate_agent_template_alignment(&mut diag);
        assert_eq!(diag.error_count(), 0);

        std::env::set_current_dir(saved).unwrap();
    }

    #[test]
    #[serial_test::serial]
    fn test_v16_missing_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let saved = std::env::current_dir().unwrap();
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
        validate_agent_template_alignment(&mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("missing"));

        std::env::set_current_dir(saved).unwrap();
    }

    // V21: validate_agent_template_count
    #[test]
    #[serial_test::serial]
    fn test_v21_matching_count() {
        let tmp = tempfile::tempdir().unwrap();
        let saved = std::env::current_dir().unwrap();
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
        validate_agent_template_count(&mut diag);
        assert_eq!(diag.error_count(), 0);

        std::env::set_current_dir(saved).unwrap();
    }

    #[test]
    #[serial_test::serial]
    fn test_v21_mismatched_count() {
        let tmp = tempfile::tempdir().unwrap();
        let saved = std::env::current_dir().unwrap();
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
        validate_agent_template_count(&mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("mismatch"));

        std::env::set_current_dir(saved).unwrap();
    }
}
