use crate::context::{LintContext, ManifestState};
use crate::diagnostic::DiagnosticCollector;
use regex::Regex;

/// V1: Validate .claude-plugin/plugin.json
pub fn validate_plugin_json(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    let f = ".claude-plugin/plugin.json";
    let val = match &ctx.plugin_json {
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

    let name = val.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let version = val.get("version").and_then(|v| v.as_str()).unwrap_or("");

    if name.is_empty() {
        diag.fail(&format!("{f} missing required field: name"));
    }
    if version.is_empty() {
        diag.fail(&format!("{f} missing required field: version"));
    } else {
        let semver_re = Regex::new(r"^[0-9]+\.[0-9]+\.[0-9]+$").unwrap();
        if !semver_re.is_match(version) {
            diag.fail(&format!(
                "{f} version '{version}' is not strict MAJOR.MINOR.PATCH semver"
            ));
        }
    }
}

/// V2: Validate .claude-plugin/marketplace.json
pub fn validate_marketplace_json(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    let f = ".claude-plugin/marketplace.json";
    let val = match &ctx.marketplace_json {
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

    let mp_name = val.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let mp_owner = val
        .get("owner")
        .and_then(|o| o.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if mp_name.is_empty() {
        diag.fail(&format!("{f} missing required field: name"));
    }
    if mp_owner.is_empty() {
        diag.fail(&format!("{f} missing required field: owner.name"));
    }

    let plugins = val.get("plugins").and_then(|v| v.as_array());
    match plugins {
        None => {
            diag.fail(&format!("{f} has empty plugins array"));
        }
        Some(arr) if arr.is_empty() => {
            diag.fail(&format!("{f} has empty plugins array"));
        }
        Some(arr) => {
            for (i, plugin) in arr.iter().enumerate() {
                let pname = plugin.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let has_source = match plugin.get("source") {
                    Some(s) => {
                        (s.is_string() && !s.as_str().unwrap_or("").is_empty()) || s.is_object()
                    }
                    None => false,
                };
                if pname.is_empty() || !has_source {
                    diag.fail(&format!(
                        "{f} has plugin entry with missing/invalid name or source (plugins[{i}])"
                    ));
                }
            }
        }
    }
}

/// V12: Validate marketplace.json enriched metadata (larch convention)
pub fn validate_marketplace_enriched(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    let f = ".claude-plugin/marketplace.json";
    let val = match &ctx.marketplace_json {
        ManifestState::Parsed(v) => v,
        _ => return, // Missing/invalid already reported by V2
    };

    let email = val
        .get("owner")
        .and_then(|o| o.get("email"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if email.is_empty() {
        diag.fail(&format!("{f} missing required field: owner.email"));
    }

    if let Some(plugins) = val.get("plugins").and_then(|v| v.as_array()) {
        for (i, plugin) in plugins.iter().enumerate() {
            let cat = plugin.get("category").and_then(|v| v.as_str()).unwrap_or("");
            if cat.is_empty() {
                diag.fail(&format!("{f} plugins[{i}] missing required field: category"));
            }
        }
    }
}

/// V13: Validate plugin.json enriched metadata (larch convention)
pub fn validate_plugin_enriched(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    let f = ".claude-plugin/plugin.json";
    let val = match &ctx.plugin_json {
        ManifestState::Parsed(v) => v,
        _ => return, // Missing/invalid already reported by V1
    };

    let desc = val
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if desc.is_empty() {
        diag.fail(&format!("{f} missing required field: description"));
    }

    let email = val
        .get("author")
        .and_then(|o| o.get("email"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if email.is_empty() {
        diag.fail(&format!("{f} missing required field: author.email"));
    }

    // keywords must be a non-empty array
    match val.get("keywords") {
        Some(kw) if kw.is_array() && !kw.as_array().unwrap().is_empty() => {}
        _ => {
            diag.fail(&format!("{f} keywords must be a non-empty array"));
        }
    }
}
