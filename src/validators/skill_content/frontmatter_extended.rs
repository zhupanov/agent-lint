use crate::diagnostic::DiagnosticCollector;
use crate::rules::LintRule;
use crate::validators::common::{is_known_tool_name, tokenize_tool_field};
use crate::validators::skills::SkillInfo;
use crate::yaml::Mapping;
use std::collections::HashSet;

/// The two skill tool-declaration fields validated by S040 (and S067 for
/// `allowed-tools`). Both accept a space- or comma-separated string or a YAML
/// list per the Claude Code skills reference.
const TOOL_FIELDS: &[&str] = &["allowed-tools", "disallowed-tools"];

pub(super) fn check_frontmatter_extended(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    // S035, S039, S043, and the S040/S067 tool rules read canonical YAML
    // values so comments, quoting, and multiline scalars cannot corrupt the
    // compared value. Invalid or non-mapping frontmatter is owned by
    // X001/S004/S005, so they skip it.
    if let Some(map) = info.frontmatter_mapping() {
        check_compatibility_length(info, map, diag);
        check_metadata_values(info, map, diag);
        check_frontmatter_backslash(info, map, diag);
        check_tool_fields(info, map, diag);
    }

    // S042 (dmi-empty-desc) is soft-retired: it was a strict subset of
    // S005/frontmatter-field-missing and no longer fires from any path.
    // S045 (tools-list-syntax) is soft-retired: a YAML list is a documented
    // accepted `allowed-tools` spelling, not a mistake, and its autofix could
    // corrupt valid YAML. Both registry codes/names remain as deprecated,
    // config-only identifiers.
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

/// S040 / S067: unrecognized or unscoped tool declarations.
///
/// Every documented spelling of `allowed-tools` and `disallowed-tools` — a
/// space- or comma-separated string or a YAML list — is tokenized by the
/// shared tool tokenizer, so `Bash(npm install, npm test)` stays one entry.
/// S040 reports each unknown entry once per (field, name) per file; S067
/// fires when any `allowed-tools` entry is exactly `Bash` (denying all of
/// Bash via `disallowed-tools` is not a scoping problem).
fn check_tool_fields(info: &SkillInfo, map: &Mapping, diag: &mut DiagnosticCollector) {
    let mut reported: HashSet<(&str, String)> = HashSet::new();
    for &field in TOOL_FIELDS {
        let Some(value) = map.get(field) else {
            continue;
        };
        let entries = tokenize_tool_field(value);
        for entry in &entries {
            // Base name is reported (argument-restriction suffix like
            // "Bash(git *)" stripped).
            let base_name = match entry.find('(') {
                Some(paren) => entry[..paren].trim(),
                None => entry.as_str(),
            };
            if base_name.is_empty() {
                continue;
            }
            if !is_known_tool_name(entry) && reported.insert((field, base_name.to_owned())) {
                diag.report(
                    LintRule::ToolsUnknown,
                    &format!(
                        "{}: {field} lists unrecognized tool '{base_name}' (tool names are case-sensitive PascalCase; may be an MCP tool — verify spelling)",
                        info.path
                    ),
                );
            }
        }
        // S067: unscoped Bash (no Bash(pattern) form) in allowed-tools only.
        if field == "allowed-tools" && entries.iter().any(|entry| entry == "Bash") {
            diag.report(
                LintRule::BashUnscoped,
                &format!(
                    "{}: allowed-tools lists unscoped Bash; prefer scoped form like Bash(git *)",
                    info.path
                ),
            );
        }
    }
}
