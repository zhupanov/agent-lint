//! Shared, source-position-aware Markdown facts.
//!
//! This module is the single parsing boundary for Markdown syntax.  It builds
//! owned facts from Comrak's AST so callers do not retain AST lifetimes or make
//! parser choices themselves.  The fence recovery pass is intentionally local:
//! agent-lint historically ignores an unclosed opener and continues looking for
//! later balanced fences, which differs from CommonMark's EOF-consuming fence.

use comrak::nodes::NodeValue;
use comrak::{Arena, Options, parse_document};

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
}

/// One live prose line with its original one-based source line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownProseLine {
    pub line: usize,
    pub text: String,
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
        let mut excluded_lines = std::collections::HashSet::new();
        let mut inline_code = Vec::new();
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
                }),
                NodeValue::BlockQuote
                | NodeValue::MultilineBlockQuote(_)
                | NodeValue::Alert(_)
                | NodeValue::CodeBlock(_) => {
                    excluded_lines.extend(data.sourcepos.start.line..=data.sourcepos.end.line);
                }
                NodeValue::Code(_) => inline_code.push((
                    data.sourcepos.start.line,
                    data.sourcepos.start.column,
                    data.sourcepos.end.line,
                    data.sourcepos.end.column,
                )),
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
                for &(start_line, start_column, end_line, end_column) in &inline_code {
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
                Some(MarkdownProseLine {
                    line: line_number,
                    text,
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

    pub fn body_prose(&self) -> &[MarkdownProseLine] {
        &self.body_prose
    }
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
                line: 5
            }]
        );
        assert_eq!(doc.fences().len(), 1);
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
}
