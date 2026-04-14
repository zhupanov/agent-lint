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
    fn test_multiple_fences() {
        let text = "a\n```\nb\n```\nc\n~~~\nd\n~~~\ne";
        let outside: Vec<&str> = lines_outside_fences(text).collect();
        let inside: Vec<&str> = lines_inside_fences(text).collect();
        assert_eq!(outside, vec!["a", "c", "e"]);
        assert_eq!(inside, vec!["b", "d"]);
    }
}
