use crate::diagnostic::DiagnosticCollector;
use crate::frontmatter;
use regex::Regex;
use std::fs;
use std::path::Path;

/// V5: Validate skills/* layout — every skills/*/ (except shared/) must contain SKILL.md.
pub fn validate_skills_layout(diag: &mut DiagnosticCollector) {
    let skills_dir = Path::new("skills");
    if !skills_dir.is_dir() {
        diag.fail("skills/ directory is missing");
        return;
    }

    let mut skill_count = 0;
    let entries = match fs::read_dir(skills_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if name == "shared" {
            continue;
        }
        let skill_md = path.join("SKILL.md");
        if !skill_md.is_file() {
            diag.fail(&format!("skills/{name}/ missing SKILL.md"));
            continue;
        }
        skill_count += 1;
    }

    if skill_count == 0 {
        diag.fail("no plugin-exported skills found under skills/");
    }
}

/// V6: Validate SKILL.md frontmatter for public skills (skills/*/SKILL.md).
pub fn validate_skill_frontmatter(diag: &mut DiagnosticCollector) {
    validate_skill_frontmatter_in_dir("skills", true, diag);
}

/// V6-adapted: Validate SKILL.md frontmatter for private skills (.claude/skills/*/SKILL.md).
pub fn validate_private_skill_frontmatter(diag: &mut DiagnosticCollector) {
    validate_skill_frontmatter_in_dir(".claude/skills", false, diag);
}

fn validate_skill_frontmatter_in_dir(
    base_dir: &str,
    check_name_match: bool,
    diag: &mut DiagnosticCollector,
) {
    let dir = Path::new(base_dir);
    if !dir.is_dir() {
        return;
    }

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let dir_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if dir_name == "shared" {
            continue;
        }

        let skill_md = path.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }

        let skill_path = format!("{base_dir}/{dir_name}/SKILL.md");
        let content = match fs::read_to_string(&skill_md) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let fm_lines = match frontmatter::extract_frontmatter(&content) {
            Some(lines) => lines,
            None => {
                diag.fail(&format!(
                    "{skill_path}: malformed frontmatter (must start with '---' on line 1, must have closing '---')"
                ));
                continue;
            }
        };

        let name = frontmatter::get_field(&fm_lines, "name");
        let desc = frontmatter::get_field(&fm_lines, "description");

        if name.is_none() {
            diag.fail(&format!(
                "{skill_path}: missing required frontmatter field 'name'"
            ));
        }
        if desc.is_none() {
            diag.fail(&format!(
                "{skill_path}: missing required frontmatter field 'description'"
            ));
        }

        if check_name_match {
            if let Some(ref n) = name {
                if n != &dir_name {
                    diag.fail(&format!(
                        "{skill_path}: frontmatter name '{n}' does not match directory '{dir_name}'"
                    ));
                }
            }
        }

        // Optional scalar fields: if present, must be non-empty.
        for field in &["argument-hint", "allowed-tools"] {
            let prefix = format!("{field}:");
            let field_present = fm_lines.iter().any(|line| line.starts_with(&prefix));
            if field_present {
                let val = frontmatter::get_field(&fm_lines, field);
                if val.is_none() {
                    diag.fail(&format!(
                        "{skill_path}: optional field '{field}' is present but empty"
                    ));
                }
            }
        }
    }
}

/// V15: Validate shared markdown reference integrity.
/// Every `${CLAUDE_PLUGIN_ROOT}/skills/shared/*.md` path referenced from
/// `skills/*/SKILL.md` must exist on disk.
pub fn validate_shared_md_references(diag: &mut DiagnosticCollector) {
    let skills_dir = Path::new("skills");
    if !skills_dir.is_dir() {
        return;
    }

    let re = Regex::new(r"\$\{CLAUDE_PLUGIN_ROOT\}/skills/shared/[a-zA-Z0-9._-]+\.md").unwrap();

    let entries = match fs::read_dir(skills_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let dir_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if dir_name == "shared" {
            continue;
        }

        let skill_md = path.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }

        let content = match fs::read_to_string(&skill_md) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let skill_path = format!("skills/{dir_name}/SKILL.md");

        for cap in re.find_iter(&content) {
            let reference = cap.as_str();
            let rel = reference.replace("${CLAUDE_PLUGIN_ROOT}/", "");
            if !Path::new(&rel).is_file() {
                diag.fail(&format!(
                    "shared markdown reference missing on disk: {reference} (in {skill_path}, expected {rel})"
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::DiagnosticCollector;

    // V5: validate_skills_layout
    #[test]
    #[serial_test::serial]
    fn test_v5_valid_skills_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write("skills/my-skill/SKILL.md", "---\nname: my-skill\n---\n").unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_skills_layout(&mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v5_missing_skills_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_skills_layout(&mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("skills/ directory is missing"));
    }

    #[test]
    #[serial_test::serial]
    fn test_v5_missing_skill_md() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        // No SKILL.md file

        let mut diag = DiagnosticCollector::new();
        validate_skills_layout(&mut diag);
        // Missing SKILL.md + no skills found = 2 errors
        assert!(diag.error_count() >= 1);
        assert!(diag.errors().iter().any(|e| e.contains("missing SKILL.md")));
    }

    #[test]
    #[serial_test::serial]
    fn test_v5_shared_dir_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write("skills/my-skill/SKILL.md", "---\nname: my-skill\n---\n").unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_skills_layout(&mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    // V6: validate_skill_frontmatter (public skills)
    #[test]
    #[serial_test::serial]
    fn test_v6_valid_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A skill\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_skill_frontmatter(&mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v6_missing_name() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\ndescription: A skill\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_skill_frontmatter(&mut diag);
        assert!(diag.error_count() >= 1);
        assert!(diag.errors().iter().any(|e| e.contains("name")));
    }

    #[test]
    #[serial_test::serial]
    fn test_v6_name_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: wrong-name\ndescription: A skill\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_skill_frontmatter(&mut diag);
        assert!(diag.error_count() >= 1);
        assert!(diag.errors().iter().any(|e| e.contains("does not match")));
    }

    #[test]
    #[serial_test::serial]
    fn test_v6_malformed_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write("skills/my-skill/SKILL.md", "no frontmatter\n").unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_skill_frontmatter(&mut diag);
        assert!(diag.error_count() >= 1);
        assert!(diag.errors().iter().any(|e| e.contains("malformed")));
    }

    // V6-adapted: validate_private_skill_frontmatter (Basic mode)
    #[test]
    #[serial_test::serial]
    fn test_v6a_valid_private_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/skills/my-skill").unwrap();
        std::fs::write(
            ".claude/skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Private skill\n---\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_private_skill_frontmatter(&mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v6a_missing_description() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/skills/my-skill").unwrap();
        std::fs::write(
            ".claude/skills/my-skill/SKILL.md",
            "---\nname: my-skill\n---\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_private_skill_frontmatter(&mut diag);
        assert!(diag.error_count() >= 1);
        assert!(diag.errors().iter().any(|e| e.contains("description")));
    }

    #[test]
    #[serial_test::serial]
    fn test_v6a_no_private_skills_dir_silent() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_private_skill_frontmatter(&mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    // V15: validate_shared_md_references
    #[test]
    #[serial_test::serial]
    fn test_v15_valid_shared_reference() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write("skills/shared/helpers.md", "# Helpers\n").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: s\n---\nSee ${CLAUDE_PLUGIN_ROOT}/skills/shared/helpers.md\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_shared_md_references(&mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v15_missing_shared_reference() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: s\n---\nSee ${CLAUDE_PLUGIN_ROOT}/skills/shared/nonexistent.md\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_shared_md_references(&mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("missing on disk"));
    }
}
