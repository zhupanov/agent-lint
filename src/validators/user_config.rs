use crate::context::{LintContext, ManifestState};
use crate::diagnostic::DiagnosticCollector;
use std::path::Path;
use walkdir::WalkDir;

/// Helper: get the userConfig object if it exists and is valid.
/// Returns None if plugin.json is missing/invalid, userConfig is absent,
/// or userConfig is not an object.
fn get_user_config<'a>(
    ctx: &'a LintContext,
    diag: &mut DiagnosticCollector,
) -> Option<&'a serde_json::Map<String, serde_json::Value>> {
    let f = ".claude-plugin/plugin.json";
    let val = match &ctx.plugin_json {
        ManifestState::Parsed(v) => v,
        _ => return None, // Missing/invalid already reported by V1
    };

    // Check if userConfig exists (distinguish absent from null)
    let uc = val.get("userConfig")?;

    match uc {
        serde_json::Value::Object(map) => Some(map),
        _ => {
            diag.fail(&format!("{f} userConfig must be an object"));
            None
        }
    }
}

/// V18: userConfig structure — each key must have a description that is a non-empty string.
pub fn validate_userconfig_structure(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    let f = ".claude-plugin/plugin.json";
    let map = match get_user_config(ctx, diag) {
        Some(m) => m,
        None => return,
    };

    for key in map.keys() {
        let entry = &map[key];
        match entry.get("description") {
            Some(desc) if desc.is_string() && !desc.as_str().unwrap_or("").is_empty() => {}
            _ => {
                diag.fail(&format!(
                    "{f} userConfig.{key} missing or invalid description (must be a non-empty string)"
                ));
            }
        }
    }
}

/// V20: userConfig key → env var mapping.
/// Every userConfig key must have a corresponding CLAUDE_PLUGIN_OPTION_<UPPER_KEY>
/// reference in at least one scripts/*.sh file.
pub fn validate_userconfig_env_mapping(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    let val = match &ctx.plugin_json {
        ManifestState::Parsed(v) => v,
        _ => return,
    };

    let user_config = match val.get("userConfig").and_then(|v| v.as_object()) {
        Some(m) => m,
        None => return,
    };

    let scripts_dir = Path::new("scripts");
    if !scripts_dir.is_dir() {
        return;
    }

    // Read all script content once
    let mut scripts_content = String::new();
    for entry in WalkDir::new(scripts_dir).into_iter().flatten() {
        if entry.path().is_file() {
            if let Some(name) = entry.path().file_name().and_then(|n| n.to_str()) {
                if name.ends_with(".sh") {
                    if let Ok(content) = std::fs::read_to_string(entry.path()) {
                        scripts_content.push_str(&content);
                        scripts_content.push('\n');
                    }
                }
            }
        }
    }

    for key in user_config.keys() {
        let upper_key = to_upper_snake_case(key);
        let env_var = format!("CLAUDE_PLUGIN_OPTION_{upper_key}");
        if !scripts_content.contains(&env_var) {
            diag.fail(&format!(
                "userConfig key '{key}' has no corresponding {env_var} reference in scripts/"
            ));
        }
    }
}

/// Convert a key to UPPER_SNAKE_CASE:
/// - Replace hyphens and dots with underscores
/// - Insert underscore before uppercase letters (camelCase → CAMEL_CASE)
/// - Uppercase everything
fn to_upper_snake_case(key: &str) -> String {
    let mut result = String::new();
    for (i, c) in key.chars().enumerate() {
        if c == '-' || c == '.' {
            result.push('_');
        } else if c.is_uppercase() && i > 0 {
            let prev = key.chars().nth(i - 1).unwrap_or('_');
            if prev.is_lowercase() {
                result.push('_');
            }
            result.push(c);
        } else {
            result.push(c);
        }
    }
    result.to_uppercase()
}

/// V23: userConfig sensitive type — if a userConfig entry has a "sensitive" field,
/// its value must be a boolean.
pub fn validate_userconfig_sensitive_type(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    let f = ".claude-plugin/plugin.json";
    let val = match &ctx.plugin_json {
        ManifestState::Parsed(v) => v,
        _ => return,
    };

    let user_config = match val.get("userConfig").and_then(|v| v.as_object()) {
        Some(m) => m,
        None => return,
    };

    for (key, entry) in user_config {
        if let Some(sensitive) = entry.get("sensitive") {
            if !sensitive.is_boolean() {
                diag.fail(&format!(
                    "{f} userConfig.{key}.sensitive must be a boolean (true/false)"
                ));
            }
        }
    }
}

/// V24: userConfig title field — every userConfig entry must have a "title" field
/// that is a non-empty string.
pub fn validate_userconfig_title(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    let f = ".claude-plugin/plugin.json";
    let val = match &ctx.plugin_json {
        ManifestState::Parsed(v) => v,
        _ => return,
    };

    let user_config = match val.get("userConfig").and_then(|v| v.as_object()) {
        Some(m) => m,
        None => return,
    };

    for key in user_config.keys() {
        let entry = &user_config[key];
        match entry.get("title") {
            Some(title) if title.is_string() && !title.as_str().unwrap_or("").is_empty() => {}
            _ => {
                diag.fail(&format!(
                    "{f} userConfig.{key} missing or invalid title (must be a non-empty string)"
                ));
            }
        }
    }
}

/// V25: userConfig type field — every userConfig entry must have a "type" field
/// that is a non-empty string.
pub fn validate_userconfig_type(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    let f = ".claude-plugin/plugin.json";
    let val = match &ctx.plugin_json {
        ManifestState::Parsed(v) => v,
        _ => return,
    };

    let user_config = match val.get("userConfig").and_then(|v| v.as_object()) {
        Some(m) => m,
        None => return,
    };

    for key in user_config.keys() {
        let entry = &user_config[key];
        match entry.get("type") {
            Some(t) if t.is_string() && !t.as_str().unwrap_or("").is_empty() => {}
            _ => {
                diag.fail(&format!(
                    "{f} userConfig.{key} missing or invalid type (must be a non-empty string)"
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_upper_snake_case() {
        assert_eq!(to_upper_snake_case("slackBotToken"), "SLACK_BOT_TOKEN");
        assert_eq!(to_upper_snake_case("slack-channel-id"), "SLACK_CHANNEL_ID");
        assert_eq!(to_upper_snake_case("slack.user.id"), "SLACK_USER_ID");
        assert_eq!(to_upper_snake_case("simple"), "SIMPLE");
    }
}
