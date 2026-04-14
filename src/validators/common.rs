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
