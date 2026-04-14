use crate::context::{LintContext, ManifestState};
use crate::diagnostic::DiagnosticCollector;
use crate::rules::LintRule;
use std::path::Path;
use walkdir::WalkDir;

fn get_user_config<'a>(
    ctx: &'a LintContext,
    diag: &mut DiagnosticCollector,
) -> Option<&'a serde_json::Map<String, serde_json::Value>> {
    let f = ".claude-plugin/plugin.json";
    let val = match &ctx.plugin_json {
        ManifestState::Parsed(v) => v,
        _ => return None,
    };

    let uc = val.get("userConfig")?;

    match uc {
        serde_json::Value::Object(map) => Some(map),
        _ => {
            diag.report(
                LintRule::UserconfigNotObject,
                &format!("{f} userConfig must be an object"),
            );
            None
        }
    }
}

/// V18: userConfig structure
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
                diag.report(
                    LintRule::UserconfigDescMissing,
                    &format!("{f} userConfig.{key} missing or invalid description (must be a non-empty string)"),
                );
            }
        }
    }
}

/// V20: userConfig key → env var mapping.
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

    // Collect all script content; if scripts/ doesn't exist, this stays empty
    // and every userConfig key will correctly trigger U003.
    let mut scripts_content = String::new();
    if scripts_dir.is_dir() {
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
    }

    for key in user_config.keys() {
        let upper_key = to_upper_snake_case(key);
        let env_var = format!("CLAUDE_PLUGIN_OPTION_{upper_key}");
        if !scripts_content.contains(&env_var) {
            diag.report(
                LintRule::UserconfigEnvMissing,
                &format!(
                    "userConfig key '{key}' has no corresponding {env_var} reference in scripts/"
                ),
            );
        }
    }
}

fn to_upper_snake_case(key: &str) -> String {
    let mut result = String::new();
    let mut prev = '_';
    for c in key.chars() {
        if c == '-' || c == '.' {
            result.push('_');
            prev = '_';
        } else if c.is_uppercase() {
            if prev.is_lowercase() {
                result.push('_');
            }
            result.push(c);
            prev = c;
        } else {
            result.push(c);
            prev = c;
        }
    }
    result.to_uppercase()
}

/// V23: userConfig sensitive type
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
                diag.report(
                    LintRule::UserconfigSensitiveType,
                    &format!("{f} userConfig.{key}.sensitive must be a boolean (true/false)"),
                );
            }
        }
    }
}

/// V24: userConfig title field
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
                diag.report(
                    LintRule::UserconfigTitleMissing,
                    &format!(
                        "{f} userConfig.{key} missing or invalid title (must be a non-empty string)"
                    ),
                );
            }
        }
    }
}

/// V25: userConfig type field
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
                diag.report(
                    LintRule::UserconfigTypeMissing,
                    &format!(
                        "{f} userConfig.{key} missing or invalid type (must be a non-empty string)"
                    ),
                );
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

    fn make_ctx(plugin: ManifestState) -> LintContext {
        LintContext {
            base_path: std::path::PathBuf::new(),
            mode: crate::context::LintMode::Plugin,
            plugin_json: plugin,
            marketplace_json: ManifestState::Missing,
            hooks_json: ManifestState::Missing,
            settings_json: ManifestState::Missing,
        }
    }

    #[test]
    fn test_v18_valid_structure() {
        let val = serde_json::json!({
            "userConfig": {
                "slackBotToken": {"description": "Bot token for Slack"}
            }
        });
        let ctx = make_ctx(ManifestState::Parsed(val));
        let mut diag = DiagnosticCollector::new();
        validate_userconfig_structure(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_v18_missing_description() {
        let val = serde_json::json!({
            "userConfig": {
                "slackBotToken": {"title": "Token"}
            }
        });
        let ctx = make_ctx(ManifestState::Parsed(val));
        let mut diag = DiagnosticCollector::new();
        validate_userconfig_structure(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("description"));
    }

    #[test]
    fn test_v18_no_userconfig_silent() {
        let val = serde_json::json!({"name": "p", "version": "1.0.0"});
        let ctx = make_ctx(ManifestState::Parsed(val));
        let mut diag = DiagnosticCollector::new();
        validate_userconfig_structure(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v20_valid_env_mapping() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("scripts").unwrap();
        std::fs::write(
            "scripts/run.sh",
            "#!/bin/bash\necho $CLAUDE_PLUGIN_OPTION_SLACK_BOT_TOKEN\n",
        )
        .unwrap();

        let val = serde_json::json!({
            "userConfig": {
                "slackBotToken": {"description": "token"}
            }
        });
        let ctx = make_ctx(ManifestState::Parsed(val));
        let mut diag = DiagnosticCollector::new();
        validate_userconfig_env_mapping(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v20_missing_env_mapping() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("scripts").unwrap();
        std::fs::write("scripts/run.sh", "#!/bin/bash\necho hello\n").unwrap();

        let val = serde_json::json!({
            "userConfig": {
                "slackBotToken": {"description": "token"}
            }
        });
        let ctx = make_ctx(ManifestState::Parsed(val));
        let mut diag = DiagnosticCollector::new();
        validate_userconfig_env_mapping(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("CLAUDE_PLUGIN_OPTION_SLACK_BOT_TOKEN"));
    }

    #[test]
    #[serial_test::serial]
    fn test_v20_no_scripts_dir_fires_error() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        // No scripts/ directory at all

        let val = serde_json::json!({
            "userConfig": {
                "slackBotToken": {"description": "token"}
            }
        });
        let ctx = make_ctx(ManifestState::Parsed(val));
        let mut diag = DiagnosticCollector::new();
        validate_userconfig_env_mapping(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("CLAUDE_PLUGIN_OPTION_SLACK_BOT_TOKEN"));
    }

    #[test]
    fn test_v23_valid_sensitive_boolean() {
        let val = serde_json::json!({"userConfig": {"token": {"sensitive": true}}});
        let ctx = make_ctx(ManifestState::Parsed(val));
        let mut diag = DiagnosticCollector::new();
        validate_userconfig_sensitive_type(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_v23_invalid_sensitive_string() {
        let val = serde_json::json!({"userConfig": {"token": {"sensitive": "yes"}}});
        let ctx = make_ctx(ManifestState::Parsed(val));
        let mut diag = DiagnosticCollector::new();
        validate_userconfig_sensitive_type(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("boolean"));
    }

    #[test]
    fn test_v24_valid_title() {
        let val = serde_json::json!({"userConfig": {"token": {"title": "Bot Token"}}});
        let ctx = make_ctx(ManifestState::Parsed(val));
        let mut diag = DiagnosticCollector::new();
        validate_userconfig_title(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_v24_missing_title() {
        let val = serde_json::json!({"userConfig": {"token": {"description": "desc"}}});
        let ctx = make_ctx(ManifestState::Parsed(val));
        let mut diag = DiagnosticCollector::new();
        validate_userconfig_title(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("title"));
    }

    #[test]
    fn test_v25_valid_type() {
        let val = serde_json::json!({"userConfig": {"token": {"type": "string"}}});
        let ctx = make_ctx(ManifestState::Parsed(val));
        let mut diag = DiagnosticCollector::new();
        validate_userconfig_type(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_v25_missing_type() {
        let val = serde_json::json!({"userConfig": {"token": {"description": "desc"}}});
        let ctx = make_ctx(ManifestState::Parsed(val));
        let mut diag = DiagnosticCollector::new();
        validate_userconfig_type(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("type"));
    }
}
