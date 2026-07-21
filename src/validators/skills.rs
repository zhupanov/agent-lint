use crate::config::ExcludeSet;
use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata};
use crate::frontmatter;
use crate::live_instructions::{InstructionSurfaceKind, LiveInstructionDocument};
use crate::markdown::MarkdownDocument;
use crate::rules::LintRule;
use crate::traversal;
use regex::Regex;
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

static RE_SHARED_MD_REF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\$\{CLAUDE_PLUGIN_ROOT\}/skills/shared/[a-zA-Z0-9._/-]+\.md").unwrap()
});

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
    /// Shared Markdown facts for this file. Content validators must consume
    /// this rather than parse the body again.
    pub document: MarkdownDocument,
    /// Whether the skill directory contains a non-empty `scripts/` subdirectory.
    pub has_scripts_dir: bool,
}

/// Walk a skills directory and collect SkillInfo for each valid skill.
/// Skips `shared/` subdirectory and excluded paths. Returns empty vec if dir doesn't exist.
pub fn collect_skills(base_dir: &str, exclude: &ExcludeSet) -> Vec<SkillInfo> {
    let dir = Path::new(base_dir);
    let subdirs = traversal::shallow_directories(dir, Path::new("."), None)
        .entries
        .into_iter()
        .filter_map(|entry| {
            let name = entry.path.file_name()?.to_str()?.to_string();
            (name != "shared" && !exclude.is_excluded(&format!("{base_dir}/{name}/SKILL.md")))
                .then_some((entry.path, name))
        })
        .collect::<Vec<_>>();

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
        let document = MarkdownDocument::parse(content);

        let fm_lines = match document.frontmatter() {
            Some(lines) => lines.to_vec(),
            None => continue, // S004 fires from existing validator
        };

        let body = document.body().to_string();
        let skill_path = format!("{base_dir}/{dir_name}/SKILL.md");

        let scripts_dir = path.join("scripts");
        let has_scripts_dir = !traversal::shallow_entries(&scripts_dir, Path::new("."), None)
            .entries
            .is_empty();

        skills.push(SkillInfo {
            path: skill_path,
            dir_name,
            fm_lines,
            body,
            document,
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
    for entry in traversal::shallow_directories(skills_dir, Path::new("."), None).entries {
        let path = entry.path;
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
            diag.report_at(
                LintRule::SkillMdMissing,
                &skill_path,
                &format!("skills/{name}/ missing SKILL.md"),
            );
            continue;
        }
        skill_count += 1;
    }

    if skill_count == 0 && excluded_count == 0 {
        diag.report_at(
            LintRule::NoExportedSkills,
            skills_dir,
            "no plugin-exported skills found under skills/",
        );
    }
}

/// V6: Validate SKILL.md frontmatter for public skills (skills/*/SKILL.md).
pub fn validate_skill_frontmatter(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    validate_skill_frontmatter_in_dir("skills", true, false, diag, exclude, None);
}

/// V6-adapted: Validate SKILL.md frontmatter for private skills (.claude/skills/*/SKILL.md).
pub fn validate_private_skill_frontmatter(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    validate_skill_frontmatter_in_dir(".claude/skills", true, false, diag, exclude, None);
}

/// Validate frontmatter and prompt content for cross-client skills in
/// `.agents/skills/`.
pub(crate) fn validate_agent_skill_frontmatter_with_prompt_pass(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    prompt_pass: &mut super::prompt_content::PromptContentPass,
) {
    validate_skill_frontmatter_in_dir(
        ".agents/skills",
        true,
        true,
        diag,
        exclude,
        Some(prompt_pass),
    );
}

fn validate_skill_frontmatter_in_dir(
    base_dir: &str,
    check_name_match: bool,
    platform_neutral: bool,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    mut prompt_pass: Option<&mut super::prompt_content::PromptContentPass>,
) {
    let dir = Path::new(base_dir);
    if !dir.is_dir() {
        return;
    }

    for entry in traversal::shallow_directories(dir, Path::new("."), None).entries {
        let path = entry.path;
        let dir_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        // `skills/shared` is plugin documentation rather than a runnable
        // skill. `.agents/skills/shared`, however, is a valid shared-agent
        // skill and must remain eligible for prompt analysis.
        if dir_name == "shared" && !platform_neutral {
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
        let document = MarkdownDocument::parse(content);
        if let Some(prompt_pass) = prompt_pass.as_deref_mut() {
            let prompt_document = LiveInstructionDocument::new(
                Path::new(&skill_path),
                InstructionSurfaceKind::Skill,
                &document,
            );
            prompt_pass.validate(&prompt_document, diag);
        }

        let fm_lines = match document.frontmatter() {
            Some(lines) => lines,
            None => {
                diag.report_at(
                    LintRule::FrontmatterMalformed,
                    &skill_path,
                    &format!(
                        "{skill_path}: malformed frontmatter (must start with '---' on line 1, must have closing '---')"
                    ),
                );
                // X002–X005 still apply to the markdown file when frontmatter is broken.
                super::markdown_structure::check_markdown_document(&skill_path, &document, diag);
                continue;
            }
        };

        // X001: strict YAML parse; CC-SK-010: hooks schema when present.
        let parsed_frontmatter = match frontmatter::parse_yaml_strict(fm_lines) {
            Ok(yaml) => {
                if !platform_neutral && let Some(hooks) = yaml.get("hooks") {
                    diag.with_subject_path(&skill_path, |diag| {
                        super::hook_schema::validate_frontmatter_hooks(
                            hooks,
                            &format!("{skill_path} frontmatter"),
                            diag,
                        );
                    });
                }
                Some(yaml)
            }
            Err((line, msg)) => {
                diag.report_at_with(
                    LintRule::FrontmatterYamlInvalid,
                    &skill_path,
                    &format!("{skill_path}:{line}: frontmatter is not valid YAML: {msg}"),
                    DiagnosticMetadata::at_line(line),
                );
                None
            }
        };

        // X002–X005: fence / XML structure on the full file.
        super::markdown_structure::check_markdown_document(&skill_path, &document, diag);

        if !platform_neutral {
            // S072: skill directory size limit.
            check_skill_dir_size(&path, &skill_path, diag);

            // S073: relative .md refs nested deeper than one level.
            check_skill_ref_depth(&skill_path, &document, diag);
        }

        let raw_name = frontmatter::get_field(fm_lines, "name");
        let raw_desc = frontmatter::get_field(fm_lines, "description");
        let canonical_name = parsed_frontmatter
            .as_ref()
            .and_then(crate::yaml::Value::as_mapping)
            .and_then(|mapping| mapping.get("name"))
            .and_then(crate::yaml::Value::as_str)
            .filter(|name| !name.is_empty());
        let canonical_desc = parsed_frontmatter
            .as_ref()
            .and_then(crate::yaml::Value::as_mapping)
            .and_then(|mapping| mapping.get("description"))
            .and_then(crate::yaml::Value::as_str)
            .filter(|description| !description.is_empty());
        // Invalid YAML is already reported by X001. For a valid document,
        // require canonical non-empty string scalars rather than treating YAML
        // syntax as a field value.
        let name_is_valid = parsed_frontmatter
            .as_ref()
            .map_or_else(|| raw_name.is_some(), |_| canonical_name.is_some());
        let desc_is_valid = parsed_frontmatter
            .as_ref()
            .map_or_else(|| raw_desc.is_some(), |_| canonical_desc.is_some());

        if !name_is_valid {
            diag.report_at(
                LintRule::FrontmatterFieldMissing,
                &skill_path,
                &format!(
                    "{skill_path}: required frontmatter field 'name' is missing or not a non-empty string"
                ),
            );
        }
        if !desc_is_valid {
            diag.report_at(
                LintRule::FrontmatterFieldMissing,
                &skill_path,
                &format!(
                    "{skill_path}: required frontmatter field 'description' is missing or not a non-empty string"
                ),
            );
        }

        if check_name_match {
            if let Some(n) = canonical_name {
                if n != dir_name {
                    diag.report_at(
                        LintRule::FrontmatterNameMismatch,
                        &skill_path,
                        &format!(
                            "{skill_path}: frontmatter name '{n}' does not match directory '{dir_name}'"
                        ),
                    );
                }
            }
        }

        // Optional scalar fields: if present, must be non-empty.
        // List lives next to KNOWN_SKILL_FRONTMATTER_FIELDS in skill_content.
        if platform_neutral {
            continue;
        }
        for field in super::skill_content::OPTIONAL_NONEMPTY_SCALAR_FIELDS {
            let prefix = format!("{field}:");
            let field_present = fm_lines.iter().any(|line| line.starts_with(&prefix));
            if field_present {
                let val = frontmatter::get_field(fm_lines, field);
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
                    diag.report_at(
                        LintRule::FrontmatterFieldEmpty,
                        &skill_path,
                        &format!("{skill_path}: optional field '{field}' is present but empty"),
                    );
                }
            }
        }
    }
}

fn check_skill_dir_size(dir: &Path, skill_path: &str, diag: &mut DiagnosticCollector) {
    let mut total = 0u64;
    for entry in traversal::recursive_files(dir, Path::new("."), None).entries {
        if let Ok(meta) = entry.path.metadata() {
            total = total.saturating_add(meta.len());
        }
    }
    if total > SKILL_DIR_SIZE_LIMIT {
        diag.report_at(
            LintRule::SkillDirOversized,
            skill_path,
            &format!(
                "{skill_path}: skill directory exceeds 8MB platform upload limit ({total} bytes)"
            ),
        );
    }
}

fn check_skill_ref_depth(
    skill_path: &str,
    document: &MarkdownDocument,
    diag: &mut DiagnosticCollector,
) {
    for link in document.links() {
        let target = link.destination.as_str();
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
            diag.report_at(
                LintRule::SkillRefNested,
                skill_path,
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

    for entry in traversal::shallow_directories(skills_dir, Path::new("."), None).entries {
        let path = entry.path;
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
                diag.report_at(
                    LintRule::SharedMdMissing,
                    &skill_path,
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
    fn name_directory_match_uses_the_canonical_yaml_scalar() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".agents/skills/valid-name").unwrap();
        std::fs::write(
            ".agents/skills/valid-name/SKILL.md",
            "---\nname: valid-name # comment\ndescription: A skill\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agent_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.diagnostics()
                .iter()
                .all(|item| item.rule != LintRule::FrontmatterNameMismatch)
        );
    }

    #[test]
    #[serial_test::serial]
    fn non_string_name_is_owned_by_frontmatter_without_name_format_findings() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".agents/skills/example").unwrap();
        std::fs::write(
            ".agents/skills/example/SKILL.md",
            "---\nname: [not-a-scalar]\ndescription: A skill\n---\nBody\n",
        )
        .unwrap();

        let exclude = crate::config::ExcludeSet::default();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agent_skill_frontmatter(&mut diag, &exclude);
        crate::validators::skill_content::validate_agent_skills_name_contract(
            ".agents/skills",
            &mut diag,
            &exclude,
        );
        assert_eq!(
            diag.diagnostics()
                .iter()
                .filter(|item| item.rule == LintRule::FrontmatterFieldMissing)
                .count(),
            1
        );
        assert!(diag.diagnostics().iter().all(|item| {
            !matches!(
                item.rule,
                LintRule::NameTooLong | LintRule::NameInvalidChars | LintRule::NameBadHyphens
            )
        }));
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
        let diagnostic = diag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.rule == LintRule::FrontmatterYamlInvalid)
            .unwrap_or_else(|| {
                panic!(
                    "expected X001: {:?}",
                    diag.diagnostics()
                        .iter()
                        .map(|diagnostic| diagnostic.message.as_str())
                        .collect::<Vec<_>>()
                )
            });
        assert_eq!(
            diagnostic.location.map(|location| location.start()),
            Some(crate::diagnostic::SourcePosition::line(3))
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
