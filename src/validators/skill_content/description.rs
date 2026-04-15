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

// S050: vague description content (plugin-only)
#[rustfmt::skip]
const GENERIC_VERBS: &[&str] = &[
    "help", "helps", "assist", "assists", "handle", "handles", "manage", "manages",
    "process", "processes", "work", "works", "deal", "deals", "do", "does",
];
#[rustfmt::skip]
const GENERIC_NOUNS: &[&str] = &[
    "things", "stuff", "data", "files", "documents", "items", "tasks", "operations", "content",
];
#[rustfmt::skip]
pub(super) const STOPWORDS: &[&str] = &[
    "the", "a", "an", "is", "are", "to", "for", "with", "and", "of", "in", "on", "it",
    "that", "this", "by", "from", "or", "as", "at", "be", "do", "so", "if", "no", "not",
    "but", "up", "out", "all", "can", "has", "had", "was", "were", "been", "have", "will",
    "would", "should", "could", "may", "might", "when", "you", "your", "use", "need",
    "needed", "using", "used",
];

// S016/S017: Description quality (plugin-only)
static RE_PERSON: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(I|you|we|my|your|our)\b").unwrap());
pub(super) static RE_TRIGGER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(use\s+when|use\s+this|use\s+for|trigger\s+when|do\s+not\s+trigger|\bwhen\b)")
        .unwrap()
});

fn is_description_vague(desc: &str) -> bool {
    let stripped = RE_TRIGGER.replace_all(desc, " ");
    let lower = stripped.to_lowercase();
    let tokens: Vec<&str> = lower
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|w| !w.is_empty())
        .collect();

    let has_generic_verb = tokens.iter().any(|t| GENERIC_VERBS.contains(t));
    let has_generic_noun = tokens.iter().any(|t| GENERIC_NOUNS.contains(t));

    let is_filler =
        |t: &&str| GENERIC_VERBS.contains(t) || GENERIC_NOUNS.contains(t) || STOPWORDS.contains(t);

    let specific_count = tokens.iter().filter(|t| !is_filler(t)).count();

    // Heuristic 1: generic verb + generic noun with fewer than 2 specific terms
    if has_generic_verb && has_generic_noun && specific_count < 2 {
        return true;
    }

    // Heuristic 2: fewer than 3 distinct meaningful words
    use std::collections::HashSet;
    let distinct_meaningful: HashSet<&str> =
        tokens.iter().filter(|t| !is_filler(t)).copied().collect();
    if distinct_meaningful.len() < 3 {
        return true;
    }

    false
}

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

    // S050: vague description content (plugin-only)
    if plugin_mode && is_description_vague(&desc) {
        diag.report(
            LintRule::DescVagueContent,
            &format!(
                "{}: description content is too vague/generic; \
                 add specific terms describing what the skill does",
                info.path
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vague_generic_verb_and_noun() {
        assert!(is_description_vague("Helps with documents"));
        assert!(is_description_vague("Processes data and handles things"));
        assert!(is_description_vague("Manages tasks"));
    }

    #[test]
    fn vague_with_trigger_phrase() {
        assert!(is_description_vague(
            "Helps with documents. Use when working with files."
        ));
        assert!(is_description_vague("Use when you need to process data"));
    }

    #[test]
    fn vague_base_verb_forms() {
        assert!(is_description_vague("Help with files"));
        assert!(is_description_vague("Handle data"));
        assert!(is_description_vague("Process stuff"));
    }

    #[test]
    fn specific_description_not_flagged() {
        assert!(!is_description_vague(
            "Extract text and tables from PDF files, fill forms, merge documents. Use when working with PDF files."
        ));
        assert!(!is_description_vague(
            "Generate descriptive commit messages by analyzing git diffs. Use when reviewing staged changes."
        ));
        assert!(!is_description_vague(
            "Analyze Excel spreadsheets, create pivot tables, generate charts. Use when analyzing .xlsx files."
        ));
    }

    #[test]
    fn technical_terms_override_generic() {
        assert!(!is_description_vague(
            "Process Kubernetes deployment data using Helm charts"
        ));
        assert!(!is_description_vague(
            "Handles GraphQL schema validation and type generation"
        ));
    }

    #[test]
    fn short_but_specific_not_flagged() {
        assert!(!is_description_vague("Parse YAML configuration files"));
        assert!(!is_description_vague(
            "Compile TypeScript to JavaScript bundles"
        ));
    }

    #[test]
    fn low_information_density() {
        assert!(is_description_vague("Does stuff"));
        assert!(is_description_vague("Works with things"));
    }
}
