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

/// Get the three-state value of a frontmatter field: Missing, Empty, or Value.
pub fn get_field_state(fm_lines: &[String], key: &str) -> FieldState {
    let prefix = format!("{key}:");
    for line in fm_lines {
        if line.starts_with(&prefix) {
            let val = &line[prefix.len()..];
            let val = val.trim_start();
            let val = if val.starts_with('"') && val.ends_with('"') && val.len() >= 2 {
                &val[1..val.len() - 1]
            } else {
                val
            };
            if val.is_empty() {
                return FieldState::Empty;
            }
            return FieldState::Value(val.to_string());
        }
    }
    FieldState::Missing
}

/// Check whether a key is present in frontmatter (regardless of value).
pub fn field_exists(fm_lines: &[String], key: &str) -> bool {
    let prefix = format!("{key}:");
    fm_lines.iter().any(|line| line.starts_with(&prefix))
}

/// Extract the body content after the frontmatter closing delimiter.
/// Returns an empty string if the content has no frontmatter or no body.
pub fn extract_body(content: &str) -> &str {
    let mut lines = content.lines();
    // Skip first line (opening ---)
    match lines.next() {
        Some("---") => {}
        _ => return "",
    }
    // Skip frontmatter lines until closing ---
    let mut offset = content.find('\n').map(|i| i + 1).unwrap_or(content.len());
    for line in lines {
        if line == "---" {
            let body_start = offset + line.len();
            if body_start < content.len() {
                // Skip the newline after closing ---
                let start = if content.as_bytes().get(body_start) == Some(&b'\n') {
                    body_start + 1
                } else {
                    body_start
                };
                return &content[start..];
            }
            return "";
        }
        offset += line.len() + 1; // +1 for newline
    }
    ""
}

/// Get the value of a top-level scalar key from frontmatter lines.
/// Strips outer double quotes and leading whitespace from the value.
/// Returns None if the key is not found or the value is empty.
/// Uses starts_with("{key}:") to match bash's index() semantics exactly.
pub fn get_field(fm_lines: &[String], key: &str) -> Option<String> {
    let prefix = format!("{key}:");
    for line in fm_lines {
        if line.starts_with(&prefix) {
            let val = &line[prefix.len()..];
            let val = val.trim_start();
            // Strip outer double quotes
            let val = if val.starts_with('"') && val.ends_with('"') && val.len() >= 2 {
                &val[1..val.len() - 1]
            } else {
                val
            };
            if val.is_empty() {
                return None;
            }
            return Some(val.to_string());
        }
    }
    None
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
