//! Markdown structure checks shared across SKILL.md, agent .md, and CLAUDE.md.
//!
//! - X002: unclosed code fence
//! - X003/X004/X005: XML tag balance (fence-aware, skips inline code)

use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata};
use crate::fence::LineClass;
use crate::markdown::{MarkdownDocument, mask_html_comments};
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

/// Run X002–X005 against a markdown file.
///
/// X002 scans the full file. X003–X005 scan the markdown **body** only (after
/// frontmatter), matching the plan's body-scoped XML rules and avoiding FPs on
/// angle brackets inside YAML values.
pub fn check_markdown_structure(path: &str, content: &str, diag: &mut DiagnosticCollector) {
    let document = MarkdownDocument::parse(content);
    check_markdown_document(path, &document, diag);
}

/// Run structure checks against an already parsed document.
pub(crate) fn check_markdown_document(
    path: &str,
    document: &MarkdownDocument,
    diag: &mut DiagnosticCollector,
) {
    if let Some(line) = document.unclosed_fence_line() {
        diag.report_at_with(
            LintRule::UnclosedCodeFence,
            path,
            &format!("{path}:{line}: unclosed code fence"),
            DiagnosticMetadata::at_line(line),
        );
    }
    check_xml_balance(path, document, diag);
}

/// 1-based line number of the first body line. Files without frontmatter start
/// at line 1. When an opening `---` has no closer, scan the whole file.
fn check_xml_balance(path: &str, document: &MarkdownDocument, diag: &mut DiagnosticCollector) {
    let body_start = document.body_start_line();
    let mut tracker = crate::fence::CodeFenceTracker::new();
    let mut in_html_comment = false;
    let mut stack: Vec<(String, usize)> = Vec::new();

    for (idx, raw_line) in document.content().lines().enumerate() {
        let line_no = idx + 1;
        if line_no < body_start {
            continue;
        }
        let class = tracker.process_line(raw_line);
        if class != LineClass::Outside {
            continue;
        }

        let inline_masked = RE_INLINE_CODE.replace_all(raw_line, "");
        let scanned = mask_html_comments(&inline_masked, &mut in_html_comment);
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
                        diag.report_at_with(
                            LintRule::XmlTagMismatched,
                            path,
                            &format!(
                                "{path}:{line_no}: mismatched closing tag '</{name}>' (open '<{open_name}>' at line {open_line})"
                            ),
                            DiagnosticMetadata::at_line(line_no),
                        );
                        stack.pop();
                    }
                    None => {
                        diag.report_at_with(
                            LintRule::XmlTagOrphan,
                            path,
                            &format!(
                                "{path}:{line_no}: closing tag '</{name}>' has no opening tag"
                            ),
                            DiagnosticMetadata::at_line(line_no),
                        );
                    }
                }
            } else {
                stack.push((name.to_string(), line_no));
            }
        }
    }

    for (name, line) in stack {
        diag.report_at_with(
            LintRule::XmlTagUnclosed,
            path,
            &format!("{path}:{line}: unclosed XML tag '<{name}>'"),
            DiagnosticMetadata::at_line(line),
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
        let diagnostic = diag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.rule == LintRule::UnclosedCodeFence)
            .unwrap();
        assert_eq!(
            diagnostic.subject_path.as_deref(),
            Some(std::path::Path::new("f.md"))
        );
        assert_eq!(
            diagnostic.location.map(|location| location.start()),
            Some(crate::diagnostic::SourcePosition::line(2))
        );
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
    fn xml_in_html_comments_is_ignored() {
        for content in [
            "<!-- <div> -->\n",
            "<!--\n<section>\n-->\n",
            "<!-- <div> --> <!-- <section> -->\n",
            "<!-- <div> --> <section>\n</section> <!-- <article> -->\n",
            "<!-- <div>\n<section>\n",
        ] {
            let mut diag = DiagnosticCollector::new_all_enabled();
            check_markdown_structure("f.md", content, &mut diag);
            assert!(codes(&diag).is_empty(), "{content:?}: {:?}", codes(&diag));
        }
    }

    #[test]
    fn tags_outside_html_comments_on_the_same_line_are_scanned() {
        let mut diag = DiagnosticCollector::new_all_enabled();
        check_markdown_structure("f.md", "<div><!-- <section> --></div>\n", &mut diag);
        assert!(codes(&diag).is_empty());

        let mut diag = DiagnosticCollector::new_all_enabled();
        check_markdown_structure("f.md", "<div><!-- <section> -->\n", &mut diag);
        assert_eq!(codes(&diag), vec!["X003"]);
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
    fn xml_diagnostics_have_their_source_line() {
        let mut diag = DiagnosticCollector::new_all_enabled();
        check_markdown_structure("f.md", "<a>\n</b>\n</c>\n<div>\n", &mut diag);

        for (rule, line) in [
            (LintRule::XmlTagMismatched, 2),
            (LintRule::XmlTagOrphan, 3),
            (LintRule::XmlTagUnclosed, 4),
        ] {
            let diagnostic = diag
                .diagnostics()
                .iter()
                .find(|diagnostic| diagnostic.rule == rule)
                .unwrap();
            assert_eq!(
                diagnostic.location.map(|location| location.start()),
                Some(crate::diagnostic::SourcePosition::line(line)),
                "{}",
                rule.code()
            );
        }
    }

    #[test]
    fn indented_fence_lookalike_does_not_hide_xml_diagnostics() {
        let mut diag = DiagnosticCollector::new_all_enabled();
        check_markdown_structure(
            "f.md",
            "    ```bash\n<div>\n</span>\n</orphan>\n<section>\n",
            &mut diag,
        );
        let reported = codes(&diag);
        assert!(reported.iter().any(|code| code == "X003"));
        assert!(reported.iter().any(|code| code == "X004"));
        assert!(reported.iter().any(|code| code == "X005"));
        assert!(!reported.iter().any(|code| code == "X002"));
    }

    #[test]
    fn balanced_prompt_tags_ok() {
        let mut diag = DiagnosticCollector::new_all_enabled();
        check_markdown_structure("f.md", "<example>\nhello\n</example>\n", &mut diag);
        assert!(codes(&diag).is_empty());
    }

    #[test]
    fn xml_in_frontmatter_ignored() {
        let mut diag = DiagnosticCollector::new_all_enabled();
        check_markdown_structure(
            "f.md",
            "---\nname: foo\ndescription: use <Tool> here\n---\nHello\n",
            &mut diag,
        );
        assert!(
            codes(&diag).is_empty(),
            "angle brackets in YAML frontmatter must not fire XML rules: {:?}",
            codes(&diag)
        );
    }

    #[test]
    fn xml_in_body_after_frontmatter_reports() {
        let mut diag = DiagnosticCollector::new_all_enabled();
        check_markdown_structure("f.md", "---\nname: foo\n---\n<div>\n", &mut diag);
        assert!(
            codes(&diag).iter().any(|c| c == "X003"),
            "expected X003 on body tag: {:?}",
            codes(&diag)
        );
    }
}
