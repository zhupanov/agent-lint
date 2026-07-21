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
    pub destination: String,
    pub line: usize,
    /// Inclusive one-based start column of the full link node.
    pub start_column: usize,
    /// Inclusive one-based end line of the full link node.
    pub end_line: usize,
    /// Inclusive one-based end column of the full link node.
    pub end_column: usize,
}

/// A source-positioned inline code span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownInlineCode {
    /// Literal contents after CommonMark fence-space stripping.
    pub literal: String,
    /// Inclusive one-based start line of the full code span (including backticks).
    pub start_line: usize,
    /// Inclusive one-based start column of the full code span.
    pub start_column: usize,
    /// Inclusive one-based end line of the full code span.
    pub end_line: usize,
    /// Inclusive one-based end column of the full code span.
    pub end_column: usize,
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
    inline_code: Vec<MarkdownInlineCode>,
    body_prose: Vec<MarkdownProseLine>,
}

impl MarkdownDocument {
    /// Parse `content` once and retain only the owned facts used by validators.
    pub fn parse(content: impl Into<String>) -> Self {
        Self::parse_with_frontmatter(content.into(), true)
    }

    /// Parse already-isolated Markdown prose. This deliberately does not
    /// interpret a leading `---` as frontmatter: callers that already removed
    /// frontmatter must retain the original body semantics.
    pub fn parse_body(content: impl Into<String>) -> Self {
        Self::parse_with_frontmatter(content.into(), false)
    }

    fn parse_with_frontmatter(content: String, parse_frontmatter: bool) -> Self {
        let (frontmatter, body_start) = if parse_frontmatter {
            frontmatter_and_body_start(&content)
        } else {
            (None, None)
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
        let mut inline_code = Vec::new();
        let mut excluded_lines = std::collections::HashSet::new();
        let mut inline_exclusions = Vec::new();
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
                NodeValue::Link(link) => links.push(MarkdownLink {
                    destination: link.url.clone(),
                    line: data.sourcepos.start.line,
                    start_column: data.sourcepos.start.column,
                    end_line: data.sourcepos.end.line,
                    end_column: data.sourcepos.end.column,
                }),
                NodeValue::BlockQuote
                | NodeValue::MultilineBlockQuote(_)
                | NodeValue::Alert(_)
                | NodeValue::CodeBlock(_) => {
                    excluded_lines.extend(data.sourcepos.start.line..=data.sourcepos.end.line);
                }
                NodeValue::Code(code) => {
                    inline_exclusions.push((
                        data.sourcepos.start.line,
                        data.sourcepos.start.column,
                        data.sourcepos.end.line,
                        data.sourcepos.end.column,
                    ));
                    inline_code.push(MarkdownInlineCode {
                        literal: code.literal.clone(),
                        start_line: data.sourcepos.start.line,
                        start_column: data.sourcepos.start.column,
                        end_line: data.sourcepos.end.line,
                        end_column: data.sourcepos.end.column,
                        num_backticks: code.num_backticks,
                    });
                }
                _ => {}
            }
        }

        let body_start_line = if parse_frontmatter {
            body_start
                .map(|start| content[..start].lines().count() + 1)
                .unwrap_or(1)
        } else {
            1
        };
        let mut tracker = CodeFenceTracker::new();
        let mut in_html_comment = false;
        let body_prose = content
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
                for &(start_line, start_column, end_line, end_column) in &inline_exclusions {
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
                        text.chars().count()
                    };
                    text = text
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
                text = mask_html_comments(&text, &mut in_html_comment);
                text = mask_quoted_text(&text);
                Some(MarkdownProseLine {
                    line: line_number,
                    text,
                    masked_inline_code_columns: inline_exclusions
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
                        .collect(),
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
            inline_code,
            body_prose,
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

    /// Source-positioned inline code spans in document order.
    pub fn inline_code(&self) -> &[MarkdownInlineCode] {
        &self.inline_code
    }

    pub fn body_prose(&self) -> &[MarkdownProseLine] {
        &self.body_prose
    }
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

fn fence_facts(content: &str) -> (Vec<MarkdownFence>, Option<usize>) {
    let fences = crate::fence::markdown_fences(content);
    let unclosed_fence_line = crate::fence::find_unclosed_fence_line(content);
    (fences, unclosed_fence_line)
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
        assert_eq!(
            doc.links(),
            [MarkdownLink {
                destination: "docs/a.md".into(),
                line: 5,
                start_column: 1,
                end_line: 5,
                end_column: 17,
            }]
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
}
