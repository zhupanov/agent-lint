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

/// Shared enum: accepted values for a `shell` field.
/// Used by skill_content frontmatter validation (S026) and hook schema
/// validation (H022), which must stay in agreement.
pub(crate) const VALID_SHELLS: &[&str] = &["bash", "powershell"];

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

/// Built-in Claude Code tool names (PascalCase). Shared by S040
/// (skill `allowed-tools`) and A019/A020 (agent `tools`/`disallowedTools`).
pub(crate) const KNOWN_TOOLS: &[&str] = &[
    "AskUserQuestion",
    "Bash",
    "Read",
    "Edit",
    "Write",
    "Grep",
    "Glob",
    "Agent",
    "Task",
    "WebFetch",
    "WebSearch",
    "Skill",
    "NotebookEdit",
    "LSP",
    "TaskCreate",
    "TaskUpdate",
    "TaskList",
    "TaskGet",
    "TaskStop",
    "TaskOutput",
];

/// Whether a single tool entry is recognized by Claude Code.
///
/// Accepts:
/// - Built-in tool names (see [`KNOWN_TOOLS`]), case-sensitive PascalCase.
/// - Tool-restricted forms like `Bash(git *)` — the `(...)` suffix is ignored.
/// - MCP tools written as `mcp__<server>__<tool>` (server and tool both required).
///
/// Returns `false` for empty input so callers can skip blank entries.
pub(crate) fn is_known_tool_name(tool: &str) -> bool {
    let tool = tool.trim();
    if tool.is_empty() {
        return false;
    }
    // Strip a trailing argument-restriction suffix, e.g. "Bash(git *)" -> "Bash".
    let base_name = match tool.find('(') {
        Some(paren) => tool[..paren].trim(),
        None => tool,
    };
    if base_name.is_empty() {
        return false;
    }
    // MCP tools: mcp__<server>__<tool> — both parts must be non-empty.
    if let Some(rest) = base_name.strip_prefix("mcp__") {
        return !rest.is_empty()
            && !rest.starts_with("__")
            && !rest.ends_with("__")
            && rest.contains("__");
    }
    KNOWN_TOOLS.contains(&base_name)
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

#[cfg(test)]
mod tool_tests {
    use super::is_known_tool_name;

    #[test]
    fn accepts_builtin_tools() {
        assert!(is_known_tool_name("Bash"));
        assert!(is_known_tool_name("Read"));
        assert!(is_known_tool_name("Agent"));
    }

    #[test]
    fn accepts_restricted_form() {
        assert!(is_known_tool_name("Bash(git *)"));
        assert!(is_known_tool_name("Grep(pat)"));
    }

    #[test]
    fn accepts_mcp_tools() {
        assert!(is_known_tool_name("mcp__server__tool"));
        assert!(is_known_tool_name("mcp__my_server__my_tool"));
    }

    #[test]
    fn rejects_mcp_malformed() {
        assert!(!is_known_tool_name("mcp__"));
        assert!(!is_known_tool_name("mcp__tool"));
        assert!(!is_known_tool_name("mcp__server__"));
    }

    #[test]
    fn rejects_unknown_tools() {
        assert!(!is_known_tool_name("UnknownTool"));
        assert!(!is_known_tool_name(""));
    }
}
