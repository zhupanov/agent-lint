use crate::diagnostic::DiagnosticCollector;
use crate::frontmatter;
use crate::rules::LintRule;
use crate::validators::skills::SkillInfo;
use crate::yaml::Mapping;

pub(super) fn check_frontmatter_extended(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    // S035, S039, and S043 read canonical YAML values so comments, quoting, and
    // multiline scalars cannot corrupt the compared value. Invalid or
    // non-mapping frontmatter is owned by X001/S004/S005, so they skip it.
    if let Some(map) = info.frontmatter_mapping() {
        check_compatibility_length(info, map, diag);
        check_metadata_values(info, map, diag);
        check_frontmatter_backslash(info, map, diag);
    }

    // S045 and S040/S067 remain line-oriented and are outside this migration's
    // scope; they keep their existing behavior.
    check_allowed_tools_list_syntax(info, diag);
    check_allowed_tools(info, diag);

    // S042 (dmi-empty-desc) is soft-retired: it was a strict subset of
    // S005/frontmatter-field-missing and no longer fires from any path. Its
    // registry code/name remain as a deprecated, config-only identifier.
}

/// S035: cap `compatibility` length, measured in Unicode scalar values so the
/// count matches user-visible characters and multiline scalars are covered. A
/// canonical non-string value is left to the field-shape rules.
fn check_compatibility_length(info: &SkillInfo, map: &Mapping, diag: &mut DiagnosticCollector) {
    if let Some(text) = map.get("compatibility").and_then(|value| value.as_str()) {
        let count = text.chars().count();
        if count > 500 {
            diag.report(
                LintRule::CompatTooLong,
                &format!(
                    "{}: 'compatibility' exceeds 500 characters ({count})",
                    info.path
                ),
            );
        }
    }
}

/// S039: `metadata`, when present, must be a mapping whose values are all
/// strings. A non-string entry value is flagged individually; a present but
/// non-mapping, non-null `metadata` yields a single shape diagnostic. Null and
/// absent stay silent.
fn check_metadata_values(info: &SkillInfo, map: &Mapping, diag: &mut DiagnosticCollector) {
    let Some(value) = map.get("metadata") else {
        return;
    };
    if let Some(metadata) = value.as_mapping() {
        for (key, entry) in metadata.iter() {
            if entry.as_str().is_none() {
                diag.report(
                    LintRule::MetadataNotString,
                    &format!(
                        "{}: metadata key '{}' has non-string value '{}' (wrap in quotes)",
                        info.path, key, entry
                    ),
                );
            }
        }
    } else if !value.is_null() {
        diag.report(
            LintRule::MetadataNotString,
            &format!("{}: metadata must be a map of string values", info.path),
        );
    }
}

/// S043: reject Windows-style backslash paths in path-configuration frontmatter
/// values. Free-prose and metadata values are exempt (see `S043_PROSE_FIELDS`).
/// Scalar values and sequence string items are scanned; other shapes are not.
fn check_frontmatter_backslash(info: &SkillInfo, map: &Mapping, diag: &mut DiagnosticCollector) {
    let has_backslash_path = map.iter().any(|(key, value)| {
        !super::S043_PROSE_FIELDS.contains(&key.as_str())
            && super::canonical_value_has_backslash_path(value)
    });
    if has_backslash_path {
        diag.report(
            LintRule::FrontmatterBackslash,
            &format!(
                "{}: Windows-style backslash path in frontmatter; use forward slashes",
                info.path
            ),
        );
    }
}

/// S045: `allowed-tools` written as a YAML list instead of a comma scalar.
fn check_allowed_tools_list_syntax(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    if frontmatter::field_exists(&info.fm_lines, "allowed-tools")
        && frontmatter::get_field(&info.fm_lines, "allowed-tools").is_none()
    {
        // Check for actual YAML list items ("- " lines after the key, possibly unindented or
        // separated by blank lines)
        let has_list_items = info
            .fm_lines
            .iter()
            .position(|l| l.starts_with("allowed-tools:"))
            .is_some_and(|i| {
                info.fm_lines[i + 1..]
                    .iter()
                    .take_while(|l| {
                        l.is_empty()
                            || l.starts_with(' ')
                            || l.starts_with('\t')
                            || l.starts_with("- ")
                    })
                    .any(|l| l.trim_start().starts_with("- "))
            });
        if has_list_items {
            diag.report(
                LintRule::ToolsListSyntax,
                &format!(
                    "{}: 'allowed-tools' uses YAML list syntax; use comma-separated scalar instead (e.g., allowed-tools: Bash, Read, Write)",
                    info.path
                ),
            );
        }
    }
}

/// S040 / S067: unrecognized or unscoped `allowed-tools` entries.
fn check_allowed_tools(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    if let Some(tools_str) = frontmatter::get_field(&info.fm_lines, "allowed-tools") {
        for tool in tools_str.split(',') {
            let tool = tool.trim();
            if tool.is_empty() {
                continue;
            }
            // Reuse the shared known-tool checker (also used by agent tools rules).
            // Base name is reported (argument-restriction suffix like "Bash(git *)" stripped).
            let base_name = match tool.find('(') {
                Some(paren) => tool[..paren].trim(),
                None => tool,
            };
            if base_name.is_empty() {
                continue;
            }
            if !crate::validators::common::is_known_tool_name(tool) {
                diag.report(
                    LintRule::ToolsUnknown,
                    &format!(
                        "{}: allowed-tools lists unrecognized tool '{}' (tool names are case-sensitive PascalCase; may be an MCP tool — verify spelling)",
                        info.path, base_name
                    ),
                );
            }
            // S067: unscoped Bash (no Bash(pattern) form)
            if tool == "Bash" {
                diag.report(
                    LintRule::BashUnscoped,
                    &format!(
                        "{}: allowed-tools lists unscoped Bash; prefer scoped form like Bash(git:*)",
                        info.path
                    ),
                );
            }
        }
    }
}
