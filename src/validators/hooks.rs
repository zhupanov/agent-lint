use crate::context::{LintContext, ManifestState};
use crate::diagnostic::DiagnosticCollector;
use regex::Regex;
use serde_json::Value;
use std::path::Path;

/// Recursively collect all string values from a JSON value.
/// Equivalent to jq '.. | strings'.
fn extract_all_strings(value: &Value) -> Vec<String> {
    let mut result = Vec::new();
    collect_strings(value, &mut result);
    result
}

fn collect_strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(s) => out.push(s.clone()),
        Value::Array(arr) => {
            for item in arr {
                collect_strings(item, out);
            }
        }
        Value::Object(map) => {
            for (_, v) in map {
                collect_strings(v, out);
            }
        }
        _ => {}
    }
}

/// Validate hook command paths in a parsed JSON value.
/// Extracts script paths matching ${CLAUDE_PLUGIN_ROOT}/...sh or $PWD/...sh
/// from all string values, then verifies each resolved path exists and is executable.
fn validate_hook_command_paths(val: &Value, label: &str, diag: &mut DiagnosticCollector) {
    let re_plugin = Regex::new(r"\$\{CLAUDE_PLUGIN_ROOT\}/[a-zA-Z0-9._/-]+\.sh").unwrap();
    let re_pwd = Regex::new(r"\$PWD/[a-zA-Z0-9._/-]+\.sh").unwrap();

    let strings = extract_all_strings(val);
    for raw in &strings {
        // Extract script paths from the string using regex (handles commands with arguments)
        for cap in re_plugin.find_iter(raw) {
            let reference = cap.as_str();
            let rel = reference.replacen("${CLAUDE_PLUGIN_ROOT}/", "", 1);
            check_hook_path(&rel, reference, label, diag);
        }
        for cap in re_pwd.find_iter(raw) {
            let reference = cap.as_str();
            let rel = reference.replacen("$PWD/", "", 1);
            check_hook_path(&rel, reference, label, diag);
        }
    }
}

fn check_hook_path(rel: &str, reference: &str, label: &str, diag: &mut DiagnosticCollector) {
    let path = Path::new(rel);
    if !path.is_file() {
        diag.fail(&format!(
            "{label}: hook command missing on disk: {reference}"
        ));
        return;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = path.metadata() {
            if meta.permissions().mode() & 0o111 == 0 {
                diag.fail(&format!(
                    "{label}: hook command not executable: {reference}"
                ));
            }
        }
    }
}

/// V3: Validate hooks/hooks.json
pub fn validate_hooks_json(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    let f = "hooks/hooks.json";
    let val = match &ctx.hooks_json {
        ManifestState::Missing => {
            diag.fail(&format!("{f} is missing"));
            return;
        }
        ManifestState::Invalid(e) => {
            diag.fail(e);
            return;
        }
        ManifestState::Parsed(v) => v,
    };

    if val.get("hooks").is_none() {
        diag.fail(&format!("{f} missing top-level 'hooks' key"));
    }

    validate_hook_command_paths(val, f, diag);
}

/// V4: Validate .claude/settings.json hook command paths
pub fn validate_settings_hooks(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    let val = match &ctx.settings_json {
        ManifestState::Missing => return, // Optional file
        ManifestState::Invalid(e) => {
            diag.fail(e);
            return;
        }
        ManifestState::Parsed(v) => v,
    };

    validate_hook_command_paths(val, ".claude/settings.json", diag);
}
