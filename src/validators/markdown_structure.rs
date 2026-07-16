//! Markdown structure checks shared across SKILL.md, agent .md, and CLAUDE.md.
//!
//! - X002: unclosed code fence
//! - X003/X004/X005: XML tag balance (fence-aware, skips inline code)

use crate::diagnostic::DiagnosticCollector;
use crate::fence::{self, CodeFenceTracker, LineClass};
use crate::rules::LintRule;
use regex::Regex;
use std::sync::LazyLock;

/// HTML void elements that never take a closing tag.
const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// Opening/closing/self-closing tags. Requires a letter after `<` so comparisons
/// (`a < b`) and numeric placeholders (`<1`) are ignored.
static RE_XML_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<(/)?([A-Za-z][A-Za-z0-9:_-]*)(?:\s[^<>]*?)?(/)?>").unwrap());

/// Inline code span: single backtick run (not a fence).
static RE_INLINE_CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`+[^`]*`+").unwrap());

/// Run X002–X005 against a markdown file's full content.
pub fn check_markdown_structure(path: &str, content: &str, diag: &mut DiagnosticCollector) {
    check_unclosed_fence(path, content, diag);
    check_xml_balance(path, content, diag);
}

fn check_unclosed_fence(path: &str, content: &str, diag: &mut DiagnosticCollector) {
    if let Some(line) = fence::find_unclosed_fence_line(content) {
        diag.report(
            LintRule::UnclosedCodeFence,
            &format!("{path}:{line}: unclosed code fence"),
        );
    }
}

fn check_xml_balance(path: &str, content: &str, diag: &mut DiagnosticCollector) {
    let mut tracker = CodeFenceTracker::new();
    let mut stack: Vec<(String, usize)> = Vec::new();

    for (idx, raw_line) in content.lines().enumerate() {
        let line_no = idx + 1;
        let class = tracker.process_line(raw_line);
        if class != LineClass::Outside {
            continue;
        }

        let scanned = RE_INLINE_CODE.replace_all(raw_line, "");
        for caps in RE_XML_TAG.captures_iter(&scanned) {
            let is_close = caps.get(1).is_some();
            let name = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            let self_closing = caps.get(3).is_some();
            if name.is_empty() || self_closing {
                continue;
            }
            let lower = name.to_ascii_lowercase();
            if VOID_ELEMENTS.contains(&lower.as_str()) {
                continue;
            }

            if is_close {
                match stack.last() {
                    Some((open_name, _)) if open_name.eq_ignore_ascii_case(name) => {
                        stack.pop();
                    }
                    Some((open_name, open_line)) => {
                        diag.report(
                            LintRule::XmlTagMismatched,
                            &format!(
                                "{path}:{line_no}: mismatched closing tag '</{name}>' (open '<{open_name}>' at line {open_line})"
                            ),
                        );
                        stack.pop();
                    }
                    None => {
                        diag.report(
                            LintRule::XmlTagOrphan,
                            &format!(
                                "{path}:{line_no}: closing tag '</{name}>' has no opening tag"
                            ),
                        );
                    }
                }
            } else {
                stack.push((name.to_string(), line_no));
            }
        }
    }

    for (name, line) in stack {
        diag.report(
            LintRule::XmlTagUnclosed,
            &format!("{path}:{line}: unclosed XML tag '<{name}>'"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::DiagnosticCollector;

    fn codes(diag: &DiagnosticCollector) -> Vec<String> {
        diag.diagnostics()
            .iter()
            .map(|d| d.rule.code().to_string())
            .collect()
    }

    #[test]
    fn unclosed_fence_reports_opener_line() {
        let mut diag = DiagnosticCollector::new_all_enabled();
        check_markdown_structure("f.md", "# Hi\n```bash\necho hi\n", &mut diag);
        assert!(codes(&diag).iter().any(|c| c == "X002"));
    }

    #[test]
    fn balanced_fence_ok() {
        let mut diag = DiagnosticCollector::new_all_enabled();
        check_markdown_structure("f.md", "```bash\necho hi\n```\n", &mut diag);
        assert!(!codes(&diag).iter().any(|c| c == "X002"));
    }

    #[test]
    fn xml_in_fence_ignored() {
        let mut diag = DiagnosticCollector::new_all_enabled();
        check_markdown_structure("f.md", "```\n<div>\n```\n", &mut diag);
        assert!(codes(&diag).is_empty());
    }

    #[test]
    fn xml_in_inline_code_ignored() {
        let mut diag = DiagnosticCollector::new_all_enabled();
        check_markdown_structure("f.md", "Use `<div>` as an example.\n", &mut diag);
        assert!(codes(&diag).is_empty());
    }

    #[test]
    fn comparison_and_numeric_placeholder_ignored() {
        let mut diag = DiagnosticCollector::new_all_enabled();
        check_markdown_structure("f.md", "when a < b and use <1 for first\n", &mut diag);
        assert!(codes(&diag).is_empty());
    }

    #[test]
    fn mismatched_and_orphan_and_unclosed() {
        let mut diag = DiagnosticCollector::new_all_enabled();
        check_markdown_structure("f.md", "<a>\n</b>\n</c>\n<div>\n", &mut diag);
        let c = codes(&diag);
        assert!(c.iter().any(|x| x == "X004"));
        assert!(c.iter().any(|x| x == "X005"));
        assert!(c.iter().any(|x| x == "X003"));
    }

    #[test]
    fn balanced_prompt_tags_ok() {
        let mut diag = DiagnosticCollector::new_all_enabled();
        check_markdown_structure("f.md", "<example>\nhello\n</example>\n", &mut diag);
        assert!(codes(&diag).is_empty());
    }
}
