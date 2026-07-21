//! Shared unfinished-work marker grammar for D003, G006, and G007.
//!
//! A marker word (`TODO` / `FIXME` / `HACK` / `XXX`) is debt only when it is
//! syntactically recognizable as an unfinished-work annotation: at line start
//! after an optional Markdown heading/list/unchecked-task prefix, or
//! immediately after a source-comment introducer, and followed by `:`, an
//! owner parenthesis, or end-of-line. Ordinary prose mentioning those words is
//! intentionally clean.

use crate::diagnostic::{DiagnosticMetadata, SourceSpan};
use crate::markdown::MarkdownDocument;
use regex::Regex;
use std::sync::LazyLock;

/// Fixed actionable suggestion shared by D003, G006, and G007.
pub const UNFINISHED_WORK_SUGGESTION: &str = "Remove the unfinished-work marker before publishing.";

/// One unfinished-work marker hit with source-absolute coordinates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnfinishedWorkHit {
    /// Marker text as it appears in the source (case preserved).
    pub marker: String,
    /// One-based source line.
    pub line: usize,
    /// One-based inclusive start column of the marker word.
    pub start_column: usize,
    /// One-based exclusive end column of the marker word.
    pub end_column: usize,
}

impl UnfinishedWorkHit {
    /// Structured diagnostic metadata shared by D003, G006, and G007.
    pub fn metadata(&self) -> DiagnosticMetadata {
        DiagnosticMetadata::default()
            .with_location(SourceSpan::range(
                self.line,
                self.start_column,
                self.line,
                self.end_column,
            ))
            .with_evidence(&self.marker)
            .with_suggestion(UNFINISHED_WORK_SUGGESTION)
    }
}

/// First qualifying unfinished-work marker in document order, or `None`.
///
/// Scans top-to-bottom; on each candidate line takes the leftmost match.
/// Frontmatter, fenced/indented code, block quotes, inline code, Markdown
/// links/images, and balanced quoted prose are ignored. Qualifying HTML
/// comments, headings, and unchecked-task labels remain visible.
pub fn find_first_unfinished_work_marker(content: &str) -> Option<UnfinishedWorkHit> {
    let document = MarkdownDocument::parse(content);
    for prose in document.unfinished_work_lines() {
        let Some(match_) = find_marker_in_line(&prose.text) else {
            continue;
        };
        let original = source_line(content, prose.line).unwrap_or(prose.text.as_str());
        let marker = slice_columns(original, match_.start_column, match_.end_column)
            .unwrap_or(&match_.marker)
            .to_string();
        return Some(UnfinishedWorkHit {
            marker,
            line: prose.line,
            start_column: match_.start_column,
            end_column: match_.end_column,
        });
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LineMatch {
    marker: String,
    start_column: usize,
    end_column: usize,
}

/// Line-start form with `:` or owner parenthesis after the marker.
static RE_LINE_START_ANNOTATED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(?P<prefix>\s*(?:#{1,6}\s+|(?:[-*+]|\d+\.)\s+(?:\[[ ]\]\s+)?)?)(?P<marker>TODO|FIXME|HACK|XXX)(?::|\([^)]*\))",
    )
    .expect("unfinished-work line-start annotated regex")
});

/// Line-start form where the marker is the last non-whitespace token.
static RE_LINE_START_EOL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(?P<prefix>\s*(?:#{1,6}\s+|(?:[-*+]|\d+\.)\s+(?:\[[ ]\]\s+)?)?)(?P<marker>TODO|FIXME|HACK|XXX)\s*$",
    )
    .expect("unfinished-work line-start EOL regex")
});

/// Comment-introducer form with `:` or owner parenthesis.
/// The `*` block-comment continuation is covered by the list prefix in the
/// line-start patterns (`^\s*\*\s+MARKER`) so mid-line emphasis like `*TODO:*`
/// stays clean.
static RE_COMMENT_ANNOTATED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?P<intro>#|//|/\*|<!--)\s*(?P<marker>TODO|FIXME|HACK|XXX)(?::|\([^)]*\))")
        .expect("unfinished-work comment annotated regex")
});

/// Comment-introducer form where the marker is the last non-whitespace token.
static RE_COMMENT_EOL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?P<intro>#|//|/\*|<!--)\s*(?P<marker>TODO|FIXME|HACK|XXX)\s*$")
        .expect("unfinished-work comment EOL regex")
});

fn find_marker_in_line(line: &str) -> Option<LineMatch> {
    let mut best: Option<LineMatch> = None;

    for regex in [
        &*RE_LINE_START_ANNOTATED,
        &*RE_LINE_START_EOL,
        &*RE_COMMENT_ANNOTATED,
        &*RE_COMMENT_EOL,
    ] {
        for captures in regex.captures_iter(line) {
            let marker = captures.name("marker").expect("marker group");
            let candidate = line_match_from_regex(line, marker);
            best = Some(match best {
                Some(current) if current.start_column <= candidate.start_column => current,
                _ => candidate,
            });
        }
    }

    best
}

fn line_match_from_regex(line: &str, marker: regex::Match<'_>) -> LineMatch {
    let start_column = byte_to_column(line, marker.start());
    let end_column = byte_to_column(line, marker.end());
    LineMatch {
        marker: marker.as_str().to_string(),
        start_column,
        end_column,
    }
}

fn byte_to_column(line: &str, byte_offset: usize) -> usize {
    line.get(..byte_offset)
        .map(|prefix| prefix.chars().count() + 1)
        .unwrap_or(1)
}

fn slice_columns(line: &str, start_column: usize, end_column: usize) -> Option<&str> {
    if start_column == 0 || end_column < start_column {
        return None;
    }
    let mut start_byte = None;
    let mut end_byte = None;
    for (column, (byte_idx, _)) in line.char_indices().enumerate() {
        let column = column + 1;
        if column == start_column {
            start_byte = Some(byte_idx);
        }
        if column == end_column {
            end_byte = Some(byte_idx);
            break;
        }
    }
    if end_byte.is_none() && end_column == line.chars().count() + 1 {
        end_byte = Some(line.len());
    }
    match (start_byte, end_byte) {
        (Some(start), Some(end)) => line.get(start..end),
        _ => None,
    }
}

fn source_line(content: &str, line_number: usize) -> Option<&str> {
    content.lines().nth(line_number.checked_sub(1)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(content: &str) -> Option<UnfinishedWorkHit> {
        find_first_unfinished_work_marker(content)
    }

    fn marker_only(content: &str) -> Option<String> {
        hit(content).map(|h| h.marker)
    }

    #[test]
    fn positives_cover_grammar_and_comment_styles() {
        let cases = [
            ("TODO:\n", "TODO"),
            ("TODO: implement parser\n", "TODO"),
            ("FIXME(alice):\n", "FIXME"),
            ("FIXME(alice)\n", "FIXME"),
            ("HACK\n", "HACK"),
            ("XXX\n", "XXX"),
            ("# TODO: heading debt\n", "TODO"),
            ("## FIXME(owner): section\n", "FIXME"),
            ("- TODO: list item\n", "TODO"),
            ("* HACK: star list\n", "HACK"),
            ("1. XXX: numbered\n", "XXX"),
            ("- [ ] FIXME: unchecked\n", "FIXME"),
            ("  - [ ] TODO:\n", "TODO"),
            ("// TODO: line comment\n", "TODO"),
            ("code // FIXME: trailing\n", "FIXME"),
            ("/* HACK: block\n", "HACK"),
            (" * TODO: continuation\n", "TODO"),
            ("<!-- XXX: html -->\n", "XXX"),
            ("prefix <!-- TODO: mid -->\n", "TODO"),
            ("todo:\n", "todo"),
            ("FixMe(Owner):\n", "FixMe"),
        ];
        for (content, expected) in cases {
            assert_eq!(
                marker_only(content).as_deref(),
                Some(expected),
                "expected debt in {content:?}"
            );
        }
    }

    #[test]
    fn hard_negatives_prefer_false_negatives_for_prose() {
        let cases = [
            "Remove any TODO or FIXME markers from generated output before returning it.\n",
            "Reject output containing TODO, FIXME, HACK, or XXX markers.\n",
            "The literal marker `TODO` is prohibited in committed instructions.\n",
            "Do not hack around the permission system.\n",
            "Never use xxx as a placeholder.\n",
            "TODO list of follow-ups\n",
            "See the TODO list.\n",
            "> TODO: quoted block\n",
            "The guide says \"TODO: example\".\n",
            "Use 'FIXME:' in docs only as an example.\n",
            "Read [TODO:](https://example.com/todo).\n",
            "Read [notes](https://example.com/TODO:x).\n",
            "- [x] TODO: checked task stays clean\n",
            "*TODO:* emphasized\n",
            "```\nTODO: fenced\n```\n",
            "---\nname: x\ndescription: d\n---\nRemove TODO markers.\n",
        ];
        for content in cases {
            assert_eq!(marker_only(content), None, "expected clean: {content:?}");
        }
    }

    #[test]
    fn reports_first_marker_with_structured_location() {
        let content = "Intro\n\nRemove TODO markers.\n\n- [ ] FIXME: real debt\nTODO: later\n";
        let found = hit(content).expect("debt");
        assert_eq!(found.marker, "FIXME");
        assert_eq!(found.line, 5);
        assert_eq!(found.start_column, 7);
        assert_eq!(found.end_column, 12);
    }

    #[test]
    fn ignores_frontmatter_and_keeps_body_line_numbers() {
        let content =
            "---\nname: demo\ndescription: demo skill for tests\n---\nTODO: implement this\n";
        let found = hit(content).expect("debt");
        assert_eq!(found.marker, "TODO");
        assert_eq!(found.line, 5);
        assert_eq!(found.start_column, 1);
        assert_eq!(found.end_column, 5);
    }

    #[test]
    fn html_comment_multiline_body_is_debt() {
        let content = "<!--\nTODO: finish release instructions\n-->\n";
        let found = hit(content).expect("debt");
        assert_eq!(found.marker, "TODO");
        assert_eq!(found.line, 2);
    }
}
