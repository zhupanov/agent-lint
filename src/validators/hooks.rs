use crate::context::{LintContext, ManifestState};
use crate::diagnostic::DiagnosticCollector;
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
/// Filters for strings containing ${CLAUDE_PLUGIN_ROOT}/ or $PWD/ ending in .sh,
/// then verifies each resolved path exists on disk and is executable.
fn validate_hook_command_paths(val: &Value, label: &str, diag: &mut DiagnosticCollector) {
    let strings = extract_all_strings(val);
    for raw in &strings {
        let is_plugin_root = raw.contains("${CLAUDE_PLUGIN_ROOT}/");
        let is_pwd = raw.contains("$PWD/");
        if (!is_plugin_root && !is_pwd) || !raw.ends_with(".sh") {
            continue;
        }

        let rel = if is_plugin_root {
            raw.replace("${CLAUDE_PLUGIN_ROOT}/", "")
        } else {
            raw.replace("$PWD/", "")
        };

        if rel == *raw {
            continue; // defensive: no prefix stripped
        }

        let path = Path::new(&rel);
        if !path.is_file() {
            diag.fail(&format!("{label}: hook command missing on disk: {raw}"));
            continue;
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = path.metadata() {
                if meta.permissions().mode() & 0o111 == 0 {
                    diag.fail(&format!("{label}: hook command not executable: {raw}"));
                }
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
