/// Extract YAML frontmatter lines from a file's content.
/// The file must start with `---` on line 1 and have a closing `---`.
/// Returns None if the file is malformed.
pub fn extract_frontmatter(content: &str) -> Option<Vec<String>> {
    let mut lines = content.lines();

    // First line must be exactly "---"
    let first = lines.next()?;
    if first != "---" {
        return None;
    }

    let mut fm_lines = Vec::new();
    for line in lines {
        if line == "---" {
            return Some(fm_lines);
        }
        fm_lines.push(line.to_string());
    }

    // No closing --- found
    None
}

/// Three-state result for frontmatter field lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldState {
    /// Key not present in frontmatter.
    Missing,
    /// Key present but value is empty.
    Empty,
    /// Key present with a non-empty value.
    Value(String),
}

/// Strip outer quotes (double or single) from a string value.
fn strip_quotes(val: &str) -> &str {
    if val.len() >= 2
        && ((val.starts_with('"') && val.ends_with('"'))
            || (val.starts_with('\'') && val.ends_with('\'')))
    {
        &val[1..val.len() - 1]
    } else {
        val
    }
}

/// Extract the raw value for a key from frontmatter lines.
/// Strips leading whitespace and outer quotes (double or single).
/// Returns `None` if the key is not found, `Some("")` if the value is empty after stripping.
fn extract_raw_value<'a>(fm_lines: &'a [String], key: &str) -> Option<&'a str> {
    let prefix = format!("{key}:");
    for line in fm_lines {
        if line.starts_with(&prefix) {
            let val = line[prefix.len()..].trim_start();
            return Some(strip_quotes(val));
        }
    }
    None
}

/// Get the three-state value of a frontmatter field: Missing, Empty, or Value.
pub fn get_field_state(fm_lines: &[String], key: &str) -> FieldState {
    match extract_raw_value(fm_lines, key) {
        None => FieldState::Missing,
        Some("") => FieldState::Empty,
        Some(v) => FieldState::Value(v.to_string()),
    }
}

/// Check whether a key is present in frontmatter (regardless of value).
pub fn field_exists(fm_lines: &[String], key: &str) -> bool {
    let prefix = format!("{key}:");
    fm_lines.iter().any(|line| line.starts_with(&prefix))
}

/// Extract the body content after the frontmatter closing delimiter.
/// Returns an empty string if the content has no frontmatter or no body.
/// Handles both LF and CRLF line endings correctly.
pub fn extract_body(content: &str) -> &str {
    let bytes = content.as_bytes();
    // Check opening ---
    if !content.starts_with("---") {
        return "";
    }
    // Find end of first line (after opening ---)
    let mut pos = 3;
    if pos < bytes.len() && bytes[pos] == b'\r' {
        pos += 1;
    }
    if pos < bytes.len() && bytes[pos] == b'\n' {
        pos += 1;
    } else {
        return ""; // No newline after opening ---
    }
    // Scan for closing ---
    loop {
        if pos >= bytes.len() {
            return ""; // No closing ---
        }
        // Check if current line is exactly "---"
        if content[pos..].starts_with("---") {
            let end_marker = pos + 3;
            // Verify it's a complete line (followed by \r\n, \n, or EOF)
            if end_marker >= bytes.len()
                || bytes[end_marker] == b'\n'
                || (bytes[end_marker] == b'\r'
                    && end_marker + 1 < bytes.len()
                    && bytes[end_marker + 1] == b'\n')
            {
                // Skip past the closing --- and its line ending
                let mut body_start = end_marker;
                if body_start < bytes.len() && bytes[body_start] == b'\r' {
                    body_start += 1;
                }
                if body_start < bytes.len() && bytes[body_start] == b'\n' {
                    body_start += 1;
                }
                return if body_start < bytes.len() {
                    &content[body_start..]
                } else {
                    ""
                };
            }
        }
        // Advance to next line
        match content[pos..].find('\n') {
            Some(nl) => pos += nl + 1,
            None => return "", // No more newlines, no closing ---
        }
    }
}

/// Get the value of a top-level scalar key from frontmatter lines.
/// Strips outer quotes (double or single) and leading whitespace from the value.
/// Returns None if the key is not found or the value is empty.
/// Uses starts_with("{key}:") to match bash's index() semantics exactly.
pub fn get_field(fm_lines: &[String], key: &str) -> Option<String> {
    match extract_raw_value(fm_lines, key) {
        Some(v) if !v.is_empty() => Some(v.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_frontmatter() {
        let content = "---\nname: foo\ndescription: bar\n---\nbody text";
        let fm = extract_frontmatter(content).unwrap();
        assert_eq!(fm.len(), 2);
        assert_eq!(get_field(&fm, "name"), Some("foo".to_string()));
        assert_eq!(get_field(&fm, "description"), Some("bar".to_string()));
    }

    #[test]
    fn test_no_opening_delimiter() {
        let content = "name: foo\n---\n";
        assert!(extract_frontmatter(content).is_none());
    }

    #[test]
    fn test_no_closing_delimiter() {
        let content = "---\nname: foo\n";
        assert!(extract_frontmatter(content).is_none());
    }

    #[test]
    fn test_empty_value() {
        let content = "---\nname:\n---\n";
        let fm = extract_frontmatter(content).unwrap();
        assert_eq!(get_field(&fm, "name"), None);
    }

    #[test]
    fn test_quoted_value() {
        let content = "---\nname: \"hello world\"\n---\n";
        let fm = extract_frontmatter(content).unwrap();
        assert_eq!(get_field(&fm, "name"), Some("hello world".to_string()));
    }

    #[test]
    fn test_single_quoted_value() {
        let content = "---\nname: 'my-skill'\n---\n";
        let fm = extract_frontmatter(content).unwrap();
        assert_eq!(get_field(&fm, "name"), Some("my-skill".to_string()));
    }

    #[test]
    fn test_single_quoted_value_field_state() {
        let content = "---\nname: 'my-skill'\n---\n";
        let fm = extract_frontmatter(content).unwrap();
        assert_eq!(
            get_field_state(&fm, "name"),
            FieldState::Value("my-skill".to_string())
        );
    }

    #[test]
    fn test_double_quoted_empty_value() {
        let content = "---\nname: \"\"\n---\n";
        let fm = extract_frontmatter(content).unwrap();
        assert_eq!(get_field(&fm, "name"), None);
        assert_eq!(get_field_state(&fm, "name"), FieldState::Empty);
    }

    #[test]
    fn test_single_quoted_empty_value() {
        let content = "---\nname: ''\n---\n";
        let fm = extract_frontmatter(content).unwrap();
        assert_eq!(get_field(&fm, "name"), None);
        assert_eq!(get_field_state(&fm, "name"), FieldState::Empty);
    }

    #[test]
    fn test_key_prefix_no_false_match() {
        // "name:" should not match "name-suffix: foo"
        let content = "---\nname-suffix: foo\n---\n";
        let fm = extract_frontmatter(content).unwrap();
        assert_eq!(get_field(&fm, "name"), None);
    }

    #[test]
    fn test_field_state_missing() {
        let content = "---\nname: foo\n---\n";
        let fm = extract_frontmatter(content).unwrap();
        assert_eq!(get_field_state(&fm, "description"), FieldState::Missing);
    }

    #[test]
    fn test_field_state_empty() {
        let content = "---\nname:\n---\n";
        let fm = extract_frontmatter(content).unwrap();
        assert_eq!(get_field_state(&fm, "name"), FieldState::Empty);
    }

    #[test]
    fn test_field_state_value() {
        let content = "---\nname: foo\n---\n";
        let fm = extract_frontmatter(content).unwrap();
        assert_eq!(
            get_field_state(&fm, "name"),
            FieldState::Value("foo".to_string())
        );
    }

    #[test]
    fn test_field_exists_true() {
        let content = "---\nname: foo\n---\n";
        let fm = extract_frontmatter(content).unwrap();
        assert!(field_exists(&fm, "name"));
    }

    #[test]
    fn test_field_exists_false() {
        let content = "---\nname: foo\n---\n";
        let fm = extract_frontmatter(content).unwrap();
        assert!(!field_exists(&fm, "description"));
    }

    #[test]
    fn test_extract_body() {
        let content = "---\nname: foo\n---\nBody text here\n";
        assert_eq!(extract_body(content), "Body text here\n");
    }

    #[test]
    fn test_extract_body_empty() {
        let content = "---\nname: foo\n---\n";
        assert_eq!(extract_body(content), "");
    }

    #[test]
    fn test_extract_body_no_frontmatter() {
        let content = "Just text";
        assert_eq!(extract_body(content), "");
    }

    #[test]
    fn test_extract_body_crlf() {
        let content = "---\r\nname: foo\r\n---\r\nBody text here\r\n";
        assert_eq!(extract_body(content), "Body text here\r\n");
    }

    #[test]
    fn test_extract_body_crlf_empty() {
        let content = "---\r\nname: foo\r\n---\r\n";
        assert_eq!(extract_body(content), "");
    }

    #[test]
    fn test_extract_body_delimiter_exact_match() {
        // "----" inside frontmatter should not cut off the body
        let content = "---\nname: foo\n----\ndescription: bar\n---\nBody text\n";
        assert_eq!(extract_body(content), "Body text\n");
    }

    #[test]
    fn test_extract_body_multiline() {
        let content = "---\nname: foo\ndescription: bar\n---\nLine 1\nLine 2\nLine 3\n";
        let body = extract_body(content);
        assert_eq!(body, "Line 1\nLine 2\nLine 3\n");
        assert_eq!(body.lines().count(), 3);
    }

    #[test]
    fn test_delimiter_exact_match() {
        // "----" should NOT be treated as a closing delimiter
        let content = "---\nname: foo\n----\ndescription: bar\n---\n";
        let fm = extract_frontmatter(content).unwrap();
        // "----" is NOT the closing ---, so we should get name, ----, and description
        assert_eq!(fm.len(), 3);
        assert_eq!(get_field(&fm, "name"), Some("foo".to_string()));
        assert_eq!(get_field(&fm, "description"), Some("bar".to_string()));
    }
}
