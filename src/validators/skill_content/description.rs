use crate::diagnostic::DiagnosticCollector;
use crate::frontmatter;
use crate::rules::LintRule;
use crate::validators::skills::SkillInfo;
use regex::Regex;
use std::sync::LazyLock;

use super::name::RE_XML_TAG;

const MAX_DESC_CHARS: usize = 1024;
const MIN_DESC_CHARS: usize = 20;
const DESC_TRUNCATE_LEN: usize = 250;

// S016/S017: Description quality (plugin-only)
static RE_PERSON: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(I|you|we|my|your|our)\b").unwrap());
static RE_TRIGGER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(use\s+when|use\s+this|use\s+for|trigger\s+when|do\s+not\s+trigger|\bwhen\b)")
        .unwrap()
});

pub(super) fn check_description_quality(
    info: &SkillInfo,
    plugin_mode: bool,
    diag: &mut DiagnosticCollector,
) {
    let desc = match frontmatter::get_field(&info.fm_lines, "description") {
        Some(d) => d,
        None => return, // S005 fires from existing validator
    };

    let char_count = desc.chars().count();

    // S014: description too long
    if char_count > MAX_DESC_CHARS {
        diag.report(
            LintRule::DescTooLong,
            &format!(
                "{}: description exceeds 1024 characters ({})",
                info.path, char_count
            ),
        );
    }

    // S034: description too short
    if char_count < MIN_DESC_CHARS {
        diag.report(
            LintRule::DescTooShort,
            &format!(
                "{}: description is under 20 characters ({})",
                info.path, char_count
            ),
        );
    }

    // S015: description truncated in listing (plugin-only)
    if plugin_mode && char_count > DESC_TRUNCATE_LEN {
        diag.report(
            LintRule::DescTruncated,
            &format!(
                "{}: description exceeds 250 characters ({}) and will be truncated in skill listing",
                info.path, char_count
            ),
        );
    }

    // S016: uses first/second person (plugin-only)
    if plugin_mode && RE_PERSON.is_match(&desc) {
        diag.report(
            LintRule::DescUsesPerson,
            &format!(
                "{}: description uses first/second person; use third person for published skills",
                info.path
            ),
        );
    }

    // S017: no trigger context (plugin-only)
    if plugin_mode && !RE_TRIGGER.is_match(&desc) {
        diag.report(
            LintRule::DescNoTrigger,
            &format!(
                "{}: description lacks trigger/usage context (e.g., 'Use when...', 'Trigger when...')",
                info.path
            ),
        );
    }

    // S018: XML tags in description
    if RE_XML_TAG.is_match(&desc) {
        diag.report(
            LintRule::DescHasXml,
            &format!("{}: description contains XML/HTML tags", info.path),
        );
    }
}
