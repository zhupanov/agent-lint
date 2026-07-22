//! Markdown structure checks shared across SKILL.md, agent .md, and CLAUDE.md.
//!
//! - X002: unclosed code fence
//! - X003/X004/X005: XML tag balance (fence-aware, skips inline code)

use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata};
use crate::markdown::MarkdownDocument;
use crate::rules::LintRule;

/// HTML void elements that never take a closing tag.
const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct XmlTag {
    name: String,
    line: usize,
    is_close: bool,
    self_closing: bool,
}

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
    let mut stack: Vec<(String, usize)> = Vec::new();

    for tag in scan_xml_tags(document) {
        let line_no = tag.line;
        if tag.self_closing {
            continue;
        }
        let lower = tag.name.to_ascii_lowercase();
        if VOID_ELEMENTS.contains(&lower.as_str()) {
            continue;
        }

        if tag.is_close {
            match stack.last() {
                Some((open_name, _)) if open_name.eq_ignore_ascii_case(&tag.name) => {
                    stack.pop();
                }
                Some((open_name, open_line)) => {
                    diag.report_at_with(
                        LintRule::XmlTagMismatched,
                        path,
                        &format!(
                            "{path}:{line_no}: mismatched closing tag '</{}>' (open '<{open_name}>' at line {open_line})",
                            tag.name
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
                            "{path}:{line_no}: closing tag '</{}>' has no opening tag",
                            tag.name
                        ),
                        DiagnosticMetadata::at_line(line_no),
                    );
                }
            }
        } else {
            stack.push((tag.name, line_no));
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

/// Scan structural prose into XML-like tag tokens. This intentionally accepts
/// only a letter-led tag name, carries one candidate across physical lines,
/// and discards unfinished candidates rather than fabricating tokens.
fn scan_xml_tags(document: &MarkdownDocument) -> Vec<XmlTag> {
    let mut tags = Vec::new();
    let mut pending: Option<PendingTag> = None;
    let mut previous_line = None;
    for prose in document.structural_prose() {
        if previous_line.is_some_and(|line| line + 1 != prose.line) {
            pending = None;
        }
        scan_xml_line(&prose.text, prose.line, &mut pending, &mut tags);
        previous_line = Some(prose.line);
    }
    tags
}

#[derive(Debug, Clone)]
struct PendingTag {
    name: String,
    line: usize,
    is_close: bool,
    quote: Option<char>,
    last_non_whitespace: char,
}

fn scan_xml_line(
    line: &str,
    line_no: usize,
    pending: &mut Option<PendingTag>,
    tags: &mut Vec<XmlTag>,
) {
    let chars: Vec<char> = line.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        if let Some(tag) = pending.as_mut() {
            let ch = chars[index];
            if let Some(quote) = tag.quote {
                if ch == quote {
                    tag.quote = None;
                }
                index += 1;
                continue;
            }
            match ch {
                '\'' | '"' => {
                    tag.quote = Some(ch);
                    index += 1;
                }
                '>' => {
                    let tag = pending.take().expect("pending tag exists");
                    tags.push(XmlTag {
                        self_closing: tag.last_non_whitespace == '/',
                        name: tag.name,
                        line: tag.line,
                        is_close: tag.is_close,
                    });
                    index += 1;
                }
                '<' => {
                    *pending = None;
                }
                ch => {
                    if !ch.is_whitespace() {
                        tag.last_non_whitespace = ch;
                    }
                    index += 1;
                }
            }
            continue;
        }

        if chars[index] != '<' || escaped_by_odd_backslashes(&chars, index) {
            index += 1;
            continue;
        }
        let start = index;
        index += 1;
        let is_close = chars.get(index) == Some(&'/');
        if is_close {
            index += 1;
        }
        let name_start = index;
        if !chars.get(index).is_some_and(|ch| ch.is_ascii_alphabetic()) {
            index = start + 1;
            continue;
        }
        index += 1;
        while chars
            .get(index)
            .is_some_and(|ch| ch.is_ascii_alphanumeric() || matches!(ch, ':' | '_' | '-'))
        {
            index += 1;
        }
        let name: String = chars[name_start..index].iter().collect();
        match chars.get(index) {
            Some('>') => {
                tags.push(XmlTag {
                    name,
                    line: line_no,
                    is_close,
                    self_closing: false,
                });
                index += 1;
            }
            Some(ch) if *ch == '/' || ch.is_whitespace() => {
                *pending = Some(PendingTag {
                    last_non_whitespace: *chars.get(index).unwrap_or(&' '),
                    name,
                    line: line_no,
                    is_close,
                    quote: None,
                });
                index += 1;
            }
            None => {
                *pending = Some(PendingTag {
                    last_non_whitespace: ' ',
                    name,
                    line: line_no,
                    is_close,
                    quote: None,
                });
            }
            _ => index = start + 1,
        }
    }
}

fn escaped_by_odd_backslashes(chars: &[char], index: usize) -> bool {
    let count = chars[..index]
        .iter()
        .rev()
        .take_while(|ch| **ch == '\\')
        .count();
    count % 2 == 1
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
    fn xml_placeholders_in_list_indented_fence_are_ignored() {
        let mut diag = DiagnosticCollector::new_all_enabled();
        check_markdown_structure(
            "f.md",
            "- Result envelope:\n    ```text\n    <token> <N> <repo-relative-path>\n    <mergedAt-or-closedAt-ISO>\n    ```\n\n<live-tag>\n",
            &mut diag,
        );

        let reported = diag.diagnostics();
        assert_eq!(codes(&diag), vec!["X003"]);
        assert_eq!(
            reported[0].location.map(|location| location.start()),
            Some(crate::diagnostic::SourcePosition::line(7))
        );
    }

    #[test]
    fn xml_in_inline_code_ignored() {
        let mut diag = DiagnosticCollector::new_all_enabled();
        check_markdown_structure("f.md", "Use `<div>` as an example.\n", &mut diag);
        assert!(codes(&diag).is_empty());
    }

    #[test]
    fn commonmark_boundaries_preserve_real_multiline_tags() {
        let mut diag = DiagnosticCollector::new_all_enabled();
        check_markdown_structure(
            "f.md",
            "```lang`invalid\n<after-invalid-info>\n\n`` `<inline-literal>` ``\n\\<escaped-literal>\n<example\n  kind=\">\">\n</example>\n",
            &mut diag,
        );
        let reported = diag.diagnostics();
        assert_eq!(codes(&diag), vec!["X003"]);
        assert_eq!(
            reported[0].location.map(|location| location.start()),
            Some(crate::diagnostic::SourcePosition::line(2))
        );
    }

    #[test]
    fn xml_scanner_handles_delimiters_escapes_and_unterminated_syntax() {
        let mut diag = DiagnosticCollector::new_all_enabled();
        check_markdown_structure(
            "f.md",
            "`<one>`\n``<two ` inside>``\n```<three `` inside>```\n``<multiline\nliteral>``\n\\<escaped>\n\\\\<active>\n</active>\n<root\n note=\"a < b > c\">\n<child\n />\n</root\n>\n<unterminated\n",
            &mut diag,
        );
        assert!(codes(&diag).is_empty(), "{:?}", diag.diagnostics());

        let mut unmatched = DiagnosticCollector::new_all_enabled();
        check_markdown_structure("f.md", "`<literal>\n", &mut unmatched);
        assert_eq!(codes(&unmatched), vec!["X003"]);
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
