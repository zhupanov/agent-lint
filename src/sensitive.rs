use regex::Regex;
use std::sync::LazyLock;

static SECRET_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        Regex::new(r"sk-[a-zA-Z0-9]{20,}").unwrap(),
        Regex::new(r"ghp_[a-zA-Z0-9]{36,}").unwrap(),
        Regex::new(r"xox[bp]-[0-9][a-zA-Z0-9\-]{8,}").unwrap(),
        Regex::new(
            r#"(?i)(api[_\-]?key|api[_\-]?secret|api[_\-]?token)\s*[=:]\s*["']?[A-Za-z0-9]{20,}"#,
        )
        .unwrap(),
        Regex::new(r#"(?i)(password|secret|token)\s*[=:]\s*["'][^"']{8,}"#).unwrap(),
    ]
});

static SENSITIVE_ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)(?:api[_\-]?(?:key|secret|token)|password|secret|token)\s*[=:]\s*["']?[^\s,"'}]+"#,
    )
    .unwrap()
});

/// Conservative shared check for values that must not be echoed in output.
pub(crate) fn contains_possible_secret(content: &str) -> bool {
    SECRET_PATTERNS
        .iter()
        .any(|pattern| pattern.is_match(content))
}

/// Evidence is display-oriented and therefore uses a deliberately broader
/// convention than lint rules: omit any apparent sensitive-key assignment,
/// even when the value is short or is only a placeholder.
pub(crate) fn contains_sensitive_evidence(content: &str) -> bool {
    contains_possible_secret(content) || SENSITIVE_ASSIGNMENT.is_match(content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_supported_secret_shapes() {
        assert!(contains_possible_secret(
            "token = 'this-is-a-sensitive-value'"
        ));
        assert!(contains_possible_secret("sk-abcdefghijklmnopqrstuvwxyz"));
        assert!(!contains_possible_secret("token = '$TOKEN'"));
        assert!(contains_sensitive_evidence("API_KEY=short-value"));
        assert!(contains_sensitive_evidence("token = '$TOKEN'"));
    }
}
