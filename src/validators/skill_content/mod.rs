mod body;
mod cross_field;
mod cross_skill;
mod description;
mod frontmatter_extended;
mod frontmatter_fields;
mod mcp;
mod name;
pub(crate) mod security;

pub(crate) use description::{description_contains_xml_tags, strip_description_xml_tags};

use crate::config::ExcludeSet;
use crate::context::LintContext;
use crate::diagnostic::DiagnosticCollector;
use crate::live_instructions::{InstructionSurfaceKind, LiveInstructionDocument};
use crate::validators::skills::{
    SkillInfo, collect_agent_skills, collect_cursor_runtime_skills, collect_plugin_skill_files,
    collect_skills, collect_skills_including_shared,
};
use regex::Regex;
use std::path::Path;
use std::sync::LazyLock;

// S022/S043: Backslash paths — shared by validators and autofix.
pub(crate) static RE_BACKSLASH_PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[A-Za-z]:\\[A-Za-z]|\\[A-Za-z][A-Za-z0-9_-]+\\[A-Za-z]").unwrap()
});

// Adjacent named TeX commands are escapes, not filesystem paths. Keep this
// deliberately narrow: broadly accepting \word\word as an escape would hide
// ordinary relative paths such as \dir\file.
const NAMED_TEX_ESCAPES: &[&str] = &[
    "alpha",
    "beta",
    "gamma",
    "delta",
    "epsilon",
    "varepsilon",
    "zeta",
    "eta",
    "theta",
    "vartheta",
    "iota",
    "kappa",
    "lambda",
    "mu",
    "nu",
    "xi",
    "pi",
    "varpi",
    "rho",
    "varrho",
    "sigma",
    "varsigma",
    "tau",
    "upsilon",
    "phi",
    "varphi",
    "chi",
    "psi",
    "omega",
];

/// Whether a line contains an S022/S043 backslash path rather than a named
/// TeX escape pair. This is the shared recognition contract for validation and
/// autofix.
pub(crate) fn contains_backslash_path(line: &str) -> bool {
    RE_BACKSLASH_PATH
        .find_iter(line)
        .any(|matched| !is_named_tex_escape_pair(&line[matched.start()..]))
}

pub(crate) fn is_named_tex_escape_pair(value: &str) -> bool {
    let Some(value) = value.strip_prefix('\\') else {
        return false;
    };
    let mut segments = value.split('\\');
    let (Some(first), Some(second)) = (segments.next(), segments.next()) else {
        return false;
    };
    let second = second
        .split(|character: char| !character.is_ascii_alphabetic())
        .next()
        .unwrap_or_default();
    NAMED_TEX_ESCAPES.contains(&first) && NAMED_TEX_ESCAPES.contains(&second)
}

/// Frontmatter fields S043 must never scan or rewrite. `description`,
/// `compatibility`, and `when_to_use` are free prose (a description may
/// legitimately mention `C:\Users`), and `metadata` holds arbitrary string
/// data rather than path configuration.
pub(crate) const S043_PROSE_FIELDS: &[&str] =
    &["description", "compatibility", "when_to_use", "metadata"];

/// Whether a canonical frontmatter value carries an S043 backslash path.
/// A scalar string is checked directly and each string item of a sequence is
/// checked individually; mappings, nested collections, and non-string scalars
/// never match. This is the S043 *validator's* detector; the autofix rewrites
/// only single-line scalars, so it does its own scalar-scoped detection but
/// shares `S043_PROSE_FIELDS` and `contains_backslash_path` with this path.
pub(crate) fn canonical_value_has_backslash_path(value: &crate::yaml::Value) -> bool {
    if let Some(text) = value.as_str() {
        contains_backslash_path(text)
    } else if let Some(items) = value.as_sequence() {
        items
            .iter()
            .filter_map(|item| item.as_str())
            .any(contains_backslash_path)
    } else {
        false
    }
}

/// Canonical skill frontmatter keys (Claude Code docs + fields already linted here).
/// Used by S070 (unknown-fm-field) and kept alongside S007's empty-optional list.
pub(crate) const KNOWN_SKILL_FRONTMATTER_FIELDS: &[&str] = &[
    "name",
    "description",
    "when_to_use",
    "argument-hint",
    "arguments",
    "disable-model-invocation",
    "user-invocable",
    "allowed-tools",
    "disallowed-tools",
    "model",
    "effort",
    "context",
    "agent",
    "hooks",
    "paths",
    "shell",
    "compatibility",
    "metadata",
    "license",
];

/// Optional scalar fields that S007 flags when present but empty.
/// `paths` is handled by S071 instead (YAML list form is common).
pub(crate) const OPTIONAL_NONEMPTY_SCALAR_FIELDS: &[&str] = &["argument-hint", "allowed-tools"];

/// Validate skill content for public skills (skills/). Runs S009-S057 and S063-S071 rules.
#[cfg(test)]
pub fn validate_skill_content(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let mut prompt_pass = super::prompt_content::PromptContentPass::default();
    validate_skill_content_with_prompt_pass(diag, exclude, &mut prompt_pass);
}

#[cfg(test)]
pub(crate) fn validate_skill_content_with_prompt_pass(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    prompt_pass: &mut super::prompt_content::PromptContentPass,
) {
    let skills = collect_skills("skills", exclude);
    let agents = super::agent_discovery::runtime_inventory(true, &[], exclude);
    for info in &skills {
        run_content_checks(info, true, &agents, diag, exclude, prompt_pass);
    }
    // Cross-skill checks (plugin-only: S029, S036; both-mode: S030, S048)
    cross_skill::validate_nested_references("skills", &skills, diag);
    cross_skill::validate_orphaned_skill_files("skills", diag, exclude);
    cross_skill::validate_ref_no_toc("skills", &skills, diag, exclude);
    cross_skill::validate_generic_ref_names("skills", diag, exclude);
}

pub(crate) fn validate_discovered_skill_content_with_prompt_pass(
    ctx: &LintContext,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    prompt_pass: &mut super::prompt_content::PromptContentPass,
) {
    let skills = collect_plugin_skill_files(
        super::skill_discovery::SkillDiscovery::from_context(ctx, exclude).exported_skill_files,
        exclude,
    );
    let declared_agents = super::manifest::declared_agent_roots(ctx);
    let agents = super::agent_discovery::runtime_inventory(true, &declared_agents, exclude);
    for info in &skills {
        run_content_checks(info, true, &agents, diag, exclude, prompt_pass);
    }
    let conventional = collect_skills("skills", exclude);
    cross_skill::validate_nested_references("skills", &conventional, diag);
    cross_skill::validate_orphaned_skill_files("skills", diag, exclude);
    cross_skill::validate_ref_no_toc("skills", &conventional, diag, exclude);
    cross_skill::validate_generic_ref_names("skills", diag, exclude);
}

/// Validate skill content for private skills (.claude/skills/).
/// Runs only "both-mode" rules (excludes S016, S017, S029, S033, S036, S037, S038, S046, S047, S050, S051, S052, S053, S054, S055, S056, S057).
/// Retired S049 never emits from either path; its registry entry is config-only.
#[cfg(test)]
pub fn validate_private_skill_content(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let mut prompt_pass = super::prompt_content::PromptContentPass::default();
    validate_private_skill_content_with_prompt_pass(diag, exclude, false, &[], &mut prompt_pass);
}

pub(crate) fn validate_private_skill_content_with_prompt_pass(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    plugin_runtime: bool,
    declared_agents: &[String],
    prompt_pass: &mut super::prompt_content::PromptContentPass,
) {
    let skills = collect_skills_including_shared(".claude/skills", exclude);
    let agents =
        super::agent_discovery::runtime_inventory(plugin_runtime, declared_agents, exclude);
    for info in &skills {
        // The runtime namespace is Plugin-wide, but private skills retain their
        // established both-mode validation scope.
        run_content_checks(info, false, &agents, diag, exclude, prompt_pass);
    }
    cross_skill::validate_orphaned_skill_files(".claude/skills", diag, exclude);
    cross_skill::validate_generic_ref_names(".claude/skills", diag, exclude);
}

/// Validate the Agent Skills name and description contract for a
/// platform-gated skill surface.
/// Broader content validation remains owned by the public/private passes;
/// S031/S032 reuse `validate_agent_skills_content_security` on these surfaces.
pub(crate) fn validate_agent_skills_contract(
    base_dir: &str,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
) {
    let skills = if base_dir == ".agents/skills" {
        collect_agent_skills(exclude)
    } else {
        collect_skills(base_dir, exclude)
    };
    for info in skills {
        diag.with_subject_path(&info.path, |diag| {
            if let Some(name) = crate::frontmatter::get_strict_string_field(&info.fm_lines, "name")
            {
                name::check_agent_skills_name_contract(&info, &name, diag);
            }
            description::check_agent_skills_description_contract(&info, diag);
        });
    }
}

/// Run S031/S032 content-security checks on a platform-gated skill surface.
///
/// Secrets and non-HTTPS URLs are platform-independent defects, so these rules
/// apply wherever a skill prompt is loaded. Autofix for S031 remains scoped to
/// the Claude surfaces only.
pub(crate) fn validate_agent_skills_content_security(
    base_dir: &str,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
) {
    let skills = if base_dir == ".agents/skills" {
        collect_agent_skills(exclude)
    } else {
        collect_skills(base_dir, exclude)
    };
    for info in skills {
        diag.with_subject_path(&info.path, |diag| {
            security::check_content_security(&info, diag);
        });
    }
}

/// Run Cursor's shared runtime inventory through the Agent Skills contracts.
/// Keeping this inventory alongside CR-SK-001 prevents nested Cursor skills
/// from receiving platform-schema checks but missing S009--S011, S014, S034,
/// S031, or S032.
pub(crate) fn validate_cursor_runtime_skills_contract(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
) {
    for info in collect_cursor_runtime_skills(exclude) {
        diag.with_subject_path(&info.path, |diag| {
            if let Some(name) = crate::frontmatter::get_strict_string_field(&info.fm_lines, "name")
            {
                name::check_agent_skills_name_contract(&info, &name, diag);
            }
            description::check_agent_skills_description_contract(&info, diag);
        });
    }
}

pub(crate) fn validate_cursor_runtime_skills_content_security(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
) {
    for info in collect_cursor_runtime_skills(exclude) {
        diag.with_subject_path(&info.path, |diag| {
            security::check_content_security(&info, diag);
        });
    }
}

fn run_content_checks(
    info: &SkillInfo,
    plugin_mode: bool,
    agents: &super::agent_discovery::RuntimeAgentInventory,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    prompt_pass: &mut super::prompt_content::PromptContentPass,
) {
    diag.with_subject_path(&info.path, |diag| {
        name::check_name_format(info, plugin_mode, diag);
        description::check_description_quality(info, plugin_mode, diag);
        body::check_body_content(info, plugin_mode, diag, exclude);
        let prompt_document = LiveInstructionDocument::new(
            Path::new(&info.path),
            InstructionSurfaceKind::Skill,
            &info.document,
        );
        prompt_pass.validate(&prompt_document, diag);
        frontmatter_fields::check_frontmatter_fields(info, agents, diag);
        frontmatter_extended::check_frontmatter_extended(info, diag);
        cross_field::check_cross_field(info, plugin_mode, diag);
        security::check_content_security(info, diag);
        mcp::check_mcp_tool_refs(info, diag);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::DiagnosticCollector;
    use crate::rules::LintRule;

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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("exceeds 64")));
    }

    #[test]
    #[serial_test::serial]
    fn name_contract_uses_yaml_scalars_and_unicode_character_counts() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".agents/skills/valid-name").unwrap();
        let path = ".agents/skills/valid-name/SKILL.md";

        for name in [
            "valid-name # YAML comments are not part of the scalar",
            "\"valid\\x2dname\" # quoted escapes are decoded by YAML",
        ] {
            std::fs::write(
                path,
                format!(
                    "---\nname: {name}\ndescription: A valid skill description here\n---\nBody\n"
                ),
            )
            .unwrap();
            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_agent_skills_contract(
                ".agents/skills",
                &mut diag,
                &crate::config::ExcludeSet::default(),
            );
            assert!(
                diag.diagnostics().is_empty(),
                "{name}: {:#?}",
                diag.diagnostics()
            );
        }

        let multibyte_name = "é".repeat(64);
        std::fs::write(
            path,
            format!("---\nname: {multibyte_name}\ndescription: A valid skill description here\n---\nBody\n"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agent_skills_contract(
            ".agents/skills",
            &mut diag,
            &crate::config::ExcludeSet::default(),
        );
        assert!(
            diag.diagnostics()
                .iter()
                .all(|item| item.rule != crate::rules::LintRule::NameTooLong)
        );
        let invalid = diag
            .diagnostics()
            .iter()
            .find(|item| item.rule == crate::rules::LintRule::NameInvalidChars)
            .expect("non-ASCII name is rejected by S010");
        assert_eq!(
            invalid.location,
            Some(crate::diagnostic::SourceSpan::line(2))
        );
        assert_eq!(invalid.evidence.as_deref(), Some(multibyte_name.as_str()));
        assert_eq!(
            invalid.suggestion.as_deref(),
            Some("use only lowercase ASCII letters, digits, and single hyphens")
        );
    }

    #[test]
    #[serial_test::serial]
    fn name_contract_skips_untrustworthy_yaml_names() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".agents/skills/example").unwrap();
        let path = ".agents/skills/example/SKILL.md";

        for name in ["[not-a-scalar]", "[unterminated"] {
            std::fs::write(
                path,
                format!(
                    "---\nname: {name}\ndescription: A valid skill description here\n---\nBody\n"
                ),
            )
            .unwrap();
            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_agent_skills_contract(
                ".agents/skills",
                &mut diag,
                &crate::config::ExcludeSet::default(),
            );
            assert!(
                diag.diagnostics().is_empty(),
                "{name}: {:#?}",
                diag.diagnostics()
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn agent_skills_description_contract_uses_canonical_lengths_and_only_spec_rules() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        let cases = [
            ("too-long", "x".repeat(1025)),
            ("too-short", "x".repeat(19)),
            ("at-cap", "x".repeat(1024)),
            ("at-floor", "x".repeat(20)),
            (
                "hard-negative",
                format!("I use <tag> {}", "specific ".repeat(35)),
            ),
        ];
        for (name, description) in cases {
            let path = format!(".agents/skills/{name}/SKILL.md");
            std::fs::create_dir_all(Path::new(&path).parent().unwrap()).unwrap();
            std::fs::write(
                &path,
                format!("---\nname: {name}\ndescription: {description}\n---\nBody\n"),
            )
            .unwrap();
        }
        let block_path = ".agents/skills/block-scalar/SKILL.md";
        std::fs::create_dir_all(Path::new(block_path).parent().unwrap()).unwrap();
        std::fs::write(
            block_path,
            format!(
                "---\nname: block-scalar\ndescription: >-\n  {}\n  {}\n---\nBody\n",
                "x".repeat(600),
                "y".repeat(600)
            ),
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agent_skills_contract(
            ".agents/skills",
            &mut diag,
            &crate::config::ExcludeSet::default(),
        );
        let rules_at = |path: &str| {
            diag.diagnostics()
                .iter()
                .filter(|item| item.subject_path.as_deref() == Some(Path::new(path)))
                .map(|item| item.rule)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            rules_at(".agents/skills/too-long/SKILL.md"),
            vec![LintRule::DescTooLong]
        );
        assert_eq!(
            rules_at(".agents/skills/too-short/SKILL.md"),
            vec![LintRule::DescTooShort]
        );
        assert!(rules_at(".agents/skills/at-cap/SKILL.md").is_empty());
        assert!(rules_at(".agents/skills/at-floor/SKILL.md").is_empty());
        assert_eq!(
            rules_at(block_path),
            vec![LintRule::DescTooLong],
            "block-scalar text must be counted from its canonical YAML scalar"
        );
        assert!(
            rules_at(".agents/skills/hard-negative/SKILL.md").is_empty(),
            "Agent Skills surfaces must not inherit Claude-only S015/S016/S018 checks"
        );
    }

    #[test]
    #[serial_test::serial]
    fn agent_skills_description_contract_honors_nested_exclusions_and_overrides() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        for path in [
            "packages/keep/.agents/skills/long/SKILL.md",
            "packages/excluded/.agents/skills/long/SKILL.md",
            "packages/suppressed/.agents/skills/long/SKILL.md",
        ] {
            std::fs::create_dir_all(Path::new(path).parent().unwrap()).unwrap();
            std::fs::write(
                path,
                format!(
                    "---\nname: long\ndescription: {}\n---\nBody\n",
                    "x".repeat(1025)
                ),
            )
            .unwrap();
        }
        std::fs::write(
            "agent-lint.toml",
            r#"
[lint]
exclude = ["packages/excluded/**"]

[[lint.overrides]]
files = ["packages/suppressed/.agents/skills/long/SKILL.md"]
suppress = ["S014"]
"#,
        )
        .unwrap();

        let config = crate::config::LintConfig::load(tmp.path()).unwrap();
        let exclude = config.build_exclude_set();
        let mut diag = DiagnosticCollector::with_config(config);
        validate_agent_skills_contract(".agents/skills", &mut diag, &exclude);
        let findings = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::DescTooLong)
            .collect::<Vec<_>>();
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(
            findings[0].subject_path.as_deref(),
            Some(Path::new("packages/keep/.agents/skills/long/SKILL.md"))
        );
        assert_eq!(diag.suppressed_count(), 1);
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("outside [a-z0-9-]"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s010_rejects_every_non_contract_character_class() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".agents/skills/example").unwrap();
        let path = ".agents/skills/example/SKILL.md";

        for name in [
            "Uppercase",
            "has space",
            "under_score",
            "punctuation!",
            "café",
        ] {
            std::fs::write(
                path,
                format!(
                    "---\nname: {name}\ndescription: A valid skill description here\n---\nBody\n"
                ),
            )
            .unwrap();
            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_agent_skills_contract(
                ".agents/skills",
                &mut diag,
                &crate::config::ExcludeSet::default(),
            );
            assert!(
                diag.diagnostics()
                    .iter()
                    .any(|item| item.rule == crate::rules::LintRule::NameInvalidChars)
            );
        }
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag
            .errors()
            .iter()
            .any(|e| e.contains("starts/ends with hyphen") || e.contains("consecutive hyphens")));
    }

    // ── S010: invalid name characters ─────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s010_reports_angle_bracket_names_once() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        for (index, name) in ["my-<tag>skill", "my-</tag>skill", "my-<tag", "my->tag"]
            .iter()
            .enumerate()
        {
            let dir = format!("skills/skill-{index}");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                format!("{dir}/SKILL.md"),
                format!(
                    "---\nname: {name}\ndescription: A valid skill description here\n---\nBody content\n"
                ),
            )
            .unwrap();
        }

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        let invalid_names: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|finding| finding.rule == LintRule::NameInvalidChars)
            .collect();
        assert_eq!(invalid_names.len(), 4);
    }

    #[test]
    #[serial_test::serial]
    fn test_vendor_and_skill_names_are_valid() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        for name in ["claude-api", "anthropic-tools", "skill", "skill-creator"] {
            let dir = format!("skills/{name}");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                format!("{dir}/SKILL.md"),
                format!(
                    "---\nname: {name}\ndescription: A valid skill description here\n---\nBody content\n"
                ),
            )
            .unwrap();
        }

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.diagnostics().iter().all(|finding| {
            !matches!(
                finding.rule,
                LintRule::NameInvalidChars | LintRule::NameBadHyphens
            )
        }));
    }

    // ── Canonical description scalars ───────────────────────────────

    #[test]
    #[serial_test::serial]
    fn description_rules_use_canonical_scalar_for_every_yaml_multiline_form() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        let first = format!(
            "Extract PDF text tables OCR metadata and searchable archives {}",
            "detailed ".repeat(180).trim_end()
        );
        let continuation =
            "You can inspect <tag> scanned contracts. Use when reviewing document workflows.";
        let forms = [
            ("folded-strip", format!(">-\n  {first}\n  {continuation}")),
            ("folded-clip", format!(">\n  {first}\n  {continuation}")),
            ("literal-clip", format!("|\n  {first}\n  {continuation}")),
            ("literal-strip", format!("|-\n  {first}\n  {continuation}")),
            ("plain", format!("{first}\n  {continuation}")),
            ("double-quoted", format!("\"{first}\n  {continuation}\"")),
            ("single-quoted", format!("'{first}\n  {continuation}'")),
        ];

        for (name, description) in forms {
            let dir = format!("skills/{name}");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                format!("{dir}/SKILL.md"),
                format!(
                    "---\nname: {name}\ndescription: {description}\n---\nGarden watering schedules are documented separately.\n"
                ),
            )
            .unwrap();
        }

        let plain_content = std::fs::read_to_string("skills/plain/SKILL.md").unwrap();
        let plain_frontmatter = crate::frontmatter::extract_frontmatter(&plain_content).unwrap();
        assert!(
            crate::frontmatter::get_strict_string_field(&plain_frontmatter, "description")
                .is_some(),
            "plain scalar should parse: {:?}",
            crate::frontmatter::parse_yaml_strict(&plain_frontmatter)
        );

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());

        for name in [
            "folded-strip",
            "folded-clip",
            "literal-clip",
            "literal-strip",
            "plain",
            "double-quoted",
            "single-quoted",
        ] {
            let subject = format!("skills/{name}/SKILL.md");
            let rules: std::collections::HashSet<_> = diag
                .diagnostics()
                .iter()
                .filter(|diagnostic| diagnostic.subject_path.as_deref() == Some(subject.as_ref()))
                .map(|diagnostic| diagnostic.rule)
                .collect();
            for expected in [
                LintRule::DescTooLong,
                LintRule::DescTruncated,
                LintRule::DescUsesPerson,
                LintRule::DescHasXml,
                LintRule::DescBodyMisalign,
            ] {
                assert!(
                    rules.contains(&expected),
                    "{name} missing {expected:?}: {rules:?}"
                );
            }
            for unexpected in [
                LintRule::DescTooShort,
                LintRule::DescNoTrigger,
                LintRule::DescVagueContent,
            ] {
                assert!(
                    !rules.contains(&unexpected),
                    "{name} unexpectedly reported {unexpected:?}: {rules:?}"
                );
            }

            let content = std::fs::read_to_string(&subject).unwrap();
            let frontmatter = crate::frontmatter::extract_frontmatter(&content).unwrap();
            let canonical =
                crate::frontmatter::get_strict_string_field(&frontmatter, "description").unwrap();
            let long_diagnostic = diag
                .diagnostics()
                .iter()
                .find(|diagnostic| {
                    diagnostic.subject_path.as_deref() == Some(subject.as_ref())
                        && diagnostic.rule == LintRule::DescTooLong
                })
                .unwrap();
            assert!(
                long_diagnostic
                    .message
                    .contains(&format!("({})", canonical.chars().count())),
                "{name} must count canonical characters: {long_diagnostic:?}"
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn block_scalar_description_matches_the_equivalent_inline_description() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        let description = "Extract text and tables from PDF files, fill forms, merge documents, and convert scanned pages to searchable text with OCR fallback logic. Use when the user asks to process, split, or repair any PDF document.";
        let descriptions = vec![
            (
                "block-desc",
                "description: >-\n  Extract text and tables from PDF files, fill forms, merge documents,\n  and convert scanned pages to searchable text with OCR fallback logic.\n  Use when the user asks to process, split, or repair any PDF document.".to_string(),
            ),
            ("inline-desc", format!("description: {description}")),
        ];
        for (name, frontmatter_description) in descriptions {
            let dir = format!("skills/{name}");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                format!("{dir}/SKILL.md"),
                format!("---\nname: {name}\n{frontmatter_description}\n---\n{description}\n"),
            )
            .unwrap();
        }

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        let description_rules = [
            LintRule::DescTooLong,
            LintRule::DescTruncated,
            LintRule::DescUsesPerson,
            LintRule::DescNoTrigger,
            LintRule::DescHasXml,
            LintRule::DescTooShort,
            LintRule::DescVagueContent,
            LintRule::DescBodyMisalign,
        ];
        for diagnostic in diag.diagnostics() {
            assert!(
                !description_rules.contains(&diagnostic.rule),
                "unexpected canonical-description diagnostic: {diagnostic:?}"
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn plain_scalar_trigger_on_continuation_line_is_recognized() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/plain-desc").unwrap();
        std::fs::write(
            "skills/plain-desc/SKILL.md",
            "---\nname: plain-desc\ndescription: Extract text and tables from PDF files, fill forms,\n  merge documents, and convert scans. Use when processing PDF files.\n---\nExtract PDF text and tables while processing scans.\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.rule == LintRule::DescNoTrigger),
            "continuation-line trigger must suppress S017: {:?}",
            diag.diagnostics()
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("exceeds 1024")));
    }

    // ── S015: desc-truncated ─────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s015_combined_listing_cap_and_message_variants() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let desc = "x".repeat(900);
        let when_to_use = "y".repeat(636);
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: {desc}\nwhen_to_use: {when_to_use}\n---\nBody content\n"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.rule == LintRule::DescTruncated)
        );

        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: {desc}\n---\nBody content\n"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.rule == LintRule::DescTruncated)
        );

        let over_cap_when_to_use = "y".repeat(637);
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: {desc}\nwhen_to_use: {over_cap_when_to_use}\n---\nBody content\n"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        let message = diag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.rule == LintRule::DescTruncated)
            .map(|diagnostic| diagnostic.message.as_str())
            .expect("S015 fires above the combined listing cap");
        assert!(message.contains("combined description and when_to_use total 1537 characters"));
        assert!(message.contains("configured listing cap of 1536"));
        assert!(message.contains("skillListingMaxDescChars"));

        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!(
                "---\nname: my-skill\ndescription: {}\n---\nBody content\n",
                "z".repeat(1537)
            ),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        let message = diag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.rule == LintRule::DescTruncated)
            .map(|diagnostic| diagnostic.message.as_str())
            .expect("S015 fires for an over-cap description");
        assert!(message.contains("description totals 1537 characters"));
    }

    #[test]
    #[serial_test::serial]
    fn test_s015_counts_block_when_to_use_and_honors_configured_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let desc = "x".repeat(900);
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: {desc}\nwhen_to_use: |\n  {}\n---\nBody content\n", "y".repeat(800)),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.rule == LintRule::DescTruncated)
        );

        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!(
                "---\nname: my-skill\ndescription: {}\n---\nBody content\n",
                "z".repeat(201)
            ),
        )
        .unwrap();
        let config = crate::config::LintConfig {
            desc_truncated_max_chars: 200,
            ..Default::default()
        };
        let mut diag = DiagnosticCollector::with_config(config);
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        let message = diag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.rule == LintRule::DescTruncated)
            .map(|diagnostic| diagnostic.message.as_str())
            .expect("configured S015 cap fires at 201 characters");
        assert!(message.contains("configured listing cap of 200"));
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("consecutive bash")));
        assert!(diag.errors().iter().any(|e| e.contains("SKILL.md:6:")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s021_short_breadcrumb_still_consecutive() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n\n```bash\necho hello\n```\n\nThen run the second command:\n\n```bash\necho world\n```\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("consecutive bash")));
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
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("backslash")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s022_retains_relative_inline_and_unc_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need path validation\n---\nUse path\\to\\file, \x60C:\\Program-Files\\App\x60, and \\\\server\\share.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
            "---\nname: my-skill\ndescription: Use when you need regex validation\n---\nUse \x60\\n\\t\x60, \\d\\w, and \\alpha\\beta escapes.\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("backslash")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s022_fenced_windows_path_not_flagged() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need path validation\n---\n\x60\x60\x60text\nC:\\Users\\admin\\file.txt\n\x60\x60\x60\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("effort") && e.contains("low/medium/high/xhigh/max"))
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
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("effort")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s025_xhigh_effort_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need effort testing\neffort: xhigh\n---\nBody content\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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

        // Normal mode: S028 fires as a warning, not an error.
        let mut normal = DiagnosticCollector::new();
        validate_skill_content(&mut normal, &crate::config::ExcludeSet::default());
        assert!(
            normal
                .warnings()
                .iter()
                .any(|e| e.contains("body uses $ARGUMENTS")),
            "S028 should warn in normal mode, got: {:?}",
            normal.warnings()
        );
        assert!(
            !normal
                .errors()
                .iter()
                .any(|e| e.contains("body uses $ARGUMENTS")),
            "S028 must not be an error in normal mode, got: {:?}",
            normal.errors()
        );

        // Pedantic mode: the same warning is promoted to an error.
        let mut pedantic_config = crate::config::LintConfig::default();
        pedantic_config.apply_cli_mode(crate::config::CliMode::Pedantic);
        let mut pedantic = DiagnosticCollector::with_config(pedantic_config);
        validate_skill_content(&mut pedantic, &crate::config::ExcludeSet::default());
        assert!(
            pedantic
                .errors()
                .iter()
                .any(|e| e.contains("body uses $ARGUMENTS")),
            "S028 should error under --pedantic, got: {:?}",
            pedantic.errors()
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
        // Hint present and body uses $ARGUMENTS: neither S028 nor S069 fires,
        // at any severity channel.
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.warnings().iter().any(|e| e.contains("argument-hint")));
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
            !diag.warnings().iter().any(|e| e.contains("argument-hint"))
                && !diag.errors().iter().any(|e| e.contains("argument-hint")),
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
            "---\nname: my-skill\ndescription: A valid skill description here\n---\nFetch from http://api.internal.corp/v1\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("non-HTTPS")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s031_xml_namespace_identifier_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\nOutput SVG:\n<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\">\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("non-HTTPS")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s031_reserved_name_hosts_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\nSee http://www.example.com/guide and http://foo.test/x and http://demo.invalid/\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("non-HTTPS")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s031_reports_line_and_url_evidence() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        // Frontmatter is 3 lines (---, name, description, ---): the body's first
        // line is file line 5, the URL is on the second body line (line 6).
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\nintro line\nFetch from http://api.internal.corp/v1?token=secret\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        let finding = diag
            .diagnostics()
            .iter()
            .find(|d| d.rule == LintRule::NonHttpsUrl)
            .expect("S031 finding");
        assert_eq!(finding.location.unwrap().start().line_number(), 6);
        // Evidence is scheme+host+path only: no query string, so no secret.
        assert_eq!(
            finding.evidence.as_deref(),
            Some("http://api.internal.corp/v1")
        );
        assert!(
            finding.message.contains(":6:"),
            "message: {}",
            finding.message
        );
        assert!(finding.message.contains("http://api.internal.corp/v1"));
    }

    // ── S029: nested-ref-deep ───────────────────────────────────────

    #[test]
    fn test_shared_md_refs_use_base_dir() {
        use crate::validators::shared_md_refs::find_shared_md_refs;

        let skills = find_shared_md_refs(
            "${CLAUDE_PLUGIN_ROOT}/skills/shared/helpers.md\n\
             ${CLAUDE_PLUGIN_ROOT}/skills/shared/sub/util.md\n\
             ${CLAUDE_PLUGIN_ROOT}/other/shared/helpers.md\n",
            "skills",
        );
        assert_eq!(
            skills
                .iter()
                .map(|r| r.relative_path.as_str())
                .collect::<Vec<_>>(),
            ["skills/shared/helpers.md", "skills/shared/sub/util.md"]
        );

        let claude = find_shared_md_refs(
            "${CLAUDE_PLUGIN_ROOT}/.claude/skills/shared/helpers.md\n\
             ${CLAUDE_PLUGIN_ROOT}/skills/shared/helpers.md\n",
            ".claude/skills",
        );
        assert_eq!(claude.len(), 1);
        assert_eq!(claude[0].relative_path, ".claude/skills/shared/helpers.md");
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        // S036 should fire once per unique file, not once per referencing skill
        let errors = diag.errors();
        let toc_count = errors
            .iter()
            .filter(|e| e.contains("no headings for navigation"))
            .count();
        assert_eq!(toc_count, 1);
    }

    #[test]
    #[serial_test::serial]
    fn test_s029_s036_ignore_prefix_and_commented_refs() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        // Truncated-on-old-regex target that nests and is long/heading-free.
        let long = "line\n".repeat(101);
        std::fs::write(
            "skills/shared/prefix.md",
            format!("{long}See ${{CLAUDE_PLUGIN_ROOT}}/skills/shared/other.md\n"),
        )
        .unwrap();
        std::fs::write("skills/shared/other.md", "# Other\n").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when testing shared-ref token boundaries\n---\n\
             <!-- ${CLAUDE_PLUGIN_ROOT}/skills/shared/prefix.md -->\n\
             ${CLAUDE_PLUGIN_ROOT}/skills/shared/prefix.md.backup\n\
             ${CLAUDE_PLUGIN_ROOT}/skills/shared/prefix.mdx\n\
             ${CLAUDE_PLUGIN_ROOT}/skills/shared/prefix.md/child\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("itself references")),
            "S029 must ignore comment/prefix tokens: {:?}",
            diag.errors()
        );
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("no headings for navigation")),
            "S036 must ignore comment/prefix tokens: {:?}",
            diag.errors()
        );
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
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("not referenced")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s030_orphaned_script_without_readable_markdown_still_reports() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts").unwrap();
        std::fs::write("skills/my-skill/scripts/orphan.sh", "#!/bin/bash\n").unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.diagnostics().iter().any(|finding| {
            finding.rule == LintRule::OrphanedSkillFiles
                && finding.subject_path.as_deref()
                    == Some(std::path::Path::new("skills/my-skill/scripts/orphan.sh"))
        }));
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
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("not referenced")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s030_transitive_reference_in_nested_md_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/transitive/scripts").unwrap();
        std::fs::create_dir_all("skills/transitive/references").unwrap();
        std::fs::write("skills/transitive/scripts/rollup.sh", "#!/bin/bash\n").unwrap();
        std::fs::write(
            "skills/transitive/references/usage.md",
            "# Usage\n\nRun `rollup.sh` after packaging.\n",
        )
        .unwrap();
        std::fs::write(
            "skills/transitive/SKILL.md",
            "---\nname: transitive\ndescription: Use when testing transitive script references\n---\nSee [usage](references/usage.md)\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("rollup.sh") && e.contains("not referenced")),
            "transitive docs should count: {:?}",
            diag.errors()
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s030_near_name_shadowing_still_orphans() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts").unwrap();
        std::fs::write("skills/my-skill/scripts/run.sh", "#!/bin/bash\n").unwrap();
        std::fs::write("skills/my-skill/scripts/dry-run.sh", "#!/bin/bash\n").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when testing near-name script shadowing\n---\nPrefer dry-run.sh before applying changes.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("scripts/run.sh") && e.contains("not referenced")),
            "run.sh must not be shadowed by dry-run.sh: {:?}",
            diag.errors()
        );
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("dry-run.sh") && e.contains("not referenced"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s030_fenced_mention_still_counts() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts").unwrap();
        std::fs::write("skills/my-skill/scripts/helper.sh", "#!/bin/bash\n").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when testing fenced script mentions\n---\n```bash\n./helper.sh\n```\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("not referenced")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s030_nested_scripts_require_exact_path_or_unique_basename() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts/lib").unwrap();
        std::fs::write("skills/my-skill/scripts/lib/orphan.sh", "#!/bin/bash\n").unwrap();

        for lookalike in [
            "orphan.sh.bak",
            "orphan.sh2",
            "dry-orphan.sh",
            "scripts/lib/orphan.sh/child",
        ] {
            std::fs::write(
                "skills/my-skill/SKILL.md",
                format!(
                    "---\nname: my-skill\ndescription: Use when testing exact script references\n---\nRun `{lookalike}`.\n"
                ),
            )
            .unwrap();
            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
            assert!(
                diag.diagnostics().iter().any(|finding| {
                    finding.rule == LintRule::OrphanedSkillFiles
                        && finding.subject_path.as_deref()
                            == Some(std::path::Path::new(
                                "skills/my-skill/scripts/lib/orphan.sh",
                            ))
                }),
                "{lookalike} must not reference the nested script"
            );
        }

        for reference in [
            "scripts/lib/orphan.sh",
            "orphan.sh",
            "./scripts/lib/orphan.sh",
        ] {
            std::fs::write(
                "skills/my-skill/SKILL.md",
                format!(
                    "---\nname: my-skill\ndescription: Use when testing exact script references\n---\nRun `{reference}`.\n"
                ),
            )
            .unwrap();
            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
            assert!(
                !diag
                    .diagnostics()
                    .iter()
                    .any(|finding| finding.rule == LintRule::OrphanedSkillFiles),
                "{reference} must reference the nested script"
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_s030_duplicate_basenames_require_relative_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts/a").unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts/b").unwrap();
        std::fs::write("skills/my-skill/scripts/a/run.sh", "#!/bin/bash\n").unwrap();
        std::fs::write("skills/my-skill/scripts/b/run.sh", "#!/bin/bash\n").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when testing duplicate script basenames\n---\nRun `run.sh`.\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        let subjects: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|finding| finding.rule == LintRule::OrphanedSkillFiles)
            .filter_map(|finding| finding.subject_path.as_ref())
            .map(|path| path.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            subjects,
            [
                "skills/my-skill/scripts/a/run.sh",
                "skills/my-skill/scripts/b/run.sh"
            ]
        );

        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when testing duplicate script basenames\n---\nRun `scripts/a/run.sh` and `scripts/b/run.sh`.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|finding| finding.rule == LintRule::OrphanedSkillFiles)
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s030_scans_packaged_nested_files_and_honors_exclusion() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts/dist").unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts/lib").unwrap();
        std::fs::write("skills/my-skill/scripts/dist/packaged.sh", "#!/bin/bash\n").unwrap();
        std::fs::write("skills/my-skill/scripts/lib/excluded.sh", "#!/bin/bash\n").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when testing nested script traversal\n---\nNo script references.\n",
        )
        .unwrap();

        let exclude =
            crate::config::ExcludeSet::new(
                &["skills/my-skill/scripts/lib/excluded.sh".to_string()],
            )
            .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &exclude);
        assert!(diag.diagnostics().iter().any(|finding| {
            finding.rule == LintRule::OrphanedSkillFiles
                && finding.subject_path.as_deref()
                    == Some(std::path::Path::new(
                        "skills/my-skill/scripts/dist/packaged.sh",
                    ))
        }));
        assert!(!diag.diagnostics().iter().any(|finding| {
            finding.subject_path.as_deref()
                == Some(std::path::Path::new(
                    "skills/my-skill/scripts/lib/excluded.sh",
                ))
        }));
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn test_s030_does_not_follow_directory_symlinks() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts").unwrap();
        std::fs::create_dir_all("outside").unwrap();
        std::fs::write("outside/orphan.sh", "#!/bin/bash\n").unwrap();
        symlink(tmp.path().join("outside"), "skills/my-skill/scripts/linked").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when testing script symlink traversal\n---\nBody.\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.diagnostics().iter().any(|finding| {
            finding.subject_path.as_deref()
                == Some(std::path::Path::new(
                    "skills/my-skill/scripts/linked/orphan.sh",
                ))
        }));
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("hardcoded secret")));
    }

    #[test]
    #[serial_test::serial]
    fn content_security_reports_on_agent_and_cursor_surfaces() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        for (base, body, rule) in [
            (
                ".agents/skills",
                "Set key to sk-aBcDeFgHiJkLmNoPqRsT1234",
                LintRule::HardcodedSecret,
            ),
            (
                ".agents/skills",
                "Fetch from http://api.corp/x",
                LintRule::NonHttpsUrl,
            ),
            (
                ".cursor/skills",
                "Set key to sk-aBcDeFgHiJkLmNoPqRsT1234",
                LintRule::HardcodedSecret,
            ),
            (
                ".cursor/skills",
                "Fetch from http://api.corp/x",
                LintRule::NonHttpsUrl,
            ),
        ] {
            let skill_dir = format!("{base}/leaky");
            std::fs::create_dir_all(&skill_dir).unwrap();
            let path = format!("{skill_dir}/SKILL.md");
            std::fs::write(
                &path,
                format!(
                    "---\nname: leaky\ndescription: A valid skill description here\n---\n{body}\n"
                ),
            )
            .unwrap();
            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_agent_skills_content_security(
                base,
                &mut diag,
                &crate::config::ExcludeSet::default(),
            );
            let finding = diag
                .diagnostics()
                .iter()
                .find(|item| item.rule == rule)
                .unwrap_or_else(|| panic!("expected {rule:?} for {path}"));
            assert_eq!(
                finding.subject_path.as_ref().map(|p| p.to_string_lossy()),
                Some(std::borrow::Cow::Borrowed(path.as_str()))
            );
            std::fs::remove_dir_all(base).unwrap();
        }
    }

    #[test]
    #[serial_test::serial]
    fn content_security_respects_exclusion_and_per_file_suppression() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".agents/skills/leaky").unwrap();
        std::fs::write(
            ".agents/skills/leaky/SKILL.md",
            "---\nname: leaky\ndescription: A valid skill description here\n---\nSet key to sk-aBcDeFgHiJkLmNoPqRsT1234\n",
        )
        .unwrap();
        std::fs::create_dir_all(".agents/skills/safe").unwrap();
        std::fs::write(
            ".agents/skills/safe/SKILL.md",
            "---\nname: safe\ndescription: A valid skill description here\n---\nSet key to sk-aBcDeFgHiJkLmNoPqRsT1234\n",
        )
        .unwrap();

        let excluded =
            crate::config::ExcludeSet::new(&[".agents/skills/leaky/**".to_string()]).unwrap();
        let mut with_exclude = DiagnosticCollector::new_all_enabled();
        validate_agent_skills_content_security(".agents/skills", &mut with_exclude, &excluded);
        assert_eq!(
            with_exclude
                .diagnostics()
                .iter()
                .filter(|item| item.rule == LintRule::HardcodedSecret)
                .count(),
            1
        );
        assert_eq!(
            with_exclude.diagnostics()[0]
                .subject_path
                .as_ref()
                .map(|p| p.to_string_lossy()),
            Some(std::borrow::Cow::Borrowed(".agents/skills/safe/SKILL.md"))
        );

        std::fs::write(
            "agent-lint.toml",
            r#"
[lint]
[[lint.overrides]]
files = [".agents/skills/leaky/SKILL.md"]
suppress = ["S032"]
"#,
        )
        .unwrap();
        let config = crate::config::LintConfig::load(tmp.path()).unwrap();
        let mut overridden = DiagnosticCollector::with_config(config);
        validate_agent_skills_content_security(
            ".agents/skills",
            &mut overridden,
            &crate::config::ExcludeSet::default(),
        );
        assert_eq!(overridden.suppressed_count(), 1);
        assert_eq!(
            overridden
                .diagnostics()
                .iter()
                .filter(|item| item.rule == LintRule::HardcodedSecret)
                .count(),
            1
        );
        assert_eq!(
            overridden.diagnostics()[0]
                .subject_path
                .as_ref()
                .map(|p| p.to_string_lossy()),
            Some(std::borrow::Cow::Borrowed(".agents/skills/safe/SKILL.md"))
        );
    }

    // ── S033: name-vague ─────────────────────────────────────────────

    fn write_plugin_skill(name: &str) {
        std::fs::create_dir_all(format!("skills/{name}")).unwrap();
        std::fs::write(
            format!("skills/{name}/SKILL.md"),
            format!(
                "---\nname: {name}\ndescription: Use when exercising skill name validation thoroughly\n---\n# Skill name validation\n\nExercise skill name validation thoroughly for published plugin skills.\n"
            ),
        )
        .unwrap();
    }

    fn name_vague_count(diag: &DiagnosticCollector) -> usize {
        diag.diagnostics()
            .iter()
            .filter(|d| d.rule == LintRule::NameVague)
            .count()
    }

    #[test]
    #[serial_test::serial]
    fn test_s033_vague_name_helper() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        write_plugin_skill("helper");
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("domainless")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s033_specific_name_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        write_plugin_skill("code-review");
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(name_vague_count(&diag), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_s033_subject_nouns_and_compounds_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        for name in ["data", "files", "documents", "pdf-helper", "lint-utils"] {
            write_plugin_skill(name);
        }
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(name_vague_count(&diag), 0);
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
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_private_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        // S033 is plugin-only, should not fire in private mode
        assert_eq!(name_vague_count(&diag), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_s033_default_warning_pedantic_and_all() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        write_plugin_skill("utils");

        let mut normal = DiagnosticCollector::new();
        validate_skill_content(&mut normal, &crate::config::ExcludeSet::default());
        assert_eq!(name_vague_count(&normal), 1);
        assert!(normal.warnings().iter().any(|e| e.contains("domainless")));
        assert!(!normal.errors().iter().any(|e| e.contains("domainless")));

        let mut pedantic_config = crate::config::LintConfig::default();
        pedantic_config.apply_cli_mode(crate::config::CliMode::Pedantic);
        let mut pedantic = DiagnosticCollector::with_config(pedantic_config);
        validate_skill_content(&mut pedantic, &crate::config::ExcludeSet::default());
        assert_eq!(name_vague_count(&pedantic), 1);
        assert!(pedantic.errors().iter().any(|e| e.contains("domainless")));

        let mut all_config = crate::config::LintConfig::default();
        all_config.apply_cli_mode(crate::config::CliMode::All);
        let mut all = DiagnosticCollector::with_config(all_config);
        validate_skill_content(&mut all, &crate::config::ExcludeSet::default());
        assert_eq!(name_vague_count(&all), 1);
        assert!(all.errors().iter().any(|e| e.contains("domainless")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s033_only_focus_exclusion_and_override() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        write_plugin_skill("tools");
        write_plugin_skill("helper");

        let only = crate::config::RunPolicy::resolve(
            crate::config::CliMode::Normal,
            &["S033".to_string()],
        )
        .unwrap();
        let mut focused =
            DiagnosticCollector::with_run_policy(crate::config::LintConfig::default(), only);
        validate_skill_content(&mut focused, &crate::config::ExcludeSet::default());
        assert_eq!(name_vague_count(&focused), 2);
        assert!(
            focused
                .diagnostics()
                .iter()
                .all(|d| d.rule == LintRule::NameVague)
        );

        let excluded = crate::config::ExcludeSet::new(&["skills/helper/**".to_string()])
            .expect("exclude compiles");
        let mut with_exclude = DiagnosticCollector::new();
        validate_skill_content(&mut with_exclude, &excluded);
        assert_eq!(name_vague_count(&with_exclude), 1);
        assert!(
            with_exclude
                .diagnostics()
                .iter()
                .any(|d| d.rule == LintRule::NameVague
                    && d.message.contains("skills/tools/SKILL.md"))
        );

        std::fs::write(
            "agent-lint.toml",
            r#"
[lint]
[[lint.overrides]]
files = ["skills/tools/**"]
suppress = ["S033"]
"#,
        )
        .unwrap();
        let config = crate::config::LintConfig::load(tmp.path()).unwrap();
        let mut overridden = DiagnosticCollector::with_config(config);
        validate_skill_content(&mut overridden, &crate::config::ExcludeSet::default());
        assert_eq!(name_vague_count(&overridden), 1);
        assert!(
            overridden
                .diagnostics()
                .iter()
                .any(|d| d.rule == LintRule::NameVague
                    && d.message.contains("skills/helper/SKILL.md"))
        );
        assert_eq!(overridden.suppressed_count(), 1);
    }

    #[test]
    #[serial_test::serial]
    fn test_s033_message_and_suggestion_are_actionable() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        write_plugin_skill("utility");
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        let finding = diag
            .diagnostics()
            .iter()
            .find(|d| d.rule == LintRule::NameVague)
            .expect("S033 diagnostic");
        assert!(finding.message.contains("domainless"));
        assert_eq!(
            finding.suggestion.as_deref(),
            Some(
                "Add the missing domain or task to the exact skill name (for example 'pdf-helper' or 'lint-utils'), rather than renaming for morphology alone."
            )
        );
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        // These fields exceed S015 together; S016 still stays plugin-only.
        let long_desc = format!("Use when you need to {}", "x".repeat(875));
        let when_to_use = "y".repeat(700);
        std::fs::write(
            ".claude/skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: {long_desc}\nwhen_to_use: {when_to_use}\n---\nBody content\n"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_private_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        // S016 remains plugin-only, while configurable S015 covers private skills.
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("first/second person"))
        );
        assert!(
            diag.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.rule == LintRule::DescTruncated)
        );
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        // Create a shared .md > 100 lines with no headings
        let long_content = "line\n".repeat(101);
        std::fs::write("skills/shared/big-ref.md", &long_content).unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\nSee ${CLAUDE_PLUGIN_ROOT}/skills/shared/big-ref.md\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("no headings for navigation"))
        );
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
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("no headings for navigation"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s036_hash_and_h3_only_headings_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let mut content = String::from("# Guide\n");
        for i in 0..60 {
            content.push_str(&format!("### Section {i}\nline\n"));
        }
        assert!(content.lines().count() > 100);
        std::fs::write("skills/shared/guide.md", &content).unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when testing non-## heading navigation\n---\nSee ${CLAUDE_PLUGIN_ROOT}/skills/shared/guide.md\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("no headings for navigation"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s036_heading_only_inside_fence_warns() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let mut content = String::from("```md\n## Example heading\n```\n");
        for _ in 0..100 {
            content.push_str("line\n");
        }
        std::fs::write("skills/shared/fenced.md", &content).unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when testing fenced false headings\n---\nSee ${CLAUDE_PLUGIN_ROOT}/skills/shared/fenced.md\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("no headings for navigation"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s036_excluded_shared_file_silent() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let long_content = "line\n".repeat(101);
        std::fs::write("skills/shared/excluded.md", &long_content).unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when testing excluded shared refs\n---\nSee ${CLAUDE_PLUGIN_ROOT}/skills/shared/excluded.md\n",
        )
        .unwrap();
        let exclude =
            crate::config::ExcludeSet::new(&["skills/shared/excluded.md".to_string()]).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &exclude);
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("no headings for navigation"))
        );
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("date/year")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s038_masks_inline_code_and_link_destinations() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let path = "skills/my-skill/SKILL.md";
        for (body, expected) in [
            ("Format dates as `2025-01-01`.\n", false),
            (
                "Read [the guide](https://example.test/archive/2030).\n",
                false,
            ),
            ("This expires after 2030.\n", true),
        ] {
            std::fs::write(
                path,
                format!(
                    "---\nname: my-skill\ndescription: Use when this skill is needed\n---\n{body}"
                ),
            )
            .unwrap();
            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
            assert_eq!(
                diag.diagnostics()
                    .iter()
                    .any(|d| d.rule == LintRule::TimeSensitive),
                expected,
                "unexpected S038 result for {body}"
            );
        }
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        // A present-but-non-mapping `metadata` (here a bare boolean) reports the
        // single shape diagnostic rather than a per-entry one.
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when testing metadata validation\nmetadata: true\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("metadata must be a map of string values")),
            "S039 shape diagnostic expected, got: {:?}",
            diag.errors()
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("unrecognized tool"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s040_end_conversation_clean() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\nallowed-tools: EndConversation\n---\nBody content\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("unrecognized tool")),
            "S040 must accept EndConversation"
        );
    }

    /// Write one skill with the given frontmatter tool lines and return every
    /// S040/S067 diagnostic message. Shared by the #342 tool-field tests.
    fn tool_field_diagnostics(tool_lines: &str) -> Vec<String> {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!(
                "---\nname: my-skill\ndescription: A valid skill description here\n{tool_lines}\n---\nBody content\n"
            ),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        diag.errors()
            .iter()
            .filter(|e| e.contains("unrecognized tool") || e.contains("unscoped Bash"))
            .cloned()
            .collect()
    }

    /// #342 regressions: every documented `allowed-tools` spelling parses
    /// correctly; parenthesized commas/spaces never split entries.
    #[test]
    #[serial_test::serial]
    fn test_s040_documented_forms_are_clean() {
        for tool_lines in [
            // Space-separated (documented) — was "unrecognized tool 'Read Write'".
            "allowed-tools: Read Write",
            // Comma inside a scope is pattern text, not a separator.
            "allowed-tools: Bash(npm install, npm test), Read",
            // The documented space-separated scoped example: three entries,
            // all recognized, none firing S067.
            "allowed-tools: Bash(git add *) Bash(git commit *) Bash(git status *)",
        ] {
            let findings = tool_field_diagnostics(tool_lines);
            assert!(
                findings.is_empty(),
                "{tool_lines} must be clean, got: {findings:?}"
            );
        }
        // Flow sequence — was two nonsense diagnostics '[Bash' and 'Read]'.
        // S067 legitimately fires for the exact `Bash` entry; S040 must not.
        let findings = tool_field_diagnostics("allowed-tools: [Bash, Read]");
        assert!(
            !findings.iter().any(|e| e.contains("unrecognized tool")),
            "flow sequence must not produce S040 nonsense entries: {findings:?}"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s040_unknown_tools_fire_from_both_fields_and_all_forms() {
        // Scalar allowed-tools typo.
        let findings = tool_field_diagnostics("allowed-tools: Bsh");
        assert!(
            findings
                .iter()
                .any(|e| e.contains("allowed-tools lists unrecognized tool 'Bsh'")),
            "scalar Bsh must fire S040: {findings:?}"
        );
        // Block list containing a typo (list users previously got no
        // validation at all).
        let findings = tool_field_diagnostics("allowed-tools:\n  - Bsh\n  - Read");
        assert!(
            findings
                .iter()
                .any(|e| e.contains("allowed-tools lists unrecognized tool 'Bsh'")),
            "block-list Bsh must fire S040: {findings:?}"
        );
        // disallowed-tools is validated too, and the message names the field.
        let findings = tool_field_diagnostics("disallowed-tools: AskUserQuestin");
        assert!(
            findings
                .iter()
                .any(|e| e.contains("disallowed-tools lists unrecognized tool 'AskUserQuestin'")),
            "disallowed-tools typo must fire S040 naming the field: {findings:?}"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s040_duplicate_unknowns_report_once_per_field_and_name() {
        let findings = tool_field_diagnostics("allowed-tools: Bsh, Bsh");
        assert_eq!(
            findings
                .iter()
                .filter(|e| e.contains("unrecognized tool 'Bsh'"))
                .count(),
            1,
            "duplicate unknown entries must report once per (field, name): {findings:?}"
        );
        // The same unknown name in both fields reports once per field.
        let findings = tool_field_diagnostics("allowed-tools: Bsh\ndisallowed-tools: Bsh");
        assert_eq!(
            findings
                .iter()
                .filter(|e| e.contains("unrecognized tool 'Bsh'"))
                .count(),
            2,
            "each field reports its own unknown entry: {findings:?}"
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
            "---\nname: my-skill\ndescription: A valid skill description here\ncontext: fork\n---\nThis skill is about weather data.\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("fork") && e.contains("task"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s041_fork_review_verbs_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\ncontext: fork\n---\nReview the diff and check for bugs. Summarize findings and report back concisely.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("fork") && e.contains("task")),
            "S041 must not fire on review/check/summarize/report fork prompts"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s041_defaults_to_warning() {
        assert_eq!(
            crate::rules::LintRule::ForkNoTask.default_severity(),
            crate::rules::DefaultSeverity::Warning
        );
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\ncontext: fork\n---\nThis skill is about weather data.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.warnings()
                .iter()
                .any(|e| e.contains("fork") && e.contains("task")),
            "S041 should fire as a warning under default config"
        );
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("fork") && e.contains("task")),
            "S041 should not fire as an error under default config"
        );
    }

    // ── S042: dmi-empty-desc (soft-retired) ──────────────────────────
    // S042 is a strict subset of S005 and no longer fires from any path; S005
    // remains the sole diagnostic for a missing/empty description.

    #[test]
    #[serial_test::serial]
    fn test_s042_retired_dmi_empty_desc_reports_only_s005() {
        for description in [
            "description:",
            "description: \"\"",
            "description: [not, a, string]",
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let _guard = crate::test_helpers::CwdGuard::new();
            std::env::set_current_dir(tmp.path()).unwrap();
            std::fs::create_dir_all("skills/my-skill").unwrap();
            std::fs::write(
                "skills/my-skill/SKILL.md",
                format!(
                    "---\nname: my-skill\n{description}\ndisable-model-invocation: true\n---\nBody content\n"
                ),
            )
            .unwrap();
            let exclude = crate::config::ExcludeSet::default();
            let mut diag = DiagnosticCollector::new_all_enabled();
            // S005 lives in the frontmatter pass; the (retired) S042 lived in the
            // content pass. Run both so we see the whole picture on one file.
            crate::validators::skills::validate_skill_frontmatter(&mut diag, &exclude);
            validate_skill_content(&mut diag, &exclude);
            // S005 is the sole diagnostic; S042 never fires.
            assert!(
                fires(diag.diagnostics(), LintRule::FrontmatterFieldMissing),
                "S005 must fire for {description:?}"
            );
            assert!(
                diag.diagnostics()
                    .iter()
                    .all(|d| d.rule != LintRule::DmiEmptyDesc),
                "S042 must not fire for {description:?}, got: {:?}",
                diag.diagnostics()
                    .iter()
                    .map(|d| d.rule.code())
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_s042_retired_even_with_dmi_and_valid_desc() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when the skill should be user-only\ndisable-model-invocation: true\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.diagnostics()
                .iter()
                .all(|d| d.rule != LintRule::DmiEmptyDesc)
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
    fn test_s046_numbered_list_with_continuation_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let mut body = "Some text\n".repeat(295);
        body.push_str(
            "1. Step number 1 does something.\n   Continuation detail for step 1.\n\
             2. Step number 2 does something.\n   Continuation detail for step 2.\n\
             3. Step number 3 does something.\n   Continuation detail for step 3.\n",
        );
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\n{body}"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("workflow structure")),
            "S046 must accept numbered lists with indented continuation lines"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s046_paren_numbered_list_workflow() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let mut body = "Some text\n".repeat(298);
        body.push_str("1) First step\n2) Second step\n3) Third step\n");
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\n{body}"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("workflow structure")),
            "S046 must accept CommonMark 1) ordered lists"
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
    fn test_s047_plural_examples_heading_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let mut body = "Some text\n".repeat(200);
        body.push_str("## Examples\n");
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\n{body}"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("examples or templates")),
            "S047 must accept ## Examples"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s047_plural_examples_bold_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let mut body = "Some text\n".repeat(200);
        body.push_str("**Examples:**\n");
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\n{body}"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("examples or templates")),
            "S047 must accept **Examples:**"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s047_plural_templates_heading_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let mut body = "Some text\n".repeat(200);
        body.push_str("## Templates\n");
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\n{body}"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("examples or templates")),
            "S047 must accept ## Templates"
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("description keywords not reflected in body")),
            "S054 should not fire when body is empty (S020 handles that)"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s054_changelog_inflection_alignment_clean() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/changelog").unwrap();
        std::fs::write(
            "skills/changelog/SKILL.md",
            "---\nname: changelog\ndescription: Generates changelogs and commit summaries from git diffs. Use when releasing versions.\n---\nGenerate a changelog entry, write a commit summary for each change, analyze the git diff, and record the released version.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("description keywords not reflected in body")),
            "S054 should score 8/8 after stemming for the changelog evidence case"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s016_s017_default_warning_pedantic_and_all() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/person").unwrap();
        std::fs::write(
            "skills/person/SKILL.md",
            "---\nname: person\ndescription: I can help you process uploaded files for analysis\n---\nBody content for person pronoun severity coverage.\n",
        )
        .unwrap();
        std::fs::create_dir_all("skills/trigger").unwrap();
        std::fs::write(
            "skills/trigger/SKILL.md",
            "---\nname: trigger\ndescription: A skill that analyzes repository source trees carefully\n---\nBody content for missing trigger severity coverage.\n",
        )
        .unwrap();

        let mut normal = DiagnosticCollector::new();
        validate_skill_content(&mut normal, &crate::config::ExcludeSet::default());
        assert!(
            normal
                .warnings()
                .iter()
                .any(|e| e.contains("first/second person")),
            "S016 defaults to warning: {:?}",
            normal.warnings()
        );
        assert!(
            normal.warnings().iter().any(|e| e.contains("trigger")),
            "S017 defaults to warning: {:?}",
            normal.warnings()
        );
        assert!(
            !normal
                .errors()
                .iter()
                .any(|e| e.contains("first/second person") || e.contains("trigger")),
            "S016/S017 must not be errors in normal mode"
        );

        let mut pedantic_config = crate::config::LintConfig::default();
        pedantic_config.apply_cli_mode(crate::config::CliMode::Pedantic);
        let mut pedantic = DiagnosticCollector::with_config(pedantic_config);
        validate_skill_content(&mut pedantic, &crate::config::ExcludeSet::default());
        assert!(
            pedantic
                .errors()
                .iter()
                .any(|e| e.contains("first/second person"))
        );
        assert!(pedantic.errors().iter().any(|e| e.contains("trigger")));

        let mut all_config = crate::config::LintConfig::default();
        all_config.apply_cli_mode(crate::config::CliMode::All);
        let mut all = DiagnosticCollector::with_config(all_config);
        validate_skill_content(&mut all, &crate::config::ExcludeSet::default());
        assert!(
            all.errors()
                .iter()
                .any(|e| e.contains("first/second person"))
        );
        assert!(all.errors().iter().any(|e| e.contains("trigger")));
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
    fn test_config_suppress_suppresses_rule() {
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

        // Without config: S020 fires (default-error), S034 is silently
        // skipped (default-suppressed).
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("no content")));
        assert!(
            !diag.errors().iter().any(|e| e.contains("under 20")),
            "S034 should be default-suppressed"
        );

        // With config suppressing S020
        use crate::rules::LintRule;
        let config = crate::config::LintConfig {
            suppress: std::collections::HashSet::from([LintRule::BodyEmpty]),
            error: std::collections::HashSet::from([LintRule::DescTooShort]),
            warn: std::collections::HashSet::new(),
            exclude: vec![],
            ..crate::config::LintConfig::default()
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
            suppress: std::collections::HashSet::new(),
            error: std::collections::HashSet::new(),
            warn: std::collections::HashSet::from([LintRule::DescTooShort]),
            exclude: vec![],
            ..crate::config::LintConfig::default()
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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

    // ── S045: tools-list-syntax (soft-retired, #342) ───────────────

    #[test]
    #[serial_test::serial]
    fn test_s045_yaml_list_is_a_documented_form_and_never_fires() {
        // A YAML list is a documented accepted `allowed-tools` spelling; the
        // retired S045 must not fire and S007 must not call the list empty.
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\nallowed-tools:\n  - Bash(git *)\n  - Read\n---\nBody content\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        // Run both validators (skills frontmatter + content) like the full pipeline would
        crate::validators::skills::validate_skill_frontmatter(
            &mut diag,
            &crate::config::ExcludeSet::default(),
        );
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag.errors().iter().any(|e| e.contains("list syntax")),
            "retired S045 must not fire on a YAML list, got: {:?}",
            diag.errors()
        );
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("allowed-tools") && e.contains("present but empty")),
            "S007 must not call a documented YAML list empty, got: {:?}",
            diag.errors()
        );
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("unrecognized tool") || e.contains("unscoped Bash")),
            "scoped/recognized list entries must stay clean, got: {:?}",
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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

        let mut diag = DiagnosticCollector::new_all_enabled();
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

        let mut diag = DiagnosticCollector::new_all_enabled();
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

        let mut diag = DiagnosticCollector::new_all_enabled();
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

        let mut diag = DiagnosticCollector::new_all_enabled();
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

        let mut diag = DiagnosticCollector::new_all_enabled();
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

        let mut diag = DiagnosticCollector::new_all_enabled();
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

        let mut diag = DiagnosticCollector::new_all_enabled();
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

    #[test]
    #[serial_test::serial]
    fn test_s044_evidence_line_because_no_fire() {
        // Regression for the leaf-#251 evidence line: the only "context" is the
        // substring `use` inside *Because*, which must not pass the word-boundary gate.
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A skill\n---\nBecause the `user_id` column is indexed, lookups stay fast.\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("MCP tool reference")),
            "Should not fire S044 on the 'Because' evidence line, got: {:?}",
            diag.errors()
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s044_substring_context_no_fire() {
        // Substrings of the vocabulary in ordinary prose must not pass the gate:
        // *reused* is not *use*, *user* is not *use*, *prune* is not *run*.
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A skill\n---\nThe `user_id` is reused across requests.\nAsk the user about `retry_count`.\nWe prune `old_entries` nightly.\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("MCP tool reference")),
            "Should not fire S044 on substring-only context lines, got: {:?}",
            diag.errors()
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s044_word_boundary_positives_fire() {
        // Genuine invocation vocabulary (including inflections and the plural noun)
        // must still fire once per distinct identifier.
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        // "Use `create_issue`" omits "tool" so the `use` branch is exercised alone;
        // "call"/"Invoke" lines likewise carry no "tool", isolating those branches.
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A skill\n---\nUse `create_issue` to file bugs.\nRun the `sync_data` tool.\ncall `fetch_records` first\nInvoke `update_row` afterwards.\nthese tools: `list_files`\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        for identifier in [
            "create_issue",
            "sync_data",
            "fetch_records",
            "update_row",
            "list_files",
        ] {
            assert_eq!(
                diag.errors()
                    .iter()
                    .filter(|e| e.contains(identifier) && e.contains("MCP tool reference"))
                    .count(),
                1,
                "Expected exactly one S044 for '{identifier}', got: {:?}",
                diag.errors()
            );
        }
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
    fn test_s048_recurses_outside_scripts_and_honors_exclusion() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/references").unwrap();
        std::fs::create_dir_all("skills/my-skill/notes/deeper").unwrap();
        std::fs::create_dir_all("skills/my-skill/examples").unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when testing recursive reference names\n---\nBody.\n",
        )
        .unwrap();
        std::fs::write("skills/my-skill/references/doc.md", "reference").unwrap();
        std::fs::write("skills/my-skill/notes/deeper/a.md", "excluded reference").unwrap();
        std::fs::write("skills/my-skill/examples/TEST.MD", "example").unwrap();
        std::fs::write("skills/my-skill/scripts/doc.md", "script asset").unwrap();
        std::fs::write("skills/my-skill/references/api-contract.md", "descriptive").unwrap();

        let exclude =
            crate::config::ExcludeSet::new(&["skills/my-skill/notes/deeper/a.md".to_string()])
                .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &exclude);
        let subjects: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|finding| finding.rule == LintRule::RefNameGeneric)
            .filter_map(|finding| finding.subject_path.as_ref())
            .map(|path| path.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            subjects,
            [
                "skills/my-skill/examples/TEST.MD",
                "skills/my-skill/references/doc.md"
            ]
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_private_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("non-descriptive reference file name"))
        );
    }

    // ── S049: name-not-gerund (retired; config alias retained) ───────

    #[test]
    #[serial_test::serial]
    fn test_s049_never_emits_under_all_for_non_gerund_names() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        write_plugin_skill("code-review");
        write_plugin_skill("string-utils");
        write_plugin_skill("pdf");

        let mut all_config = crate::config::LintConfig::default();
        all_config.apply_cli_mode(crate::config::CliMode::All);
        assert!(
            all_config.error.contains(&LintRule::NameNotGerund),
            "retired S049 remains selectable under --all for config compatibility"
        );
        let mut diag = DiagnosticCollector::with_config(all_config);
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|d| d.rule == LintRule::NameNotGerund),
            "retired S049 must stay inert even under --all"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s049_config_aliases_still_parse() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            r#"
[lint]
suppress = ["S049", "name-not-gerund"]
error = ["S049"]
warn = ["name-not-gerund"]
"#,
        )
        .unwrap();
        let config = crate::config::LintConfig::load(tmp.path()).unwrap();
        assert!(config.suppress.contains(&LintRule::NameNotGerund));
        // suppress wins over error/warn during load.
        assert!(!config.error.contains(&LintRule::NameNotGerund));
        assert!(!config.warn.contains(&LintRule::NameNotGerund));
        assert_eq!(
            LintRule::from_code_or_name("S049"),
            Some(LintRule::NameNotGerund)
        );
        assert_eq!(
            LintRule::from_code_or_name("name-not-gerund"),
            Some(LintRule::NameNotGerund)
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s049_promoted_via_config_still_inert() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        write_plugin_skill("code-review");
        let config = crate::config::LintConfig {
            error: std::collections::HashSet::from([LintRule::NameNotGerund]),
            ..crate::config::LintConfig::default()
        };
        assert!(config.error.contains(&LintRule::NameNotGerund));
        let mut diag = DiagnosticCollector::with_config(config);
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|d| d.rule == LintRule::NameNotGerund),
            "promoting retired S049 via config must not resurrect emissions"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s049_private_mode_also_inert() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude/skills/code-review").unwrap();
        std::fs::write(
            ".claude/skills/code-review/SKILL.md",
            "---\nname: code-review\ndescription: A valid skill description here\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_private_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|d| d.rule == LintRule::NameNotGerund)
        );
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag.errors().iter().any(|e| e.contains("synonym group")),
            "S053 should not count terms inside code fences"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s053_masks_inline_code_and_link_destinations_but_keeps_labels() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let path = "skills/my-skill/SKILL.md";
        for (body, expected) in [
            (
                "Run `git fetch` and `git pull`, then retrieve the changes.\n",
                false,
            ),
            (
                "Read [the documentation](https://example.test/endpoint/route/url).\n",
                false,
            ),
            (
                "Read [endpoint route URL](https://example.test/docs).\n",
                true,
            ),
        ] {
            std::fs::write(
                path,
                format!(
                    "---\nname: my-skill\ndescription: Use when this skill is needed\n---\n{body}"
                ),
            )
            .unwrap();
            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
            assert_eq!(
                diag.diagnostics()
                    .iter()
                    .any(|d| d.rule == LintRule::TerminologyInconsistent),
                expected,
                "unexpected S053 result for {body}"
            );
        }
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
    fn test_s055_sh_with_compound_or_exit_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts").unwrap();
        std::fs::write(
            "skills/my-skill/scripts/robust.sh",
            "#!/bin/bash\ncp \"$1\" \"$2\" || { echo \"copy failed\" >&2; exit 1; }\necho step1\necho step2\necho step3\n",
        )
        .unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n## Dependencies\n\npip install foo\n\n## Verify\n\nRun scripts/robust.sh to verify.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("lacks error handling")),
            "S055 must accept || {{ ...; exit 1; }} compound handlers"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s055_sh_with_if_not_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts").unwrap();
        std::fs::write(
            "skills/my-skill/scripts/robust.sh",
            "#!/bin/bash\nif ! grep -q done \"$2\"; then\n  echo \"marker missing\" >&2\n  exit 1\nfi\necho done\n",
        )
        .unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n## Dependencies\n\npip install foo\n\n## Verify\n\nRun scripts/robust.sh to verify.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("lacks error handling")),
            "S055 must accept if ! cmd negated-command guards"
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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

    #[test]
    #[serial_test::serial]
    fn test_s055_nested_script_subject_and_ordering() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts/lib").unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts/z").unwrap();
        std::fs::write(
            "skills/my-skill/scripts/lib/bad.sh",
            "#!/bin/bash\necho hello\necho world\necho foo\necho bar\necho done\n",
        )
        .unwrap();
        std::fs::write(
            "skills/my-skill/scripts/z/also-bad.sh",
            "#!/bin/bash\necho hello\necho world\necho foo\necho bar\necho done\n",
        )
        .unwrap();
        std::fs::write(
            "skills/my-skill/scripts/lib/good.sh",
            "#!/bin/bash\nset -euo pipefail\necho step1\necho step2\necho step3\n",
        )
        .unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n## Dependencies\n\npip install foo\n\n## Verify\n\nRun scripts to verify.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        let findings: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|d| d.rule == LintRule::ScriptErrhandMissing)
            .collect();
        assert_eq!(
            findings.len(),
            2,
            "expected two nested bad scripts: {findings:?}"
        );
        let subjects: Vec<_> = findings
            .iter()
            .map(|d| {
                d.subject_path
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default()
            })
            .collect();
        assert_eq!(
            subjects,
            vec![
                "skills/my-skill/scripts/lib/bad.sh".to_string(),
                "skills/my-skill/scripts/z/also-bad.sh".to_string(),
            ],
            "nested subjects must be full paths in deterministic order"
        );
        assert!(
            findings[0]
                .message
                .starts_with("skills/my-skill/scripts/lib/bad.sh:"),
            "message must begin with the script path, got: {}",
            findings[0].message
        );
        assert!(
            !findings.iter().any(|d| d
                .subject_path
                .as_ref()
                .is_some_and(|p| p.ends_with("SKILL.md"))),
            "S055 must not attribute findings to SKILL.md"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s055_extension_cases_case_insensitive() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts").unwrap();
        let bad = "#!/bin/bash\necho hello\necho world\necho foo\necho bar\necho done\n";
        let bad_py = "import sys\nimport os\ndef main():\n    print('hello')\nmain()\n";
        for (name, body) in [
            ("a.sh", bad),
            ("b.SH", bad),
            ("c.bash", bad),
            ("d.BASH", bad),
            ("e.py", bad_py),
            ("f.PY", bad_py),
        ] {
            std::fs::write(format!("skills/my-skill/scripts/{name}"), body).unwrap();
        }
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n## Dependencies\n\npip install foo\n\n## Verify\n\nRun scripts to verify.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        let subjects: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|d| d.rule == LintRule::ScriptErrhandMissing)
            .filter_map(|d| {
                d.subject_path
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
            })
            .collect();
        assert_eq!(
            subjects,
            vec![
                "skills/my-skill/scripts/a.sh".to_string(),
                "skills/my-skill/scripts/b.SH".to_string(),
                "skills/my-skill/scripts/c.bash".to_string(),
                "skills/my-skill/scripts/d.BASH".to_string(),
                "skills/my-skill/scripts/e.py".to_string(),
                "skills/my-skill/scripts/f.PY".to_string(),
            ]
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s055_shebang_extensionless_and_env_forms() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts").unwrap();
        let bad_sh = "echo hello\necho world\necho foo\necho bar\necho done\n";
        let bad_py = "import sys\nimport os\ndef main():\n    print('hello')\nmain()\n";
        std::fs::write(
            "skills/my-skill/scripts/direct-sh",
            format!("#!/bin/sh\n{bad_sh}"),
        )
        .unwrap();
        std::fs::write(
            "skills/my-skill/scripts/env-bash",
            format!("#!/usr/bin/env bash\n{bad_sh}"),
        )
        .unwrap();
        std::fs::write(
            "skills/my-skill/scripts/env-s-zsh",
            format!("#!/usr/bin/env -S zsh\n{bad_sh}"),
        )
        .unwrap();
        std::fs::write(
            "skills/my-skill/scripts/env-python",
            format!("#!/usr/bin/env python3\n{bad_py}"),
        )
        .unwrap();
        std::fs::write(
            "skills/my-skill/scripts/env-s-python",
            format!("#!/usr/bin/env -S python\n{bad_py}"),
        )
        .unwrap();
        std::fs::write(
            "skills/my-skill/scripts/node-helper",
            format!("#!/usr/bin/env node\n{bad_sh}"),
        )
        .unwrap();
        std::fs::write(
            "skills/my-skill/scripts/plain-data",
            "line1\nline2\nline3\nline4\nline5\n",
        )
        .unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n## Dependencies\n\npip install foo\n\n## Verify\n\nRun scripts to verify.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        let subjects: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|d| d.rule == LintRule::ScriptErrhandMissing)
            .filter_map(|d| {
                d.subject_path
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
            })
            .collect();
        assert_eq!(
            subjects,
            vec![
                "skills/my-skill/scripts/direct-sh".to_string(),
                "skills/my-skill/scripts/env-bash".to_string(),
                "skills/my-skill/scripts/env-python".to_string(),
                "skills/my-skill/scripts/env-s-python".to_string(),
                "skills/my-skill/scripts/env-s-zsh".to_string(),
            ],
            "node shebang and extensionless non-scripts must stay ignored; got {subjects:?}"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s055_python_requires_both_try_and_except() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts").unwrap();
        std::fs::write(
            "skills/my-skill/scripts/try_finally.py",
            "import sys\ntry:\n    do_something()\nfinally:\n    cleanup()\n",
        )
        .unwrap();
        std::fs::write(
            "skills/my-skill/scripts/except_only.py",
            "import sys\ndef main():\n    print('x')\nexcept Exception:\n    pass\n",
        )
        .unwrap();
        std::fs::write(
            "skills/my-skill/scripts/both.py",
            "import sys\ntry:\n    do_something()\nexcept Exception as e:\n    print(e)\n",
        )
        .unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n## Dependencies\n\npip install foo\n\n## Verify\n\nRun scripts to verify.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        let subjects: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|d| d.rule == LintRule::ScriptErrhandMissing)
            .filter_map(|d| {
                d.subject_path
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
            })
            .collect();
        assert_eq!(
            subjects,
            vec![
                "skills/my-skill/scripts/except_only.py".to_string(),
                "skills/my-skill/scripts/try_finally.py".to_string(),
            ],
            "try/finally and except-without-try must fire; try/except must pass"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s055_per_file_suppress_and_exclude() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts/lib").unwrap();
        let bad = "#!/bin/bash\necho hello\necho world\necho foo\necho bar\necho done\n";
        std::fs::write("skills/my-skill/scripts/keep-bad.sh", bad).unwrap();
        std::fs::write("skills/my-skill/scripts/suppress-me.sh", bad).unwrap();
        std::fs::write("skills/my-skill/scripts/lib/excluded.sh", bad).unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n## Dependencies\n\npip install foo\n\n## Verify\n\nRun scripts to verify.\n",
        )
        .unwrap();
        std::fs::write(
            "agent-lint.toml",
            r#"
[lint]
exclude = ["skills/my-skill/scripts/lib/excluded.sh"]

[[lint.overrides]]
files = ["skills/my-skill/scripts/suppress-me.sh"]
suppress = ["S055"]
"#,
        )
        .unwrap();
        let config = crate::config::LintConfig::load(tmp.path()).unwrap();
        let exclude = config.build_exclude_set();
        let mut diag = DiagnosticCollector::with_config(config);
        validate_skill_content(&mut diag, &exclude);
        let findings: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|d| d.rule == LintRule::ScriptErrhandMissing)
            .collect();
        assert_eq!(findings.len(), 1, "expected only keep-bad.sh: {findings:?}");
        assert_eq!(
            findings[0]
                .subject_path
                .as_ref()
                .map(|p| p.to_string_lossy()),
            Some(std::borrow::Cow::Borrowed(
                "skills/my-skill/scripts/keep-bad.sh"
            ))
        );
        assert_eq!(diag.suppressed_count(), 1);
    }

    #[test]
    #[serial_test::serial]
    fn test_s055_only_and_modes() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts").unwrap();
        std::fs::write(
            "skills/my-skill/scripts/bad.sh",
            "#!/bin/bash\necho hello\necho world\necho foo\necho bar\necho done\n",
        )
        .unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n## Dependencies\n\npip install foo\n\n## Verify\n\nRun scripts/bad.sh to verify.\n",
        )
        .unwrap();

        let only = crate::config::RunPolicy::resolve(
            crate::config::CliMode::Normal,
            &["S055".to_string()],
        )
        .unwrap();
        let mut focused =
            DiagnosticCollector::with_run_policy(crate::config::LintConfig::default(), only);
        validate_skill_content(&mut focused, &crate::config::ExcludeSet::default());
        assert!(
            focused
                .diagnostics()
                .iter()
                .all(|d| d.rule == LintRule::ScriptErrhandMissing)
        );
        assert_eq!(
            focused
                .diagnostics()
                .iter()
                .filter(|d| d.rule == LintRule::ScriptErrhandMissing)
                .count(),
            1
        );

        let mut pedantic_config = crate::config::LintConfig::default();
        pedantic_config.apply_cli_mode(crate::config::CliMode::Pedantic);
        let mut pedantic = DiagnosticCollector::with_config(pedantic_config);
        validate_skill_content(&mut pedantic, &crate::config::ExcludeSet::default());
        assert!(
            pedantic
                .diagnostics()
                .iter()
                .any(|d| d.rule == LintRule::ScriptErrhandMissing)
        );

        let mut all_config = crate::config::LintConfig::default();
        all_config.apply_cli_mode(crate::config::CliMode::All);
        let mut all = DiagnosticCollector::with_config(all_config);
        validate_skill_content(&mut all, &crate::config::ExcludeSet::default());
        assert!(
            all.diagnostics()
                .iter()
                .any(|d| d.rule == LintRule::ScriptErrhandMissing
                    && d.severity == crate::diagnostic::Severity::Error)
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("alternatives without stating a default")),
            "S056 should fire when 'if' is mid-line (not at start)"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s056_masks_inline_code_and_link_destinations_and_uses_paragraph_suppressors() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let path = "skills/my-skill/SKILL.md";
        for body in [
            "The build failed, rerun the tests or check the logs.\n",
            "`Choose tool A or tool B or tool C.`\n",
            "Read [documentation](https://example.test/use/tool/a/or/b/or/c).\n",
            "Choose parser A, parser B, or parser C.\nUse parser A by default.\n",
            "Choose parser A, parser B, or parser C.\n\nUse parser A by default.\n",
            "- Choose parser A, parser B, or parser C.\n- Use parser A by default.\n",
        ] {
            std::fs::write(
                path,
                format!(
                    "---\nname: my-skill\ndescription: A valid skill description here\n---\n{body}"
                ),
            )
            .unwrap();
            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
            let fires = diag
                .diagnostics()
                .iter()
                .any(|d| d.rule == LintRule::BodyNoDefault);
            if body.contains("\n\n") || body.starts_with("- ") {
                assert!(
                    fires,
                    "S056 should retain paragraph and list boundaries: {body}"
                );
            } else {
                assert!(!fires, "S056 should ignore or suppress: {body}");
            }
        }
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
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
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("undocumented magic number")),
            "S057 should NOT fire on comment lines containing assignment patterns"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s057_skips_flag_values_and_checks_every_assignment_in_order() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let path = "skills/my-skill/SKILL.md";
        for (line, expected) in [
            ("grep -r pattern --max-count=50 src/", None),
            ("java -Dprop=8", None),
            ("enabled=1 timeout=47", Some("timeout=47")),
            ("--max-count=50 timeout=47", Some("timeout=47")),
            ("enabled=1 timeout=60", None),
        ] {
            std::fs::write(
                path,
                format!("---\nname: my-skill\ndescription: A valid skill description here\n---\n```bash\n{line}\n```\n"),
            )
            .unwrap();
            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
            let finding = diag
                .diagnostics()
                .iter()
                .find(|d| d.rule == LintRule::MagicNumberUndoc);
            match expected {
                Some(expected) => assert!(
                    finding.is_some_and(|d| d.message.contains(expected)),
                    "S057 should report {expected}: {finding:?}"
                ),
                None => assert!(finding.is_none(), "S057 should ignore: {line}"),
            }
        }
    }

    // ── S063: model-invalid ──────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s063_model_typo() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\nmodel: sonet\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("'model'") && e.contains("sonet")),
            "S063 should fire for model typo, got: {:?}",
            diag.errors()
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s063_model_valid() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\nmodel: sonnet\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("'model'")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s063_a014_shared_vocabulary_table() {
        let cases: &[(&str, bool)] = &[
            ("fable", true),
            ("opusplan", true),
            ("best", true),
            ("claude-fable-5", true),
            ("haiku[1m]", false),
            ("inherit[1m]", false),
            ("sonet", false),
        ];
        for &(model, expect_valid) in cases {
            let tmp = tempfile::tempdir().unwrap();
            let _guard = crate::test_helpers::CwdGuard::new();
            std::env::set_current_dir(tmp.path()).unwrap();
            std::fs::create_dir_all("skills/my-skill").unwrap();
            std::fs::write(
                "skills/my-skill/SKILL.md",
                format!(
                    "---\nname: my-skill\ndescription: A valid skill description here\nmodel: {model}\n---\nBody\n"
                ),
            )
            .unwrap();
            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
            let fires = diag
                .errors()
                .iter()
                .any(|e| e.contains("'model'") && e.contains(model));
            assert_eq!(
                !fires,
                expect_valid,
                "S063 verdict mismatch for model={model:?}, errors={:?}",
                diag.errors()
            );
        }
    }

    // ── S064: agent-no-fork ──────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s064_agent_without_fork() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\nagent: Explore\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("'agent'") && e.contains("context: fork")),
            "S064 should fire, got: {:?}",
            diag.errors()
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s064_agent_with_fork_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\ncontext: fork\nagent: Explore\n---\nResearch the codebase thoroughly and report findings.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("without 'context: fork'")),
            "S064 should not fire when context: fork is set"
        );
    }

    // ── S065: agent-unknown ──────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s065_missing_custom_agent() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\ncontext: fork\nagent: my-reviewer\n---\nResearch the topic thoroughly and summarize.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("my-reviewer") && e.contains("not found")),
            "S065 should fire for missing custom agent, got: {:?}",
            diag.errors()
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s065_custom_agent_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::write("agents/my-reviewer.md", "---\nname: my-reviewer\ndescription: Reviews code carefully and thoroughly\n---\nBody\n").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\ncontext: fork\nagent: my-reviewer\n---\nResearch the topic thoroughly and summarize.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag.errors().iter().any(|e| e.contains("not found")),
            "S065 should not fire when custom agent exists"
        );
    }

    #[test]
    #[serial_test::serial]
    fn s065_resolves_nested_declared_names_stems_and_private_plugin_union() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("agents/review").unwrap();
        std::fs::create_dir_all(".claude/agents/review").unwrap();
        for (skill, agent) in [
            ("by-name", "code-reviewer"),
            ("by-stem", "reviewer-v2"),
            ("private", "helper"),
            ("excluded", "excluded-helper"),
        ] {
            std::fs::create_dir_all(format!("skills/{skill}")).unwrap();
            std::fs::write(
                format!("skills/{skill}/SKILL.md"),
                format!(
                    "---\nname: {skill}\ndescription: A valid skill description\ncontext: fork\nagent: {agent}\n---\nBody\n"
                ),
            )
            .unwrap();
        }
        std::fs::write(
            "agents/review/reviewer-v2.md",
            "---\nname: code-reviewer\ndescription: Review code thoroughly\n---\nBody\n",
        )
        .unwrap();
        std::fs::write(
            ".claude/agents/review/helper.md",
            "---\nname: helper\ndescription: Help with reviews\n---\nBody\n",
        )
        .unwrap();
        std::fs::write(
            ".claude/agents/review/excluded.md",
            "---\nname: excluded-helper\ndescription: Help without linting\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        let exclude =
            crate::config::ExcludeSet::new(&[".claude/agents/review/excluded.md".to_string()])
                .unwrap();
        validate_skill_content(&mut diag, &exclude);
        assert!(
            !diag
                .errors()
                .iter()
                .any(|error| error.contains("custom agent")),
            "nested name, stem fallback, private union, and excluded agents must resolve: {:?}",
            diag.errors()
        );
    }

    #[test]
    #[serial_test::serial]
    fn s065_uses_canonical_shapes_skips_namespaced_ids_and_keeps_basic_roots_private() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude/skills/by-comment").unwrap();
        std::fs::create_dir_all(".claude/skills/by-colon").unwrap();
        std::fs::create_dir_all(".claude/skills/by-plugin-only").unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::write(
            "agents/plugin-only.md",
            "---\nname: plugin-only\ndescription: Plugin-only agent\n---\nBody\n",
        )
        .unwrap();
        std::fs::write(
            ".claude/skills/by-comment/SKILL.md",
            "---\nname: by-comment\ndescription: A valid skill description\ncontext: fork\nagent: Explore # builtin note\n---\nBody\n",
        )
        .unwrap();
        std::fs::write(
            ".claude/skills/by-colon/SKILL.md",
            "---\nname: by-colon\ndescription: A valid skill description\ncontext: fork\nagent: plugin:review:security\n---\nBody\n",
        )
        .unwrap();
        std::fs::write(
            ".claude/skills/by-plugin-only/SKILL.md",
            "---\nname: by-plugin-only\ndescription: A valid skill description\ncontext: fork\nagent: plugin-only\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_private_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        let errors = diag.errors();
        let s065 = errors
            .iter()
            .filter(|error| error.contains("custom agent") || error.contains("'agent' must"))
            .collect::<Vec<_>>();
        assert_eq!(
            s065.len(),
            1,
            "only Basic's plugin-root reference must fail: {s065:?}"
        );
        assert!(s065[0].contains("plugin-only"));
    }

    #[test]
    #[serial_test::serial]
    fn s065_resolves_manifest_declared_direct_and_nested_agent_roots() {
        use crate::context::{LintContext, LintMode};

        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude-plugin").unwrap();
        std::fs::create_dir_all("skills/declared-name").unwrap();
        std::fs::create_dir_all("skills/declared-stem").unwrap();
        std::fs::create_dir_all("skills/declared-direct").unwrap();
        std::fs::create_dir_all("custom-agents/deep").unwrap();
        std::fs::write(
            ".claude-plugin/plugin.json",
            r#"{"name":"test-plugin","version":"1.0.0","agents":["./custom-agents","./direct.md","./custom-agents/deep"]}"#,
        )
        .unwrap();
        std::fs::write(
            "custom-agents/deep/reviewer-v2.md",
            "---\nname: declared-reviewer\ndescription: Review code thoroughly\n---\nBody\n",
        )
        .unwrap();
        std::fs::write(
            "direct.md",
            "---\nname: direct-reviewer\ndescription: Review direct requests\n---\nBody\n",
        )
        .unwrap();
        for (skill, agent) in [
            ("declared-name", "declared-reviewer"),
            ("declared-stem", "reviewer-v2"),
            ("declared-direct", "direct-reviewer"),
        ] {
            std::fs::write(
                format!("skills/{skill}/SKILL.md"),
                format!(
                    "---\nname: {skill}\ndescription: A valid skill description\ncontext: fork\nagent: {agent}\n---\nProvide an actionable review.\n"
                ),
            )
            .unwrap();
        }

        let ctx = LintContext::new(std::path::Path::new("."), LintMode::Plugin);
        let mut prompt_pass = crate::validators::prompt_content::PromptContentPass::default();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_discovered_skill_content_with_prompt_pass(
            &ctx,
            &mut diag,
            &crate::config::ExcludeSet::default(),
            &mut prompt_pass,
        );
        assert!(
            !diag
                .errors()
                .iter()
                .any(|error| error.contains("custom agent")),
            "manifest roots, including a direct file and overlap, must resolve: {:?}",
            diag.errors()
        );
    }

    #[test]
    #[serial_test::serial]
    fn s065_reports_each_non_string_shape_and_skips_invalid_yaml() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        for (name, agent) in [
            ("boolean", "true"),
            ("number", "7"),
            ("sequence", "[Explore]"),
            ("mapping", "{name: Explore}"),
            ("tagged", "!custom Explore"),
        ] {
            std::fs::create_dir_all(format!("skills/{name}")).unwrap();
            std::fs::write(
                format!("skills/{name}/SKILL.md"),
                format!(
                    "---\nname: {name}\ndescription: A valid skill description\ncontext: fork\nagent: {agent}\n---\nProvide an actionable review.\n"
                ),
            )
            .unwrap();
        }
        std::fs::create_dir_all("skills/invalid-yaml").unwrap();
        std::fs::write(
            "skills/invalid-yaml/SKILL.md",
            "---\nname: invalid-yaml\n\tagent: missing-reviewer\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        let errors = diag.errors();
        let invalid_shapes = errors
            .iter()
            .filter(|error| error.contains("'agent' must be a non-empty string"))
            .collect::<Vec<_>>();
        assert_eq!(
            invalid_shapes.len(),
            5,
            "each non-string shape needs one S065: {errors:?}"
        );
        assert!(
            !errors
                .iter()
                .any(|error| error.contains("invalid-yaml/SKILL.md") && error.contains("agent")),
            "invalid YAML is owned by X001 and must not cascade into S065: {errors:?}"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s065_fork_without_agent_ok() {
        // CC-SK-003 dropped: agent defaults to general-purpose per Claude Code docs
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\ncontext: fork\n---\nResearch the topic thoroughly and summarize.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag.errors().iter().any(|e| e.contains("'agent'")),
            "fork without agent must not error (defaults to general-purpose)"
        );
    }

    // ── S066: side-effect-auto ───────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s066_deploy_without_dmi() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/deploy").unwrap();
        std::fs::write(
            "skills/deploy/SKILL.md",
            "---\nname: deploy\ndescription: Use when deploying the application to production\n---\nDeploy the app\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("side-effect-named") || e.contains("disable-model-invocation")),
            "S066 should fire, got: {:?}",
            diag.errors()
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s066_deploy_with_dmi_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/deploy").unwrap();
        std::fs::write(
            "skills/deploy/SKILL.md",
            "---\nname: deploy\ndescription: Use when deploying the application to production\ndisable-model-invocation: true\n---\nDeploy the app\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("side-effect-named")),
            "S066 should not fire when DMI is true"
        );
    }

    // ── S067: bash-unscoped ──────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s067_unscoped_bash_fires_in_all_three_spellings() {
        for tool_lines in [
            "allowed-tools: Bash, Read",
            "allowed-tools: [Bash]",
            "allowed-tools:\n  - Bash",
        ] {
            let findings = tool_field_diagnostics(tool_lines);
            assert!(
                findings.iter().any(|e| e.contains("unscoped Bash")),
                "S067 should fire for {tool_lines}, got: {findings:?}"
            );
        }
    }

    /// #342: the diagnostic recommends the current permission-rule spelling
    /// `Bash(git *)`, never the stale `Bash(git:*)` colon form. Full-text
    /// assertion so the recommendation cannot drift back.
    #[test]
    #[serial_test::serial]
    fn test_s067_message_recommends_current_scoped_form() {
        let findings = tool_field_diagnostics("allowed-tools: Bash");
        assert!(
            findings.iter().any(|e| e
                == "skills/my-skill/SKILL.md: allowed-tools lists unscoped Bash; prefer scoped form like Bash(git *)"),
            "S067 must recommend exactly Bash(git *), got: {findings:?}"
        );
        assert!(
            !findings.iter().any(|e| e.contains("Bash(git:*)")),
            "S067 must not recommend the colon spelling: {findings:?}"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s067_scoped_bash_ok() {
        for tool_lines in [
            "allowed-tools: Bash(git *), Read",
            "allowed-tools: Bash(git add:*), Read",
        ] {
            let findings = tool_field_diagnostics(tool_lines);
            assert!(
                !findings.iter().any(|e| e.contains("unscoped Bash")),
                "scoped Bash must not fire S067 for {tool_lines}: {findings:?}"
            );
        }
    }

    /// Denying all of Bash is not a scoping problem: `disallowed-tools: Bash`
    /// must not fire S067.
    #[test]
    #[serial_test::serial]
    fn test_s067_disallowed_tools_bash_does_not_fire() {
        let findings = tool_field_diagnostics("disallowed-tools: Bash");
        assert!(
            findings.is_empty(),
            "disallowed-tools: Bash must stay clean, got: {findings:?}"
        );
    }

    // ── S068: injection-overflow ─────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s068_too_many_injections() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n!`a`\n!`b`\n!`c`\n!`d`\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        let diagnostic = diag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.rule == LintRule::InjectionOverflow)
            .expect("S068 should fire");
        assert_eq!(diagnostic.location.unwrap().start().line_number(), 8);
        assert_eq!(
            diagnostic.message,
            "skills/my-skill/SKILL.md: body has 4 dynamic injections (!`…`); prefer at most 3"
        );
        assert!(diagnostic.evidence.is_none());
        assert!(diagnostic.suggestion.is_none());
    }

    #[test]
    #[serial_test::serial]
    fn test_s068_three_injections_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n!`a`\n!`b`\n!`c`\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("dynamic injections"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s068_ignores_bang_prefixed_fence_info_strings() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n```!note\nfirst\n```\n```!custom\nsecond\n```\n```!note\nthird\n```\n```!custom\nfourth\n```\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.rule == LintRule::InjectionOverflow)
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s068_mixed_tokens_and_fence_info_strings_owns_fourth_token_line() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n!`one` !`two`\n```!note\nexample\n```\n!`three`\n```!custom\nexample\n```\n!`four`\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        let diagnostic = diag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.rule == LintRule::InjectionOverflow)
            .expect("the fourth inline token should produce S068");
        assert_eq!(diagnostic.location.unwrap().start().line_number(), 13);
    }

    #[test]
    #[serial_test::serial]
    fn test_s068_counts_inline_tokens_inside_fenced_examples() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n```text\n!`one`\n!`two`\n!`three`\n!`four`\n```\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.rule == LintRule::InjectionOverflow)
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s068_counts_boundary_qualified_tokens_inside_inline_code() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n`` !`one` ``\n`` !`two` ``\n`` !`three` ``\n`` !`four` ``\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.rule == LintRule::InjectionOverflow)
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s068_rejects_empty_escaped_and_non_token_forms() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n!``\n\\!`escaped`\n`ordinary backticks`\n!note\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.rule == LintRule::InjectionOverflow)
        );
    }

    // ── S069: hint-no-args ───────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s069_hint_without_arguments() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\nargument-hint: <file>\n---\nBody with no args reference\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("argument-hint") && e.contains("$ARGUMENTS")),
            "S069 should fire, got: {:?}",
            diag.errors()
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s069_hint_with_arguments_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\nargument-hint: <file>\n---\nProcess $ARGUMENTS carefully\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("never references $ARGUMENTS"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s069_positional_args_with_hint_ok() {
        // Evidence-1 fixture from issue #355: a skill that references its
        // arguments through the positional form ($1/$2) with an argument-hint
        // set must not fire S069.
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\nargument-hint: <pr-number> [priority]\n---\nReview PR #$1 with priority $2. Fetch it with gh.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("never references $ARGUMENTS")),
            "positional $1/$2 should suppress S069, got: {:?}",
            diag.errors()
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s069_braced_positional_with_hint_ok() {
        // `${2}` (braced positional) in prose counts as an argument reference.
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\nargument-hint: <first> <second>\n---\nApply ${2} to the target after checking things.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("never references $ARGUMENTS")),
            "braced positional should suppress S069, got: {:?}",
            diag.errors()
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s069_fenced_positional_still_fires() {
        // A positional ref that appears only inside a code fence (S060's
        // awk/shell territory) must NOT suppress S069.
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\nargument-hint: <file>\n---\nSome prose here.\n\n```bash\nawk '{print $1}'\n```\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("never references $ARGUMENTS")),
            "fence-only positional ref should not suppress S069, got: {:?}",
            diag.errors()
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s069_multidigit_and_identifier_still_fire() {
        // `$10` (argument-10-shaped) and `$1x` (identifier-shaped) are not
        // positional references, so S069 still fires when they are the only
        // `$1`-looking tokens present.
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\nargument-hint: <file>\n---\nSpend $10 then set $1x somewhere.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("never references $ARGUMENTS")),
            "$10/$1x must not suppress S069, got: {:?}",
            diag.errors()
        );
    }

    // ── S070: unknown-fm-field ───────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s070_unknown_field() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\nmodell: sonnet\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("unknown skill frontmatter field") && e.contains("modell")),
            "S070 should fire, got: {:?}",
            diag.errors()
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s070_known_fields_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\nwhen_to_use: Use for model override checks\nmodel: inherit\neffort: high\ndisallowed-tools: AskUserQuestion\nlicense: MIT\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("unknown skill frontmatter"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s070_yaml_comment_not_unknown() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n# note: not a field\nmodel: inherit\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("unknown skill frontmatter")),
            "YAML comments must not trigger S070, got: {:?}",
            diag.errors()
        );
    }

    // ── S071: paths-empty ────────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s071_paths_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\npaths:\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("'paths'") && e.contains("empty")),
            "S071 should fire, got: {:?}",
            diag.errors()
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s071_paths_scalar_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\npaths: \"**/*.ts\"\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("'paths'") && e.contains("empty"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s071_paths_yaml_list_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\npaths:\n  - \"**/*.ts\"\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("'paths'") && e.contains("empty"))
        );
    }

    // ════════════════════════════════════════════════════════════════
    // Canonical-YAML field-type migration (issue #341)
    //
    // These exercise the input-handling change: trailing comments, YAML 1.2
    // boolean casing, quoting, and value shapes now read through canonical YAML.
    // ════════════════════════════════════════════════════════════════

    /// Lint one public skill (directory `dir`) written with `content`. The
    /// caller owns the `CwdGuard`, so `serial_test` ordering stays explicit.
    fn lint_skill_in(dir: &str, content: &str) -> Vec<crate::diagnostic::Diagnostic> {
        std::fs::create_dir_all(format!("skills/{dir}")).unwrap();
        std::fs::write(format!("skills/{dir}/SKILL.md"), content).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        diag.diagnostics().to_vec()
    }

    fn fires(diags: &[crate::diagnostic::Diagnostic], rule: LintRule) -> bool {
        diags.iter().any(|d| d.rule == rule)
    }

    fn message_for(diags: &[crate::diagnostic::Diagnostic], rule: LintRule) -> String {
        diags
            .iter()
            .find(|d| d.rule == rule)
            .map(|d| d.message.clone())
            .unwrap_or_default()
    }

    fn skill_with_field(field_line: &str) -> String {
        format!(
            "---\nname: subject\ndescription: A valid skill description here\n{field_line}\n---\nBody content here\n"
        )
    }

    // ── S023: canonical booleans, casing, comments, shapes ───────────

    #[test]
    #[serial_test::serial]
    fn s023_canonical_bool_table() {
        let cases: &[(&str, bool)] = &[
            ("user-invocable: true", false),
            ("user-invocable: false", false),
            ("user-invocable: True", false), // YAML 1.2 casing
            ("user-invocable: TRUE", false), // YAML 1.2 casing
            ("user-invocable: true # allow slash use", false), // trailing comment
            ("user-invocable: \"true\"", false), // quoted compat form
            ("user-invocable: yes", true),
            ("user-invocable: 1", true),
            ("user-invocable: [true]", true),
        ];
        for (field_line, should_fire) in cases {
            let tmp = tempfile::tempdir().unwrap();
            let _guard = crate::test_helpers::CwdGuard::new();
            std::env::set_current_dir(tmp.path()).unwrap();
            let diags = lint_skill_in("subject", &skill_with_field(field_line));
            assert_eq!(
                fires(&diags, LintRule::BoolFieldInvalid),
                *should_fire,
                "S023 verdict mismatch for {field_line:?}"
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn s023_renders_canonical_value_not_raw_text() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let diags = lint_skill_in("subject", &skill_with_field("user-invocable: yes # nope"));
        assert!(
            message_for(&diags, LintRule::BoolFieldInvalid).contains("got 'yes'"),
            "comment must not leak into the rendered value: {}",
            message_for(&diags, LintRule::BoolFieldInvalid)
        );
    }

    // ── S024/S025/S026/S063: canonical scalar vocabularies ───────────

    #[test]
    #[serial_test::serial]
    fn enum_fields_read_canonical_scalars() {
        // (field_line, rule, should_fire)
        let cases: &[(&str, LintRule, bool)] = &[
            (
                "context: fork # run forked",
                LintRule::ContextFieldInvalid,
                false,
            ),
            ("context: forked", LintRule::ContextFieldInvalid, true),
            ("context: [fork]", LintRule::ContextFieldInvalid, true),
            (
                "effort: high # default",
                LintRule::EffortFieldInvalid,
                false,
            ),
            ("effort: extreme", LintRule::EffortFieldInvalid, true),
            ("effort: 5", LintRule::EffortFieldInvalid, true),
            ("shell: bash # posix", LintRule::ShellFieldInvalid, false),
            ("shell: zsh", LintRule::ShellFieldInvalid, true),
            (
                "model: sonnet # fast default",
                LintRule::ModelInvalid,
                false,
            ),
            ("model: sonet", LintRule::ModelInvalid, true),
            ("model: [sonnet]", LintRule::ModelInvalid, true),
        ];
        for (field_line, rule, should_fire) in cases {
            let tmp = tempfile::tempdir().unwrap();
            let _guard = crate::test_helpers::CwdGuard::new();
            std::env::set_current_dir(tmp.path()).unwrap();
            let diags = lint_skill_in("subject", &skill_with_field(field_line));
            assert_eq!(
                fires(&diags, *rule),
                *should_fire,
                "{} verdict mismatch for {field_line:?}",
                rule.code()
            );
        }
    }

    // ── S027/S066 gates read commented canonical booleans ────────────

    #[test]
    #[serial_test::serial]
    fn s027_fires_on_commented_boolean_gates() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let diags = lint_skill_in(
            "subject",
            "---\nname: subject\ndescription: A valid skill description here\ndisable-model-invocation: true # manual only\nuser-invocable: false\n---\nBody content here\n",
        );
        assert!(fires(&diags, LintRule::SkillUnreachable));
    }

    #[test]
    #[serial_test::serial]
    fn s066_does_not_fire_when_dmi_true_is_commented() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        // Evidence 2: side-effect-named skill with a commented dmi:true.
        let diags = lint_skill_in(
            "deploy-site",
            "---\nname: deploy-site\ndescription: Use when deploying the site to production\ndisable-model-invocation: true # keep manual\n---\nDeploy the site\n",
        );
        assert!(
            !fires(&diags, LintRule::SideEffectAuto),
            "S066 must invert on a commented dmi:true"
        );
    }

    // ── S064: only a usable string agent gates on context ────────────

    #[test]
    #[serial_test::serial]
    fn s064_agent_shape_table() {
        // (agent line + optional context, S064 fires?, S065 fires?)
        // Refinement B: an unusable agent shape emits S065 only, never S064.
        let cases: &[(&str, bool, bool)] = &[
            ("agent: Explore", true, false), // string builtin, no fork → S064 only
            ("context: fork # note\nagent: Explore", false, false), // commented fork → clean
            ("agent:", false, true),         // null → S065 owns
            ("agent: null", false, true),    // null → S065 owns
            ("agent: [Explore]", false, true), // sequence → S065 owns
            ("agent: \"\"", false, true),    // empty string → S065 owns
        ];
        for (field_lines, s064, s065) in cases {
            let tmp = tempfile::tempdir().unwrap();
            let _guard = crate::test_helpers::CwdGuard::new();
            std::env::set_current_dir(tmp.path()).unwrap();
            let diags = lint_skill_in(
                "subject",
                &format!(
                    "---\nname: subject\ndescription: A valid skill description here\n{field_lines}\n---\nResearch and report.\n"
                ),
            );
            assert_eq!(
                fires(&diags, LintRule::AgentNoFork),
                *s064,
                "S064 verdict mismatch for {field_lines:?}"
            );
            assert_eq!(
                fires(&diags, LintRule::AgentUnknown),
                *s065,
                "S065 verdict mismatch for {field_lines:?}"
            );
        }
    }

    // ── S070: canonical keys, not raw lines ──────────────────────────

    #[test]
    #[serial_test::serial]
    fn s070_reads_canonical_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        // Quoted and spaced key spellings resolve to their real key → no S070.
        let clean = lint_skill_in(
            "known",
            "---\nname : known\n\"description\": A valid skill description here\n\"model\": inherit\n---\nBody content here\n",
        );
        assert!(
            !fires(&clean, LintRule::UnknownFmField),
            "quoted/spaced known keys must not fire S070: {:?}",
            clean
                .iter()
                .filter(|d| d.rule == LintRule::UnknownFmField)
                .map(|d| d.message.as_str())
                .collect::<Vec<_>>()
        );

        // A genuinely unknown key still fires.
        let unknown = lint_skill_in("subject", &skill_with_field("modell: sonnet"));
        assert!(fires(&unknown, LintRule::UnknownFmField));
    }

    // ── S071: canonical shape of `paths` ─────────────────────────────

    #[test]
    #[serial_test::serial]
    fn s071_paths_shape_table() {
        let cases: &[(&str, bool)] = &[
            ("paths: []", true),            // empty flow sequence (Evidence 4)
            ("paths: \"\"", true),          // empty string
            ("paths: null", true),          // explicit null
            ("paths: {}", true),            // mapping shape
            ("paths: [\"\", \"\"]", true),  // sequence of empty strings
            ("paths: [\"src/**\"]", false), // non-empty flow sequence
            ("paths: \"**/*.ts\"", false),  // non-empty string
        ];
        for (field_line, should_fire) in cases {
            let tmp = tempfile::tempdir().unwrap();
            let _guard = crate::test_helpers::CwdGuard::new();
            std::env::set_current_dir(tmp.path()).unwrap();
            let diags = lint_skill_in("subject", &skill_with_field(field_line));
            assert_eq!(
                fires(&diags, LintRule::PathsEmpty),
                *should_fire,
                "S071 verdict mismatch for {field_line:?}"
            );
        }
    }

    // ── S039: canonical metadata shapes ──────────────────────────────

    #[test]
    #[serial_test::serial]
    fn s039_metadata_value_table() {
        // (metadata block, S039 should fire?)
        let cases: &[(&str, bool)] = &[
            ("metadata:\n  count: 1 # note", true), // commented numeric entry
            ("metadata:\n  tags:\n    - a", true),  // sequence entry
            ("metadata:\n  slot: null", true),      // null entry
            ("metadata: production", true),         // non-mapping scalar
            ("metadata:\n  nested:\n    a: b", true), // nested-mapping entry
            ("metadata:\n  version: \"1.0\"", false), // quoted string entry
            ("metadata:\n  channel: stable", false), // plain string entry
            ("metadata:", false),                   // null metadata — silent
        ];
        for (block, should_fire) in cases {
            let tmp = tempfile::tempdir().unwrap();
            let _guard = crate::test_helpers::CwdGuard::new();
            std::env::set_current_dir(tmp.path()).unwrap();
            let diags = lint_skill_in(
                "subject",
                &format!(
                    "---\nname: subject\ndescription: A valid skill description here\n{block}\n---\nBody content here\n"
                ),
            );
            assert_eq!(
                fires(&diags, LintRule::MetadataNotString),
                *should_fire,
                "S039 verdict mismatch for {block:?}"
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn s039_non_mapping_metadata_uses_shape_message() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let diags = lint_skill_in("subject", &skill_with_field("metadata: production"));
        assert!(
            message_for(&diags, LintRule::MetadataNotString)
                .contains("metadata must be a map of string values")
        );
    }

    // ── S035: character count, comments, multiline scalars ───────────

    #[test]
    #[serial_test::serial]
    fn s035_measures_characters_not_bytes() {
        // 180 CJK characters = 540 UTF-8 bytes: clean by character count.
        let cjk = "世".repeat(180);
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let diags = lint_skill_in(
            "subject",
            &skill_with_field(&format!("compatibility: {cjk}")),
        );
        assert!(
            !fires(&diags, LintRule::CompatTooLong),
            "180 CJK chars must be clean"
        );
    }

    #[test]
    #[serial_test::serial]
    fn s035_strips_trailing_comment_before_counting() {
        let value = "x".repeat(490);
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let diags = lint_skill_in(
            "subject",
            &skill_with_field(&format!("compatibility: {value} # requires macOS")),
        );
        assert!(
            !fires(&diags, LintRule::CompatTooLong),
            "490 chars plus a comment must be clean"
        );
    }

    #[test]
    #[serial_test::serial]
    fn s035_counts_multiline_block_scalar() {
        let value = "a".repeat(501);
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let diags = lint_skill_in(
            "subject",
            &format!(
                "---\nname: subject\ndescription: A valid skill description here\ncompatibility: |-\n  {value}\n---\nBody content here\n"
            ),
        );
        assert!(fires(&diags, LintRule::CompatTooLong));
        assert!(
            message_for(&diags, LintRule::CompatTooLong).contains("(501)"),
            "count must be characters: {}",
            message_for(&diags, LintRule::CompatTooLong)
        );
    }

    // ── S028/S069: canonical argument-hint presence ──────────────────

    #[test]
    #[serial_test::serial]
    fn s028_quoted_and_commented_hint_counts_as_set() {
        for hint in [
            "\"argument-hint\": \"<feature>\"", // quoted key (Evidence 4)
            "argument-hint: <feature> # note",  // trailing comment
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let _guard = crate::test_helpers::CwdGuard::new();
            std::env::set_current_dir(tmp.path()).unwrap();
            let diags = lint_skill_in(
                "subject",
                &format!(
                    "---\nname: subject\ndescription: A valid skill description here\n{hint}\n---\nUse $ARGUMENTS as input.\n"
                ),
            );
            assert!(
                !fires(&diags, LintRule::ArgsNoHint),
                "S028 must not fire when the hint is set via {hint:?}"
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn s069_gate_engages_for_quoted_key_hint() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let diags = lint_skill_in(
            "subject",
            "---\nname: subject\ndescription: A valid skill description here\n\"argument-hint\": \"<x>\"\n---\nBody never mentions the token.\n",
        );
        assert!(fires(&diags, LintRule::HintNoArgs));
    }

    #[test]
    #[serial_test::serial]
    fn s028_null_hint_counts_as_unset() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let diags = lint_skill_in(
            "subject",
            "---\nname: subject\ndescription: A valid skill description here\nargument-hint:\nmodel: inherit\n---\nUse $ARGUMENTS as input.\n",
        );
        assert!(
            fires(&diags, LintRule::ArgsNoHint),
            "a null argument-hint counts as unset"
        );
    }

    // ── S043: field scope + prose exemption ──────────────────────────

    #[test]
    #[serial_test::serial]
    fn s043_scope_table() {
        // (frontmatter body between ---, S043 should fire?)
        let cases: &[(&str, bool)] = &[
            ("paths: C:\\Users\\me\\file", true),            // path field
            ("argument-hint: C:\\Users\\file", true),        // path field
            ("paths:\n  - C:\\Users\\a", true),              // sequence item
            ("description: See C:\\Users\\me\\file", false), // prose exempt
            ("compatibility: needs C:\\Windows\\x", false),  // prose exempt
            ("when_to_use: from C:\\Users\\me", false),      // prose exempt
            ("metadata:\n  path: C:\\Users\\x", false),      // metadata exempt
        ];
        for (block, should_fire) in cases {
            let tmp = tempfile::tempdir().unwrap();
            let _guard = crate::test_helpers::CwdGuard::new();
            std::env::set_current_dir(tmp.path()).unwrap();
            let diags = lint_skill_in(
                "subject",
                &format!(
                    "---\nname: subject\ndescription: A valid skill description here\n{block}\n---\nBody content here\n"
                ),
            );
            assert_eq!(
                fires(&diags, LintRule::FrontmatterBackslash),
                *should_fire,
                "S043 verdict mismatch for {block:?}"
            );
        }
    }

    // ── Invalid YAML skips every migrated rule (X001 owns) ───────────

    #[test]
    #[serial_test::serial]
    fn invalid_yaml_skips_field_type_rules() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        // A tab-indented line is invalid YAML; model: sonet must not reach S063.
        let diags = lint_skill_in(
            "subject",
            "---\nname: subject\ndescription: A valid skill description here\n\tmodel: sonet\n---\nBody content here\n",
        );
        for rule in [
            LintRule::BoolFieldInvalid,
            LintRule::ContextFieldInvalid,
            LintRule::EffortFieldInvalid,
            LintRule::ShellFieldInvalid,
            LintRule::SkillUnreachable,
            LintRule::ModelInvalid,
            LintRule::AgentNoFork,
            LintRule::SideEffectAuto,
            LintRule::UnknownFmField,
            LintRule::PathsEmpty,
            LintRule::MetadataNotString,
            LintRule::CompatTooLong,
            LintRule::FrontmatterBackslash,
            LintRule::ArgsNoHint,
            LintRule::HintNoArgs,
        ] {
            assert!(
                !fires(&diags, rule),
                "{} must skip invalid YAML (X001 owns)",
                rule.code()
            );
        }
    }

    // ── Evidence 1: the fully-valid commented file stays clean ───────

    #[test]
    #[serial_test::serial]
    fn evidence_commented_fields_produce_no_field_type_findings() {
        // Acceptance: the trailing-comment file and its comment-free equivalent
        // lint identically (both produce no field-type findings).
        let field_type_rules = [
            LintRule::BoolFieldInvalid,
            LintRule::ContextFieldInvalid,
            LintRule::EffortFieldInvalid,
            LintRule::ShellFieldInvalid,
            LintRule::ModelInvalid,
            LintRule::AgentNoFork,
        ];
        for (label, body) in [
            (
                "commented",
                "model: sonnet # fast default\ncontext: fork # run forked\nagent: Explore\nuser-invocable: true # allow slash use",
            ),
            (
                "uncommented",
                "model: sonnet\ncontext: fork\nagent: Explore\nuser-invocable: true",
            ),
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let _guard = crate::test_helpers::CwdGuard::new();
            std::env::set_current_dir(tmp.path()).unwrap();
            let diags = lint_skill_in(
                "comments",
                &format!(
                    "---\nname: comments\ndescription: Use when exercising commented frontmatter fields\n{body}\n---\nResearch the codebase and report findings.\n"
                ),
            );
            for rule in field_type_rules {
                assert!(
                    !fires(&diags, rule),
                    "{} false-positives on the {label} fixture: {:?}",
                    rule.code(),
                    diags.iter().map(|d| d.rule.code()).collect::<Vec<_>>()
                );
            }
        }
    }
}
