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
