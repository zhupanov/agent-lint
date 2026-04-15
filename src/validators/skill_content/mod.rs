mod body;
mod cross_field;
mod cross_skill;
mod description;
mod frontmatter_extended;
mod frontmatter_fields;
mod mcp;
mod name;
mod security;

use crate::config::ExcludeSet;
use crate::diagnostic::DiagnosticCollector;
use crate::validators::skills::{SkillInfo, collect_skills};
use regex::Regex;
use std::sync::LazyLock;

// S022/S043: Backslash paths — shared between body.rs and frontmatter_extended.rs
pub(super) static RE_BACKSLASH_PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[A-Za-z]:\\[A-Za-z]|\\[A-Za-z][A-Za-z0-9_-]*\\[A-Za-z]").unwrap()
});

/// Validate skill content for public skills (skills/). Runs all S009-S057 rules.
pub fn validate_skill_content(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let skills = collect_skills("skills", exclude);
    for info in &skills {
        run_content_checks(info, true, diag, exclude);
    }
    // Cross-skill checks (plugin-only: S029, S036; both-mode: S030, S048)
    cross_skill::validate_nested_references("skills", &skills, diag);
    cross_skill::validate_orphaned_skill_files("skills", diag, exclude);
    cross_skill::validate_ref_no_toc("skills", &skills, diag);
    cross_skill::validate_generic_ref_names("skills", diag, exclude);
}

/// Validate skill content for private skills (.claude/skills/).
/// Runs only "both-mode" rules (excludes S015, S016, S017, S029, S033, S036, S037, S038, S046, S047, S049, S050, S051, S052, S053, S054, S055, S056, S057).
pub fn validate_private_skill_content(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let skills = collect_skills(".claude/skills", exclude);
    for info in &skills {
        run_content_checks(info, false, diag, exclude);
    }
    cross_skill::validate_orphaned_skill_files(".claude/skills", diag, exclude);
    cross_skill::validate_generic_ref_names(".claude/skills", diag, exclude);
}

fn run_content_checks(
    info: &SkillInfo,
    plugin_mode: bool,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
) {
    name::check_name_format(info, plugin_mode, diag);
    description::check_description_quality(info, plugin_mode, diag);
    body::check_body_content(info, plugin_mode, diag, exclude);
    frontmatter_fields::check_frontmatter_fields(info, diag);
    frontmatter_extended::check_frontmatter_extended(info, diag);
    cross_field::check_cross_field(info, plugin_mode, diag);
    security::check_content_security(info, diag);
    mcp::check_mcp_tool_refs(info, diag);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::DiagnosticCollector;

    // ── S009: name-too-long ──────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s009_name_within_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\nBody content\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("exceeds 64")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s009_name_too_long() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let long_name = "a".repeat(65);
        std::fs::create_dir_all(format!("skills/{long_name}")).unwrap();
        std::fs::write(
            format!("skills/{long_name}/SKILL.md"),
            format!(
                "---\nname: {long_name}\ndescription: A valid skill description here\n---\nBody\n"
            ),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("exceeds 64")));
    }

    // ── S010: name-invalid-chars ─────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s010_valid_name() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill-123").unwrap();
        std::fs::write(
            "skills/my-skill-123/SKILL.md",
            "---\nname: my-skill-123\ndescription: A valid skill description here\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("outside [a-z0-9-]"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s010_uppercase_name() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: My-Skill\ndescription: A valid skill description here\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("outside [a-z0-9-]"))
        );
    }

    // ── S011: name-bad-hyphens ───────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s011_consecutive_hyphens() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my--skill\ndescription: A valid skill description here\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("consecutive hyphens"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s011_leading_hyphen() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: -my-skill\ndescription: Use when testing hyphen rules\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("starts/ends with hyphen"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s011_trailing_hyphen() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill-\ndescription: Use when testing hyphen rules\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("starts/ends with hyphen"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s011_valid_hyphens_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-good-skill\ndescription: Use when testing hyphen rules\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag
            .errors()
            .iter()
            .any(|e| e.contains("starts/ends with hyphen") || e.contains("consecutive hyphens")));
    }

    // ── S012: name-reserved-word ─────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s012_reserved_word() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: claude-helper\ndescription: A valid skill description here\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("reserved word")));
    }

    // ── S013: name-has-xml ──────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s013_name_with_xml_tag() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-<tag>skill\ndescription: A valid skill description here\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("name") && e.contains("XML/HTML tags"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s013_name_without_xml_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\nBody content\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("XML/HTML tags") && e.contains("name"))
        );
    }

    // ── S014: desc-too-long ──────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s014_desc_too_long() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let long_desc = "x".repeat(1025);
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: {long_desc}\n---\nBody\n"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("exceeds 1024")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s014_multibyte_chars_count_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        // 1025 CJK characters (3 bytes each) = 3075 bytes but only 1025 chars
        let desc = "\u{4e00}".repeat(1025);
        assert_eq!(desc.chars().count(), 1025);
        assert!(desc.len() > 1025);
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: {desc}\n---\nBody\n"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("exceeds 1024")));
    }

    // ── S015: desc-truncated ─────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s015_desc_truncated_in_plugin_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let long_desc = format!("Use when you need {}", "x".repeat(240));
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: {long_desc}\n---\nBody content\n"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("truncated")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s015_desc_250_chars_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        // "Use when the task needs " = 24 chars + 226 x's = exactly 250 chars
        let desc = format!("Use when the task needs {}", "x".repeat(226));
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: {desc}\n---\nBody content\n"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("truncated")));
    }

    // ── S016: desc-uses-person ───────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s016_desc_uses_you() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need to analyze code\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("first/second person"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s016_desc_third_person_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when the project needs code analysis and review\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("first/second person"))
        );
    }

    // ── S017: desc-no-trigger ────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s017_desc_no_trigger_context() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A skill that does things with code\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("trigger")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s017_desc_with_trigger_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when the project needs analysis\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("trigger")));
    }

    // ── S018: desc-has-xml ───────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s018_desc_with_xml() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when <b>important</b> tasks need doing\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("description") && e.contains("XML"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s018_desc_without_xml_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when important tasks need doing well\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("description") && e.contains("XML"))
        );
    }

    // ── S019: body-too-long ──────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s019_body_too_long() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let body = "line\n".repeat(501);
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!(
                "---\nname: my-skill\ndescription: A valid skill description here\n---\n{body}"
            ),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("exceeds 500 lines"))
        );
    }

    // ── S020: body-empty ─────────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s020_body_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("no content after frontmatter"))
        );
    }

    // ── S021: consecutive-bash ───────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s021_consecutive_bash_blocks() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n\n```bash\necho hello\n```\n\n```bash\necho world\n```\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("consecutive bash")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s021_bash_blocks_with_prose_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n\n```bash\necho hello\n```\n\nThen run the second command:\n\n```bash\necho world\n```\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("consecutive bash")));
    }

    // ── S022: backslash-path ────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s022_windows_path_in_body() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need path validation\n---\nUse the file at C:\\Users\\admin\\file.txt\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("backslash")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s022_forward_slash_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need path validation\n---\nUse the file at /Users/admin/file.txt\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("backslash")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s022_regex_escape_not_flagged() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need regex validation\n---\nUse regex like \\s and \\n to match patterns\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("backslash")));
    }

    // ── S023: bool-field-invalid ─────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s023_invalid_bool() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\nuser-invocable: yes\n---\nBody\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("must be true or false"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s023_valid_bool() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\nuser-invocable: true\n---\nBody\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("must be true or false"))
        );
    }

    // ── S024: context-field-invalid ─────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s024_invalid_context() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need context testing\ncontext: invalid\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("context") && e.contains("fork"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s024_valid_context_fork() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need context testing\ncontext: fork\n---\nRun the analysis.\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("context") && e.contains("must be"))
        );
    }

    // ── S025: effort-field-invalid ───────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s025_invalid_effort() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need effort testing\neffort: extreme\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("effort") && e.contains("low/medium/high/max"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s025_valid_effort() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need effort testing\neffort: high\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("effort")));
    }

    // ── S026: shell-field-invalid ────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s026_invalid_shell() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need shell testing\nshell: zsh\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("shell") && e.contains("bash/powershell"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s026_valid_shell() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need shell testing\nshell: bash\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("shell") && e.contains("must be"))
        );
    }

    // ── S027: skill-unreachable ──────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s027_unreachable_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\ndisable-model-invocation: true\nuser-invocable: false\n---\nBody\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("unreachable")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s027_reachable_skill_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when testing reachability\ndisable-model-invocation: true\nuser-invocable: true\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("unreachable")));
    }

    // ── S028: args-no-hint ───────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s028_args_without_hint() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\nUse $ARGUMENTS as input\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("$ARGUMENTS") && e.contains("argument-hint"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s028_args_with_hint_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\nargument-hint: <feature>\n---\nUse $ARGUMENTS as input\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("argument-hint")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s028_args_in_code_fence_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        // $ARGUMENTS only inside a code fence -- should NOT trigger S028
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\nSome body text\n\n```bash\necho $ARGUMENTS\n```\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag.errors().iter().any(|e| e.contains("argument-hint")),
            "$ARGUMENTS inside code fence should not trigger S028"
        );
    }

    // ── S031: non-https-url ──────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s031_http_url() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\nFetch from http://api.example.net/data\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("non-HTTPS")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s031_localhost_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\nFetch from http://localhost:8080/data\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("non-HTTPS")));
    }

    // ── S029: nested-ref-deep ───────────────────────────────────────

    #[test]
    fn test_shared_ref_regex_uses_base_dir() {
        let re = cross_skill::shared_ref_regex("skills");
        assert!(re.is_match("${CLAUDE_PLUGIN_ROOT}/skills/shared/helpers.md"));
        assert!(re.is_match("${CLAUDE_PLUGIN_ROOT}/skills/shared/sub/util.md"));
        assert!(!re.is_match("${CLAUDE_PLUGIN_ROOT}/other/shared/helpers.md"));

        let re2 = cross_skill::shared_ref_regex(".claude/skills");
        assert!(re2.is_match("${CLAUDE_PLUGIN_ROOT}/.claude/skills/shared/helpers.md"));
        assert!(!re2.is_match("${CLAUDE_PLUGIN_ROOT}/skills/shared/helpers.md"));
    }

    #[test]
    #[serial_test::serial]
    fn test_s029_nested_reference_fires() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        // Create a shared .md that itself references another shared .md
        std::fs::write(
            "skills/shared/level1.md",
            "# Level 1\nSee ${CLAUDE_PLUGIN_ROOT}/skills/shared/level2.md for details\n",
        )
        .unwrap();
        std::fs::write("skills/shared/level2.md", "# Level 2\nContent\n").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing\n---\nRefer to ${CLAUDE_PLUGIN_ROOT}/skills/shared/level1.md\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("itself references"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s029_flat_reference_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/shared/flat.md",
            "# Flat\nNo nested references here\n",
        )
        .unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing\n---\nRefer to ${CLAUDE_PLUGIN_ROOT}/skills/shared/flat.md\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("itself references"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s029_multi_skill_same_nested_ref() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::create_dir_all("skills/skill-a").unwrap();
        std::fs::create_dir_all("skills/skill-b").unwrap();
        std::fs::write(
            "skills/shared/nested.md",
            "# Nested\nSee ${CLAUDE_PLUGIN_ROOT}/skills/shared/other.md\n",
        )
        .unwrap();
        std::fs::write("skills/shared/other.md", "# Other\n").unwrap();
        std::fs::write(
            "skills/skill-a/SKILL.md",
            "---\nname: skill-a\ndescription: Use when you need skill A for testing\n---\nRef ${CLAUDE_PLUGIN_ROOT}/skills/shared/nested.md\n",
        ).unwrap();
        std::fs::write(
            "skills/skill-b/SKILL.md",
            "---\nname: skill-b\ndescription: Use when you need skill B for testing\n---\nRef ${CLAUDE_PLUGIN_ROOT}/skills/shared/nested.md\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        // Both skills reference the same nested shared file -- S029 should fire for each
        let errors = diag.errors();
        let nested_count = errors
            .iter()
            .filter(|e| e.contains("itself references"))
            .count();
        assert_eq!(nested_count, 2);
    }

    #[test]
    #[serial_test::serial]
    fn test_s036_multi_skill_deduplicates_per_file() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::create_dir_all("skills/skill-a").unwrap();
        std::fs::create_dir_all("skills/skill-b").unwrap();
        // Create a large shared .md without headings (>100 lines)
        let long_content = "line\n".repeat(101);
        std::fs::write("skills/shared/big.md", &long_content).unwrap();
        std::fs::write(
            "skills/skill-a/SKILL.md",
            "---\nname: skill-a\ndescription: Use when you need skill A for testing\n---\nRef ${CLAUDE_PLUGIN_ROOT}/skills/shared/big.md\n",
        ).unwrap();
        std::fs::write(
            "skills/skill-b/SKILL.md",
            "---\nname: skill-b\ndescription: Use when you need skill B for testing\n---\nRef ${CLAUDE_PLUGIN_ROOT}/skills/shared/big.md\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        // S036 should fire once per unique file, not once per referencing skill
        let errors = diag.errors();
        let toc_count = errors
            .iter()
            .filter(|e| e.contains("no ## headings"))
            .count();
        assert_eq!(toc_count, 1);
    }

    // ── S030: orphaned-skill-files ───────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s030_orphaned_script() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts").unwrap();
        std::fs::write("skills/my-skill/scripts/orphan.sh", "#!/bin/bash\n").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\nNo script refs\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("not referenced")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s030_referenced_script_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts").unwrap();
        std::fs::write("skills/my-skill/scripts/helper.sh", "#!/bin/bash\n").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\nRun helper.sh\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("not referenced")));
    }

    // ── S032: hardcoded-secret ──────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s032_openai_key_pattern() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need secret detection testing\n---\nSet key to sk-aBcDeFgHiJkLmNoPqRsT1234\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("hardcoded secret")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s032_github_token_pattern() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need secret detection testing\n---\nToken is ghp_abcdefghijklmnopqrstuvwxyz1234567890\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("hardcoded secret")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s032_no_secrets_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need secret detection testing\n---\nUse the $API_KEY environment variable for authentication\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("hardcoded secret")));
    }

    // ── S033: name-vague ─────────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s033_vague_name_helper() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/helper").unwrap();
        std::fs::write(
            "skills/helper/SKILL.md",
            "---\nname: helper\ndescription: Use when you need help with various tasks\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("vague")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s033_specific_name_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/code-review").unwrap();
        std::fs::write(
            "skills/code-review/SKILL.md",
            "---\nname: code-review\ndescription: Use when code changes need thorough review\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("vague")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s033_private_mode_skips() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude/skills/helper").unwrap();
        std::fs::write(
            ".claude/skills/helper/SKILL.md",
            "---\nname: helper\ndescription: A valid skill description here\n---\nBody content\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_private_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        // S033 is plugin-only, should not fire in private mode
        assert!(!diag.errors().iter().any(|e| e.contains("vague")));
    }

    // ── S034: desc-too-short ─────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s034_desc_too_short() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Short\n---\nBody content\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("under 20")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s034_multibyte_chars_count_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        // 19 CJK characters (3 bytes each) = 57 bytes but only 19 chars
        let desc = "\u{4e00}".repeat(19);
        assert_eq!(desc.chars().count(), 19);
        assert!(desc.len() > 19);
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: {desc}\n---\nBody\n"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("under 20")));
    }

    // ── Private skill (basic mode) excludes plugin-only rules ────────

    #[test]
    #[serial_test::serial]
    fn test_private_skill_skips_plugin_only_rules() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude/skills/my-skill").unwrap();
        // This description uses "you" (would trigger S016 in plugin mode) and is >250 chars
        let long_desc = format!("Use when you need to {}", "x".repeat(250));
        std::fs::write(
            ".claude/skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: {long_desc}\n---\nBody content\n"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_private_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        // S016 (person) and S015 (truncated) should NOT fire in basic mode
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("first/second person"))
        );
        assert!(!diag.errors().iter().any(|e| e.contains("truncated")));
    }

    // ── Integration: mode dispatch ───────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_integration_plugin_mode_runs_all_rules() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        // Name with uppercase (S010) + uses "you" in desc (S016)
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: My-Skill\ndescription: I help you do things and more stuff here\n---\nBody content here\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        // Both S010 and S016 should fire in plugin mode
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("outside [a-z0-9-]"))
        );
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("first/second person"))
        );
    }

    // ── Integration: config round-tripping ───────────────────────────

    #[test]
    fn test_new_rules_lookup_by_code_and_name() {
        use crate::rules::LintRule;
        // Verify S009-S057 rules round-trip via code and name lookups
        let new_rules = [
            ("S009", "name-too-long"),
            ("S010", "name-invalid-chars"),
            ("S011", "name-bad-hyphens"),
            ("S012", "name-reserved-word"),
            ("S013", "name-has-xml"),
            ("S014", "desc-too-long"),
            ("S015", "desc-truncated"),
            ("S016", "desc-uses-person"),
            ("S017", "desc-no-trigger"),
            ("S018", "desc-has-xml"),
            ("S019", "body-too-long"),
            ("S020", "body-empty"),
            ("S021", "consecutive-bash"),
            ("S022", "backslash-path"),
            ("S023", "bool-field-invalid"),
            ("S024", "context-field-invalid"),
            ("S025", "effort-field-invalid"),
            ("S026", "shell-field-invalid"),
            ("S027", "skill-unreachable"),
            ("S028", "args-no-hint"),
            ("S029", "nested-ref-deep"),
            ("S030", "orphaned-skill-files"),
            ("S031", "non-https-url"),
            ("S032", "hardcoded-secret"),
            ("S033", "name-vague"),
            ("S034", "desc-too-short"),
            ("S035", "compat-too-long"),
            ("S036", "ref-no-toc"),
            ("S037", "body-no-refs"),
            ("S038", "time-sensitive"),
            ("S039", "metadata-not-string"),
            ("S040", "tools-unknown"),
            ("S041", "fork-no-task"),
            ("S042", "dmi-empty-desc"),
            ("S043", "frontmatter-backslash"),
            ("S044", "mcp-tool-unqualified"),
            ("S045", "tools-list-syntax"),
            ("S046", "body-no-workflow"),
            ("S047", "body-no-examples"),
            ("S048", "ref-name-generic"),
            ("S049", "name-not-gerund"),
            ("S050", "desc-vague-content"),
            ("S051", "script-deps-missing"),
            ("S052", "script-verify-missing"),
            ("S053", "terminology-inconsistent"),
            ("S054", "desc-body-misalign"),
            ("S055", "script-errhand-missing"),
            ("S056", "body-no-default"),
            ("S057", "magic-number-undoc"),
        ];
        for (code, name) in &new_rules {
            assert!(
                LintRule::from_code_or_name(code).is_some(),
                "Failed to look up rule by code: {code}"
            );
            assert!(
                LintRule::from_code_or_name(name).is_some(),
                "Failed to look up rule by name: {name}"
            );
            // Round-trip
            let rule = LintRule::from_code_or_name(code).unwrap();
            assert_eq!(rule.code(), *code);
            assert_eq!(rule.name(), *name);
        }
    }

    // ── S035: compat-too-long ────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s035_compat_too_long() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let long_compat = "x".repeat(501);
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: A valid skill description here\ncompatibility: {long_compat}\n---\nBody content\n"),
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("compatibility") && e.contains("500"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s035_compat_within_limit_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let compat = "x".repeat(500);
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: Use when testing compat limits\ncompatibility: {compat}\n---\nBody content\n"),
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("compatibility") && e.contains("500"))
        );
    }

    // ── S036: ref-no-toc ───────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s036_ref_no_toc() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        // Create a shared .md > 100 lines with no ## headings
        let long_content = "line\n".repeat(101);
        std::fs::write("skills/shared/big-ref.md", &long_content).unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\nSee ${CLAUDE_PLUGIN_ROOT}/skills/shared/big-ref.md\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("no ## headings")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s036_ref_with_headings_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let mut content = String::from("## Section 1\n");
        for _ in 0..100 {
            content.push_str("line\n");
        }
        std::fs::write("skills/shared/big-ref.md", &content).unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\nSee ${CLAUDE_PLUGIN_ROOT}/skills/shared/big-ref.md\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("no ## headings")));
    }

    // ── S037: body-no-refs ───────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s037_body_no_refs() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let body = "Some text without any file references\n".repeat(301);
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\n{body}"),
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("300 lines") && e.contains("file references"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s037_body_with_refs_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let mut body = "Some text\n".repeat(300);
        body.push_str("Run scripts/helper.sh to do something\n");
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\n{body}"),
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("300 lines") && e.contains("file references"))
        );
    }

    // ── S038: time-sensitive ─────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s038_time_sensitive() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\nThis expires after 2030 so plan accordingly.\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("date/year")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s038_year_in_code_fence_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\n\n```bash\necho 2030\n```\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("date/year")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s038_private_mode_skips() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude/skills/my-skill").unwrap();
        std::fs::write(
            ".claude/skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\nThis expires after 2030.\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_private_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("date/year")));
    }

    // ── S039: metadata-not-string ────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s039_metadata_bare_bool() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\nmetadata:\n  enabled: true\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("metadata") && e.contains("non-string"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s039_metadata_inline_value() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when testing metadata validation\nmetadata: true\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("metadata") && e.contains("non-string"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s039_metadata_quoted_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when testing metadata validation\nmetadata:\n  version: \"1.0\"\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("metadata") && e.contains("non-string"))
        );
    }

    // ── S040: tools-unknown ──────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s040_unknown_tool() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\nallowed-tools: Bash, Read, FakeToolXyz\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("FakeToolXyz")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s040_valid_tools() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\nallowed-tools: Bash, Read, Write, Grep, Glob\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("unrecognized tool"))
        );
    }

    // ── S041: fork-no-task ───────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s041_fork_no_task() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\ncontext: fork\n---\nThis is just guidelines about how to behave.\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("fork") && e.contains("task"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s041_fork_with_task_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\ncontext: fork\n---\nRun the analysis and generate a report.\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("fork") && e.contains("task"))
        );
    }

    // ── S042: dmi-empty-desc ─────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s042_dmi_empty_desc() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription:\ndisable-model-invocation: true\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("disable-model-invocation") && e.contains("empty"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s042_dmi_with_desc_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when the skill should be user-only\ndisable-model-invocation: true\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("disable-model-invocation") && e.contains("empty"))
        );
    }

    // ── S043: frontmatter-backslash ──────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s043_frontmatter_backslash() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\nargument-hint: C:\\Users\\file\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("backslash") && e.contains("frontmatter"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s043_forward_slash_frontmatter_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when testing frontmatter paths\nargument-hint: /usr/local/bin/tool\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("backslash") && e.contains("frontmatter"))
        );
    }

    // ── S046: body-no-workflow ─────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s046_body_no_workflow() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let body = "Some plain text without workflow structure\n".repeat(301);
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\n{body}"),
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("workflow structure"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s046_body_with_workflow_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let mut body = "Some text\n".repeat(300);
        body.push_str("## Steps\n");
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\n{body}"),
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("workflow structure"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s046_short_body_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\nShort body without workflow.\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("workflow structure"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s046_private_mode_skips() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude/skills/my-skill").unwrap();
        let body = "Some plain text without workflow\n".repeat(301);
        std::fs::write(
            ".claude/skills/my-skill/SKILL.md",
            format!(
                "---\nname: my-skill\ndescription: A valid skill description here\n---\n{body}"
            ),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_private_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("workflow structure"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s046_workflow_in_fence_not_counted() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let mut body = "Some text\n".repeat(295);
        body.push_str(
            "```\n## Steps\n- [ ] item\n**Step 1**\n1. first\n2. second\n3. third\n```\n",
        );
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\n{body}"),
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("workflow structure"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s046_numbered_list_workflow() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let mut body = "Some text\n".repeat(298);
        body.push_str("1. First step\n2. Second step\n3. Third step\n");
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\n{body}"),
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("workflow structure"))
        );
    }

    // ── S047: body-no-examples ──────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s047_body_no_examples() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let body = "Some plain text without examples\n".repeat(201);
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\n{body}"),
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("examples or templates"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s047_body_with_examples_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let mut body = "Some text\n".repeat(200);
        body.push_str("## Example\n");
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\n{body}"),
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("examples or templates"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s047_short_body_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\nShort body without examples.\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("examples or templates"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s047_private_mode_skips() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude/skills/my-skill").unwrap();
        let body = "Some plain text without examples\n".repeat(201);
        std::fs::write(
            ".claude/skills/my-skill/SKILL.md",
            format!(
                "---\nname: my-skill\ndescription: A valid skill description here\n---\n{body}"
            ),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_private_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("examples or templates"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s047_example_in_fence_not_counted() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let mut body = "Some text\n".repeat(196);
        body.push_str("```\n## Example\n**Input:**\n**Output:**\n## Usage\n```\n");
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\n{body}"),
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("examples or templates"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s046_s047_independence() {
        // 301-line body with examples but no workflow: only S046 fires
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let mut body = "Some text\n".repeat(300);
        body.push_str("## Example\n");
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\n{body}"),
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("workflow structure")),
            "S046 should fire"
        );
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("examples or templates")),
            "S047 should not fire"
        );
    }

    // ── S051: script-deps-missing ────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s051_script_with_deps_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts").unwrap();
        std::fs::write("skills/my-skill/scripts/run.sh", "#!/bin/bash\n").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n## Dependencies\n\npip install requests\n\nRun scripts/run.sh\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("dependency/package")),
            "S051 should not fire when deps keywords present"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s051_script_without_deps() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts").unwrap();
        std::fs::write("skills/my-skill/scripts/run.sh", "#!/bin/bash\n").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\nThis skill runs a script to do things.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("dependency/package")),
            "S051 should fire when no deps keywords"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s051_non_script_skill_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\nJust a plain skill with no scripts.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("dependency/package")),
            "S051 should not fire for non-script skill"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s051_private_mode_skips() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude/skills/my-skill/scripts").unwrap();
        std::fs::write(".claude/skills/my-skill/scripts/run.sh", "#!/bin/bash\n").unwrap();
        std::fs::write(
            ".claude/skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\nThis skill runs a script to do things.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_private_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("dependency/package")),
            "S051 should not fire in private mode"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s051_deps_in_code_fence_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts").unwrap();
        std::fs::write("skills/my-skill/scripts/run.sh", "#!/bin/bash\n").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\nRun the script.\n\n```bash\npip install requests\n```\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("dependency/package")),
            "S051 should not fire when deps keyword is inside code fence"
        );
    }

    // ── S052: script-verify-missing ───────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s052_script_with_verify_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts").unwrap();
        std::fs::write("skills/my-skill/scripts/run.sh", "#!/bin/bash\n").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n## Dependencies\n\npip install foo\n\n## Validation\n\nRun the script and verify the output.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("verification/validation")),
            "S052 should not fire when verify keywords present"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s052_script_without_verify() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts").unwrap();
        std::fs::write("skills/my-skill/scripts/run.sh", "#!/bin/bash\n").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n## Dependencies\n\npip install foo\n\nThis skill does stuff.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("verification/validation")),
            "S052 should fire when no verify keywords"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s052_non_script_skill_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\nJust a plain skill.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("verification/validation")),
            "S052 should not fire for non-script skill"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s052_private_mode_skips() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude/skills/my-skill/scripts").unwrap();
        std::fs::write(".claude/skills/my-skill/scripts/run.sh", "#!/bin/bash\n").unwrap();
        std::fs::write(
            ".claude/skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n## Dependencies\n\npip install foo\n\nThis skill does stuff.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_private_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("verification/validation")),
            "S052 should not fire in private mode"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s052_verify_in_code_fence_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts").unwrap();
        std::fs::write("skills/my-skill/scripts/run.sh", "#!/bin/bash\n").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n## Dependencies\n\npip install foo\n\n```bash\n# verify the output\necho 'done'\n```\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("verification/validation")),
            "S052 should not fire when verify keyword is inside code fence"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s051_md_only_body_ref_does_not_fire() {
        // A skill referencing only .md files should NOT be classified as script-backed
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\nSee ${CLAUDE_PLUGIN_ROOT}/skills/shared/helpers.md for details.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("dependency/package")),
            "S051 should not fire for skill referencing only .md files"
        );
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("verification/validation")),
            "S052 should not fire for skill referencing only .md files"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s051_s052_detected_via_body_ref() {
        // Script detected via body .sh reference (no scripts/ dir)
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\nRun setup.sh to configure the environment.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("dependency/package")),
            "S051 should fire for body .sh reference without deps"
        );
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("verification/validation")),
            "S052 should fire for body .sh reference without verify"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s051_s052_independence() {
        // Script-backed skill with deps but no verify: only S052 fires
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts").unwrap();
        std::fs::write("skills/my-skill/scripts/run.sh", "#!/bin/bash\n").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n## Dependencies\n\npip install foo\n\nThis does stuff.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("dependency/package")),
            "S051 should not fire when deps present"
        );
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("verification/validation")),
            "S052 should fire when no verify"
        );
    }

    // ── S054: desc-body-misalign ──────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s054_misaligned_desc_body_fires() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Kubernetes deployment scaling orchestration\n---\nThis skill handles testing and linting of source code.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("description keywords not reflected in body")),
            "S054 should fire when description keywords are absent from body"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s054_aligned_desc_body_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Extract PDF tables and merge documents\n---\nThis skill extracts tables from PDF files and merges documents together.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("description keywords not reflected in body")),
            "S054 should not fire when description keywords appear in body"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s054_short_description_skips() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Parse YAML\n---\nThis skill does something completely unrelated.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("description keywords not reflected in body")),
            "S054 should skip when description has fewer than 3 keywords"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s054_private_mode_skips() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude/skills/my-skill").unwrap();
        std::fs::write(
            ".claude/skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Kubernetes deployment scaling orchestration\n---\nThis skill handles testing and linting of source code.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_private_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("description keywords not reflected in body")),
            "S054 should not fire in private (both) mode"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s054_trigger_phrase_stripped() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need Kubernetes deployment scaling\n---\nThis skill manages Kubernetes deployment and scaling of pods.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("description keywords not reflected in body")),
            "S054 should strip trigger phrases before extracting keywords"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s054_exactly_three_keywords_runs() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        // "analyze" + "typescript" + "interfaces" = exactly 3 keywords
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Analyze TypeScript interfaces\n---\nThis skill analyzes TypeScript interfaces for correctness.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("description keywords not reflected in body")),
            "S054 should not fire when keywords are aligned (exactly 3 keywords at MIN_KEYWORDS boundary)"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s054_empty_body_no_fire() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Kubernetes deployment scaling orchestration\n---\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("description keywords not reflected in body")),
            "S054 should not fire when body is empty (S020 handles that)"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Boundary tests
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    #[serial_test::serial]
    fn test_s009_boundary_64_chars_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let name64 = "a".repeat(64);
        std::fs::create_dir_all(format!("skills/{name64}")).unwrap();
        std::fs::write(
            format!("skills/{name64}/SKILL.md"),
            format!(
                "---\nname: {name64}\ndescription: Use when testing name length boundary\n---\nBody\n"
            ),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("exceeds 64")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s014_boundary_1024_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        // "Use when testing " = 17 chars + 1007 x's = exactly 1024
        let desc = format!("Use when testing {}", "x".repeat(1007));
        assert_eq!(desc.chars().count(), 1024);
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: {desc}\n---\nBody\n"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("exceeds 1024")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s019_boundary_500_lines_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let body = "line\n".repeat(500);
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!(
                "---\nname: my-skill\ndescription: Use when testing body length boundary\n---\n{body}"
            ),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("exceeds 500")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s020_non_empty_body_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when testing body presence\n---\nHas body content\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("no content after frontmatter"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s034_boundary_20_chars_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        // exactly 20 characters
        let desc = "Use when needed now!";
        assert_eq!(desc.chars().count(), 20);
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: {desc}\n---\nBody\n"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("under 20")));
    }

    // ═══════════════════════════════════════════════════════════════════
    // collect_skills edge cases
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    #[serial_test::serial]
    fn test_collect_skills_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills").unwrap();
        let skills = collect_skills("skills", &crate::config::ExcludeSet::default());
        assert!(skills.is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn test_collect_skills_missing_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let skills = collect_skills("skills", &crate::config::ExcludeSet::default());
        assert!(skills.is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn test_collect_skills_skips_malformed_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/good-skill").unwrap();
        std::fs::create_dir_all("skills/bad-skill").unwrap();
        std::fs::write(
            "skills/good-skill/SKILL.md",
            "---\nname: good-skill\ndescription: A valid skill\n---\nBody\n",
        )
        .unwrap();
        // Malformed: no closing ---
        std::fs::write(
            "skills/bad-skill/SKILL.md",
            "---\nname: bad-skill\nno closing\n",
        )
        .unwrap();
        let skills = collect_skills("skills", &crate::config::ExcludeSet::default());
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].dir_name, "good-skill");
    }

    #[test]
    #[serial_test::serial]
    fn test_collect_skills_skips_shared() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill\n---\nBody\n",
        )
        .unwrap();
        std::fs::write("skills/shared/helpers.md", "# Helpers\n").unwrap();
        let skills = collect_skills("skills", &crate::config::ExcludeSet::default());
        assert_eq!(skills.len(), 1);
    }

    #[test]
    #[serial_test::serial]
    fn test_collect_skills_populates_body() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill\n---\nBody content here\n",
        )
        .unwrap();
        let skills = collect_skills("skills", &crate::config::ExcludeSet::default());
        assert_eq!(skills.len(), 1);
        assert!(skills[0].body.contains("Body content here"));
        assert!(!skills[0].body.contains("---"));
    }

    // ═══════════════════════════════════════════════════════════════════
    // Config integration tests
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    #[serial_test::serial]
    fn test_config_ignore_suppresses_new_rule() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        // Body empty (S020) + desc too short (S034). Use trigger context to avoid S017.
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when short\n---\n",
        )
        .unwrap();

        // Without config: S020 and S034 should fire
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("no content")));
        assert!(diag.errors().iter().any(|e| e.contains("under 20")));

        // With config ignoring S020
        use crate::rules::LintRule;
        let config = crate::config::LintConfig {
            ignore: std::collections::HashSet::from([LintRule::BodyEmpty]),
            warn: std::collections::HashSet::new(),
            exclude: vec![],
        };
        let mut diag2 = DiagnosticCollector::with_config(config);
        validate_skill_content(&mut diag2, &crate::config::ExcludeSet::default());
        // S020 suppressed, S034 still fires
        assert!(!diag2.errors().iter().any(|e| e.contains("no content")));
        assert!(diag2.errors().iter().any(|e| e.contains("under 20")));
        assert_eq!(
            diag2.suppressed_count(),
            1,
            "S020 should be counted as suppressed"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_config_warn_downgrades_new_rule() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when short\n---\n",
        )
        .unwrap();

        use crate::rules::LintRule;
        let config = crate::config::LintConfig {
            ignore: std::collections::HashSet::new(),
            warn: std::collections::HashSet::from([LintRule::DescTooShort]),
            exclude: vec![],
        };
        let mut diag = DiagnosticCollector::with_config(config);
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        // S034 downgraded to warning, not counted as error
        assert!(!diag.errors().iter().any(|e| e.contains("under 20")));
        assert!(diag.warnings().iter().any(|e| e.contains("under 20")));
    }

    // ═══════════════════════════════════════════════════════════════════
    // End-to-end mode dispatch integration tests
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    #[serial_test::serial]
    fn test_mixed_repo_both_modes_run() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        // Public skill with name issue (S010: uppercase)
        std::fs::create_dir_all("skills/My-Skill").unwrap();
        std::fs::write(
            "skills/My-Skill/SKILL.md",
            "---\nname: My-Skill\ndescription: Use when testing mixed mode validation\n---\nBody content\n",
        )
        .unwrap();

        // Private skill -- should NOT fire S016 (plugin-only person check)
        std::fs::create_dir_all(".claude/skills/helper").unwrap();
        std::fs::write(
            ".claude/skills/helper/SKILL.md",
            "---\nname: helper\ndescription: Helps you do things more efficiently here\n---\nBody content\n",
        )
        .unwrap();

        // Plugin mode runs both public and private
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        validate_private_skill_content(&mut diag, &crate::config::ExcludeSet::default());

        // S010 fires for public "My-Skill"
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("outside [a-z0-9-]"))
        );
        // S016 should NOT fire for private skill (plugin_mode=false)
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("first/second person") && e.contains(".claude"))
        );
    }

    // ── S044: tools-list-syntax ────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s044_yaml_list_triggers() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\nallowed-tools:\n  - Bash\n  - Read\n---\nBody content\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors().iter().any(|e| e.contains("list syntax")),
            "Expected S044 diagnostic about list syntax, got: {:?}",
            diag.errors()
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s044_scalar_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\nallowed-tools: Bash, Read\n---\nBody content\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag.errors().iter().any(|e| e.contains("list syntax")),
            "Unexpected S044 diagnostic for scalar allowed-tools: {:?}",
            diag.errors()
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s044_absent_no_diagnostic() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\nBody content\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag.errors().iter().any(|e| e.contains("list syntax")),
            "Unexpected S044 diagnostic when allowed-tools absent: {:?}",
            diag.errors()
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s044_no_double_report_with_s007() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\nallowed-tools:\n  - Bash\n  - Read\n---\nBody content\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        // Run both validators (skills frontmatter + content) like the full pipeline would
        crate::validators::skills::validate_skill_frontmatter(
            &mut diag,
            &crate::config::ExcludeSet::default(),
        );
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        // S044 should fire exactly once
        let s044_count = diag
            .errors()
            .iter()
            .filter(|e| e.contains("list syntax"))
            .count();
        assert_eq!(
            s044_count,
            1,
            "Expected exactly 1 S044 diagnostic, got {s044_count}: {:?}",
            diag.errors()
        );
        // S007 should NOT fire for allowed-tools (suppressed in favor of S044)
        let s007_allowed_tools = diag
            .errors()
            .iter()
            .filter(|e| e.contains("allowed-tools") && e.contains("present but empty"))
            .count();
        assert_eq!(
            s007_allowed_tools,
            0,
            "S007 should not fire for allowed-tools when S044 applies, got: {:?}",
            diag.errors()
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s046_boundary_300_lines_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let body = "Some text without workflow\n".repeat(300);
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: Use when testing boundary\n---\n{body}"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("workflow structure")),
            "S046 should not fire at exactly 300 lines"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s047_boundary_200_lines_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let body = "Some text without examples\n".repeat(200);
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: Use when testing boundary\n---\n{body}"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("examples or templates")),
            "S047 should not fire at exactly 200 lines"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_valid_skill_zero_errors() {
        // A fully valid skill should produce zero errors from all content checks
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/reviewing-code").unwrap();
        std::fs::write(
            "skills/reviewing-code/SKILL.md",
            "---\nname: reviewing-code\ndescription: Use when code changes need thorough review and analysis\nuser-invocable: true\neffort: high\nshell: bash\nargument-hint: <PR number or branch name>\n---\n\n# Code Review\n\nPerform a thorough code review of the specified changes.\n\n## Steps\n\n1. Run the analysis on $ARGUMENTS\n2. Generate a summary report\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        let skill_errors: Vec<_> = diag
            .errors()
            .iter()
            .filter(|e| e.contains("skills/reviewing-code"))
            .cloned()
            .collect();
        assert!(
            skill_errors.is_empty(),
            "Expected zero errors for valid skill, got: {skill_errors:?}"
        );
    }

    // ── S044: mcp-tool-unqualified ─────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s044_unqualified_mcp_tool_fires() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A skill\n---\nUse the `create_issue` tool to file bugs.\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("create_issue") && e.contains("MCP tool reference")),
            "Expected S044 for unqualified MCP tool, got: {:?}",
            diag.errors()
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s044_qualified_tool_no_fire() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A skill\n---\nUse the `GitHub:create_issue` tool.\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("MCP tool reference")),
            "Should not fire S044 for qualified tool, got: {:?}",
            diag.errors()
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s044_builtin_tool_no_fire() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A skill\n---\nUse the `task_create` tool.\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("MCP tool reference")),
            "Should not fire S044 for built-in tool, got: {:?}",
            diag.errors()
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s044_inside_code_fence_no_fire() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A skill\n---\n```bash\nUse the `create_issue` tool\n```\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("MCP tool reference")),
            "Should not fire S044 inside code fence, got: {:?}",
            diag.errors()
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s044_no_context_word_no_fire() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A skill\n---\nCheck `exit_code` value after completion.\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("MCP tool reference")),
            "Should not fire S044 without context word, got: {:?}",
            diag.errors()
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s044_private_skill_fires() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/skills/my-skill").unwrap();
        std::fs::write(
            ".claude/skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A skill\n---\nUse the `create_issue` tool.\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_private_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("create_issue") && e.contains("MCP tool reference")),
            "Expected S044 in private mode, got: {:?}",
            diag.errors()
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s044_dedup_same_tool_fires_once() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A skill\n---\nUse the `create_issue` tool.\nCall `create_issue` again.\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(
            diag.errors()
                .iter()
                .filter(|e| e.contains("create_issue") && e.contains("MCP tool reference"))
                .count(),
            1,
            "Expected exactly one S044 diagnostic for duplicate tool, got: {:?}",
            diag.errors()
        );
    }

    // ── S048: ref-name-generic ──────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s048_generic_ref_name_doc1() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need testing of ref names\n---\nBody\n",
        )
        .unwrap();
        std::fs::write("skills/my-skill/doc1.md", "some content").unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("non-descriptive reference file name"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s048_descriptive_name_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need testing of ref names\n---\nBody\n",
        )
        .unwrap();
        std::fs::write("skills/my-skill/api-reference.md", "some content").unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("non-descriptive reference file name"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s048_skill_md_excluded() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need testing of ref names\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("SKILL.md") && e.contains("non-descriptive"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s048_scripts_subdir_excluded() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need testing of ref names\n---\nBody\n",
        )
        .unwrap();
        std::fs::write("skills/my-skill/scripts/doc1.md", "some content").unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("scripts/doc1.md") && e.contains("non-descriptive"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s048_single_letter_name() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need testing of ref names\n---\nBody\n",
        )
        .unwrap();
        std::fs::write("skills/my-skill/a.md", "some content").unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("non-descriptive reference file name"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s048_numeric_name() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need testing of ref names\n---\nBody\n",
        )
        .unwrap();
        std::fs::write("skills/my-skill/02.md", "some content").unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("non-descriptive reference file name"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s048_plain_stem_no_digits() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need testing of ref names\n---\nBody\n",
        )
        .unwrap();
        std::fs::write("skills/my-skill/data.md", "some content").unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("non-descriptive reference file name"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s048_private_mode_also_fires() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude/skills/my-skill").unwrap();
        std::fs::write(
            ".claude/skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\nBody\n",
        )
        .unwrap();
        std::fs::write(".claude/skills/my-skill/file1.md", "some content").unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_private_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("non-descriptive reference file name"))
        );
    }

    // ── S049: name-not-gerund ───────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s049_no_gerund_fires() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/code-review").unwrap();
        std::fs::write(
            "skills/code-review/SKILL.md",
            "---\nname: code-review\ndescription: Use when code changes need thorough review\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("gerund")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s049_gerund_name_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/processing-pdfs").unwrap();
        std::fs::write(
            "skills/processing-pdfs/SKILL.md",
            "---\nname: processing-pdfs\ndescription: Use when you need to process PDF documents\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("gerund")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s049_exception_string_still_fires() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/string-utils").unwrap();
        std::fs::write(
            "skills/string-utils/SKILL.md",
            "---\nname: string-utils\ndescription: Use when you need string manipulation utilities\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        // "string" ends in "ing" but is in the exception list — S049 should fire
        // because there are no actual gerund words, only a non-gerund exception
        assert!(diag.errors().iter().any(|e| e.contains("gerund")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s049_private_mode_skips() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude/skills/code-review").unwrap();
        std::fs::write(
            ".claude/skills/code-review/SKILL.md",
            "---\nname: code-review\ndescription: A valid skill description here\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_private_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        // S049 is plugin-only, should not fire in private mode
        assert!(!diag.errors().iter().any(|e| e.contains("gerund")));
    }

    // ── S050: desc-vague-content ────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s050_vague_description_flagged() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Helps with documents. Use when working with files.\n---\nBody content\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors().iter().any(|e| e.contains("vague/generic")),
            "Expected S050 to flag vague description"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s050_specific_description_passes() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/pdf-tool").unwrap();
        std::fs::write(
            "skills/pdf-tool/SKILL.md",
            "---\nname: pdf-tool\ndescription: Extract text and tables from PDF files, fill forms, merge documents. Use when working with PDF files.\n---\nBody content\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag.errors().iter().any(|e| e.contains("vague/generic")),
            "S050 should not flag specific descriptions"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s050_private_skill_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude/skills/my-skill").unwrap();
        std::fs::write(
            ".claude/skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Helps with documents. Use when working with files.\n---\nBody content\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_private_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        // S050 is plugin-only, should not fire in private mode
        assert!(
            !diag.errors().iter().any(|e| e.contains("vague/generic")),
            "S050 should not fire for private skills"
        );
    }

    // ── S053: terminology-inconsistent ───────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s053_three_variants_triggers() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill that does useful things here\n---\n\
             Use the endpoint to access data.\n\
             The route should be configured.\n\
             Check the url for the response.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("synonym group") && e.contains("endpoint")),
            "S053 should fire when 3+ synonym variants are used"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s053_two_variants_no_trigger() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill that does useful things here\n---\n\
             Use the endpoint to access data.\n\
             The route should be configured.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag.errors().iter().any(|e| e.contains("synonym group")),
            "S053 should not fire when only 2 variants are used"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s053_fence_isolation() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill that does useful things here\n---\n\
             Use the endpoint to access data.\n\
             The route should be configured.\n\
             ```bash\n\
             curl $url\n\
             ```\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag.errors().iter().any(|e| e.contains("synonym group")),
            "S053 should not count terms inside code fences"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s053_case_insensitive() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill that does useful things here\n---\n\
             The Endpoint should be stable.\n\
             Configure the URL properly.\n\
             Set up the Route correctly.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors().iter().any(|e| e.contains("synonym group")),
            "S053 should match case-insensitively"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s053_private_mode_skips() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude/skills/my-skill").unwrap();
        std::fs::write(
            ".claude/skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill that does useful things here\n---\n\
             Use the endpoint to access data.\n\
             The route should be configured.\n\
             Check the url for the response.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_private_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag.errors().iter().any(|e| e.contains("synonym group")),
            "S053 should not fire for private skills"
        );
    }

    // ── S055: script-errhand-missing ─────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s055_sh_with_set_e_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts").unwrap();
        std::fs::write(
            "skills/my-skill/scripts/run.sh",
            "#!/bin/bash\nset -euo pipefail\necho hello\necho world\necho done\n",
        )
        .unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n## Dependencies\n\npip install foo\n\n## Verify\n\nRun scripts/run.sh to verify.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("lacks error handling")),
            "S055 should not fire when set -e present"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s055_sh_with_trap_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts").unwrap();
        std::fs::write(
            "skills/my-skill/scripts/run.sh",
            "#!/bin/bash\ntrap 'echo failed' ERR\necho step1\necho step2\necho step3\n",
        )
        .unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n## Dependencies\n\npip install foo\n\n## Verify\n\nRun scripts/run.sh to verify.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("lacks error handling")),
            "S055 should not fire when trap present"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s055_sh_with_or_exit_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts").unwrap();
        std::fs::write(
            "skills/my-skill/scripts/run.sh",
            "#!/bin/bash\ncommand1 || exit 1\necho step1\necho step2\necho step3\n",
        )
        .unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n## Dependencies\n\npip install foo\n\n## Verify\n\nRun scripts/run.sh to verify.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("lacks error handling")),
            "S055 should not fire when || exit present"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s055_sh_without_error_handling() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts").unwrap();
        std::fs::write(
            "skills/my-skill/scripts/run.sh",
            "#!/bin/bash\necho hello\necho world\necho foo\necho bar\necho done\n",
        )
        .unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n## Dependencies\n\npip install foo\n\n## Verify\n\nRun scripts/run.sh to verify.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("lacks error handling")),
            "S055 should fire when no error handling"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s055_py_with_try_except_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts").unwrap();
        std::fs::write(
            "skills/my-skill/scripts/run.py",
            "import sys\ntry:\n    do_something()\nexcept Exception as e:\n    print(e)\n",
        )
        .unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n## Dependencies\n\npip install foo\n\n## Verify\n\nRun scripts/run.py to verify.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("lacks error handling")),
            "S055 should not fire when try/except present"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s055_py_without_error_handling() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts").unwrap();
        std::fs::write(
            "skills/my-skill/scripts/run.py",
            "import sys\nimport os\ndef main():\n    print('hello')\nmain()\n",
        )
        .unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n## Dependencies\n\npip install foo\n\n## Verify\n\nRun scripts/run.py to verify.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("lacks error handling")),
            "S055 should fire when no try/except"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s055_non_script_skill_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\nJust a plain skill with no scripts.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("lacks error handling")),
            "S055 should not fire for non-script skill"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s055_private_mode_skips() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude/skills/my-skill/scripts").unwrap();
        std::fs::write(
            ".claude/skills/my-skill/scripts/run.sh",
            "#!/bin/bash\necho hello\necho world\necho foo\necho bar\necho done\n",
        )
        .unwrap();
        std::fs::write(
            ".claude/skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\nThis skill runs a script.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_private_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("lacks error handling")),
            "S055 should not fire in private mode"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s055_short_script_skips() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts").unwrap();
        std::fs::write(
            "skills/my-skill/scripts/run.sh",
            "#!/bin/bash\necho hello\n",
        )
        .unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n## Dependencies\n\npip install foo\n\n## Verify\n\nRun scripts/run.sh to verify.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("lacks error handling")),
            "S055 should not fire for short scripts"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s055_mixed_scripts_partial() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts").unwrap();
        std::fs::write(
            "skills/my-skill/scripts/good.sh",
            "#!/bin/bash\nset -euo pipefail\necho step1\necho step2\necho step3\n",
        )
        .unwrap();
        std::fs::write(
            "skills/my-skill/scripts/bad.sh",
            "#!/bin/bash\necho hello\necho world\necho foo\necho bar\necho done\n",
        )
        .unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n## Dependencies\n\npip install foo\n\n## Verify\n\nRun scripts to verify.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        let errors = diag.errors();
        let errhand_errors: Vec<_> = errors
            .iter()
            .filter(|e| e.contains("lacks error handling"))
            .collect();
        assert_eq!(
            errhand_errors.len(),
            1,
            "S055 should fire for exactly one script (bad.sh), got: {:?}",
            errhand_errors
        );
        assert!(
            errhand_errors[0].contains("bad.sh"),
            "S055 should fire for bad.sh, not good.sh"
        );
    }

    // ── S056: body-no-default ───────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s056_unframed_alternatives() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\nYou can use pypdf, or pdfplumber, or PyMuPDF to extract text.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("alternatives without stating a default")),
            "S056 should fire for unframed alternatives"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s056_comma_list_alternatives() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\nYou can use pypdf, pdfplumber, or PyMuPDF to extract text.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("alternatives without stating a default")),
            "S056 should fire for comma-list alternatives"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s056_conditional_framing_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\nIf you need text extraction, use pdfplumber or pypdf or PyMuPDF.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("alternatives without stating a default")),
            "S056 should NOT fire when line starts with 'If'"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s056_default_stated_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\nWe recommend pdfplumber, pypdf, or PyMuPDF for text extraction.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("alternatives without stating a default")),
            "S056 should NOT fire when 'recommend' is present"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s056_binary_choice_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\nUse pdfplumber or PyMuPDF for extraction.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("alternatives without stating a default")),
            "S056 should NOT fire for binary choice (only one 'or')"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s056_inside_code_fence_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\n```\nYou can use pypdf, or pdfplumber, or PyMuPDF to extract text.\n```\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("alternatives without stating a default")),
            "S056 should NOT fire inside code fences"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s056_private_mode_skips() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude/skills/my-skill").unwrap();
        std::fs::write(
            ".claude/skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\nYou can use pypdf, or pdfplumber, or PyMuPDF to extract text.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_private_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("alternatives without stating a default")),
            "S056 should NOT fire in private mode"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s056_when_framing_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\nWhen processing files, use tool A or tool B or tool C.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("alternatives without stating a default")),
            "S056 should NOT fire when line starts with 'When'"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s056_default_keyword_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\nUse tool A or tool B or tool C; tool A is the default.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("alternatives without stating a default")),
            "S056 should NOT fire when 'default' keyword is present"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s056_prefer_keyword_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\nPrefer tool A over tool B or tool C for most cases.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("alternatives without stating a default")),
            "S056 should NOT fire when 'Prefer' keyword is present"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s056_midline_if_still_fires() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\nUse tool A or tool B or tool C if you need more speed.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("alternatives without stating a default")),
            "S056 should fire when 'if' is mid-line (not at start)"
        );
    }

    // ── S057: magic-number-undoc ───────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s057_magic_number_fires() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\n```bash\nTIMEOUT = 47\n```\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        let errors = diag.errors();
        assert!(
            errors
                .iter()
                .any(|e| e.contains("undocumented magic number") && e.contains("TIMEOUT = 47")),
            "S057 should fire for undocumented magic number, got: {:?}",
            errors
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s057_well_known_value_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\n```bash\nPORT = 443\n```\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("undocumented magic number")),
            "S057 should NOT fire for well-known value 443"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s057_zero_one_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\n```bash\nFLAG = 0\nCOUNT = 1\n```\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("undocumented magic number")),
            "S057 should NOT fire for 0 or 1"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s057_same_line_comment_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\n```bash\nTIMEOUT = 47 # slow network tolerance\n```\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("undocumented magic number")),
            "S057 should NOT fire when same-line comment exists"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s057_preceding_line_comment_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\n```bash\n# Timeout for slow networks\nTIMEOUT = 47\n```\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("undocumented magic number")),
            "S057 should NOT fire when preceding line is a comment"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s057_outside_fence_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\nTIMEOUT = 47\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("undocumented magic number")),
            "S057 should NOT fire outside code fences"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s057_private_mode_skips() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude/skills/my-skill").unwrap();
        std::fs::write(
            ".claude/skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\n```bash\nTIMEOUT = 47\n```\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_private_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("undocumented magic number")),
            "S057 should NOT fire in private mode"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s057_float_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\n```python\nRATIO = 3.14\n```\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("undocumented magic number")),
            "S057 should NOT fire for float values like 3.14"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s057_comment_line_with_number_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\n```python\n# TIMEOUT = 47\nx = 1\n```\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("undocumented magic number")),
            "S057 should NOT fire on comment lines containing assignment patterns"
        );
    }
}
