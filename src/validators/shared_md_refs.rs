//! Shared Markdown reference recognition for S008, S029, and S036.
//!
//! One scanner owns brace/brace-less `$CLAUDE_PLUGIN_ROOT` spellings, `.md`
//! token boundaries, and HTML-comment dormancy so the three consumers cannot
//! drift.

use crate::markdown::mask_html_comments;

/// One source-positioned shared Markdown reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SharedMdRef {
    /// Authored token, including `$CLAUDE_PLUGIN_ROOT/` or `${CLAUDE_PLUGIN_ROOT}/`.
    pub reference: String,
    /// Repository-relative path after stripping the plugin-root prefix.
    pub relative_path: String,
    /// One-based line of the first character of `reference`.
    pub line: usize,
}

/// Find shared Markdown references under `{base_dir}/shared/` in `content`.
///
/// HTML comments are masked with the shared Markdown comment masker before
/// scanning. Fenced and inline code are intentionally left intact.
pub(crate) fn find_shared_md_refs(content: &str, base_dir: &str) -> Vec<SharedMdRef> {
    let masked = mask_html_comments_document(content);
    let shared_prefix = format!("{base_dir}/shared/");
    let mut refs = Vec::new();
    let bytes = masked.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'$' {
            i += 1;
            continue;
        }
        if let Some(found) = match_shared_md_ref(&masked, i, &shared_prefix) {
            refs.push(SharedMdRef {
                reference: found.reference,
                relative_path: found.relative_path,
                line: line_at_offset(&masked, i),
            });
            i = found.end;
        } else {
            i += 1;
        }
    }
    refs
}

/// Whether `content` contains any live shared Markdown reference under
/// `{base_dir}/shared/`.
pub(crate) fn contains_shared_md_ref(content: &str, base_dir: &str) -> bool {
    !find_shared_md_refs(content, base_dir).is_empty()
}

struct MatchedRef {
    reference: String,
    relative_path: String,
    end: usize,
}

fn match_shared_md_ref(content: &str, start: usize, shared_prefix: &str) -> Option<MatchedRef> {
    let rest = &content[start..];
    let after_var = if let Some(stripped) = rest.strip_prefix("${CLAUDE_PLUGIN_ROOT}/") {
        stripped
    } else if let Some(stripped) = rest.strip_prefix("$CLAUDE_PLUGIN_ROOT/") {
        stripped
    } else {
        return None;
    };
    let var_len = rest.len() - after_var.len();
    if !after_var.starts_with(shared_prefix) {
        return None;
    }
    let path_start = start + var_len + shared_prefix.len();
    if path_start >= content.len() {
        return None;
    }

    let mut end = path_start;
    let mut best_end = None;
    while end < content.len() {
        let ch = content[end..].chars().next()?;
        if !is_shared_path_char(ch) {
            break;
        }
        end += ch.len_utf8();
        if content[path_start..end].ends_with(".md") && is_md_terminator(content, end) {
            // Require a non-empty path before `.md`.
            if end - path_start > ".md".len() {
                best_end = Some(end);
            }
        }
    }

    let end = best_end?;
    let reference = content[start..end].to_string();
    let relative_path = reference
        .strip_prefix("${CLAUDE_PLUGIN_ROOT}/")
        .or_else(|| reference.strip_prefix("$CLAUDE_PLUGIN_ROOT/"))
        .unwrap_or(&reference)
        .to_string();
    Some(MatchedRef {
        reference,
        relative_path,
        end,
    })
}

fn is_shared_path_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '/' | '-')
}

fn is_md_terminator(content: &str, pos: usize) -> bool {
    let Some(ch) = content[pos..].chars().next() else {
        return true;
    };
    if ch == '#' || ch == '?' {
        return true;
    }
    if ch.is_ascii_whitespace()
        || matches!(ch, '"' | '\'' | '`' | ')' | ']' | '}' | '>' | ',' | ';')
    {
        return true;
    }
    if ch == '.' || ch == ':' {
        let after = pos + ch.len_utf8();
        return content[after..]
            .chars()
            .next()
            .is_none_or(|next| next.is_ascii_whitespace());
    }
    false
}

fn line_at_offset(content: &str, offset: usize) -> usize {
    content[..offset].bytes().filter(|b| *b == b'\n').count() + 1
}

/// Mask HTML comments across a full document using the shared line-oriented
/// masker, preserving line breaks so source lines stay aligned.
fn mask_html_comments_document(content: &str) -> String {
    let mut in_comment = false;
    let mut out = String::with_capacity(content.len());
    for line in content.split_inclusive('\n') {
        let (body, ending) = if let Some(without_lf) = line.strip_suffix('\n') {
            if let Some(without_crlf) = without_lf.strip_suffix('\r') {
                (without_crlf, "\r\n")
            } else {
                (without_lf, "\n")
            }
        } else {
            (line, "")
        };
        out.push_str(&mask_html_comments(body, &mut in_comment));
        out.push_str(ending);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_braced_and_brace_less_forms() {
        let content = "\
See ${CLAUDE_PLUGIN_ROOT}/skills/shared/a.md and \
$CLAUDE_PLUGIN_ROOT/skills/shared/b.md.\n";
        let refs = find_shared_md_refs(content, "skills");
        assert_eq!(
            refs.iter()
                .map(|r| r.relative_path.as_str())
                .collect::<Vec<_>>(),
            ["skills/shared/a.md", "skills/shared/b.md"]
        );
    }

    #[test]
    fn rejects_prefix_extensions() {
        let content = "\
${CLAUDE_PLUGIN_ROOT}/skills/shared/prefix.md.backup
${CLAUDE_PLUGIN_ROOT}/skills/shared/prefix.mdx
${CLAUDE_PLUGIN_ROOT}/skills/shared/prefix.md/child
";
        assert!(find_shared_md_refs(content, "skills").is_empty());
    }

    #[test]
    fn accepts_delimiters_and_sentence_final_punctuation() {
        let content = "\
(${CLAUDE_PLUGIN_ROOT}/skills/shared/paren.md)
`${CLAUDE_PLUGIN_ROOT}/skills/shared/tick.md`
\"${CLAUDE_PLUGIN_ROOT}/skills/shared/quote.md\"
${CLAUDE_PLUGIN_ROOT}/skills/shared/hash.md#section
${CLAUDE_PLUGIN_ROOT}/skills/shared/query.md?mode=x
See ${CLAUDE_PLUGIN_ROOT}/skills/shared/sentence.md.
";
        let refs = find_shared_md_refs(content, "skills");
        assert_eq!(refs.len(), 6);
    }

    #[test]
    fn ignores_html_comment_tokens() {
        let content = "\
<!-- ${CLAUDE_PLUGIN_ROOT}/skills/shared/commented.md -->
<!--
${CLAUDE_PLUGIN_ROOT}/skills/shared/multiline.md
-->
<!-- x --> ${CLAUDE_PLUGIN_ROOT}/skills/shared/live.md <!-- y -->
<!-- ${CLAUDE_PLUGIN_ROOT}/skills/shared/unterminated.md
";
        let refs = find_shared_md_refs(content, "skills");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].relative_path, "skills/shared/live.md");
        assert_eq!(refs[0].line, 5);
    }

    #[test]
    fn respects_base_dir() {
        let content = "${CLAUDE_PLUGIN_ROOT}/.claude/skills/shared/helpers.md\n";
        assert_eq!(find_shared_md_refs(content, "skills").len(), 0);
        assert_eq!(find_shared_md_refs(content, ".claude/skills").len(), 1);
    }
}
