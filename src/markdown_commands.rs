//! Source-positioned Markdown command surfaces and a conservative shell lexer.
//!
//! Validators own domain grammar (for example npm `run` recognition). This
//! module only extracts actionable command text from Markdown and tokenizes it
//! without executing or expanding shell syntax.
//!
//! # Extraction boundaries
//!
//! - **Inline code:** every inline-code node in the document body.
//! - **Shell fences:** first info word is `bash`, `sh`, `shell`, `zsh`, or
//!   `console`. For `console`, one leading `$ ` or `> ` prompt is stripped per
//!   logical line. Non-shell fences are hard negatives.
//! - **Prose:** live body prose outside frontmatter, fenced/indented code, block
//!   quotes, links, masked quotes, and identifiable example scopes. A bare
//!   `npm` token is actionable only after line start, whitespace, or opening
//!   punctuation `([{<'\":;,`. Same-clause prose negation (`do not`, `don't`,
//!   `never`, `avoid`, `must not`) skips the match; prefer a false negative when
//!   ambiguous. Fenced and inline commands remain operative regardless of
//!   surrounding prose.
//!
//! # Tokenization boundaries
//!
//! Respects single/double quotes, backslash-newline continuation, `;`, `&&`,
//! `||`, and newlines. Substitutions (`$()`, `${}`, backticks) and heredocs
//! (`<<`) cause the whole fragment to be skipped rather than guessed.
//! Malformed/unclosed quotes are likewise skipped.

use crate::live_instructions;
use crate::markdown::MarkdownDocument;
use std::ops::Range;

/// Where a command fragment was recovered from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandSurface {
    InlineCode,
    ShellFence { console: bool },
    Prose,
}

/// One actionable command fragment with a contiguous logical command text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandFragment {
    pub surface: CommandSurface,
    /// Command text fed to the shell tokenizer (console prompts already stripped).
    pub text: String,
    /// Source byte offset for each byte in `text` (same length as `text`).
    pub source_offsets: Vec<usize>,
}

impl CommandFragment {
    fn source_range(&self) -> Range<usize> {
        match (self.source_offsets.first(), self.source_offsets.last()) {
            (Some(&start), Some(&last)) => start..last + 1,
            _ => 0..0,
        }
    }

    fn token_range(&self, start: usize, end: usize) -> Range<usize> {
        if self.source_offsets.is_empty() || start >= end || end > self.source_offsets.len() {
            return self.source_range();
        }
        let first = self.source_offsets[start];
        let last = self.source_offsets[end - 1];
        first..last + 1
    }
}

/// A shell token with its byte range inside the original Markdown source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellToken {
    pub text: String,
    pub source_range: Range<usize>,
}

/// Extract actionable command fragments from a Markdown document.
pub fn command_fragments(document: &MarkdownDocument) -> Vec<CommandFragment> {
    let content = document.content();
    let line_starts = line_start_offsets(content);
    let mut fragments = Vec::new();

    for code in document.inline_code() {
        if code.start_line < document.body_start_line() {
            continue;
        }
        let Some(span) = span_byte_range(
            content,
            &line_starts,
            code.start_line,
            code.start_column,
            code.end_line,
            code.end_column,
        ) else {
            continue;
        };
        let Some(text_range) = locate_subslice(content, span, &code.literal) else {
            continue;
        };
        fragments.push(CommandFragment {
            surface: CommandSurface::InlineCode,
            text: code.literal.clone(),
            source_offsets: (text_range.start..text_range.end).collect(),
        });
    }

    for fence in document.fences() {
        let Some(kind) = shell_fence_kind(&fence.info) else {
            continue;
        };
        let mut text = String::new();
        let mut source_offsets = Vec::new();
        for (line_number, line) in &fence.body {
            let (line_text, column_offset) = match kind {
                ShellFenceKind::Console => strip_console_prompt(line),
                ShellFenceKind::Shell => (line.clone(), 0),
            };
            let Some(line_start) = line_starts.get(line_number.saturating_sub(1)).copied() else {
                continue;
            };
            let source_line = content
                .lines()
                .nth(line_number.saturating_sub(1))
                .unwrap_or("");
            let prefix_bytes = source_line
                .chars()
                .take(column_offset)
                .map(|ch| ch.len_utf8())
                .sum::<usize>();
            let chunk_start = line_start + prefix_bytes;
            if !text.is_empty() {
                // Preserve a newline so the tokenizer can honor line boundaries
                // and backslash continuations across fence lines.
                let newline_at = line_start.saturating_sub(1);
                text.push('\n');
                source_offsets.push(newline_at);
            }
            append_mapped(&mut text, &mut source_offsets, chunk_start, &line_text);
        }
        if text.trim().is_empty() {
            continue;
        }
        fragments.push(CommandFragment {
            surface: CommandSurface::ShellFence {
                console: matches!(kind, ShellFenceKind::Console),
            },
            text,
            source_offsets,
        });
    }

    let example_scopes = live_instructions::example_scopes_for(document);
    for (index, line) in document.body_prose().iter().enumerate() {
        if example_scopes.get(index).copied().unwrap_or(false) {
            continue;
        }
        let Some(line_start) = line_starts.get(line.line.saturating_sub(1)).copied() else {
            continue;
        };
        for npm_char in prose_npm_char_indices(&line.text) {
            let column = npm_char + 1;
            if inside_link(document, line.line, column) {
                continue;
            }
            let byte_in_line: usize = line
                .text
                .chars()
                .take(npm_char)
                .map(|ch| ch.len_utf8())
                .sum();
            if prose_command_negated(&line.text, byte_in_line) {
                continue;
            }
            let rest = &line.text[byte_in_line..];
            let byte_start = line_start + byte_in_line;
            fragments.push(CommandFragment {
                surface: CommandSurface::Prose,
                text: rest.to_string(),
                source_offsets: (byte_start..byte_start + rest.len()).collect(),
            });
        }
    }

    fragments.sort_by_key(|fragment| fragment.source_range().start);
    fragments
}

/// Tokenize shell/console command text without executing or expanding it.
///
/// Returns `None` when the fragment contains substitutions, heredocs, or
/// malformed quotes that must not be guessed.
pub fn tokenize_shell_commands(
    _content: &str,
    fragment: &CommandFragment,
) -> Option<Vec<Vec<ShellToken>>> {
    if !fragment_is_safe(&fragment.text) {
        return None;
    }

    let mut commands = Vec::new();
    let mut current = Vec::new();
    let bytes = fragment.text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let ch = bytes[index];
        if ch == b' ' || ch == b'\t' || ch == b'\r' {
            index += 1;
            continue;
        }
        if ch == b'\n' {
            if !current.is_empty() {
                commands.push(std::mem::take(&mut current));
            }
            index += 1;
            continue;
        }
        if ch == b'\\' && index + 1 < bytes.len() && matches!(bytes[index + 1], b'\n' | b'\r') {
            index += 1;
            if index < bytes.len() && bytes[index] == b'\r' {
                index += 1;
            }
            if index < bytes.len() && bytes[index] == b'\n' {
                index += 1;
            }
            continue;
        }
        if ch == b';' {
            if !current.is_empty() {
                commands.push(std::mem::take(&mut current));
            }
            index += 1;
            continue;
        }
        if (ch == b'&' || ch == b'|') && index + 1 < bytes.len() && bytes[index + 1] == ch {
            if !current.is_empty() {
                commands.push(std::mem::take(&mut current));
            }
            index += 2;
            continue;
        }

        let start = index;
        if ch == b'\'' || ch == b'"' {
            let quote = ch;
            index += 1;
            let mut closed = false;
            while index < bytes.len() {
                if bytes[index] == b'\\' && quote == b'"' && index + 1 < bytes.len() {
                    index += 2;
                    continue;
                }
                if bytes[index] == quote {
                    index += 1;
                    closed = true;
                    break;
                }
                index += 1;
            }
            if !closed {
                return None;
            }
        } else {
            while index < bytes.len() {
                let c = bytes[index];
                if c.is_ascii_whitespace()
                    || c == b';'
                    || ((c == b'&' || c == b'|')
                        && index + 1 < bytes.len()
                        && bytes[index + 1] == c)
                    || (c == b'\\'
                        && index + 1 < bytes.len()
                        && matches!(bytes[index + 1], b'\n' | b'\r'))
                {
                    break;
                }
                index += 1;
            }
        }

        if start == index {
            return None;
        }
        current.push(ShellToken {
            text: fragment.text[start..index].to_string(),
            source_range: fragment.token_range(start, index),
        });
    }

    if !current.is_empty() {
        commands.push(current);
    }
    Some(commands)
}

/// Whether prose immediately before an `npm` match is directly negated.
///
/// Only same-clause cues on the same line count. Prefer a false negative when
/// ambiguous at warning severity.
pub fn prose_command_negated(line: &str, npm_byte_index: usize) -> bool {
    let before = line.get(..npm_byte_index).unwrap_or("");
    let clause_break = before
        .char_indices()
        .rev()
        .find_map(|(index, ch)| {
            matches!(ch, '.' | '!' | '?' | ';').then_some(index + ch.len_utf8())
        })
        .unwrap_or(0);
    let clause = before[clause_break..].to_ascii_lowercase();
    ["must not", "do not", "don't", "never", "avoid"]
        .iter()
        .any(|phrase| clause.contains(phrase))
}

#[derive(Clone, Copy)]
enum ShellFenceKind {
    Shell,
    Console,
}

fn shell_fence_kind(info: &str) -> Option<ShellFenceKind> {
    match info
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "bash" | "sh" | "shell" | "zsh" => Some(ShellFenceKind::Shell),
        "console" => Some(ShellFenceKind::Console),
        _ => None,
    }
}

fn strip_console_prompt(line: &str) -> (String, usize) {
    for prompt in ["$ ", "> "] {
        if let Some(rest) = line.strip_prefix(prompt) {
            return (rest.to_string(), prompt.chars().count());
        }
    }
    (line.to_string(), 0)
}

fn fragment_is_safe(text: &str) -> bool {
    if text.contains("<<") {
        return false;
    }
    let bytes = text.as_bytes();
    let mut index = 0;
    let mut in_single = false;
    let mut in_double = false;
    while index < bytes.len() {
        let ch = bytes[index];
        if in_single {
            if ch == b'\'' {
                in_single = false;
            }
            index += 1;
            continue;
        }
        if in_double {
            if ch == b'\\' && index + 1 < bytes.len() {
                index += 2;
                continue;
            }
            if ch == b'"' {
                in_double = false;
            }
            index += 1;
            continue;
        }
        match ch {
            b'\'' => in_single = true,
            b'"' => in_double = true,
            b'`' => return false,
            b'$' if index + 1 < bytes.len() && matches!(bytes[index + 1], b'(' | b'{') => {
                return false;
            }
            _ => {}
        }
        index += 1;
    }
    !in_single && !in_double
}

fn prose_npm_char_indices(text: &str) -> Vec<usize> {
    let chars: Vec<char> = text.chars().collect();
    let mut starts = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if npm_at(&chars, index) {
            let before = index.checked_sub(1).map(|i| chars[i]);
            if is_command_boundary(before) {
                starts.push(index);
            }
            index += 3;
            continue;
        }
        index += 1;
    }
    starts
}

fn npm_at(chars: &[char], index: usize) -> bool {
    chars.get(index..index + 3) == Some(&['n', 'p', 'm'])
        && chars
            .get(index + 3)
            .is_none_or(|ch| ch.is_whitespace() || "([{<'\":;,".contains(*ch) || *ch == '-')
}

fn is_command_boundary(before: Option<char>) -> bool {
    match before {
        None => true,
        Some(ch) if ch.is_whitespace() => true,
        Some(ch) if "([{<'\":;,".contains(ch) => true,
        _ => false,
    }
}

fn inside_link(document: &MarkdownDocument, line: usize, column: usize) -> bool {
    document.links().iter().any(|link| {
        (line, column) >= (link.line, link.start_column)
            && (line, column) <= (link.end_line, link.end_column)
    })
}

fn line_start_offsets(content: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (index, ch) in content.char_indices() {
        if ch == '\n' {
            starts.push(index + 1);
        }
    }
    starts
}

fn span_byte_range(
    content: &str,
    line_starts: &[usize],
    start_line: usize,
    start_column: usize,
    end_line: usize,
    end_column: usize,
) -> Option<Range<usize>> {
    let start = offset_at(content, line_starts, start_line, start_column)?;
    let end_char = offset_at(content, line_starts, end_line, end_column)?;
    let ch = content[end_char..].chars().next()?;
    let end = end_char + ch.len_utf8();
    if start > end || end > content.len() {
        return None;
    }
    Some(start..end)
}

fn offset_at(content: &str, line_starts: &[usize], line: usize, column: usize) -> Option<usize> {
    if column == 0 || line == 0 {
        return None;
    }
    let line_start = *line_starts.get(line.saturating_sub(1))?;
    let line_text = content.get(line_start..)?.split('\n').next()?;
    let mut cols = 1usize;
    for (byte_offset, _ch) in line_text.char_indices() {
        if cols == column {
            return Some(line_start + byte_offset);
        }
        cols += 1;
    }
    if cols == column {
        Some(line_start + line_text.len())
    } else {
        None
    }
}

fn append_mapped(text: &mut String, offsets: &mut Vec<usize>, source_start: usize, chunk: &str) {
    for (index, _) in chunk.as_bytes().iter().enumerate() {
        offsets.push(source_start + index);
    }
    text.push_str(chunk);
}

fn locate_subslice(content: &str, range: Range<usize>, needle: &str) -> Option<Range<usize>> {
    let slice = content.get(range.clone())?;
    let rel = slice.find(needle)?;
    let start = range.start + rel;
    Some(start..start + needle.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::MarkdownDocument;

    #[test]
    fn extracts_inline_shell_fence_console_and_prose() {
        let source = "\
# Title

Run `npm run inline-one`.

```bash
npm run fenced-bash
```

```json
{\"scripts\":{\"ignore\":\"x\"}}
```

```console
$ npm run console-one
> npm run console-two
```

Do not run npm run negated.

Please run npm run prose-one now.

## Examples

npm run example-only
";
        let doc = MarkdownDocument::parse(source);
        let fragments = command_fragments(&doc);
        let texts: Vec<_> = fragments.iter().map(|f| f.text.as_str()).collect();
        assert!(texts.iter().any(|t| t.contains("inline-one")), "{texts:?}");
        assert!(texts.contains(&"npm run fenced-bash"), "{texts:?}");
        assert!(
            texts
                .iter()
                .any(|t| t.contains("console-one") && t.contains("console-two")),
            "{texts:?}"
        );
        assert!(
            texts.iter().any(|t| t.starts_with("npm run prose-one")),
            "{texts:?}"
        );
        assert!(!texts.iter().any(|t| t.contains("ignore")), "{texts:?}");
        assert!(
            !texts.iter().any(|t| t.contains("example-only")),
            "{texts:?}"
        );
        assert!(!texts.iter().any(|t| t.contains("negated")), "{texts:?}");
    }

    #[test]
    fn fence_backslash_continuation_keeps_one_logical_command() {
        let source = "```bash\nnpm run continued \\\n  --flag\n```\n";
        let doc = MarkdownDocument::parse(source);
        let fragment = &command_fragments(&doc)[0];
        let commands = tokenize_shell_commands(doc.content(), fragment).unwrap();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0][0].text, "npm");
        assert_eq!(commands[0][1].text, "run");
        assert_eq!(commands[0][2].text, "continued");
        assert_eq!(commands[0][3].text, "--flag");
    }

    #[test]
    fn tokenizer_respects_quotes_separators_and_skips_substitutions() {
        let doc = MarkdownDocument::parse("`npm run a && npm run b; npm run 'c:d'`\n");
        let fragment = &command_fragments(&doc)[0];
        let commands = tokenize_shell_commands(doc.content(), fragment).unwrap();
        assert_eq!(commands.len(), 3);
        assert_eq!(commands[0][2].text, "a");
        assert_eq!(commands[1][2].text, "b");
        assert_eq!(commands[2][2].text, "'c:d'");

        let bad = MarkdownDocument::parse("`npm run $(echo x)`\n");
        let bad_fragment = &command_fragments(&bad)[0];
        assert!(tokenize_shell_commands(bad.content(), bad_fragment).is_none());

        let heredoc = MarkdownDocument::parse("`npm run x <<EOF`\n");
        let heredoc_fragment = &command_fragments(&heredoc)[0];
        assert!(tokenize_shell_commands(heredoc.content(), heredoc_fragment).is_none());
    }

    #[test]
    fn prose_negation_is_clause_local() {
        assert!(prose_command_negated("Do not run npm run build", 11));
        assert!(!prose_command_negated(
            "Never invent. Run npm run build",
            18
        ));
    }

    #[test]
    fn unicode_and_crlf_keep_stable_locations() {
        let source = "Run `npm run café`\r\n";
        let doc = MarkdownDocument::parse(source);
        let fragment = &command_fragments(&doc)[0];
        assert_eq!(fragment.text, "npm run café");
        let commands = tokenize_shell_commands(doc.content(), fragment).unwrap();
        assert_eq!(commands[0][2].text, "café");
        assert_eq!(&doc.content()[commands[0][2].source_range.clone()], "café");
    }
}
