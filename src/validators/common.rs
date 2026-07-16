use regex::Regex;
use std::sync::LazyLock;

/// Shared regex: matches characters outside [a-z0-9-] in names.
/// Used by skill_content name validation and agents name validation.
pub(crate) static RE_NAME_INVALID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^a-z0-9-]").unwrap());

/// Shared regex: matches TODO/FIXME/HACK/XXX markers (case-insensitive).
/// Used by hygiene TODO scanning and docs TODO scanning.
/// Note: docs.rs previously defined this as `RE_TODO` with the same pattern.
pub(crate) static RE_TODO_MARKER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(TODO|FIXME|HACK|XXX)\b").unwrap());

/// Model aliases accepted by Claude Code `/model` plus skill-only `inherit`.
/// Full Anthropic model IDs (`claude-…`) are also accepted.
/// Shared by skill frontmatter (S063) and future agent frontmatter validation.
const MODEL_ALIASES: &[&str] = &[
    "inherit",
    "default",
    "best",
    "sonnet",
    "opus",
    "haiku",
    "fable",
    "sonnet[1m]",
    "opus[1m]",
    "opusplan",
    "opusplan[1m]",
    "fable[1m]",
];

/// Return true if `value` is a recognized Claude Code model alias or ID.
pub(crate) fn is_valid_model_value(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() {
        return false;
    }
    if MODEL_ALIASES.contains(&value) {
        return true;
    }
    // Full Anthropic model IDs and version pins (e.g. claude-sonnet-4-5, claude-opus-4-6).
    value.starts_with("claude-")
}

#[cfg(test)]
mod model_tests {
    use super::is_valid_model_value;

    #[test]
    fn accepts_aliases_and_full_ids() {
        assert!(is_valid_model_value("sonnet"));
        assert!(is_valid_model_value("opus[1m]"));
        assert!(is_valid_model_value("inherit"));
        assert!(is_valid_model_value("claude-sonnet-4-5"));
        assert!(is_valid_model_value("claude-opus-4-6"));
    }

    #[test]
    fn rejects_typos_and_empty() {
        assert!(!is_valid_model_value("sonet"));
        assert!(!is_valid_model_value(""));
        assert!(!is_valid_model_value("gpt-4"));
    }
}
