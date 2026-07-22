use crate::diagnostic::DiagnosticCollector;
use crate::frontmatter;
use crate::rules::LintRule;
use crate::validators::skills::SkillInfo;
use regex::Regex;
use std::sync::LazyLock;

const MAX_DESC_CHARS: usize = 1024;
const MIN_DESC_CHARS: usize = 20;

// S018: tag-shaped spans in descriptions (`</?[A-Za-z][^<>]*>`).
// Autolink exclusions (://, mailto:, bare email) are applied after matching.
static RE_XML_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"</?[A-Za-z][^<>]*>").unwrap());

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

// S016: first/second-person pronoun tokens (plugin-only)
#[rustfmt::skip]
const PERSON_PRONOUNS: &[&str] = &["you", "we", "my", "your", "our"];
#[rustfmt::skip]
const PERSON_CONTRACTIONS: &[&str] = &[
    "i'm", "i'll", "i've", "i'd",
    "you're", "you'll", "you've", "you'd",
    "we're", "we'll", "we've", "we'd",
];

// S017: Description quality (plugin-only)
pub(super) static RE_TRIGGER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(use\s+when|use\s+this|use\s+for|trigger\s+when|do\s+not\s+trigger|\bwhen\b)")
        .unwrap()
});

/// True when `inner` (content between `<` and `>`) is a Markdown autolink, not a tag.
fn is_markdown_autolink_inner(inner: &str) -> bool {
    if inner.contains("://") {
        return true;
    }
    let trimmed = inner.trim();
    if trimmed.is_empty() || trimmed.chars().any(char::is_whitespace) {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if let Some(address) = lower.strip_prefix("mailto:") {
        return !address.is_empty() && !address.chars().any(char::is_whitespace);
    }
    // Bare email: exactly one `@`, nonempty local and domain, no whitespace.
    let Some((local, domain)) = trimmed.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.is_empty()
        && !domain.contains('@')
        && !local.chars().any(char::is_whitespace)
        && !domain.chars().any(char::is_whitespace)
}

/// True when a matched angle-span is an S018 XML/HTML tag (not an autolink).
fn is_xml_tag_match(matched: &str) -> bool {
    let Some(inner) = matched
        .strip_prefix('<')
        .and_then(|rest| rest.strip_suffix('>'))
    else {
        return false;
    };
    !is_markdown_autolink_inner(inner)
}

/// Whether a description contains S018-flaggable XML/HTML tags.
pub(crate) fn description_contains_xml_tags(desc: &str) -> bool {
    RE_XML_TAG
        .find_iter(desc)
        .any(|m| is_xml_tag_match(m.as_str()))
}

/// Strip S018-flaggable XML/HTML tags from a description (shared with autofix).
pub(crate) fn strip_description_xml_tags(desc: &str) -> String {
    RE_XML_TAG
        .replace_all(desc, |caps: &regex::Captures| {
            let matched = caps.get(0).map(|m| m.as_str()).unwrap_or("");
            if is_xml_tag_match(matched) {
                String::new()
            } else {
                matched.to_string()
            }
        })
        .into_owned()
}

/// Trim leading/trailing punctuation. Interior `/`, `.`, and apostrophes stay
/// because tokenization only splits on whitespace.
fn trim_token_punctuation(token: &str) -> &str {
    let mut start = 0;
    let mut end = token.len();
    while start < end {
        let ch = token[start..].chars().next().unwrap();
        if ch.is_alphanumeric() {
            break;
        }
        start += ch.len_utf8();
    }
    while end > start {
        let ch = token[..end].chars().next_back().unwrap();
        if ch.is_alphanumeric() {
            break;
        }
        end -= ch.len_utf8();
    }
    &token[start..end]
}

/// Normalize curly apostrophes so contraction matching is ASCII-case-insensitive.
fn normalize_apostrophes(token: &str) -> String {
    token.replace('\u{2019}', "'")
}

/// S016: token-based first/second-person detection.
fn description_uses_person(desc: &str) -> bool {
    for raw in desc.split_whitespace() {
        let token = trim_token_punctuation(raw);
        if token.is_empty() {
            continue;
        }
        // Case-sensitive bare `I`.
        if token == "I" {
            return true;
        }
        let lower = normalize_apostrophes(token).to_ascii_lowercase();
        if PERSON_PRONOUNS.contains(&lower.as_str())
            || PERSON_CONTRACTIONS.contains(&lower.as_str())
        {
            return true;
        }
    }
    false
}

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
    // Description-quality rules require the canonical YAML scalar so valid
    // multiline forms are evaluated as one description. Invalid YAML and
    // non-string values are owned by the frontmatter validators.
    let desc = match frontmatter::get_strict_string_field(&info.fm_lines, "description") {
        Some(d) => d,
        None => return,
    };

    let char_count = desc.chars().count();

    check_description_length(info, char_count, diag);

    // S015: Claude Code lists the canonical description and when_to_use together.
    let when_to_use = frontmatter::get_strict_string_field(&info.fm_lines, "when_to_use");
    let listing_len = char_count
        + when_to_use
            .as_ref()
            .map_or(0, |value| value.chars().count());
    let listing_cap = diag.config().desc_truncated_max_chars;
    if listing_len > listing_cap {
        let field_summary = if when_to_use.is_some() {
            format!("combined description and when_to_use total {listing_len} characters")
        } else {
            format!("description totals {listing_len} characters")
        };
        diag.report(
            LintRule::DescTruncated,
            &format!(
                "{}: {field_summary}, exceeding the configured listing cap of {listing_cap}; Claude Code truncates each skill-listing entry at skillListingMaxDescChars (default 1,536) — put the key use case first",
                info.path,
            ),
        );
    }

    // S016: uses first/second person (plugin-only)
    if plugin_mode && description_uses_person(&desc) {
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
    if description_contains_xml_tags(&desc) {
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

/// Run the specification-owned description length checks shared by Claude and
/// cross-client Agent Skills surfaces. Callers own frontmatter validity; only
/// a canonical non-empty string scalar reaches this helper.
pub(super) fn check_agent_skills_description_contract(
    info: &SkillInfo,
    diag: &mut DiagnosticCollector,
) {
    let Some(desc) = frontmatter::get_strict_string_field(&info.fm_lines, "description") else {
        return;
    };
    check_description_length(info, desc.chars().count(), diag);
}

fn check_description_length(info: &SkillInfo, char_count: usize, diag: &mut DiagnosticCollector) {
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

    #[test]
    fn s016_hard_negatives() {
        assert!(!description_uses_person(
            "Optimize file I/O operations for large datasets, i.e. streaming reads. Use when profiling disk throughput."
        ));
        assert!(!description_uses_person(
            "Explains acronyms, e.g. HTTP, when onboarding. Use when documenting APIs."
        ));
        assert!(!description_uses_person(
            "Tunes CI pipelines for monorepos. Use when builds are slow."
        ));
        assert!(!description_uses_person(
            "Tracks IT budgets across teams. Use when planning spend."
        ));
    }

    #[test]
    fn s016_positives() {
        assert!(description_uses_person(
            "I can help you process files. Use when ingesting uploads."
        ));
        assert!(description_uses_person(
            "Use when you need to export reports from the warehouse."
        ));
        assert!(description_uses_person(
            "I'm a helper for release notes. Use when cutting a release."
        ));
        assert!(description_uses_person(
            "Tracks your progress through migrations. Use when upgrading schemas."
        ));
        assert!(description_uses_person(
            "Improves our workflow. Use when coordinating releases."
        ));
        assert!(description_uses_person(
            "I'll summarize the diff. Use when reviewing PRs."
        ));
        assert!(description_uses_person(
            "You’re blocked without credentials. Use when rotating secrets."
        ));
        assert!(description_uses_person("Use when you need exports."));
        assert!(description_uses_person("Tracks progress for you."));
    }

    #[test]
    fn s018_hard_negatives() {
        assert!(!description_contains_xml_tags(
            "Partition datasets when row count < 10000 or file size > 50MB before uploading. Use when preparing bulk imports."
        ));
        assert!(!description_contains_xml_tags("Compare a < b carefully."));
        assert!(!description_contains_xml_tags("Threshold is <10> items."));
        assert!(!description_contains_xml_tags(
            "Fetch docs from <https://example.com>. Use when syncing references."
        ));
        assert!(!description_contains_xml_tags(
            "Sends deployment alerts to <ops@example.com>. Use when production releases complete."
        ));
        assert!(!description_contains_xml_tags(
            "Opens <mailto:ops@example.com> for escalation. Use when on-call handoff is required."
        ));
        assert!(!description_contains_xml_tags(
            "Opens <MAILTO:Ops@Example.com> for escalation. Use when on-call handoff is required."
        ));
    }

    #[test]
    fn s018_positives() {
        assert!(description_contains_xml_tags("Contains a <tag> marker."));
        assert!(description_contains_xml_tags("Contains a </div> closer."));
        assert!(description_contains_xml_tags("Contains a <br/> break."));
        assert!(description_contains_xml_tags("References a <file> path."));
        assert!(description_contains_xml_tags(
            "Namespace <svg:path> still tags."
        ));
    }

    #[test]
    fn s018_malformed_emails_are_not_autolink_exemptions() {
        // `<@…>` is not tag-shaped (no leading letter), so it stays clean.
        assert!(!description_contains_xml_tags("Bad local <@example.com>."));
        // Empty domain / empty mailto are not autolinks; tag-shaped forms still flag.
        assert!(description_contains_xml_tags("Bad domain <ops@>."));
        assert!(description_contains_xml_tags(
            "Empty mailto <mailto:> is not an autolink exemption."
        ));
    }

    #[test]
    fn s018_strip_preserves_autolinks_and_removes_tags() {
        let mixed = "Alert <ops@example.com> via <tag> and <https://example.com> plus <br/>.";
        let stripped = strip_description_xml_tags(mixed);
        assert_eq!(
            stripped,
            "Alert <ops@example.com> via  and <https://example.com> plus ."
        );
        assert_eq!(strip_description_xml_tags(&stripped), stripped);
    }

    #[test]
    fn s018_strip_is_noop_on_comparisons() {
        let prose =
            "Partition datasets when row count < 10000 or file size > 50MB before uploading.";
        assert_eq!(strip_description_xml_tags(prose), prose);
    }
}
