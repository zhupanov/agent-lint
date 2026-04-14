use crate::diagnostic::DiagnosticCollector;
use crate::frontmatter;
use crate::rules::LintRule;
use crate::validators::skills::SkillInfo;

use super::RE_BACKSLASH_PATH;

pub(super) fn check_frontmatter_extended(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    // S035: compatibility field too long
    if let frontmatter::FieldState::Value(val) =
        frontmatter::get_field_state(&info.fm_lines, "compatibility")
    {
        if val.len() > 500 {
            diag.report(
                LintRule::CompatTooLong,
                &format!(
                    "{}: 'compatibility' exceeds 500 characters ({})",
                    info.path,
                    val.len()
                ),
            );
        }
    }

    // S039: metadata values should be strings
    // Look for metadata lines in frontmatter that have bare true/false/numeric values
    let mut in_metadata = false;
    for line in &info.fm_lines {
        if line == "metadata:" || line.starts_with("metadata:") {
            // Check for inline scalar value on the metadata: line itself
            let inline_val = line["metadata:".len()..].trim();
            if !inline_val.is_empty()
                && !inline_val.starts_with('"')
                && !inline_val.starts_with('\'')
                && (inline_val == "true"
                    || inline_val == "false"
                    || inline_val.parse::<f64>().is_ok())
            {
                diag.report(
                    LintRule::MetadataNotString,
                    &format!(
                        "{}: metadata has non-string inline value '{}' (wrap in quotes)",
                        info.path, inline_val
                    ),
                );
            }
            in_metadata = true;
            continue;
        }
        if in_metadata {
            // Metadata entries are indented (e.g., "  key: value")
            if !line.starts_with(' ') && !line.starts_with('\t') {
                break; // End of metadata block
            }
            if let Some(colon_pos) = line.find(':') {
                let val = line[colon_pos + 1..].trim();
                if !val.is_empty()
                    && !val.starts_with('"')
                    && !val.starts_with('\'')
                    && (val == "true" || val == "false" || val.parse::<f64>().is_ok())
                {
                    let key = line[..colon_pos].trim();
                    diag.report(
                        LintRule::MetadataNotString,
                        &format!(
                            "{}: metadata key '{}' has non-string value '{}' (wrap in quotes)",
                            info.path, key, val
                        ),
                    );
                }
            }
        }
    }

    // S040: allowed-tools unknown
    if let Some(tools_str) = frontmatter::get_field(&info.fm_lines, "allowed-tools") {
        let known_tools = [
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
        for tool in tools_str.split(',') {
            let tool = tool.trim();
            // Skip tool patterns like "Bash(git *)" -- extract base name
            let base_name = if let Some(paren) = tool.find('(') {
                tool[..paren].trim()
            } else {
                tool
            };
            if base_name.is_empty() {
                continue;
            }
            if !known_tools.contains(&base_name) {
                diag.report(
                    LintRule::ToolsUnknown,
                    &format!(
                        "{}: allowed-tools lists unrecognized tool '{}' (tool names are case-sensitive PascalCase; may be an MCP tool — verify spelling)",
                        info.path, base_name
                    ),
                );
            }
        }
    }

    // S042: disable-model-invocation: true with empty/missing description
    if frontmatter::get_field(&info.fm_lines, "disable-model-invocation").as_deref() == Some("true")
    {
        match frontmatter::get_field_state(&info.fm_lines, "description") {
            frontmatter::FieldState::Missing | frontmatter::FieldState::Empty => {
                diag.report(
                    LintRule::DmiEmptyDesc,
                    &format!(
                        "{}: disable-model-invocation: true but description is empty/missing (user-only skills need descriptions for the / menu)",
                        info.path
                    ),
                );
            }
            frontmatter::FieldState::Value(_) => {}
        }
    }

    // S043: backslash paths in frontmatter
    for line in &info.fm_lines {
        if RE_BACKSLASH_PATH.is_match(line) {
            diag.report(
                LintRule::FrontmatterBackslash,
                &format!(
                    "{}: Windows-style backslash path in frontmatter; use forward slashes",
                    info.path
                ),
            );
            break; // Report once per file
        }
    }
}
