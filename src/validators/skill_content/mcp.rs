use crate::diagnostic::DiagnosticCollector;
use crate::rules::LintRule;
use crate::validators::skills::SkillInfo;
use regex::Regex;
use std::collections::HashSet;
use std::sync::LazyLock;

// S044: Backtick-quoted snake_case identifier with at least one underscore.
// Captures the identifier inside backticks.
static RE_BACKTICK_SNAKE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"`([a-z][a-z0-9]*(?:_[a-z0-9]+)+)`").unwrap());

// Context vocabulary suggesting a tool invocation, matched on word boundaries so
// prose substrings ("use" inside "Because"/"user", "run" inside "prune", "call"
// inside "recall") do not pass the gate. Case-insensitive via the inline `(?i)`
// flag; includes common inflections of each verb plus the singular/plural noun
// "tool(s)".
static RE_CONTEXT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:use[sd]?|using|call(?:s|ed|ing)?|invoke[sd]?|invoking|run(?:s|ning)?|ran|execute[sd]?|executing|tools?)\b",
    )
    .unwrap()
});

// Built-in platform tools in snake_case form. Single-word tools (bash, read,
// etc.) are excluded automatically by the regex's underscore requirement.
// This list covers multi-word built-ins that a user might write in snake_case.
// See also: KNOWN_TOOLS in common.rs (PascalCase, shared by S040 and A019/A020).
const BUILTIN_TOOLS_SNAKE: &[&str] = &[
    "task_create",
    "task_update",
    "task_list",
    "task_get",
    "task_stop",
    "task_output",
    "ask_user_question",
    "notebook_edit",
    "web_fetch",
    "web_search",
];

/// S044: Detect backtick-quoted MCP tool references that lack a `ServerName:` prefix.
///
/// Only flags identifiers on lines that also contain an invocation-context word
/// (e.g., "use", "call", "tool"), matched on word boundaries so prose substrings
/// like "use" inside "Because" do not pass the gate, reducing false positives on
/// generic snake_case variables.
/// Runs in both plugin and private skill modes (no `plugin_mode` gate).
/// Scans `info.body` only (post-frontmatter content).
pub(super) fn check_mcp_tool_refs(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    if info.body.trim().is_empty() {
        return;
    }

    let mut reported: HashSet<String> = HashSet::new();

    for line in crate::fence::lines_outside_fences(&info.body) {
        // Skip lines with no invocation-context word (word-boundary match).
        if !RE_CONTEXT.is_match(line) {
            continue;
        }

        for cap in RE_BACKTICK_SNAKE.captures_iter(line) {
            let identifier = &cap[1];
            // Note: colon-qualified forms like `BigQuery:bigquery_schema` are already
            // excluded by the regex — neither `:` nor uppercase letters are in [a-z0-9_].

            // Skip built-in platform tools
            if BUILTIN_TOOLS_SNAKE.contains(&identifier) {
                continue;
            }

            // Report each distinct tool name once per file
            if !reported.insert(identifier.to_string()) {
                continue;
            }

            diag.report(
                LintRule::McpToolUnqualified,
                &format!(
                    "{}: backtick-quoted '{}' may be an MCP tool reference without ServerName: prefix (use ServerName:tool_name format)",
                    info.path, identifier
                ),
            );
        }
    }
}
