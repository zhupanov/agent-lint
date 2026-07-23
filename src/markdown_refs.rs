//! Source-positioned Markdown reference adapter for I003/D005/L005/S062.
//!
//! Built on [`MarkdownDocument`]'s Comrak parse. Callers must not re-scan raw
//! lines with backtick or markdown-link regexes for these rules.

use crate::live_instructions::{example_scopes_for, is_example_heading};
use crate::markdown::MarkdownDocument;
use crate::repo_path::{ResolutionBase, normalize_separators};
use std::ops::Range;
use std::path::Path;

/// Kind of extracted reference node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkdownRefKind {
    InlineCode,
    Link,
}

/// One source-positioned reference with its containing live prose clause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownReference {
    pub kind: MarkdownRefKind,
    /// Original authored token or destination spelling.
    pub raw: String,
    /// CommonMark-decoded token or destination for classification and path
    /// resolution. `raw` remains the source evidence shown to users.
    pub decoded: String,
    /// Byte range of `raw` in the source document.
    pub byte_range: Range<usize>,
    /// Containing live prose clause, when the reference sits in live prose.
    pub clause: Option<String>,
    /// True when the reference is under an example heading/section, HTML
    /// comment, blockquote, or other non-live context for S062 classification.
    pub excluded_from_always_load: bool,
}

/// Extract inline-code and link references in source byte order.
pub fn markdown_references(content: &str) -> Vec<MarkdownReference> {
    let document = MarkdownDocument::parse(content);
    collect_references(&document)
}

fn collect_references(document: &MarkdownDocument) -> Vec<MarkdownReference> {
    let content = document.content();
    let example_line_set = example_line_numbers(document);
    let html_comment_ranges = html_comment_byte_ranges(content);
    let mut refs = Vec::new();

    for code in document.inline_code() {
        let excluded = !is_live_prose_line(document, code.start_line)
            || example_line_set.contains(&code.start_line)
            || ranges_overlap(&html_comment_ranges, &code.literal_byte_range);
        let clause = if excluded {
            None
        } else {
            clause_containing(content, code.literal_byte_range.start)
        };
        refs.push(MarkdownReference {
            kind: MarkdownRefKind::InlineCode,
            raw: code.raw_literal.clone(),
            decoded: code.literal.clone(),
            byte_range: code.literal_byte_range.clone(),
            clause,
            excluded_from_always_load: excluded,
        });
    }

    for link in document.links() {
        let excluded = !is_live_prose_line(document, link.line)
            || example_line_set.contains(&link.line)
            || ranges_overlap(&html_comment_ranges, &link.destination_byte_range);
        let clause = if excluded {
            None
        } else {
            clause_containing(content, link.destination_byte_range.start)
        };
        refs.push(MarkdownReference {
            kind: MarkdownRefKind::Link,
            raw: link.raw_destination.clone(),
            decoded: link.destination.clone(),
            byte_range: link.destination_byte_range.clone(),
            clause,
            excluded_from_always_load: excluded,
        });
    }

    refs.sort_by_key(|item| item.byte_range.start);
    refs
}

/// Percent-decode a destination exactly once. Invalid sequences are left intact.
pub fn percent_decode_once(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (from_hex(bytes[index + 1]), from_hex(bytes[index + 2]))
        {
            out.push((hi << 4) | lo);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| raw.to_string())
}

fn from_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Whether a Markdown link destination is external, protocol-relative, or a pure fragment.
pub fn is_external_or_fragment_destination(destination: &str) -> bool {
    let trimmed = destination.trim();
    trimmed.is_empty()
        || trimmed.starts_with('#')
        || trimmed.contains("://")
        || trimmed.starts_with("mailto:")
        || trimmed.starts_with("//")
}

/// Resolution base for an S062 / prompt-budget reference.
pub fn prompt_resolution_base(kind: MarkdownRefKind, raw: &str) -> ResolutionBase {
    match kind {
        MarkdownRefKind::Link => ResolutionBase::SourceRelative,
        MarkdownRefKind::InlineCode => {
            let normalized = normalize_separators(raw.trim_start_matches("./"));
            let first = normalized.split('/').next().unwrap_or("");
            if matches!(first, "skills" | "docs" | "agents" | "scripts" | ".claude") {
                // `.claude/skills/...` shares the `.claude` first component.
                if first == ".claude" {
                    let mut parts = normalized.split('/');
                    let _ = parts.next();
                    if parts.next() == Some("skills") {
                        return ResolutionBase::RepositoryRoot;
                    }
                    return ResolutionBase::SourceRelative;
                }
                return ResolutionBase::RepositoryRoot;
            }
            ResolutionBase::SourceRelative
        }
    }
}

/// Explicit repository-root plain-path prefixes retained for S062.
pub fn is_root_plain_md_prefix(path: &str) -> bool {
    let normalized = normalize_separators(path);
    ["skills/", ".claude/skills/", "docs/", "agents/", "scripts/"]
        .iter()
        .any(|prefix| normalized.starts_with(prefix))
}

/// Classify whether a clause mandates loading the referenced prompt source.
pub fn clause_is_mandatory_load(clause: &str) -> bool {
    let lower = clause.to_ascii_lowercase();
    // Negation and conditional cues always win, including for explicit `@`
    // directives (`@path if needed`, `never @path`).
    if contains_phrase(&lower, NEGATION_CUES) {
        return false;
    }
    let trimmed = lower.trim();
    if trimmed.starts_with('@') {
        return true;
    }
    let has_verb = contains_word(&lower, &["read", "load", "open"]);
    let has_strength = contains_word(
        &lower,
        &[
            "before",
            "first",
            "completely",
            "always",
            "entire",
            "required",
            "must",
        ],
    );
    has_verb && has_strength
}

/// Whether a live clause rejects an explicit `@` import from the always-loaded
/// closure. Explicit `@` directives are mandatory without strength cues, but
/// still honor negation and conditional prose.
pub fn clause_rejects_mandatory_at_import(clause: &str) -> bool {
    contains_phrase(&clause.to_ascii_lowercase(), NEGATION_CUES)
}

const NEGATION_CUES: &[&str] = &[
    "do not",
    "don't",
    "never",
    "must not",
    "need not",
    "cannot",
    "can't",
    "without",
    "if",
    "when",
    "unless",
    "as needed",
    "optional",
    "only when",
    "may",
];

fn contains_phrase(haystack: &str, phrases: &[&str]) -> bool {
    phrases.iter().any(|phrase| {
        if phrase.contains(' ') || phrase.contains('\'') {
            let padded = format!(" {haystack} ");
            let needle = format!(" {phrase} ");
            padded.contains(&needle)
                || haystack.starts_with(phrase)
                    && haystack
                        .as_bytes()
                        .get(phrase.len())
                        .is_none_or(|b| !b.is_ascii_alphanumeric())
        } else {
            contains_word(haystack, &[phrase])
        }
    })
}

fn contains_word(haystack: &str, words: &[&str]) -> bool {
    let mut start = 0usize;
    let bytes = haystack.as_bytes();
    while start < bytes.len() {
        while start < bytes.len() && !bytes[start].is_ascii_alphanumeric() && bytes[start] != b'\''
        {
            start += 1;
        }
        if start >= bytes.len() {
            break;
        }
        let mut end = start;
        while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'\'') {
            end += 1;
        }
        let token = &haystack[start..end];
        if words.contains(&token) {
            return true;
        }
        start = end;
    }
    false
}

fn example_line_numbers(document: &MarkdownDocument) -> std::collections::HashSet<usize> {
    let mut lines = std::collections::HashSet::new();
    for (line, is_example) in document
        .body_prose()
        .iter()
        .zip(example_scopes_for(document))
    {
        if is_example {
            lines.insert(line.line);
        }
    }
    for link in document.links() {
        if under_example_heading(document, link.line) {
            lines.insert(link.line);
        }
    }
    for code in document.inline_code() {
        if under_example_heading(document, code.start_line) {
            lines.insert(code.start_line);
        }
    }
    lines
}

fn under_example_heading(document: &MarkdownDocument, line: usize) -> bool {
    let Some(example) = document
        .headings()
        .iter()
        .rfind(|heading| heading.line <= line && is_example_heading(&heading.text))
    else {
        return false;
    };
    !document.headings().iter().any(|heading| {
        heading.line > example.line && heading.line <= line && heading.level <= example.level
    })
}

fn is_live_prose_line(document: &MarkdownDocument, line: usize) -> bool {
    document.body_prose().iter().any(|prose| prose.line == line)
}

fn html_comment_byte_ranges(content: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let bytes = content.as_bytes();
    let mut index = 0;
    while index + 3 < bytes.len() {
        if &bytes[index..index + 4] == b"<!--" {
            let start = index;
            index += 4;
            while index + 2 < bytes.len() && &bytes[index..index + 3] != b"-->" {
                index += 1;
            }
            let end = if index + 2 < bytes.len() {
                index + 3
            } else {
                content.len()
            };
            ranges.push(start..end);
            index = end;
        } else {
            index += 1;
        }
    }
    ranges
}

fn ranges_overlap(ranges: &[Range<usize>], target: &Range<usize>) -> bool {
    ranges
        .iter()
        .any(|range| range.start < target.end && target.start < range.end)
}

fn clause_containing(content: &str, byte_offset: usize) -> Option<String> {
    if byte_offset >= content.len() {
        return None;
    }
    // Walk the physical line, then split into clauses on `.` / `;` / em dash
    // outside reference node ranges (approximated by skipping inside `...` and [...](...)).
    let line_start = content[..byte_offset]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let line_end = content[byte_offset..]
        .find('\n')
        .map_or(content.len(), |index| byte_offset + index);
    let line = &content[line_start..line_end];
    let relative = byte_offset - line_start;
    let clauses = split_clauses(line);
    clauses
        .into_iter()
        .find(|(range, _)| range.start <= relative && relative < range.end)
        .map(|(_, text)| text)
}

fn split_clauses(line: &str) -> Vec<(Range<usize>, String)> {
    let mut clauses = Vec::new();
    let mut start = 0usize;
    let mut index = 0usize;
    let bytes = line.as_bytes();
    while index < bytes.len() {
        if bytes[index] == b'`' {
            let ticks = bytes[index..]
                .iter()
                .take_while(|byte| **byte == b'`')
                .count();
            index += ticks;
            while index + ticks <= bytes.len() {
                if bytes[index..index + ticks].iter().all(|byte| *byte == b'`')
                    && (index + ticks == bytes.len() || bytes[index + ticks] != b'`')
                {
                    index += ticks;
                    break;
                }
                index += 1;
            }
            // An unclosed multi-backtick run stops the byte scan up to
            // `ticks - 1` bytes before the end of the line, which can be
            // inside a multibyte scalar; snap forward so the `line[index..]`
            // slice below stays on a char boundary.
            while index < bytes.len() && !line.is_char_boundary(index) {
                index += 1;
            }
            continue;
        }
        if bytes[index] == b'[' {
            if let Some(close) = line[index..].find("](") {
                let after = index + close + 2;
                if let Some(end) = find_link_destination_end(&line[after..]) {
                    index = after + end + 1;
                    continue;
                }
            }
        }
        let rest = &line[index..];
        if rest.starts_with('—') || rest.starts_with('–') {
            push_clause(&mut clauses, line, start, index);
            index += rest.chars().next().map_or(1, |ch| ch.len_utf8());
            start = index;
            continue;
        }
        if matches!(bytes[index], b'.' | b';') {
            push_clause(&mut clauses, line, start, index + 1);
            index += 1;
            start = index;
            continue;
        }
        // Advance by the full scalar so the `line[index..]` slice above never
        // lands inside a multibyte character (issue #600).
        index += rest.chars().next().map_or(1, |ch| ch.len_utf8());
    }
    push_clause(&mut clauses, line, start, line.len());
    clauses
}

fn push_clause(clauses: &mut Vec<(Range<usize>, String)>, line: &str, start: usize, end: usize) {
    if start >= end || end > line.len() {
        return;
    }
    let text = line[start..end].trim();
    if text.is_empty() {
        return;
    }
    let leading = line[start..end].len() - line[start..end].trim_start().len();
    let trailing = line[start..end].len() - line[start..end].trim_end().len();
    let range = (start + leading)..(end - trailing);
    clauses.push((range, text.to_string()));
}

/// Byte offset of the ASCII delimiter closing the destination: the unnested
/// `)` for a bare destination, or the `>` of an angle-bracketed one. The
/// caller resumes one byte past it, so returning one past `>` here would land
/// mid-scalar when a multibyte character immediately follows.
fn find_link_destination_end(after: &str) -> Option<usize> {
    let bytes = after.as_bytes();
    if bytes.first() == Some(&b'<') {
        return after.find('>');
    }
    let mut depth = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' if index + 1 < bytes.len() => index += 2,
            b'(' => {
                depth += 1;
                index += 1;
            }
            b')' if depth == 0 => return Some(index),
            b')' => {
                depth -= 1;
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

/// Shared suggestions for path findings.
pub const SUGGEST_CREATE_OR_CORRECT: &str =
    "correct the path or create the referenced repository file";
pub const SUGGEST_REPLACE_SYMLINK: &str =
    "replace it with a non-symlinked repository-relative path";

/// Helper retained for tests that need the owning path identity.
#[allow(dead_code)]
pub fn source_path_display(path: &Path) -> String {
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_angle_title_and_percent_encoded_destinations() {
        let content = "See [a](<docs/a.md> \"title\") and [b](docs/%20b.md).\n";
        let refs = markdown_references(content);
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].raw, "docs/a.md");
        assert_eq!(refs[1].raw, "docs/%20b.md");
        assert_eq!(refs[0].decoded, "docs/a.md");
        assert_eq!(refs[1].decoded, "docs/%20b.md");
        assert_eq!(percent_decode_once(&refs[1].raw), "docs/ b.md");
    }

    #[test]
    fn extracts_multi_backtick_and_link_destinations() {
        let content = "See ``docs/a.md`` and [b](docs/b.md).\n";
        let refs = markdown_references(content);
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].raw, "docs/a.md");
        assert_eq!(refs[0].kind, MarkdownRefKind::InlineCode);
        assert_eq!(refs[1].raw, "docs/b.md");
        assert_eq!(refs[1].decoded, "docs/b.md");
        assert_eq!(refs[1].kind, MarkdownRefKind::Link);
    }

    #[test]
    fn link_references_preserve_authored_and_decoded_destinations() {
        let refs = markdown_references("[nested](<docs/a\\(b\\).md> \"title\")\n");
        assert_eq!(refs[0].raw, "docs/a\\(b\\).md");
        assert_eq!(refs[0].decoded, "docs/a(b).md");
    }

    #[test]
    fn splits_mixed_clauses_for_mandatory_classification() {
        let content = "Do not read `a.md` completely; always read `b.md` first.\n";
        let refs = markdown_references(content);
        assert_eq!(refs.len(), 2);
        assert!(!clause_is_mandatory_load(
            refs[0].clause.as_deref().unwrap()
        ));
        assert!(clause_is_mandatory_load(refs[1].clause.as_deref().unwrap()));
    }

    #[test]
    fn explicit_at_imports_respect_negation_before_mandatory_positive() {
        assert!(clause_is_mandatory_load("@reference.md"));
        assert!(!clause_is_mandatory_load("@reference.md if needed"));
        assert!(!clause_is_mandatory_load("never @reference.md"));
        assert!(!clause_rejects_mandatory_at_import("@reference.md"));
        assert!(clause_rejects_mandatory_at_import(
            "Do not load @reference.md"
        ));
        assert!(clause_rejects_mandatory_at_import(
            "@reference.md only when required"
        ));
    }

    #[test]
    fn percent_decodes_once() {
        assert_eq!(percent_decode_once("docs/%2E%2E/x.md"), "docs/../x.md");
        assert_eq!(percent_decode_once("docs/%zz.md"), "docs/%zz.md");
    }

    #[test]
    fn prompt_bases_follow_reference_kind() {
        assert_eq!(
            prompt_resolution_base(MarkdownRefKind::Link, "references/a.md"),
            ResolutionBase::SourceRelative
        );
        assert_eq!(
            prompt_resolution_base(MarkdownRefKind::InlineCode, "docs/a.md"),
            ResolutionBase::RepositoryRoot
        );
        assert_eq!(
            prompt_resolution_base(MarkdownRefKind::InlineCode, "references/a.md"),
            ResolutionBase::SourceRelative
        );
        assert_eq!(
            prompt_resolution_base(MarkdownRefKind::InlineCode, ".claude/skills/a.md"),
            ResolutionBase::RepositoryRoot
        );
    }

    #[test]
    fn issue_600_utf8_arrow_reproducer_selects_owning_clause() {
        // Exact reproducer from issue #600: the 3-byte `↔` scalar used to
        // leave the clause scanner mid-character and panic.
        let content = "- `docs/issue-anchored-plan.md`: **LIVE** /design \u{2194} /implement wire format, clarification round-trip, and pause pointer\n";
        let refs = markdown_references(content);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].raw, "docs/issue-anchored-plan.md");
        let clause = refs[0].clause.as_deref().expect("live prose clause");
        assert!(clause.contains('\u{2194}'), "{clause}");
        assert!(clause.contains("`docs/issue-anchored-plan.md`"), "{clause}");
    }

    #[test]
    fn multibyte_scalars_keep_clause_ranges_on_char_boundaries() {
        // One scalar per UTF-8 width: 1, 2, 3, and 4 bytes.
        for scalar in ["x", "\u{e9}", "\u{2194}", "\u{1f680}"] {
            let lines = [
                // before the clause holding the reference
                format!("{scalar} read `docs/a.md` completely first; then stop."),
                // inside the reference clause
                format!("Always read `docs/a.md` {scalar} first."),
                // inside the inline code itself
                format!("Read `docs/{scalar}.md` completely first."),
                // before and inside a link clause
                format!("{scalar} see [a]({scalar}.md) now. Then read `docs/a.md` first always."),
                // touching em and en dash clause boundaries
                format!("Read `docs/a.md` first{scalar}\u{2014}{scalar}always."),
                format!("Read `docs/a.md` first{scalar}\u{2013}{scalar}always."),
                // trailing an unclosed multi-backtick run
                format!("Read `docs/a.md` completely first. ``ab{scalar}"),
                // immediately after an angle-bracket link destination
                format!("Do [a](<docs/a.md>{scalar} x) and read `docs/b.md` completely first."),
            ];
            for line in lines {
                for (range, text) in split_clauses(&line) {
                    assert!(line.is_char_boundary(range.start), "{line:?} {range:?}");
                    assert!(line.is_char_boundary(range.end), "{line:?} {range:?}");
                    assert_eq!(&line[range.clone()], text, "{line:?}");
                }
                for reference in markdown_references(&format!("{line}\n")) {
                    assert!(reference.clause.is_some(), "{line:?} {reference:?}");
                }
            }
        }
    }

    #[test]
    fn multibyte_prefix_still_selects_the_owning_reference_clause() {
        // Scalars before a reference shift byte offsets; the reference must
        // resolve inside its own clause, not a neighbor (issue #600).
        let content =
            "\u{1f680}\u{2194} intro; never read `docs/a.md`; always read `docs/b.md` first.\n";
        let refs = markdown_references(content);
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].raw, "docs/a.md");
        assert_eq!(refs[1].raw, "docs/b.md");
        assert!(!clause_is_mandatory_load(
            refs[0].clause.as_deref().unwrap()
        ));
        assert!(clause_is_mandatory_load(refs[1].clause.as_deref().unwrap()));
    }

    #[test]
    fn unclosed_multi_backtick_tail_keeps_the_owning_clause() {
        let content = "Read `docs/a.md` completely first. ``ab\u{2194}\n";
        let refs = markdown_references(content);
        assert_eq!(refs[0].raw, "docs/a.md");
        assert_eq!(
            refs[0].clause.as_deref(),
            Some("Read `docs/a.md` completely first.")
        );
    }

    #[test]
    fn angle_destination_followed_by_multibyte_scalar_keeps_the_owning_clause() {
        let content = "Do [a](<docs/a.md>\u{2194} x) and read `docs/b.md` completely first.\n";
        let refs = markdown_references(content);
        let code = refs
            .iter()
            .find(|reference| reference.kind == MarkdownRefKind::InlineCode)
            .expect("inline code reference");
        assert_eq!(code.raw, "docs/b.md");
        assert!(
            code.clause
                .as_deref()
                .unwrap()
                .contains("read `docs/b.md` completely first"),
            "{:?}",
            code.clause
        );
    }

    #[test]
    fn angle_destination_scan_resumes_immediately_after_the_close() {
        // The scan deliberately resumes one byte after `>` (it previously
        // skipped two), so a clause delimiter hard against a malformed angle
        // destination now splits. Well-formed links place `)`, space, or tab
        // there, none of which are clause delimiters.
        let clauses = split_clauses("[a](<x>.md) then");
        assert_eq!(
            clauses
                .iter()
                .map(|(_, text)| text.as_str())
                .collect::<Vec<_>>(),
            ["[a](<x>.", "md) then"]
        );
    }
}
