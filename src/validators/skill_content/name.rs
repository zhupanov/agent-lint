use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata, SourceSpan};
use crate::frontmatter;
use crate::rules::LintRule;
use crate::validators::common::RE_NAME_INVALID;
use crate::validators::skills::SkillInfo;

pub(super) const MAX_SKILL_NAME_LEN: usize = 64;

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

    // S033: vague name (plugin-only)
    if plugin_mode {
        let vague_names = [
            "helper",
            "helpers",
            "utils",
            "utility",
            "tools",
            "data",
            "files",
            "documents",
        ];
        if vague_names.contains(&name.as_str()) {
            diag.report(
                LintRule::NameVague,
                &format!(
                    "{}: name '{}' is too vague/generic for a published skill",
                    info.path, name
                ),
            );
        }
    }

    // S049: name not in gerund form (plugin-only)
    if plugin_mode {
        const NON_GERUND_ING: &[&str] = &[
            "string", "ring", "spring", "king", "thing", "bling", "sing", "wing", "ping", "sting",
            "swing", "bring", "cling", "fling", "sling", "wring",
        ];
        let has_gerund = name
            .split('-')
            .any(|word| word.ends_with("ing") && !NON_GERUND_ING.contains(&word));
        if !has_gerund {
            diag.report(
                LintRule::NameNotGerund,
                &format!(
                    "{}: name '{}' does not use gerund form (consider e.g. 'processing-pdfs', 'reviewing-code')",
                    info.path, name
                ),
            );
        }
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
