use crate::config::ExcludeSet;
use crate::diagnostic::DiagnosticCollector;
use crate::frontmatter;
use crate::rules::LintRule;
use regex::Regex;
use std::fs;
use std::path::Path;
use std::sync::LazyLock;
use walkdir::WalkDir;

static RE_SHARED_MD_REF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\$\{CLAUDE_PLUGIN_ROOT\}/skills/shared/[a-zA-Z0-9._/-]+\.md").unwrap()
});

/// Relative markdown link targets nested deeper than one directory level.
static RE_RELATIVE_MD_LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\]\((?:\./)?([^)]+\.md)\)").unwrap());

const SKILL_DIR_SIZE_LIMIT: u64 = 8 * 1024 * 1024;

/// Pre-parsed data for a single SKILL.md file.
#[allow(dead_code)]
pub struct SkillInfo {
    /// Display path, e.g. "skills/my-skill/SKILL.md"
    pub path: String,
    /// Directory name, e.g. "my-skill"
    pub dir_name: String,
    /// Frontmatter lines (between the --- delimiters).
    pub fm_lines: Vec<String>,
    /// Body content after the frontmatter closing delimiter.
    pub body: String,
    /// Whether the skill directory contains a non-empty `scripts/` subdirectory.
    pub has_scripts_dir: bool,
}

/// Walk a skills directory and collect SkillInfo for each valid skill.
/// Skips `shared/` subdirectory and excluded paths. Returns empty vec if dir doesn't exist.
pub fn collect_skills(base_dir: &str, exclude: &ExcludeSet) -> Vec<SkillInfo> {
    let dir = Path::new(base_dir);
    let subdirs = super::walk::read_subdirs(dir, base_dir, exclude, true);

    let mut skills = Vec::new();
    for (path, dir_name) in subdirs {
        let skill_md = path.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }

        let content = match fs::read_to_string(&skill_md) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let fm_lines = match frontmatter::extract_frontmatter(&content) {
            Some(lines) => lines,
            None => continue, // S004 fires from existing validator
        };

        let body = frontmatter::extract_body(&content).to_string();
        let skill_path = format!("{base_dir}/{dir_name}/SKILL.md");

        let scripts_dir = path.join("scripts");
        let has_scripts_dir = scripts_dir.is_dir()
            && fs::read_dir(&scripts_dir)
                .ok()
                .is_some_and(|mut e| matches!(e.next(), Some(Ok(_))));

        skills.push(SkillInfo {
            path: skill_path,
            dir_name,
            fm_lines,
            body,
            has_scripts_dir,
        });
    }
    skills
}

/// V5: Validate skills/* layout — every skills/*/ (except shared/) must contain SKILL.md.
pub fn validate_skills_layout(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let skills_dir = Path::new("skills");
    if !skills_dir.is_dir() {
        return;
    }

    let mut skill_count = 0;
    let mut excluded_count = 0;
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
        let skill_path = format!("skills/{name}/SKILL.md");
        if exclude.is_excluded(&skill_path) {
            excluded_count += 1;
            continue;
        }
        let skill_md = path.join("SKILL.md");
        if !skill_md.is_file() {
            diag.report(
                LintRule::SkillMdMissing,
                &format!("skills/{name}/ missing SKILL.md"),
            );
            continue;
        }
        skill_count += 1;
    }

    if skill_count == 0 && excluded_count == 0 {
        diag.report(
            LintRule::NoExportedSkills,
            "no plugin-exported skills found under skills/",
        );
    }
}

/// V6: Validate SKILL.md frontmatter for public skills (skills/*/SKILL.md).
pub fn validate_skill_frontmatter(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    validate_skill_frontmatter_in_dir("skills", true, diag, exclude);
}

/// V6-adapted: Validate SKILL.md frontmatter for private skills (.claude/skills/*/SKILL.md).
pub fn validate_private_skill_frontmatter(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    validate_skill_frontmatter_in_dir(".claude/skills", true, diag, exclude);
}

fn validate_skill_frontmatter_in_dir(
    base_dir: &str,
    check_name_match: bool,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
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
        if exclude.is_excluded(&skill_path) {
            continue;
        }
        let content = match fs::read_to_string(&skill_md) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let fm_lines = match frontmatter::extract_frontmatter(&content) {
            Some(lines) => lines,
            None => {
                diag.report(
                    LintRule::FrontmatterMalformed,
                    &format!(
                        "{skill_path}: malformed frontmatter (must start with '---' on line 1, must have closing '---')"
                    ),
                );
                // X002–X005 still apply to the markdown file when frontmatter is broken.
                super::markdown_structure::check_markdown_structure(&skill_path, &content, diag);
                continue;
            }
        };

        // X001: strict YAML parse; CC-SK-010: hooks schema when present.
        match frontmatter::parse_yaml_strict(&fm_lines) {
            Ok(yaml) => {
                if let Some(hooks) = yaml.get("hooks") {
                    super::hook_schema::validate_frontmatter_hooks(
                        hooks,
                        &format!("{skill_path} frontmatter"),
                        diag,
                    );
                }
            }
            Err((line, msg)) => {
                diag.report(
                    LintRule::FrontmatterYamlInvalid,
                    &format!("{skill_path}:{line}: frontmatter is not valid YAML: {msg}"),
                );
            }
        }

        // X002–X005: fence / XML structure on the full file.
        super::markdown_structure::check_markdown_structure(&skill_path, &content, diag);

        // S072: skill directory size limit.
        check_skill_dir_size(&path, &skill_path, diag);

        // S073: relative .md refs nested deeper than one level.
        check_skill_ref_depth(&skill_path, &content, diag);

        let name = frontmatter::get_field(&fm_lines, "name");
        let desc = frontmatter::get_field(&fm_lines, "description");

        if name.is_none() {
            diag.report(
                LintRule::FrontmatterFieldMissing,
                &format!("{skill_path}: missing required frontmatter field 'name'"),
            );
        }
        if desc.is_none() {
            diag.report(
                LintRule::FrontmatterFieldMissing,
                &format!("{skill_path}: missing required frontmatter field 'description'"),
            );
        }

        if check_name_match {
            if let Some(ref n) = name {
                if n != &dir_name {
                    diag.report(
                        LintRule::FrontmatterNameMismatch,
                        &format!(
                            "{skill_path}: frontmatter name '{n}' does not match directory '{dir_name}'"
                        ),
                    );
                }
            }
        }

        // Optional scalar fields: if present, must be non-empty.
        // List lives next to KNOWN_SKILL_FRONTMATTER_FIELDS in skill_content.
        for field in super::skill_content::OPTIONAL_NONEMPTY_SCALAR_FIELDS {
            let prefix = format!("{field}:");
            let field_present = fm_lines.iter().any(|line| line.starts_with(&prefix));
            if field_present {
                let val = frontmatter::get_field(&fm_lines, field);
                if val.is_none() {
                    // For allowed-tools: suppress S007 if YAML list items follow (S045 handles that case)
                    if *field == "allowed-tools" {
                        let has_list_items = fm_lines
                            .iter()
                            .position(|l| l.starts_with("allowed-tools:"))
                            .is_some_and(|i| {
                                fm_lines[i + 1..]
                                    .iter()
                                    .take_while(|l| {
                                        l.is_empty()
                                            || l.starts_with(' ')
                                            || l.starts_with('\t')
                                            || l.starts_with("- ")
                                    })
                                    .any(|l| l.trim_start().starts_with("- "))
                            });
                        if has_list_items {
                            continue; // S045 in frontmatter_extended.rs handles this
                        }
                    }
                    diag.report(
                        LintRule::FrontmatterFieldEmpty,
                        &format!("{skill_path}: optional field '{field}' is present but empty"),
                    );
                }
            }
        }
    }
}

fn check_skill_dir_size(dir: &Path, skill_path: &str, diag: &mut DiagnosticCollector) {
    let mut total = 0u64;
    for entry in WalkDir::new(dir).into_iter().filter_map(Result::ok) {
        if entry.file_type().is_file() {
            if let Ok(meta) = entry.metadata() {
                total = total.saturating_add(meta.len());
            }
        }
    }
    if total > SKILL_DIR_SIZE_LIMIT {
        diag.report(
            LintRule::SkillDirOversized,
            &format!(
                "{skill_path}: skill directory exceeds 8MB platform upload limit ({total} bytes)"
            ),
        );
    }
}

fn check_skill_ref_depth(skill_path: &str, content: &str, diag: &mut DiagnosticCollector) {
    let body = frontmatter::extract_body(content);
    for caps in RE_RELATIVE_MD_LINK.captures_iter(body) {
        let target = caps.get(1).map(|m| m.as_str()).unwrap_or("");
        if target.starts_with("http://")
            || target.starts_with("https://")
            || target.starts_with('/')
            || target.contains("${CLAUDE_PLUGIN_ROOT}")
        {
            continue;
        }
        let depth = target
            .split('/')
            .filter(|p| !p.is_empty() && *p != ".")
            .count();
        // One nesting level = dir/file.md (2 components). Deeper is flagged.
        if depth > 2 {
            diag.report(
                LintRule::SkillRefNested,
                &format!(
                    "{skill_path}: skill file reference '{target}' is nested deeper than one level"
                ),
            );
        }
    }
}

/// V15: Validate shared markdown reference integrity.
/// Every `${CLAUDE_PLUGIN_ROOT}/skills/shared/**/*.md` path referenced from
/// `skills/*/SKILL.md` must exist on disk.
pub fn validate_shared_md_references(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let skills_dir = Path::new("skills");
    if !skills_dir.is_dir() {
        return;
    }

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

        let skill_path = format!("skills/{dir_name}/SKILL.md");
        if exclude.is_excluded(&skill_path) {
            continue;
        }

        let content = match fs::read_to_string(&skill_md) {
            Ok(c) => c,
            Err(_) => continue,
        };

        for cap in RE_SHARED_MD_REF.find_iter(&content) {
            let reference = cap.as_str();
            let rel = reference.replace("${CLAUDE_PLUGIN_ROOT}/", "");
            if !Path::new(&rel).is_file() {
                diag.report(
                    LintRule::SharedMdMissing,
                    &format!(
                        "shared markdown reference missing on disk: {reference} (in {skill_path}, expected {rel})"
                    ),
                );
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

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skills_layout(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v5_missing_skills_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skills_layout(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v5_missing_skill_md() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        // No SKILL.md file

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skills_layout(&mut diag, &crate::config::ExcludeSet::default());
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

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skills_layout(&mut diag, &crate::config::ExcludeSet::default());
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

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
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

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
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

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
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

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
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

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_private_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
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

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_private_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.error_count() >= 1);
        assert!(diag.errors().iter().any(|e| e.contains("description")));
    }

    #[test]
    #[serial_test::serial]
    fn test_v6a_no_private_skills_dir_silent() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_private_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
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

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_shared_md_references(&mut diag, &crate::config::ExcludeSet::default());
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

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_shared_md_references(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("missing on disk"));
    }

    #[test]
    #[serial_test::serial]
    fn test_v15_subdirectory_shared_reference() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/shared/sub").unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write("skills/shared/sub/util.md", "# Util\n").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: s\n---\nSee ${CLAUDE_PLUGIN_ROOT}/skills/shared/sub/util.md\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_shared_md_references(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v15_missing_subdirectory_shared_reference() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: s\n---\nSee ${CLAUDE_PLUGIN_ROOT}/skills/shared/sub/missing.md\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_shared_md_references(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("missing on disk"));
    }

    #[test]
    #[serial_test::serial]
    fn test_x001_invalid_yaml_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        // Tab indentation is invalid YAML.
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\n\tdescription: bad\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.diagnostics()
                .iter()
                .any(|d| d.rule == LintRule::FrontmatterYamlInvalid),
            "expected X001: {:?}",
            diag.diagnostics()
                .iter()
                .map(|d| d.message.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_cc_sk_010_skill_hooks_schema() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A skill for testing hooks\nhooks:\n  NotAnEvent:\n    - hooks:\n        - type: command\n          command: echo hi\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.diagnostics()
                .iter()
                .any(|d| d.rule == LintRule::HookEventInvalid && d.message.contains("frontmatter")),
            "expected H008 on skill frontmatter: {:?}",
            diag.diagnostics()
                .iter()
                .map(|d| format!("{}:{}", d.rule.code(), d.message))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_x002_unclosed_fence_in_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A skill for fence testing\n---\n```bash\necho hi\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.diagnostics()
                .iter()
                .any(|d| d.rule == LintRule::UnclosedCodeFence)
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s073_deep_relative_md_link() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A skill for depth testing\n---\nSee [deep](refs/deep/nested/file.md)\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.diagnostics()
                .iter()
                .any(|d| d.rule == LintRule::SkillRefNested)
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s006_private_skill_name_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/skills/my-skill").unwrap();
        std::fs::write(
            ".claude/skills/my-skill/SKILL.md",
            "---\nname: other-name\ndescription: A skill for basic mode\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_private_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.diagnostics()
                .iter()
                .any(|d| d.rule == LintRule::FrontmatterNameMismatch)
        );
    }
}
