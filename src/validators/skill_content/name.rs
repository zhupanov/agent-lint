use crate::diagnostic::DiagnosticCollector;
use crate::frontmatter;
use crate::rules::LintRule;
use crate::validators::common::RE_NAME_INVALID;
use crate::validators::skills::SkillInfo;
use regex::Regex;
use std::sync::LazyLock;

pub(super) const MAX_SKILL_NAME_LEN: usize = 64;

// S013/S018: XML tags in name/description
pub(super) static RE_XML_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").unwrap());

pub(super) fn check_name_format(
    info: &SkillInfo,
    plugin_mode: bool,
    diag: &mut DiagnosticCollector,
) {
    let name = match frontmatter::get_field(&info.fm_lines, "name") {
        Some(n) => n,
        None => return, // S005 fires from existing validator
    };

    // S009: name too long
    if name.len() > MAX_SKILL_NAME_LEN {
        diag.report(
            LintRule::NameTooLong,
            &format!(
                "{}: name '{}' exceeds 64 characters ({})",
                info.path,
                name,
                name.len()
            ),
        );
    }

    // S010: invalid characters
    if RE_NAME_INVALID.is_match(&name) {
        diag.report(
            LintRule::NameInvalidChars,
            &format!(
                "{}: name '{}' contains characters outside [a-z0-9-]",
                info.path, name
            ),
        );
    }

    // S011: bad hyphens
    if name.starts_with('-') || name.ends_with('-') || name.contains("--") {
        diag.report(
            LintRule::NameBadHyphens,
            &format!(
                "{}: name '{}' starts/ends with hyphen or contains consecutive hyphens",
                info.path, name
            ),
        );
    }

    // S012: reserved words
    let lower = name.to_lowercase();
    if lower.contains("anthropic") || lower.contains("claude") {
        diag.report(
            LintRule::NameReservedWord,
            &format!(
                "{}: name '{}' contains reserved word ('anthropic' or 'claude')",
                info.path, name
            ),
        );
    }

    // S013: XML tags in name
    if RE_XML_TAG.is_match(&name) {
        diag.report(
            LintRule::NameHasXml,
            &format!("{}: name '{}' contains XML/HTML tags", info.path, name),
        );
    }

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
}
