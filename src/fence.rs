//! Code fence tracking for Markdown documents.
//!
//! Properly handles opening/closing fences with backtick/tilde counts per CommonMark spec:
//! - A fence opens with 3+ consecutive backticks or tildes at the start of a line
//! - A fence closes only when the same character appears with >= the opening count
//! - Backtick fences cannot be closed by tilde fences and vice versa

/// Classification of a line after fence-state processing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineClass {
    /// Line is outside any code fence (prose content).
    Outside,
    /// Line is inside a code fence (code content).
    Inside,
    /// Line is a fence delimiter (opening or closing fence marker).
    Delimiter,
}

/// A balanced fenced code block with source line metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownFence {
    pub info: String,
    pub start_line: usize,
    pub end_line: usize,
    pub body: Vec<(usize, String)>,
}

const BREADCRUMB_MAX_CHARS: usize = 160;
const BREADCRUMB_MAX_LINE_CHARS: usize = 100;
const BREADCRUMB_MAX_LINES: usize = 2;

/// Extract balanced CommonMark-style backtick and tilde fences.
///
/// Openers may be indented by at most three spaces. A closer must use the
/// same marker and at least the opener's marker count. Unclosed openers are
/// skipped so they cannot hide later valid fences.
pub fn markdown_fences(content: &str) -> Vec<MarkdownFence> {
    let lines: Vec<&str> = content.lines().collect();
    let mut result = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let trimmed = lines[index].trim_start();
        let Some(marker) = trimmed.chars().next().filter(|ch| *ch == '`' || *ch == '~') else {
            index += 1;
            continue;
        };
        let count = trimmed.chars().take_while(|ch| *ch == marker).count();
        if count < 3 || lines[index].len() - trimmed.len() > 3 {
            index += 1;
            continue;
        }
        let info = trimmed[count..].trim().to_string();
        let mut cursor = index + 1;
        let mut body = Vec::new();
        let mut closed = false;
        while cursor < lines.len() {
            let close = lines[cursor].trim();
            let close_indent = lines[cursor].len() - lines[cursor].trim_start().len();
            let close_count = close.chars().take_while(|ch| *ch == marker).count();
            if close_indent <= 3 && close_count >= count && close.chars().all(|ch| ch == marker) {
                result.push(MarkdownFence {
                    info,
                    start_line: index + 1,
                    end_line: cursor + 1,
                    body,
                });
                index = cursor + 1;
                closed = true;
                break;
            }
            body.push((cursor + 1, lines[cursor].to_string()));
            cursor += 1;
        }
        if !closed {
            index += 1;
        }
    }
    result
}

/// Return adjacent shell-fence pairs that should be combined.
///
/// This is the shared policy host for S021 across SKILL.md bodies and skill
/// reference files. Short prose breadcrumbs and HTML comments do not create a
/// tool boundary. Reason-bearing pragmas and the documented example/driver
/// carve-outs do.
pub fn consecutive_bash_pairs(content: &str) -> Vec<(usize, usize)> {
    let fences: Vec<_> = markdown_fences(content)
        .into_iter()
        .filter(is_shell_fence)
        .collect();
    let lines: Vec<&str> = content.lines().collect();
    fences
        .windows(2)
        .filter_map(|pair| {
            let first = &pair[0];
            let second = &pair[1];
            let gap = &lines[first.end_line..second.start_line - 1];
            (gap_is_adjacent(gap)
                && !fence_has_suppression(first)
                && !fence_has_suppression(second)
                && !is_carved_out_pair(content, first, second, gap))
            .then_some((first.start_line, second.start_line))
        })
        .collect()
}

fn is_shell_fence(fence: &MarkdownFence) -> bool {
    fence
        .info
        .split_whitespace()
        .next()
        .is_some_and(|language| {
            matches!(
                language.to_ascii_lowercase().as_str(),
                "bash" | "sh" | "shell"
            )
        })
}

fn gap_is_adjacent(lines: &[&str]) -> bool {
    let mut visible = Vec::new();
    let mut in_comment = false;
    for line in lines {
        let trimmed = line.trim();
        if in_comment {
            if trimmed.ends_with("-->") {
                in_comment = false;
            }
            continue;
        }
        if trimmed.starts_with("<!--") {
            in_comment = !trimmed.ends_with("-->");
            continue;
        }
        if !trimmed.is_empty() {
            visible.push(trimmed);
        }
    }
    visible.len() <= BREADCRUMB_MAX_LINES
        && visible.iter().map(|line| line.len()).sum::<usize>() <= BREADCRUMB_MAX_CHARS
        && visible.iter().all(|line| {
            line.len() <= BREADCRUMB_MAX_LINE_CHARS
                && !line.starts_with('#')
                && !line.starts_with(['-', '*', '+', '>', '|'])
                && !line.chars().next().is_some_and(|ch| ch.is_ascii_digit())
        })
}

fn fence_has_suppression(fence: &MarkdownFence) -> bool {
    let nonblank: Vec<_> = fence
        .body
        .iter()
        .filter(|(_, line)| !line.trim().is_empty())
        .collect();
    nonblank.iter().any(|(_, line)| {
        const MARKER: &str = "lint-consecutive-bash: ok";
        line.find(MARKER).is_some_and(|index| {
            let has_reason = !line[index + MARKER.len()..].trim().is_empty();
            let before = line[..index].trim();
            let standalone = line.trim_start().starts_with("# lint-consecutive-bash: ok");
            has_reason
                && ((standalone && nonblank.len() > 1)
                    || (!before.is_empty() && !before.starts_with('#')))
        })
    })
}

fn is_carved_out_pair(
    content: &str,
    first: &MarkdownFence,
    second: &MarkdownFence,
    gap: &[&str],
) -> bool {
    let lines: Vec<&str> = content.lines().collect();
    let preceding = &lines[first.start_line.saturating_sub(4)..first.start_line - 1];
    let full_text = [
        preceding.join("\n"),
        first.info.clone(),
        fence_body(first),
        gap.join("\n"),
        second.info.clone(),
        fence_body(second),
    ]
    .join("\n");
    let lower = full_text.to_ascii_lowercase();
    if lower
        .split(|ch: char| !ch.is_alphanumeric())
        .any(|word| word == "wrong")
        && lower
            .split(|ch: char| !ch.is_alphanumeric())
            .any(|word| word == "correct")
    {
        return true;
    }

    let pair_text = [fence_body(first), gap.join("\n"), fence_body(second)].join("\n");
    let pair_lower = pair_text.to_ascii_lowercase();
    let design_context = pair_lower.contains("/design")
        || pair_lower.contains(" design ")
        || pair_lower.contains("design driver")
        || pair_lower.contains("design-step");
    design_context
        && [
            "pause",
            "resume",
            "design-step",
            "design_action",
            "skills/design/scripts",
        ]
        .iter()
        .any(|marker| pair_lower.contains(marker))
}

fn fence_body(fence: &MarkdownFence) -> String {
    fence
        .body
        .iter()
        .map(|(_, line)| line.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Return the 1-based line number of an unclosed fence opener, if any.
pub fn find_unclosed_fence_line(content: &str) -> Option<usize> {
    let mut tracker = CodeFenceTracker::new();
    let mut opener: Option<usize> = None;
    for (idx, line) in content.lines().enumerate() {
        let was_inside = tracker.in_fence();
        let class = tracker.process_line(line);
        if class == LineClass::Delimiter && tracker.in_fence() && !was_inside {
            opener = Some(idx + 1);
        } else if class == LineClass::Delimiter && !tracker.in_fence() {
            opener = None;
        }
    }
    if tracker.in_fence() { opener } else { None }
}

/// Tracks code fence state while iterating over lines.
pub struct CodeFenceTracker {
    fence_char: Option<char>,
    fence_len: usize,
}

impl CodeFenceTracker {
    pub fn new() -> Self {
        Self {
            fence_char: None,
            fence_len: 0,
        }
    }

    /// Returns whether the tracker is currently inside a code fence.
    pub fn in_fence(&self) -> bool {
        self.fence_char.is_some()
    }

    /// Process a line and classify it. Handles leading whitespace internally.
    pub fn process_line(&mut self, line: &str) -> LineClass {
        let trimmed = line.trim_start();

        if let Some(fc) = self.fence_char {
            // Currently inside a fence — check for closing
            if let Some((ch, count)) = fence_start(trimmed) {
                if ch == fc
                    && count >= self.fence_len
                    && is_only_whitespace_after(trimmed, ch, count)
                {
                    // Closing fence
                    self.fence_char = None;
                    self.fence_len = 0;
                    return LineClass::Delimiter;
                }
            }
            LineClass::Inside
        } else {
            // Not inside a fence — check for opening
            if let Some((ch, count)) = fence_start(trimmed) {
                self.fence_char = Some(ch);
                self.fence_len = count;
                return LineClass::Delimiter;
            }
            LineClass::Outside
        }
    }
}

/// Returns an iterator over lines that are outside code fences.
/// Fence delimiter lines are excluded.
pub fn lines_outside_fences(text: &str) -> impl Iterator<Item = &str> {
    let mut tracker = CodeFenceTracker::new();
    text.lines()
        .filter(move |line| tracker.process_line(line) == LineClass::Outside)
}

/// Returns an iterator over lines that are inside code fences.
/// Fence delimiter lines are excluded.
pub fn lines_inside_fences(text: &str) -> impl Iterator<Item = &str> {
    let mut tracker = CodeFenceTracker::new();
    text.lines()
        .filter(move |line| tracker.process_line(line) == LineClass::Inside)
}

/// Check if a trimmed line starts with 3+ backticks or tildes.
/// Returns the fence character and its count.
fn fence_start(trimmed: &str) -> Option<(char, usize)> {
    let first = trimmed.chars().next()?;
    if first != '`' && first != '~' {
        return None;
    }
    let count = trimmed.chars().take_while(|&c| c == first).count();
    if count >= 3 {
        Some((first, count))
    } else {
        None
    }
}

/// Check if the rest of the line after the fence chars is only whitespace.
/// This is required for closing fences (closers cannot have info strings).
fn is_only_whitespace_after(trimmed: &str, ch: char, count: usize) -> bool {
    trimmed[ch.len_utf8() * count..].trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_fence() {
        let text = "before\n```\ncode\n```\nafter";
        let outside: Vec<&str> = lines_outside_fences(text).collect();
        let inside: Vec<&str> = lines_inside_fences(text).collect();
        assert_eq!(outside, vec!["before", "after"]);
        assert_eq!(inside, vec!["code"]);
    }

    #[test]
    fn test_nested_fence_4_backticks() {
        let text = "prose\n````\ninner ```\nstill inside\n````\nafter";
        let outside: Vec<&str> = lines_outside_fences(text).collect();
        let inside: Vec<&str> = lines_inside_fences(text).collect();
        assert_eq!(outside, vec!["prose", "after"]);
        assert_eq!(inside, vec!["inner ```", "still inside"]);
    }

    #[test]
    fn test_tilde_fence() {
        let text = "before\n~~~\ntilde code\n~~~\nafter";
        let outside: Vec<&str> = lines_outside_fences(text).collect();
        let inside: Vec<&str> = lines_inside_fences(text).collect();
        assert_eq!(outside, vec!["before", "after"]);
        assert_eq!(inside, vec!["tilde code"]);
    }

    #[test]
    fn test_mixed_fence_types_no_cross_close() {
        // Backtick fence cannot be closed by tildes
        let text = "before\n```\ncode\n~~~\nstill code\n```\nafter";
        let outside: Vec<&str> = lines_outside_fences(text).collect();
        let inside: Vec<&str> = lines_inside_fences(text).collect();
        assert_eq!(outside, vec!["before", "after"]);
        assert_eq!(inside, vec!["code", "~~~", "still code"]);
    }

    #[test]
    fn test_unclosed_fence() {
        let text = "before\n```\ncode\nmore code";
        let outside: Vec<&str> = lines_outside_fences(text).collect();
        let inside: Vec<&str> = lines_inside_fences(text).collect();
        assert_eq!(outside, vec!["before"]);
        assert_eq!(inside, vec!["code", "more code"]);
    }

    #[test]
    fn test_language_tag_on_opener() {
        let text = "before\n```python\nprint('hello')\n```\nafter";
        let outside: Vec<&str> = lines_outside_fences(text).collect();
        let inside: Vec<&str> = lines_inside_fences(text).collect();
        assert_eq!(outside, vec!["before", "after"]);
        assert_eq!(inside, vec!["print('hello')"]);
    }

    #[test]
    fn test_closing_fence_trailing_whitespace() {
        let text = "before\n```\ncode\n```   \nafter";
        let outside: Vec<&str> = lines_outside_fences(text).collect();
        assert_eq!(outside, vec!["before", "after"]);
    }

    #[test]
    fn test_closing_fence_with_info_string_does_not_close() {
        // A closing fence line with extra text is not a valid closer
        let text = "before\n```\ncode\n```notacloser\nmore code\n```\nafter";
        let outside: Vec<&str> = lines_outside_fences(text).collect();
        let inside: Vec<&str> = lines_inside_fences(text).collect();
        assert_eq!(outside, vec!["before", "after"]);
        assert_eq!(inside, vec!["code", "```notacloser", "more code"]);
    }

    #[test]
    fn test_leading_whitespace_handled() {
        let text = "before\n   ```\n  code\n   ```\nafter";
        let outside: Vec<&str> = lines_outside_fences(text).collect();
        assert_eq!(outside, vec!["before", "after"]);
    }

    #[test]
    fn test_longer_closing_fence() {
        // Closing fence can be longer than opener
        let text = "before\n```\ncode\n`````\nafter";
        let outside: Vec<&str> = lines_outside_fences(text).collect();
        assert_eq!(outside, vec!["before", "after"]);
    }

    #[test]
    fn test_shorter_closing_fence_does_not_close() {
        // 5-backtick opener cannot be closed by 3-backtick line
        let text = "before\n`````\ncode\n```\nstill code\n`````\nafter";
        let outside: Vec<&str> = lines_outside_fences(text).collect();
        let inside: Vec<&str> = lines_inside_fences(text).collect();
        assert_eq!(outside, vec!["before", "after"]);
        assert_eq!(inside, vec!["code", "```", "still code"]);
    }

    #[test]
    fn test_delimiter_lines_excluded_from_both() {
        let text = "a\n```\nb\n```\nc";
        let outside: Vec<&str> = lines_outside_fences(text).collect();
        let inside: Vec<&str> = lines_inside_fences(text).collect();
        // Delimiter lines (```) should not appear in either
        assert!(!outside.contains(&"```"));
        assert!(!inside.contains(&"```"));
        assert_eq!(outside, vec!["a", "c"]);
        assert_eq!(inside, vec!["b"]);
    }

    #[test]
    fn test_process_line_classification() {
        let mut tracker = CodeFenceTracker::new();
        assert_eq!(tracker.process_line("prose"), LineClass::Outside);
        assert_eq!(tracker.process_line("```bash"), LineClass::Delimiter);
        assert_eq!(tracker.process_line("echo hi"), LineClass::Inside);
        assert_eq!(tracker.process_line("```"), LineClass::Delimiter);
        assert_eq!(tracker.process_line("more prose"), LineClass::Outside);
    }

    #[test]
    fn test_in_fence_state() {
        let mut tracker = CodeFenceTracker::new();
        assert!(!tracker.in_fence());
        tracker.process_line("```");
        assert!(tracker.in_fence());
        tracker.process_line("code");
        assert!(tracker.in_fence());
        tracker.process_line("```");
        assert!(!tracker.in_fence());
    }

    #[test]
    fn find_unclosed_fence_line_reports_opener() {
        assert_eq!(find_unclosed_fence_line("a\n```\nb\n"), Some(2));
        assert_eq!(find_unclosed_fence_line("```\nb\n```\n"), None);
    }

    #[test]
    fn test_multiple_fences() {
        let text = "a\n```\nb\n```\nc\n~~~\nd\n~~~\ne";
        let outside: Vec<&str> = lines_outside_fences(text).collect();
        let inside: Vec<&str> = lines_inside_fences(text).collect();
        assert_eq!(outside, vec!["a", "c", "e"]);
        assert_eq!(inside, vec!["b", "d"]);
    }

    #[test]
    fn markdown_fences_preserve_info_body_and_lines() {
        let fences = markdown_fences("before\n````bash title\necho hi\n```\necho still\n````\n");
        assert_eq!(fences.len(), 1);
        assert_eq!(fences[0].info, "bash title");
        assert_eq!(fences[0].start_line, 2);
        assert_eq!(fences[0].end_line, 6);
        assert_eq!(fences[0].body.len(), 3);
    }

    #[test]
    fn markdown_fences_support_tildes_and_skip_unclosed_openers() {
        let fences = markdown_fences("```bash\nunclosed\n~~~sh\necho hi\n~~~\n");
        assert_eq!(fences.len(), 1);
        assert_eq!(fences[0].info, "sh");
        assert_eq!(fences[0].start_line, 3);
    }

    #[test]
    fn consecutive_bash_policy_honors_suppressions_and_carve_outs() {
        let ordinary = "```bash\necho one\n```\nThen continue:\n```bash\necho two\n```\n";
        assert_eq!(consecutive_bash_pairs(ordinary), [(1, 5)]);

        let suppressed = "```bash\n# lint-consecutive-bash: ok separate tool boundary\necho one\n```\n```bash\necho two\n```\n";
        assert!(consecutive_bash_pairs(suppressed).is_empty());

        let example = "WRONG:\n```bash\necho one\n```\nCORRECT:\n```bash\necho two\n```\n";
        assert!(consecutive_bash_pairs(example).is_empty());

        let design = "```bash\ndesign-step3-mav.sh --phase pre\n```\nBoundary: vote.\nThen resume:\n```bash\ndesign-step3-mav.sh --phase post\n```\n";
        assert!(consecutive_bash_pairs(design).is_empty());
    }

    #[test]
    fn markdown_fences_reject_over_indented_closer() {
        let fences = markdown_fences("```bash\necho one\n    ```\n```bash\necho two\n```\n");
        assert_eq!(fences.len(), 1);
        assert_eq!(fences[0].start_line, 1);
        assert_eq!(fences[0].end_line, 6);
        assert_eq!(fences[0].body[1].1, "    ```");
    }
}
