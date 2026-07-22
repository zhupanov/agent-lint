use std::ops::Range;

/// A complete leading YAML frontmatter block, retaining byte ranges in its
/// owning source so validators can attach file-relative structured metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeadingFrontmatter<'a> {
    pub yaml: &'a str,
    pub yaml_range: Range<usize>,
    pub delimiter_range: Range<usize>,
    pub body: &'a str,
}

/// Leading-frontmatter discovery result.
///
/// A file with no logical opener is body-only. An opener is `---` followed
/// only by horizontal whitespace and a LF/CRLF line ending. A closing marker
/// has the same delimiter grammar and may end at EOF.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeadingFrontmatterState<'a> {
    Absent { body: &'a str },
    Complete(LeadingFrontmatter<'a>),
    Unterminated { delimiter_range: Range<usize> },
}

/// Split a document at a logical leading YAML frontmatter block.
///
/// This deliberately owns only delimiter/body recognition; callers retain
/// their own YAML schema policy. It is shared infrastructure for Claude
/// configuration surfaces that accept body-only Markdown files.
pub fn leading_frontmatter(content: &str) -> LeadingFrontmatterState<'_> {
    let Some(open_end) = logical_delimiter_end(content, 0, true) else {
        return LeadingFrontmatterState::Absent { body: content };
    };
    let opening_range = 0..open_end;
    let yaml_start = open_end;
    let mut line_start = open_end;
    while line_start < content.len() {
        if let Some(close_end) = logical_delimiter_end(content, line_start, false) {
            return LeadingFrontmatterState::Complete(LeadingFrontmatter {
                yaml: &content[yaml_start..line_start],
                yaml_range: yaml_start..line_start,
                delimiter_range: opening_range,
                body: &content[close_end..],
            });
        }
        let Some(newline) = content[line_start..].find('\n') else {
            break;
        };
        line_start += newline + 1;
    }
    LeadingFrontmatterState::Unterminated {
        delimiter_range: opening_range,
    }
}

fn logical_delimiter_end(content: &str, start: usize, require_newline: bool) -> Option<usize> {
    let newline = content[start..].find('\n').map(|offset| start + offset);
    let (line_end, end) = match newline {
        Some(newline) => (newline, newline + 1),
        None if !require_newline => (content.len(), content.len()),
        None => return None,
    };
    let line = content[start..line_end]
        .strip_suffix('\r')
        .unwrap_or(&content[start..line_end]);
    (line.starts_with("---") && line[3..].bytes().all(|byte| matches!(byte, b' ' | b'\t')))
        .then_some(end)
}

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
/// Note: uses `starts_with("{key}:")` — the trailing colon prevents prefix collisions
/// (e.g., looking up "name" won't match a "namespace:" line).
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

/// Strict YAML parse failure for frontmatter (AS-016 / X001).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YamlStrictError {
    /// 1-based line within the **file** (opening `---` is line 1).
    pub file_line: usize,
    /// 1-based column within the YAML line when the parser supplies one.
    pub column: Option<usize>,
    /// Human explanation without untranslated parser coordinates.
    pub message: String,
}

/// Strict YAML parse of frontmatter lines (AS-016 / X001).
///
/// On success returns the parsed document. On failure returns a file-relative
/// location and a message that does not embed the parser's YAML-relative
/// `at line N, column M` coordinates.
pub fn parse_yaml_strict(fm_lines: &[String]) -> Result<crate::yaml::Value, YamlStrictError> {
    // Restore the trailing newline that line extraction dropped. A frontmatter
    // block whose final line is a bare `key:` (a null value) is valid YAML, but
    // the parser rejects a document that ends at a key with no following
    // newline. The only value this changes is a keep-chomped (`|+`/`>+`) block
    // scalar on the final line, which regains the trailing newline the real
    // file carries before the closing `---`; every other document is identical.
    let text = format!("{}\n", fm_lines.join("\n"));
    match crate::yaml::parse(&text) {
        Ok(value) => Ok(value),
        Err(err) => {
            // YAML text starts on file line 2 (after the opening ---).
            let yaml_line = crate::yaml::error_line(&err).unwrap_or(1);
            let file_line = yaml_line.saturating_add(1);
            let column = crate::yaml::error_column(&err);
            Err(YamlStrictError {
                file_line,
                column,
                message: strip_parser_location_prefix(&err.to_string()),
            })
        }
    }
}

/// Strip parser location (and the redundant `YAML parse error` wrapper) so X001
/// keeps a single file-relative line authority in the rendered diagnostic.
/// Shared with the Cursor rule validator, which additionally strips the
/// colon-less trailing `at line N, column M` form of anchor/alias errors.
pub fn strip_parser_location_prefix(message: &str) -> String {
    static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"^(?:YAML parse error|deserialization error)(?: at line \d+, column \d+)?:\s*",
        )
        .expect("location strip regex")
    });
    let stripped = RE.replace(message, "");
    if stripped.as_ref() == message {
        // Fall back: remove an embedded ` at line N, column M:` if the prefix
        // shape differs from the common parse-error forms.
        static EMBEDDED: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new(r" at line \d+, column \d+:").expect("embedded location strip")
        });
        EMBEDDED.replace(message, ":").into_owned()
    } else {
        stripped.into_owned()
    }
}

/// One-based file line of an unindented simple top-level mapping key
/// (`name:`, `description:`, …), or `None` when the helper cannot map the key
/// exactly (quoted/explicit/merged/indented forms).
pub fn simple_top_level_key_line(fm_lines: &[String], key: &str) -> Option<usize> {
    simple_top_level_key_index(fm_lines, key).map(|index| index + 2)
}

/// Zero-based frontmatter index of an unindented simple top-level mapping key.
pub fn simple_top_level_key_index(fm_lines: &[String], key: &str) -> Option<usize> {
    let prefix = format!("{key}:");
    fm_lines.iter().position(|line| line.starts_with(&prefix))
}

/// Read a non-empty string field from strictly parsed mapping frontmatter.
///
/// This is for consumers that require a trustworthy schema value rather than
/// the legacy line-oriented field lookup. Invalid YAML, non-mapping documents,
/// non-string values, and empty strings do not yield a value.
pub fn get_strict_string_field(fm_lines: &[String], key: &str) -> Option<String> {
    let yaml = parse_yaml_strict(fm_lines).ok()?;
    canonical_nonempty_string_field(&yaml, key).map(str::to_owned)
}

/// Read a non-empty string field from an already strictly parsed frontmatter
/// document. This is the shared canonical-YAML contract for validators and
/// autofixers that need a scalar value.
pub fn canonical_nonempty_string_field<'a>(
    yaml: &'a crate::yaml::Value,
    key: &str,
) -> Option<&'a str> {
    yaml.as_mapping()?
        .get(key)?
        .as_str()
        .filter(|value| !value.is_empty())
}

/// Interpret a canonical YAML value as an accepted skill boolean field value.
///
/// Returns `Some(true)`/`Some(false)` for a YAML boolean (any YAML 1.2 casing)
/// or the quoted strings `"true"`/`"false"` kept accepted for compatibility.
/// Every other value — other strings, numbers, null, sequences, mappings —
/// yields `None`. This is the shared contract for S023 (accept iff `Some`) and
/// the S027/S066 gates (which branch on the concrete boolean).
pub fn canonical_bool_value(value: &crate::yaml::Value) -> Option<bool> {
    if let Some(boolean) = value.as_bool() {
        return Some(boolean);
    }
    match value.as_str() {
        Some("true") => Some(true),
        Some("false") => Some(false),
        _ => None,
    }
}

/// Whether a top-level field in an already strictly parsed frontmatter mapping
/// is present but canonically empty. YAML null and the empty string are empty;
/// every other YAML value, including sequences, is not.
pub fn canonical_field_is_empty(yaml: &crate::yaml::Value, key: &str) -> bool {
    yaml.as_mapping()
        .and_then(|mapping| mapping.get(key))
        .is_some_and(|value| value.is_null() || value.as_str().is_some_and(str::is_empty))
}

/// Determine whether an optional field is empty using the same policy as S007.
/// Strictly parsed frontmatter uses canonical YAML values; invalid YAML retains
/// the rule's established line-oriented fallback.
pub fn optional_field_is_empty(
    fm_lines: &[String],
    parsed_frontmatter: Option<&crate::yaml::Value>,
    key: &str,
) -> bool {
    parsed_frontmatter.map_or_else(
        || get_field(fm_lines, key).is_none(),
        |yaml| canonical_field_is_empty(yaml, key),
    )
}

/// Determine whether an optional field is present using the same parsed-YAML
/// versus invalid-YAML fallback boundary as S007.
pub fn optional_field_is_present(
    fm_lines: &[String],
    parsed_frontmatter: Option<&crate::yaml::Value>,
    key: &str,
) -> bool {
    parsed_frontmatter.map_or_else(
        || field_exists(fm_lines, key),
        |yaml| {
            yaml.as_mapping()
                .is_some_and(|mapping| mapping.get(key).is_some())
        },
    )
}

/// Convert a YAML value to JSON for reuse by JSON-shaped validators (e.g. hooks).
pub fn yaml_to_json(value: &crate::yaml::Value) -> Option<serde_json::Value> {
    serde_json::to_value(value).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leading_frontmatter_handles_logical_delimiters_and_body_only_files() {
        let body_only = "Instructions without metadata\n";
        assert_eq!(
            leading_frontmatter(body_only),
            LeadingFrontmatterState::Absent { body: body_only }
        );

        let source = "--- \t\r\nname: concise\r\n---  \r\nBody\r\n";
        let LeadingFrontmatterState::Complete(block) = leading_frontmatter(source) else {
            panic!("logical CRLF delimiters must form frontmatter");
        };
        assert_eq!(block.yaml, "name: concise\r\n");
        assert_eq!(block.body, "Body\r\n");
    }

    #[test]
    fn leading_frontmatter_distinguishes_unterminated_attempts() {
        let source = "---\nname: concise\n";
        assert!(matches!(
            leading_frontmatter(source),
            LeadingFrontmatterState::Unterminated { .. }
        ));
        assert!(matches!(
            leading_frontmatter("----\nnot frontmatter\n"),
            LeadingFrontmatterState::Absent { .. }
        ));
    }

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
    fn canonical_field_helpers_use_parsed_yaml_values() {
        let yaml = crate::yaml::parse(
            "name: >-\n  continued-name\nargument-hint:\n  '[issue-number]'\nempty: \"\"\nnull: null\ntools: [Read]\n",
        )
        .unwrap();

        assert_eq!(
            canonical_nonempty_string_field(&yaml, "name"),
            Some("continued-name")
        );
        assert!(!canonical_field_is_empty(&yaml, "argument-hint"));
        assert!(canonical_field_is_empty(&yaml, "empty"));
        assert!(canonical_field_is_empty(&yaml, "null"));
        assert!(!canonical_field_is_empty(&yaml, "tools"));
        assert!(optional_field_is_present(&[], Some(&yaml), "argument-hint"));

        let invalid = vec!["argument-hint:".to_string(), "\tinvalid: yaml".to_string()];
        assert!(optional_field_is_empty(&invalid, None, "argument-hint"));
        assert!(optional_field_is_present(&invalid, None, "argument-hint"));
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
    fn test_parse_yaml_strict_ok() {
        let fm = extract_frontmatter("---\nname: foo\nhooks:\n  Stop:\n    - hooks:\n        - type: command\n          command: echo\n---\n").unwrap();
        let yaml = parse_yaml_strict(&fm).unwrap();
        assert!(yaml.get("hooks").is_some());
        assert!(yaml_to_json(yaml.get("hooks").unwrap()).is_some());
    }

    #[test]
    fn test_parse_yaml_strict_reports_file_line() {
        let fm = extract_frontmatter("---\nname: foo\n\tbad: tab\n---\n").unwrap();
        let err = parse_yaml_strict(&fm).unwrap_err();
        assert_eq!(err.file_line, 3, "file line should account for opening ---");
        assert!(
            !err.message.contains("at line"),
            "parser coordinates must not remain in the message: {}",
            err.message
        );
        assert!(
            !err.message.contains("YAML parse error"),
            "redundant parser wrapper must be stripped: {}",
            err.message
        );
        assert!(err.column.is_some());
    }

    #[test]
    fn strict_string_field_requires_valid_mapping_and_scalar() {
        let cases = [
            (
                "description: A usable routing description",
                Some("A usable routing description"),
            ),
            ("description:\n  - A usable routing description", None),
            ("- description: A usable routing description", None),
            (
                "description: A usable routing description\ndescription: duplicate",
                None,
            ),
            ("description: [A usable routing description", None),
            ("description: \"\"", None),
        ];

        for (source, expected) in cases {
            let fm = source.lines().map(str::to_owned).collect::<Vec<_>>();
            assert_eq!(
                get_strict_string_field(&fm, "description").as_deref(),
                expected,
                "{source}"
            );
        }
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
