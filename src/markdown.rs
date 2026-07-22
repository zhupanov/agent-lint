//! Shared, source-position-aware Markdown facts.
//!
//! This module is the single parsing boundary for Markdown syntax.  It builds
//! owned facts from Comrak's AST so callers do not retain AST lifetimes or make
//! parser choices themselves.  The fence recovery pass is intentionally local:
//! agent-lint historically ignores an unclosed opener and continues looking for
//! later balanced fences, which differs from CommonMark's EOF-consuming fence.

use comrak::nodes::NodeValue;
use comrak::{Arena, Options, parse_document};
use std::ops::RangeInclusive;

use crate::fence::{CodeFenceTracker, LineClass, MarkdownFence};

/// A source-positioned Markdown heading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownHeading {
    pub level: u8,
    pub line: usize,
    pub text: String,
}

/// A source-positioned Markdown link destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownLink {
    /// Destination after Comrak's CommonMark decode (not percent-decoded again).
    pub destination: String,
    /// Authored destination spelling recovered from the source span.
    pub raw_destination: String,
    /// UTF-8 byte range of the authored destination only.
    pub destination_byte_range: std::ops::Range<usize>,
    pub line: usize,
    /// Inclusive one-based start column of the full link node.
    pub start_column: usize,
    /// Inclusive one-based end line of the full link node.
    pub end_line: usize,
    /// Inclusive one-based end column of the full link node.
    pub end_column: usize,
    /// UTF-8 byte range of the full link node.
    pub byte_range: std::ops::Range<usize>,
}

/// A source-positioned Markdown image destination (never treated as a link).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownImage {
    pub destination: String,
    pub raw_destination: String,
    pub destination_byte_range: std::ops::Range<usize>,
    pub line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
    pub byte_range: std::ops::Range<usize>,
}

/// A source-positioned inline code span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownInlineCode {
    /// Literal contents after CommonMark fence-space stripping.
    pub literal: String,
    /// Authored interior spelling recovered from the source span.
    pub raw_literal: String,
    /// UTF-8 byte range of the authored interior (excluding delimiters).
    pub literal_byte_range: std::ops::Range<usize>,
    /// Inclusive one-based start line of the full code span (including backticks).
    pub start_line: usize,
    /// Inclusive one-based start column of the full code span.
    pub start_column: usize,
    /// Inclusive one-based end line of the full code span.
    pub end_line: usize,
    /// Inclusive one-based end column of the full code span.
    pub end_column: usize,
    /// UTF-8 byte range of the full code span including backticks.
    pub byte_range: std::ops::Range<usize>,
    /// Number of opening backticks.
    pub num_backticks: usize,
}

/// One live prose line with its original one-based source line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownProseLine {
    pub line: usize,
    pub text: String,
    /// Original one-based character columns whose contents were masked because
    /// they belong to inline code. Consumers that normalize prose can retain a
    /// stable mask boundary without recovering the code contents.
    pub masked_inline_code_columns: Vec<RangeInclusive<usize>>,
}

/// An owned view of a Markdown document and the syntax facts validators share.
#[derive(Debug, Clone)]
pub struct MarkdownDocument {
    content: String,
    body_is_content: bool,
    frontmatter: Option<Vec<String>>,
    body_start: Option<usize>,
    fences: Vec<MarkdownFence>,
    unclosed_fence_line: Option<usize>,
    headings: Vec<MarkdownHeading>,
    links: Vec<MarkdownLink>,
    #[allow(dead_code)] // retained so images never leak into links()
    images: Vec<MarkdownImage>,
    inline_code: Vec<MarkdownInlineCode>,
    body_prose: Vec<MarkdownProseLine>,
    /// Live prose for control validators that intentionally consume inline
    /// machine-result tokens. All other prose exclusions still apply.
    body_control_prose: Vec<MarkdownProseLine>,
    /// Source lines owned by parsed Markdown code blocks. Unlike the streaming
    /// fence tracker, this includes fences nested under list indentation.
    code_block_lines: std::collections::HashSet<usize>,
    /// Candidate lines for unfinished-work marker scanning (D003/G006/G007).
    /// Same exclusions as `body_prose`, except HTML comments stay visible and
    /// Markdown link/image spans are masked so label/destination prose cannot
    /// look like debt markers.
    unfinished_work_lines: Vec<MarkdownProseLine>,
}

impl MarkdownDocument {
    /// Parse `content` once and retain only the owned facts used by validators.
    pub fn parse(content: impl Into<String>) -> Self {
        Self::parse_with_mode(content.into(), FrontmatterMode::Structural)
    }

    /// Parse already-isolated Markdown prose. This deliberately does not
    /// interpret a leading `---` as frontmatter: callers that already removed
    /// frontmatter must retain the original body semantics.
    pub fn parse_body(content: impl Into<String>) -> Self {
        Self::parse_with_mode(content.into(), FrontmatterMode::None)
    }

    /// Parse live instruction prose under the shared frontmatter recovery policy.
    ///
    /// Returns `None` when an exact opener has no closer, matching the
    /// documented Q-rule skip for unterminated frontmatter. Complete blocks
    /// (valid, invalid, or non-object YAML) expose only the body after the
    /// closer, including BOM-prefixed openers, while preserving original
    /// source line numbers.
    pub fn parse_for_prompt_content(content: impl Into<String>) -> Option<Self> {
        let content = content.into();
        match crate::frontmatter::exact_leading_frontmatter(&content) {
            crate::frontmatter::LeadingFrontmatterState::Unterminated { .. } => None,
            crate::frontmatter::LeadingFrontmatterState::Absent { .. } => {
                Some(Self::parse_with_mode(content, FrontmatterMode::None))
            }
            crate::frontmatter::LeadingFrontmatterState::Complete(_) => Some(
                Self::parse_with_mode(content, FrontmatterMode::PromptRecovery),
            ),
        }
    }

    fn parse_with_mode(content: String, mode: FrontmatterMode) -> Self {
        let (frontmatter, body_start, parse_frontmatter) = match mode {
            FrontmatterMode::None => (None, None, false),
            FrontmatterMode::Structural => {
                let (frontmatter, body_start) = frontmatter_and_body_start(&content);
                (frontmatter, body_start, true)
            }
            FrontmatterMode::PromptRecovery => {
                let (frontmatter, body_start) = frontmatter_and_body_start_for_prompt(&content);
                (frontmatter, body_start, true)
            }
        };
        let (fences, unclosed_fence_line) = fence_facts(&content);

        let arena = Arena::new();
        let mut options = Options::default();
        if parse_frontmatter {
            options.extension.front_matter_delimiter = Some("---".to_string());
        }
        let root = parse_document(&arena, &content, &options);
        let mut headings = Vec::new();
        let mut links = Vec::new();
        let mut images = Vec::new();
        let mut inline_code = Vec::new();
        let mut excluded_lines = std::collections::HashSet::new();
        let mut code_block_lines = std::collections::HashSet::new();
        let mut inline_exclusions = Vec::new();
        let mut link_exclusions = Vec::new();
        let mut link_ranges = Vec::new();
        for node in root.descendants() {
            let data = node.data.borrow();
            match &data.value {
                NodeValue::Heading(heading) => {
                    let line = data.sourcepos.start.line;
                    let text = node
                        .descendants()
                        .filter_map(|child| match &child.data.borrow().value {
                            NodeValue::Text(text) => Some(text.clone()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("");
                    headings.push(MarkdownHeading {
                        level: heading.level,
                        line,
                        text,
                    });
                }
                NodeValue::Link(link) => {
                    let byte_range = byte_range_for_sourcepos(&content, data.sourcepos);
                    let (raw_destination, destination_byte_range) =
                        destination_from_link_span(&content, byte_range.clone(), &link.url);
                    links.push(MarkdownLink {
                        destination: link.url.clone(),
                        raw_destination,
                        destination_byte_range,
                        line: data.sourcepos.start.line,
                        start_column: data.sourcepos.start.column,
                        end_line: data.sourcepos.end.line,
                        end_column: data.sourcepos.end.column,
                        byte_range,
                    });
                    link_exclusions.push((
                        data.sourcepos.start.line,
                        data.sourcepos.start.column,
                        data.sourcepos.end.line,
                        data.sourcepos.end.column,
                    ));
                    link_ranges.push((
                        data.sourcepos.start.line,
                        data.sourcepos.start.column,
                        data.sourcepos.end.line,
                        data.sourcepos.end.column,
                    ));
                }
                NodeValue::Image(image) => {
                    let byte_range = byte_range_for_sourcepos(&content, data.sourcepos);
                    let (raw_destination, destination_byte_range) =
                        destination_from_link_span(&content, byte_range.clone(), &image.url);
                    images.push(MarkdownImage {
                        destination: image.url.clone(),
                        raw_destination,
                        destination_byte_range,
                        line: data.sourcepos.start.line,
                        start_column: data.sourcepos.start.column,
                        end_line: data.sourcepos.end.line,
                        end_column: data.sourcepos.end.column,
                        byte_range,
                    });
                    link_exclusions.push((
                        data.sourcepos.start.line,
                        data.sourcepos.start.column,
                        data.sourcepos.end.line,
                        data.sourcepos.end.column,
                    ));
                }
                NodeValue::BlockQuote | NodeValue::MultilineBlockQuote(_) | NodeValue::Alert(_) => {
                    excluded_lines.extend(data.sourcepos.start.line..=data.sourcepos.end.line);
                }
                NodeValue::CodeBlock(_) => {
                    let lines = data.sourcepos.start.line..=data.sourcepos.end.line;
                    excluded_lines.extend(lines.clone());
                    code_block_lines.extend(lines);
                }
                NodeValue::Code(code) => {
                    inline_exclusions.push((
                        data.sourcepos.start.line,
                        data.sourcepos.start.column,
                        data.sourcepos.end.line,
                        data.sourcepos.end.column,
                    ));
                    let byte_range = byte_range_for_sourcepos(&content, data.sourcepos);
                    let (raw_literal, literal_byte_range) = inline_code_literal_from_span(
                        &content,
                        byte_range.clone(),
                        code.num_backticks,
                    );
                    inline_code.push(MarkdownInlineCode {
                        literal: code.literal.clone(),
                        raw_literal,
                        literal_byte_range,
                        start_line: data.sourcepos.start.line,
                        start_column: data.sourcepos.start.column,
                        end_line: data.sourcepos.end.line,
                        end_column: data.sourcepos.end.column,
                        byte_range,
                        num_backticks: code.num_backticks,
                    });
                }
                _ => {}
            }
        }

        let link_destination_exclusions = link_destination_exclusions(&content, &link_ranges);

        let body_start_line = if parse_frontmatter {
            body_start
                .map(|start| content[..start].lines().count() + 1)
                .unwrap_or(1)
        } else {
            1
        };
        let mut debt_tracker = CodeFenceTracker::new();
        let body_prose = collect_body_prose(
            &content,
            body_start_line,
            &excluded_lines,
            &inline_exclusions,
            &link_destination_exclusions,
            true,
        );
        let body_control_prose = collect_body_prose(
            &content,
            body_start_line,
            &excluded_lines,
            &inline_exclusions,
            &link_destination_exclusions,
            false,
        );

        let unfinished_work_lines = content
            .lines()
            .enumerate()
            .filter_map(|(index, line)| {
                let line_number = index + 1;
                if line_number < body_start_line
                    || debt_tracker.process_line(line) != LineClass::Outside
                    || excluded_lines.contains(&line_number)
                {
                    return None;
                }

                let mut text = line.to_string();
                text = mask_column_ranges(&text, line_number, &inline_exclusions);
                text = mask_column_ranges(&text, line_number, &link_exclusions);
                text = mask_quoted_text(&text);
                Some(MarkdownProseLine {
                    line: line_number,
                    text,
                    masked_inline_code_columns: masked_ranges_for_line(
                        line,
                        line_number,
                        &inline_exclusions,
                    ),
                })
            })
            .collect();

        Self {
            content,
            body_is_content: !parse_frontmatter,
            frontmatter,
            body_start,
            fences,
            unclosed_fence_line,
            headings,
            links,
            images,
            inline_code,
            body_prose,
            body_control_prose,
            code_block_lines,
            unfinished_work_lines,
        }
    }

    pub fn content(&self) -> &str {
        &self.content
    }

    pub fn frontmatter(&self) -> Option<&[String]> {
        self.frontmatter.as_deref()
    }

    /// Body after a complete frontmatter block, if present.
    pub fn body(&self) -> &str {
        if self.body_is_content {
            return &self.content;
        }
        self.body_start
            .and_then(|start| self.content.get(start..))
            .unwrap_or("")
    }

    pub fn body_start_line(&self) -> usize {
        if self.body_is_content {
            return 1;
        }
        self.body_start
            .map(|start| self.content[..start].lines().count() + 1)
            .unwrap_or(1)
    }

    pub fn fences(&self) -> &[MarkdownFence] {
        &self.fences
    }

    pub fn unclosed_fence_line(&self) -> Option<usize> {
        self.unclosed_fence_line
    }

    pub fn headings(&self) -> &[MarkdownHeading] {
        &self.headings
    }

    pub fn links(&self) -> &[MarkdownLink] {
        &self.links
    }

    /// Source-positioned image nodes in document order.
    #[allow(dead_code)] // consumed by L005 hard-negative coverage via absence from links()
    pub fn images(&self) -> &[MarkdownImage] {
        &self.images
    }

    /// Source-positioned inline code spans in document order.
    pub fn inline_code(&self) -> &[MarkdownInlineCode] {
        &self.inline_code
    }

    pub fn body_prose(&self) -> &[MarkdownProseLine] {
        &self.body_prose
    }

    /// Live prose that retains inline code for validators whose contract
    /// explicitly includes machine-readable result/status tokens.
    pub(crate) fn body_control_prose(&self) -> &[MarkdownProseLine] {
        &self.body_control_prose
    }

    /// Body lines suitable for structural scanners. Inline code and HTML
    /// comments are masked from their source-positioned CommonMark facts, but
    /// ordinary quoted prose is retained because it can be part of a tag's
    /// attribute syntax.
    pub(crate) fn structural_prose(&self) -> Vec<MarkdownProseLine> {
        let inline_exclusions: Vec<_> = self
            .inline_code
            .iter()
            .map(|code| {
                (
                    code.start_line,
                    code.start_column,
                    code.end_line,
                    code.end_column,
                )
            })
            .collect();
        let mut tracker = CodeFenceTracker::new();
        let mut in_html_comment = false;
        self.content
            .lines()
            .enumerate()
            .filter_map(|(index, line)| {
                let line_number = index + 1;
                if line_number < self.body_start_line()
                    || tracker.process_line(line) != LineClass::Outside
                    || self.code_block_lines.contains(&line_number)
                {
                    return None;
                }
                let text = mask_html_comments(
                    &mask_column_ranges(line, line_number, &inline_exclusions),
                    &mut in_html_comment,
                );
                Some(MarkdownProseLine {
                    line: line_number,
                    text,
                    masked_inline_code_columns: masked_ranges_for_line(
                        line,
                        line_number,
                        &inline_exclusions,
                    ),
                })
            })
            .collect()
    }

    /// Lines eligible for unfinished-work marker classification.
    ///
    /// Unlike [`body_prose`], HTML comments remain visible so
    /// `<!-- TODO: ... -->` can qualify, and Markdown link/image spans are
    /// masked so marker words inside labels or destinations stay clean.
    pub fn unfinished_work_lines(&self) -> &[MarkdownProseLine] {
        &self.unfinished_work_lines
    }
}

/// Return source-column ranges occupied by inline and autolink destinations.
///
/// Comrak exposes a link node's full source span but not a separate span for
/// its destination. Restricting this source scan to those parsed link spans
/// keeps Markdown syntax in code or ordinary prose from being mistaken for a
/// link while preserving visible link labels for prose validators.
fn link_destination_exclusions(
    content: &str,
    link_ranges: &[(usize, usize, usize, usize)],
) -> Vec<(usize, usize, usize, usize)> {
    let chars: Vec<char> = content.chars().collect();
    let mut positions = Vec::with_capacity(chars.len());
    let mut line = 1;
    let mut column = 1;
    for character in &chars {
        positions.push((line, column));
        if *character == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }

    let in_link_range = |index: usize| {
        let (line, column) = positions[index];
        link_ranges
            .iter()
            .any(|&(start_line, start_column, end_line, end_column)| {
                (line > start_line || (line == start_line && column >= start_column))
                    && (line < end_line || (line == end_line && column <= end_column))
            })
    };
    let mut exclusions = Vec::new();
    let mut index = 0;
    while index + 1 < chars.len() {
        if chars[index] == ']' && chars[index + 1] == '(' && in_link_range(index) {
            let start = index + 2;
            let mut cursor = start;
            let mut depth = 1;
            while cursor < chars.len() {
                if chars[cursor] == '\\' {
                    cursor += 2;
                    continue;
                }
                match chars[cursor] {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            add_character_ranges(&mut exclusions, &positions, start, cursor);
                            index = cursor;
                            break;
                        }
                    }
                    _ => {}
                }
                cursor += 1;
            }
        } else if chars[index] == '<' && in_link_range(index) {
            if let Some(end) = ((index + 1)..chars.len()).find(|&cursor| chars[cursor] == '>') {
                add_character_ranges(&mut exclusions, &positions, index + 1, end);
                index = end;
            }
        }
        index += 1;
    }
    exclusions
}

fn add_character_ranges(
    ranges: &mut Vec<(usize, usize, usize, usize)>,
    positions: &[(usize, usize)],
    start: usize,
    end: usize,
) {
    if start >= end {
        return;
    }
    let mut range_start = None;
    let mut previous = None;
    for &(line, column) in positions.iter().take(end).skip(start) {
        if previous.is_some_and(|(previous_line, previous_column)| {
            line != previous_line || column != previous_column + 1
        }) {
            let (start_line, start_column) = range_start.expect("range has a start");
            let (end_line, end_column) = previous.expect("range has a previous position");
            ranges.push((start_line, start_column, end_line, end_column));
            range_start = None;
        }
        if range_start.is_none() {
            range_start = Some((line, column));
        }
        previous = Some((line, column));
    }
    if let (Some((start_line, start_column)), Some((end_line, end_column))) =
        (range_start, previous)
    {
        ranges.push((start_line, start_column, end_line, end_column));
    }
}

/// Build live prose from the canonical Markdown exclusions. Control-oriented
/// consumers may retain inline code because result/status assignments are part
/// of their grammar; ordinary prompt consumers keep it masked.
fn collect_body_prose(
    content: &str,
    body_start_line: usize,
    excluded_lines: &std::collections::HashSet<usize>,
    inline_exclusions: &[(usize, usize, usize, usize)],
    link_destination_exclusions: &[(usize, usize, usize, usize)],
    mask_inline_code: bool,
) -> Vec<MarkdownProseLine> {
    let mut tracker = CodeFenceTracker::new();
    let mut in_html_comment = false;
    content
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let line_number = index + 1;
            if line_number < body_start_line
                || tracker.process_line(line) != LineClass::Outside
                || excluded_lines.contains(&line_number)
            {
                return None;
            }

            let mut text = line.to_string();
            if mask_inline_code {
                text = mask_column_ranges(&text, line_number, inline_exclusions);
            }
            text = mask_column_ranges(&text, line_number, link_destination_exclusions);
            text = mask_html_comments(&text, &mut in_html_comment);
            text = mask_quoted_text(&text);
            Some(MarkdownProseLine {
                line: line_number,
                text,
                masked_inline_code_columns: if mask_inline_code {
                    masked_ranges_for_line(line, line_number, inline_exclusions)
                } else {
                    Vec::new()
                },
            })
        })
        .collect()
}

fn mask_column_ranges(
    text: &str,
    line_number: usize,
    ranges: &[(usize, usize, usize, usize)],
) -> String {
    let mut masked = text.to_string();
    for &(start_line, start_column, end_line, end_column) in ranges {
        if line_number < start_line || line_number > end_line {
            continue;
        }
        let first = if line_number == start_line {
            start_column
        } else {
            1
        };
        let last = if line_number == end_line {
            end_column
        } else {
            masked.chars().count()
        };
        masked = masked
            .chars()
            .enumerate()
            .map(|(column, ch)| {
                if (first..=last).contains(&(column + 1)) {
                    ' '
                } else {
                    ch
                }
            })
            .collect();
    }
    masked
}

fn masked_ranges_for_line(
    line: &str,
    line_number: usize,
    ranges: &[(usize, usize, usize, usize)],
) -> Vec<RangeInclusive<usize>> {
    ranges
        .iter()
        .filter_map(|&(start_line, start_column, end_line, end_column)| {
            if line_number < start_line || line_number > end_line {
                return None;
            }
            let first = if line_number == start_line {
                start_column
            } else {
                1
            };
            let last = if line_number == end_line {
                end_column
            } else {
                line.chars().count()
            };
            Some(first..=last)
        })
        .collect()
}

/// Replace HTML comments with spaces while retaining source columns.
///
/// The state is intentionally caller-owned so scanners can carry a comment
/// through successive lines of one Markdown document.
pub(crate) fn mask_html_comments(text: &str, in_comment: &mut bool) -> String {
    let mut chars: Vec<char> = text.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        if *in_comment {
            if chars[index..].starts_with(&['-', '-', '>']) {
                chars[index..index + 3].fill(' ');
                *in_comment = false;
                index += 3;
            } else {
                chars[index] = ' ';
                index += 1;
            }
        } else if chars[index..].starts_with(&['<', '!', '-', '-']) {
            chars[index..index + 4].fill(' ');
            *in_comment = true;
            index += 4;
        } else {
            index += 1;
        }
    }
    chars.into_iter().collect()
}

/// Remove balanced quoted examples while preserving columns. Apostrophes in
/// contractions and possessives are not treated as quote delimiters.
fn mask_quoted_text(text: &str) -> String {
    let mut chars: Vec<char> = text.chars().collect();
    for (opening, closing) in [('"', '"'), ('“', '”'), ('‘', '’'), ('\'', '\'')] {
        let mut index = 0;
        while index < chars.len() {
            if chars[index] != opening || !is_quote_opener(&chars, index, opening) {
                index += 1;
                continue;
            }
            let Some(end) = ((index + 1)..chars.len()).find(|&candidate| {
                chars[candidate] == closing && is_quote_closer(&chars, candidate, closing)
            }) else {
                index += 1;
                continue;
            };
            chars[index..=end].fill(' ');
            index = end + 1;
        }
    }
    chars.into_iter().collect()
}

fn is_quote_opener(chars: &[char], index: usize, quote: char) -> bool {
    quote != '\''
        || (index == 0 || !chars[index - 1].is_alphanumeric())
            && chars
                .get(index + 1)
                .is_some_and(|next| !next.is_whitespace())
}

fn is_quote_closer(chars: &[char], index: usize, quote: char) -> bool {
    quote != '\''
        || (index + 1 == chars.len() || !chars[index + 1].is_alphanumeric())
            && index > 0
            && !chars[index - 1].is_whitespace()
}

fn frontmatter_and_body_start(content: &str) -> (Option<Vec<String>>, Option<usize>) {
    let mut lines = content.split_inclusive('\n');
    let Some(first) = lines.next() else {
        return (None, None);
    };
    if first.trim_end_matches(['\r', '\n']) != "---" {
        return (None, None);
    }

    let mut frontmatter = Vec::new();
    let mut offset = first.len();
    for line in lines {
        let text = line.trim_end_matches(['\r', '\n']);
        offset += line.len();
        if text == "---" {
            return (Some(frontmatter), Some(offset));
        }
        frontmatter.push(text.to_string());
    }
    (None, None)
}

enum FrontmatterMode {
    None,
    Structural,
    PromptRecovery,
}

fn frontmatter_and_body_start_for_prompt(content: &str) -> (Option<Vec<String>>, Option<usize>) {
    // Prompt recovery shares Cursor CU002/CU003's optional-BOM and exact
    // delimiter grammar. Structural `parse` keeps its byte-exact opener so
    // sibling surfaces (skills, agents) retain their own BOM contracts.
    match crate::frontmatter::exact_leading_frontmatter(content) {
        crate::frontmatter::LeadingFrontmatterState::Complete(block) => {
            let frontmatter = block
                .yaml
                .lines()
                .map(|line| line.trim_end_matches('\r').to_string())
                .collect();
            let body_start = content.len() - block.body.len();
            (Some(frontmatter), Some(body_start))
        }
        crate::frontmatter::LeadingFrontmatterState::Absent { .. }
        | crate::frontmatter::LeadingFrontmatterState::Unterminated { .. } => (None, None),
    }
}

fn fence_facts(content: &str) -> (Vec<MarkdownFence>, Option<usize>) {
    let fences = crate::fence::markdown_fences(content);
    let unclosed_fence_line = crate::fence::find_unclosed_fence_line(content);
    (fences, unclosed_fence_line)
}

/// Convert a Comrak sourcepos (1-based inclusive columns) into a UTF-8 byte range.
fn byte_range_for_sourcepos(
    content: &str,
    sourcepos: comrak::nodes::Sourcepos,
) -> std::ops::Range<usize> {
    let start = offset_for_line_column(content, sourcepos.start.line, sourcepos.start.column);
    let end =
        offset_for_line_column(content, sourcepos.end.line, sourcepos.end.column).map(|offset| {
            content[offset..]
                .chars()
                .next()
                .map_or(offset, |ch| offset + ch.len_utf8())
        });
    match (start, end) {
        (Some(start), Some(end)) if start <= end && end <= content.len() => start..end,
        _ => 0..0,
    }
}

fn offset_for_line_column(content: &str, line: usize, column: usize) -> Option<usize> {
    if line == 0 || column == 0 {
        return None;
    }
    let mut current_line = 1usize;
    let mut line_start = 0usize;
    for (offset, ch) in content.char_indices() {
        if current_line == line {
            let mut columns = 1usize;
            for (rel, line_ch) in content[line_start..].char_indices() {
                if columns == column {
                    return Some(line_start + rel);
                }
                if line_ch == '\n' {
                    break;
                }
                columns += 1;
            }
            return None;
        }
        if ch == '\n' {
            current_line += 1;
            line_start = offset + ch.len_utf8();
        }
    }
    if current_line == line {
        let mut columns = 1usize;
        for (rel, line_ch) in content[line_start..].char_indices() {
            if columns == column {
                return Some(line_start + rel);
            }
            if line_ch == '\n' {
                break;
            }
            columns += 1;
        }
    }
    None
}

fn inline_code_literal_from_span(
    content: &str,
    byte_range: std::ops::Range<usize>,
    num_backticks: usize,
) -> (String, std::ops::Range<usize>) {
    if byte_range.end > content.len() || byte_range.start >= byte_range.end {
        return (String::new(), byte_range);
    }
    let span = &content[byte_range.clone()];
    let opener = "`".repeat(num_backticks.max(1));
    let Some(without_opener) = span.strip_prefix(opener.as_str()) else {
        return (span.to_string(), byte_range);
    };
    let Some(interior) = without_opener.strip_suffix(opener.as_str()) else {
        return (span.to_string(), byte_range);
    };
    let start = byte_range.start + opener.len();
    let end = start + interior.len();
    (interior.to_string(), start..end)
}

fn destination_from_link_span(
    content: &str,
    byte_range: std::ops::Range<usize>,
    fallback: &str,
) -> (String, std::ops::Range<usize>) {
    if byte_range.end > content.len() || byte_range.start >= byte_range.end {
        return (fallback.to_string(), byte_range);
    }
    let span = &content[byte_range.clone()];
    let Some(dest_local_start) = span.find("](").map(|index| index + 2) else {
        return (fallback.to_string(), byte_range);
    };
    let after = &span[dest_local_start..];
    if let Some(rest) = after.strip_prefix('<') {
        let Some(close) = rest.find('>') else {
            return (fallback.to_string(), byte_range);
        };
        let raw = &rest[..close];
        let start = byte_range.start + dest_local_start + 1;
        return (raw.to_string(), start..start + raw.len());
    }

    let mut end = 0usize;
    let mut depth = 0usize;
    let bytes = after.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' if index + 1 < bytes.len() => {
                index += 2;
                end = index;
            }
            b'(' => {
                depth += 1;
                index += 1;
                end = index;
            }
            b')' if depth == 0 => break,
            b')' => {
                depth -= 1;
                index += 1;
                end = index;
            }
            b' ' | b'\t' | b'\n' | b'\r' if depth == 0 => break,
            _ => {
                index += 1;
                end = index;
            }
        }
    }
    let mut raw = after.get(..end).unwrap_or("");
    if let Some((path, _)) = raw.rsplit_once(" \"") {
        raw = path;
    } else if let Some((path, _)) = raw.rsplit_once(" '") {
        raw = path;
    }
    raw = raw.trim_end();
    let start = byte_range.start + dest_local_start;
    (raw.to_string(), start..start + raw.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_owns_frontmatter_body_and_ast_facts() {
        let doc = MarkdownDocument::parse(
            "---\r\nname: example\r\n---\r\n# Heading\r\n[link](docs/a.md)\r\n```bash\r\necho hi\r\n```\r\n",
        );
        assert_eq!(
            doc.frontmatter(),
            Some(["name: example".to_string()].as_slice())
        );
        assert!(doc.body().starts_with("# Heading"));
        assert_eq!(
            doc.headings(),
            [MarkdownHeading {
                level: 1,
                line: 4,
                text: "Heading".into(),
            }]
        );
        assert_eq!(doc.links().len(), 1);
        let link = &doc.links()[0];
        assert_eq!(link.destination, "docs/a.md");
        assert_eq!(link.raw_destination, "docs/a.md");
        assert_eq!(link.line, 5);
        assert_eq!(link.start_column, 1);
        assert_eq!(link.end_line, 5);
        assert_eq!(link.end_column, 17);
        assert_eq!(
            &doc.content()[link.destination_byte_range.clone()],
            "docs/a.md"
        );
        assert_eq!(doc.fences().len(), 1);
    }

    #[test]
    fn document_owns_positioned_inline_code() {
        let doc = MarkdownDocument::parse("Use `npm run build` now.\n");
        assert_eq!(doc.inline_code().len(), 1);
        let code = &doc.inline_code()[0];
        assert_eq!(code.literal, "npm run build");
        assert_eq!(code.start_line, 1);
        assert_eq!(code.start_column, 5);
        assert_eq!(code.end_line, 1);
        assert_eq!(code.num_backticks, 1);
    }

    #[test]
    fn recovery_keeps_later_balanced_fence_after_unclosed_opener() {
        let doc = MarkdownDocument::parse("```bash\nunclosed\n~~~sh\necho hi\n~~~\n");
        assert_eq!(doc.unclosed_fence_line(), Some(1));
        assert_eq!(doc.fences().len(), 1);
        assert_eq!(doc.fences()[0].start_line, 3);
    }

    #[test]
    fn isolated_body_does_not_hide_a_leading_thematic_break() {
        let doc = MarkdownDocument::parse_body("---\n# Important\nYou should verify.\n");
        assert!(doc.frontmatter().is_none());
        assert!(doc.body().starts_with("---"));
    }

    #[test]
    fn prose_masks_html_and_balanced_quoted_examples() {
        let doc = MarkdownDocument::parse_body(
            "Do not expose secrets. <!-- Never apologize. -->\n\
             <!-- Never apologize.\n\
             Avoid explanations. -->\n\
             The guide says \"Never apologize.\"\n\
             <div>\n\
             Never apologize.\n\
             </div>\n\
             Don't be verbose.\n\
             'Avoid explanations.'\n",
        );

        assert_eq!(
            doc.body_prose()
                .iter()
                .map(|line| line.text.trim())
                .collect::<Vec<_>>(),
            [
                "Do not expose secrets.",
                "",
                "",
                "The guide says",
                "<div>",
                "Never apologize.",
                "</div>",
                "Don't be verbose.",
                ""
            ]
        );
    }

    #[test]
    fn prose_masks_link_destinations_but_preserves_labels() {
        let source = "Use [endpoint route URL](https://example.test/endpoint/route/url) now.\n";
        let doc = MarkdownDocument::parse_body(source);
        let prose = &doc.body_prose()[0].text;
        assert!(prose.contains("endpoint route URL"));
        assert!(!prose.contains("example.test"));
        assert_eq!(prose.chars().count(), source.trim_end().chars().count());
        assert_eq!(
            prose.as_str(),
            format!(
                "Use [endpoint route URL]({}) now.",
                " ".repeat("https://example.test/endpoint/route/url".chars().count())
            )
        );
        assert_eq!(
            doc.links()[0].destination,
            "https://example.test/endpoint/route/url"
        );
    }

    #[test]
    fn control_prose_retains_inline_result_tokens_but_not_quoted_examples() {
        let doc = MarkdownDocument::parse_body(
            "Report `CODER_RESULT=no-progress` when no fix is possible.\n\
             The guide says \"Report `CODER_RESULT=bail`.\"\n",
        );

        assert!(!doc.body_prose()[0].text.contains("CODER_RESULT"));
        assert!(
            doc.body_control_prose()[0]
                .text
                .contains("`CODER_RESULT=no-progress`")
        );
        assert!(!doc.body_control_prose()[1].text.contains("CODER_RESULT"));
    }

    #[test]
    fn unfinished_work_lines_keep_html_comments_and_mask_links() {
        let doc = MarkdownDocument::parse_body(
            "Remove TODO markers. <!-- TODO: real -->\n\
             Read [TODO:](https://example.com/TODO:x).\n\
             The guide says \"FIXME: example\".\n",
        );
        let lines: Vec<_> = doc
            .unfinished_work_lines()
            .iter()
            .map(|line| line.text.as_str())
            .collect();
        assert!(lines[0].contains("<!-- TODO: real -->"));
        assert!(!lines[1].contains("TODO"));
        assert!(lines[1].starts_with("Read "));
        assert!(lines[2].starts_with("The guide says"));
        assert!(!lines[2].contains("FIXME"));
    }
}
