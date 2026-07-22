use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata, SourceSpan};
use crate::frontmatter;
use crate::rules::LintRule;
use crate::validators::common::RE_NAME_INVALID;
use crate::validators::skills::SkillInfo;

pub(super) const MAX_SKILL_NAME_LEN: usize = 64;

/// Exact skill names that are domainless implementation labels for published
/// plugin skills (S033). Broad subject nouns such as `data`, `files`, and
/// `documents` are intentionally omitted — they can be concise, accurate
/// names (comparable to accepted platform examples like `pdf` / `docx`).
/// Matching remains exact-name only; compounds such as `pdf-helper` are not
/// flagged.
pub(super) const VAGUE_SKILL_NAMES: &[&str] = &[
    "helper",  // pure role label with no domain or task
    "helpers", // plural of the same domainless role label
    "utils",   // implementation bucket, not a skill subject
    "utility", // singular form of the same implementation bucket
    "tools",   // domainless toolkit label
];

pub(super) fn check_name_format(
    info: &SkillInfo,
    plugin_mode: bool,
    diag: &mut DiagnosticCollector,
) {
    let name = match frontmatter::get_strict_string_field(&info.fm_lines, "name") {
        Some(n) => n,
        // Invalid YAML and non-string fields are owned by the frontmatter
        // validators. Do not add format findings without a trustworthy name.
        None => return,
    };

    check_agent_skills_name_contract(info, &name, diag);

    // S033: vague name (plugin-only). S049 (`name-not-gerund`) is retired and
    // never emits; the registry entry remains only for config compatibility.
    if plugin_mode && VAGUE_SKILL_NAMES.contains(&name.as_str()) {
        diag.report_with(
            LintRule::NameVague,
            &format!(
                "{}: name '{}' is domainless; add the missing domain or task (e.g. 'pdf-helper' or 'lint-utils', not 'helper')",
                info.path, name
            ),
            DiagnosticMetadata::default().with_suggestion(
                "Add the missing domain or task to the exact skill name (for example 'pdf-helper' or 'lint-utils'), rather than renaming for morphology alone.",
            ),
        );
    }
}

/// Validate the interoperable Agent Skills name contract shared by every
/// supported skill surface. The caller establishes the concrete SKILL.md
/// subject so per-file policy remains centralized in `DiagnosticCollector`.
pub(super) fn check_agent_skills_name_contract(
    info: &SkillInfo,
    name: &str,
    diag: &mut DiagnosticCollector,
) {
    let location = name_field_location(&info.fm_lines);

    // S009: name too long
    let character_count = name.chars().count();
    if character_count > MAX_SKILL_NAME_LEN {
        diag.report_with(
            LintRule::NameTooLong,
            &format!(
                "{}: name '{}' exceeds 64 characters ({})",
                info.path, name, character_count
            ),
            name_metadata(location, name, "shorten the name to at most 64 characters"),
        );
    }

    // S010: invalid characters
    if RE_NAME_INVALID.is_match(name) {
        diag.report_with(
            LintRule::NameInvalidChars,
            &format!(
                "{}: name '{}' contains characters outside [a-z0-9-]",
                info.path, name
            ),
            name_metadata(
                location,
                name,
                "use only lowercase ASCII letters, digits, and single hyphens",
            ),
        );
    }

    // S011: bad hyphens
    if name.starts_with('-') || name.ends_with('-') || name.contains("--") {
        diag.report_with(
            LintRule::NameBadHyphens,
            &format!(
                "{}: name '{}' starts/ends with hyphen or contains consecutive hyphens",
                info.path, name
            ),
            name_metadata(
                location,
                name,
                "remove leading, trailing, and consecutive hyphens",
            ),
        );
    }
}

fn name_metadata(location: Option<SourceSpan>, name: &str, suggestion: &str) -> DiagnosticMetadata {
    let metadata = DiagnosticMetadata::default()
        .with_evidence(name)
        .with_suggestion(suggestion);
    match location {
        Some(location) => metadata.with_location(location),
        None => metadata,
    }
}

/// Frontmatter lines omit the opening delimiter, so their first line is file
/// line two. YAML permits forms without a simple `name:` line; those retain
/// the canonical value but intentionally have no fabricated coordinate.
fn name_field_location(fm_lines: &[String]) -> Option<SourceSpan> {
    fm_lines
        .iter()
        .position(|line| line.starts_with("name:"))
        .map(|index| SourceSpan::line(index + 2))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::DiagnosticCollector;
    use crate::markdown::MarkdownDocument;
    use crate::validators::skills::SkillInfo;

    fn skill_with_name(name: &str) -> SkillInfo {
        let content = format!(
            "---\nname: {name}\ndescription: Use when testing skill name validation thoroughly\n---\nBody\n"
        );
        let document = MarkdownDocument::parse(content);
        let fm_lines = document
            .frontmatter()
            .expect("fixture frontmatter")
            .to_vec();
        let parsed_frontmatter = crate::frontmatter::parse_yaml_strict(&fm_lines).ok();
        SkillInfo {
            path: format!("skills/{name}/SKILL.md"),
            dir_name: name.to_string(),
            fm_lines,
            parsed_frontmatter,
            body: document.body().to_string(),
            document,
            has_scripts_dir: false,
        }
    }

    fn name_vague_diagnostics(name: &str, plugin_mode: bool) -> Vec<crate::diagnostic::Diagnostic> {
        let info = skill_with_name(name);
        let mut diag = DiagnosticCollector::new_all_enabled();
        check_name_format(&info, plugin_mode, &mut diag);
        diag.diagnostics()
            .iter()
            .filter(|d| d.rule == LintRule::NameVague)
            .cloned()
            .collect()
    }

    #[test]
    fn s033_flags_every_retained_denylist_entry() {
        for name in VAGUE_SKILL_NAMES {
            let found = name_vague_diagnostics(name, true);
            assert_eq!(
                found.len(),
                1,
                "expected exactly one S033 for retained denylist entry '{name}'"
            );
            assert!(
                found[0].message.contains("domainless"),
                "S033 message should describe domainlessness, got: {}",
                found[0].message
            );
            assert!(
                found[0]
                    .suggestion
                    .as_deref()
                    .is_some_and(|s| s.contains("domain or task")),
                "S033 suggestion should ask for domain/task, got: {:?}",
                found[0].suggestion
            );
        }
    }

    #[test]
    fn s033_hard_negatives_for_subject_nouns_and_compounds() {
        for name in [
            "data",
            "files",
            "documents",
            "pdf",
            "docx",
            "xlsx",
            "api-conventions",
            "deploy",
            "code-review",
            "pdf-helper",
            "lint-utils",
            "data-tools",
            "helper-scripts",
            "document-tools",
        ] {
            assert!(
                name_vague_diagnostics(name, true).is_empty(),
                "S033 must not flag '{name}'"
            );
        }
    }

    #[test]
    fn s033_private_mode_skips_vague_names() {
        for name in VAGUE_SKILL_NAMES {
            assert!(
                name_vague_diagnostics(name, false).is_empty(),
                "S033 is plugin-only; private mode must not flag '{name}'"
            );
        }
    }

    #[test]
    fn s049_never_emits_for_non_gerund_names() {
        for name in [
            "code-review",
            "pdf",
            "docx",
            "api-conventions",
            "deploy",
            "string-utils",
            "helper",
        ] {
            let info = skill_with_name(name);
            let mut diag = DiagnosticCollector::new_all_enabled();
            check_name_format(&info, true, &mut diag);
            assert!(
                !diag
                    .diagnostics()
                    .iter()
                    .any(|d| d.rule == LintRule::NameNotGerund),
                "retired S049 must stay inert for '{name}', including under all-enabled"
            );
        }
    }
}
