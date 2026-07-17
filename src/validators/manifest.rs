use crate::context::{LintContext, ManifestState};
use crate::diagnostic::DiagnosticCollector;
use crate::rules::LintRule;
use crate::validators::common::is_valid_http_url;
use regex::Regex;
use serde_json::Value;
use std::path::Path;
use std::sync::LazyLock;

static RE_SEMVER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[0-9]+\.[0-9]+\.[0-9]+$").unwrap());

/// Windows drive-letter path prefix, e.g. `C:\` or `c:/`.
static RE_WIN_DRIVE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z]:[\\/]").unwrap());

/// The plugin manifest directory. Components must never live under it.
const PLUGIN_DIR: &str = ".claude-plugin";

/// plugin.json fields that point at plugin components.
const COMPONENT_FIELDS: &[&str] = &["commands", "agents", "skills", "hooks"];

/// Split a manifest path on POSIX and Windows separators, dropping empty and
/// `.` segments so `./foo` and `foo` agree.
fn path_segments(p: &str) -> impl Iterator<Item = &str> {
    p.split(['/', '\\']).filter(|s| !s.is_empty() && *s != ".")
}

/// Whether a manifest path is absolute rather than plugin-root-relative.
fn is_absolute_path(p: &str) -> bool {
    p.starts_with('/') || p.starts_with('\\') || RE_WIN_DRIVE.is_match(p)
}

/// Whether an optional JSON value is a string with non-whitespace content.
fn is_non_empty_string(v: Option<&Value>) -> bool {
    v.and_then(Value::as_str)
        .is_some_and(|s| !s.trim().is_empty())
}

/// Collect the declared paths of a plugin.json component field, which may hold
/// a single path or an array of paths. Non-path shapes (such as an inline
/// `hooks` object) yield nothing.
fn component_paths(val: &Value, field: &str) -> Vec<String> {
    match val.get(field) {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

/// V1: Validate .claude-plugin/plugin.json
pub fn validate_plugin_json(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    let f = ".claude-plugin/plugin.json";
    let val = match &ctx.plugin_json {
        ManifestState::Missing => {
            diag.report(LintRule::PluginJsonMissing, &format!("{f} is missing"));
            return;
        }
        ManifestState::Invalid(e) => {
            diag.report(LintRule::PluginJsonInvalid, e);
            return;
        }
        ManifestState::Parsed(v) => v,
    };

    let name = val.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let version = val.get("version").and_then(|v| v.as_str()).unwrap_or("");

    // An absent, empty, or whitespace-only name all mean "no name".
    if name.trim().is_empty() {
        diag.report(
            LintRule::PluginFieldMissing,
            &format!("{f} missing required field: name"),
        );
    }
    if version.is_empty() {
        diag.report(
            LintRule::PluginFieldMissing,
            &format!("{f} missing required field: version"),
        );
    } else {
        if !RE_SEMVER.is_match(version) {
            diag.report(
                LintRule::PluginVersionFormat,
                &format!("{f} version '{version}' is not strict MAJOR.MINOR.PATCH semver"),
            );
        }
    }
}

/// V2: Validate .claude-plugin/marketplace.json
pub fn validate_marketplace_json(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    let f = ".claude-plugin/marketplace.json";
    let val = match &ctx.marketplace_json {
        ManifestState::Missing => {
            diag.report(LintRule::MarketplaceJsonMissing, &format!("{f} is missing"));
            return;
        }
        ManifestState::Invalid(e) => {
            diag.report(LintRule::MarketplaceJsonInvalid, e);
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
        diag.report(
            LintRule::MarketplaceFieldMissing,
            &format!("{f} missing required field: name"),
        );
    }
    if mp_owner.is_empty() {
        diag.report(
            LintRule::MarketplaceFieldMissing,
            &format!("{f} missing required field: owner.name"),
        );
    }

    let plugins = val.get("plugins").and_then(|v| v.as_array());
    match plugins {
        None => {
            diag.report(
                LintRule::MarketplacePluginsEmpty,
                &format!("{f} has empty plugins array"),
            );
        }
        Some(arr) if arr.is_empty() => {
            diag.report(
                LintRule::MarketplacePluginsEmpty,
                &format!("{f} has empty plugins array"),
            );
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
                    diag.report(
                        LintRule::MarketplacePluginInvalid,
                        &format!(
                            "{f} has plugin entry with missing/invalid name or source (plugins[{i}])"
                        ),
                    );
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

    if val.get("owner").and_then(|o| o.get("email")).is_none() {
        diag.report(
            LintRule::MarketplaceEnrichedMissing,
            &format!("{f} missing required field: owner.email"),
        );
    }

    if let Some(plugins) = val.get("plugins").and_then(|v| v.as_array()) {
        for (i, plugin) in plugins.iter().enumerate() {
            let cat = plugin
                .get("category")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if cat.is_empty() {
                diag.report(
                    LintRule::MarketplaceEnrichedMissing,
                    &format!("{f} plugins[{i}] missing required field: category"),
                );
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
        diag.report(
            LintRule::PluginEnrichedMissing,
            &format!("{f} missing required field: description"),
        );
    }

    if val.get("author").and_then(|o| o.get("email")).is_none() {
        diag.report(
            LintRule::PluginEnrichedMissing,
            &format!("{f} missing required field: author.email"),
        );
    }

    // keywords must be a non-empty array
    match val.get("keywords") {
        Some(kw) if kw.is_array() && !kw.as_array().unwrap().is_empty() => {}
        _ => {
            diag.report(
                LintRule::PluginEnrichedMissing,
                &format!("{f} keywords must be a non-empty array"),
            );
        }
    }
}

/// V29: Validate plugin component paths — both the on-disk layout (M012) and
/// the paths declared in plugin.json (M012, M013).
pub fn validate_component_paths(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    let f = ".claude-plugin/plugin.json";

    // M012: components must not physically live under the manifest directory.
    for field in COMPONENT_FIELDS {
        if Path::new(PLUGIN_DIR).join(field).is_dir() {
            diag.report(
                LintRule::ComponentPathNested,
                &format!("{PLUGIN_DIR}/{field}/ must not live inside {PLUGIN_DIR}/"),
            );
        }
    }

    let val = match &ctx.plugin_json {
        ManifestState::Parsed(v) => v,
        _ => return, // Missing/invalid already reported by V1
    };

    for field in COMPONENT_FIELDS {
        for p in component_paths(val, field) {
            // M013: an absolute or escaping path is rejected outright — where it
            // would land is not meaningful, so M012 is not also evaluated.
            if is_absolute_path(&p) {
                diag.report(
                    LintRule::ComponentPathUnsafe,
                    &format!("{f} {field} path '{p}' must be relative, not absolute"),
                );
            } else if path_segments(&p).any(|s| s == "..") {
                diag.report(
                    LintRule::ComponentPathUnsafe,
                    &format!("{f} {field} path '{p}' must not use '..' traversal"),
                );
            } else if path_segments(&p).next() == Some(PLUGIN_DIR) {
                diag.report(
                    LintRule::ComponentPathNested,
                    &format!("{f} {field} path '{p}' must not point inside {PLUGIN_DIR}/"),
                );
            }
        }
    }
}

/// V30: Validate optional plugin.json metadata (M014, M015).
pub fn validate_plugin_metadata(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    let f = ".claude-plugin/plugin.json";
    let val = match &ctx.plugin_json {
        ManifestState::Parsed(v) => v,
        _ => return, // Missing/invalid already reported by V1
    };

    // M014: an author object must name the author. A bare author string is a
    // different, accepted shape and carries no name field to check.
    if let Some(author) = val.get("author") {
        if author.is_object() && !is_non_empty_string(author.get("name")) {
            diag.report(
                LintRule::AuthorNameMissing,
                &format!("{f} author.name missing or invalid (must be a non-empty string)"),
            );
        }
    }

    // M015: homepage is optional, but must be a usable http(s) URL when set.
    if let Some(homepage) = val.get("homepage") {
        let url = homepage.as_str().unwrap_or("");
        if !is_valid_http_url(url) {
            diag.report(
                LintRule::HomepageUrlInvalid,
                &format!("{f} homepage '{url}' is not a valid http(s) URL"),
            );
        }
    }
}

/// V31: Validate plugin.json lspServers entries (M016).
pub fn validate_lsp_servers(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    let f = ".claude-plugin/plugin.json";
    let val = match &ctx.plugin_json {
        ManifestState::Parsed(v) => v,
        _ => return, // Missing/invalid already reported by V1
    };

    let servers = match val.get("lspServers").and_then(|v| v.as_object()) {
        Some(m) => m,
        None => return,
    };

    for (name, entry) in servers {
        let has_command = is_non_empty_string(entry.get("command"));
        let has_extensions = entry
            .get("extensionToLanguage")
            .is_some_and(|v| v.is_object());
        if !has_command || !has_extensions {
            diag.report(
                LintRule::LspServerInvalid,
                &format!(
                    "{f} lspServers.{name} has missing/invalid command or extensionToLanguage"
                ),
            );
        }
    }
}

/// V32: Validate plugin.json channels entries (M017).
///
/// `channels` is accepted both as an object keyed by channel name and as an
/// array of entries; either way every entry must name a `server`.
pub fn validate_channels(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    let f = ".claude-plugin/plugin.json";
    let val = match &ctx.plugin_json {
        ManifestState::Parsed(v) => v,
        _ => return, // Missing/invalid already reported by V1
    };

    let entries: Vec<(String, &Value)> = match val.get("channels") {
        Some(Value::Object(map)) => map
            .iter()
            .map(|(k, v)| (format!("channels.{k}"), v))
            .collect(),
        Some(Value::Array(arr)) => arr
            .iter()
            .enumerate()
            .map(|(i, v)| (format!("channels[{i}]"), v))
            .collect(),
        _ => return,
    };

    for (label, entry) in entries {
        if !is_non_empty_string(entry.get("server")) {
            diag.report(
                LintRule::ChannelServerMissing,
                &format!("{f} {label} missing required field: server"),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::LintMode;
    use serde_json::json;

    fn make_ctx(plugin: ManifestState, marketplace: ManifestState) -> LintContext {
        LintContext {
            base_path: std::path::PathBuf::new(),
            mode: LintMode::Plugin,
            plugin_json: plugin,
            marketplace_json: marketplace,
            hooks_json: ManifestState::Missing,
            settings_json: ManifestState::Missing,
            settings_local_json: ManifestState::Missing,
        }
    }

    // V1: validate_plugin_json
    #[test]
    fn test_v1_valid_plugin_json() {
        let val = json!({"name": "my-plugin", "version": "1.2.3"});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_plugin_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_v1_missing_plugin_json() {
        let ctx = make_ctx(ManifestState::Missing, ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_plugin_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("is missing"));
    }

    #[test]
    fn test_v1_invalid_plugin_json() {
        let ctx = make_ctx(
            ManifestState::Invalid("parse error".to_string()),
            ManifestState::Missing,
        );
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_plugin_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("parse error"));
    }

    #[test]
    fn test_v1_missing_name() {
        let val = json!({"version": "1.0.0"});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_plugin_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("name"));
    }

    #[test]
    fn test_v1_invalid_semver() {
        let val = json!({"name": "p", "version": "not-a-version"});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_plugin_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("semver"));
    }

    #[test]
    fn test_v1_missing_version() {
        let val = json!({"name": "p"});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_plugin_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("version"));
    }

    // V2: validate_marketplace_json
    #[test]
    fn test_v2_valid_marketplace_json() {
        let val = json!({
            "name": "mp",
            "owner": {"name": "owner-name"},
            "plugins": [{"name": "p1", "source": "https://example.com"}]
        });
        let ctx = make_ctx(ManifestState::Missing, ManifestState::Parsed(val));
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_marketplace_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_v2_missing_marketplace_json() {
        let ctx = make_ctx(ManifestState::Missing, ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_marketplace_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("is missing"));
    }

    #[test]
    fn test_v2_empty_plugins_array() {
        let val = json!({"name": "mp", "owner": {"name": "o"}, "plugins": []});
        let ctx = make_ctx(ManifestState::Missing, ManifestState::Parsed(val));
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_marketplace_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("empty plugins array"));
    }

    #[test]
    fn test_v2_missing_owner_name() {
        let val = json!({
            "name": "mp",
            "owner": {},
            "plugins": [{"name": "p", "source": "s"}]
        });
        let ctx = make_ctx(ManifestState::Missing, ManifestState::Parsed(val));
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_marketplace_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("owner.name"));
    }

    #[test]
    fn test_v2_plugin_entry_missing_source() {
        let val = json!({
            "name": "mp",
            "owner": {"name": "o"},
            "plugins": [{"name": "p"}]
        });
        let ctx = make_ctx(ManifestState::Missing, ManifestState::Parsed(val));
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_marketplace_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("plugins[0]"));
    }

    // V12: validate_marketplace_enriched
    #[test]
    fn test_v12_valid_enriched() {
        let val = json!({
            "name": "mp",
            "owner": {"name": "o", "email": "a@b.com"},
            "plugins": [{"name": "p", "source": "s", "category": "lint"}]
        });
        let ctx = make_ctx(ManifestState::Missing, ManifestState::Parsed(val));
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_marketplace_enriched(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_v12_missing_owner_email() {
        let val = json!({
            "name": "mp",
            "owner": {"name": "o"},
            "plugins": [{"name": "p", "source": "s", "category": "lint"}]
        });
        let ctx = make_ctx(ManifestState::Missing, ManifestState::Parsed(val));
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_marketplace_enriched(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("owner.email"));
    }

    #[test]
    fn test_v12_missing_category() {
        let val = json!({
            "name": "mp",
            "owner": {"name": "o", "email": "a@b.com"},
            "plugins": [{"name": "p", "source": "s"}]
        });
        let ctx = make_ctx(ManifestState::Missing, ManifestState::Parsed(val));
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_marketplace_enriched(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("category"));
    }

    #[test]
    fn test_v12_non_string_email_no_missing_report() {
        let val = json!({
            "owner": {"name": "o", "email": 42},
            "plugins": [{"name": "p", "source": "s", "category": "lint"}]
        });
        let ctx = make_ctx(ManifestState::Missing, ManifestState::Parsed(val));
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_marketplace_enriched(&ctx, &mut diag);
        assert_eq!(
            diag.error_count(),
            0,
            "non-string email should not fire M010"
        );
    }

    #[test]
    fn test_v12_skips_when_not_parsed() {
        let ctx = make_ctx(ManifestState::Missing, ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_marketplace_enriched(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    // V13: validate_plugin_enriched
    #[test]
    fn test_v13_valid_enriched() {
        let val = json!({
            "name": "p",
            "version": "1.0.0",
            "description": "A plugin",
            "author": {"email": "a@b.com"},
            "keywords": ["lint"]
        });
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_plugin_enriched(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_v13_missing_description() {
        let val = json!({
            "name": "p",
            "version": "1.0.0",
            "author": {"email": "a@b.com"},
            "keywords": ["lint"]
        });
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_plugin_enriched(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("description"));
    }

    #[test]
    fn test_v13_empty_keywords() {
        let val = json!({
            "name": "p",
            "version": "1.0.0",
            "description": "desc",
            "author": {"email": "a@b.com"},
            "keywords": []
        });
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_plugin_enriched(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("keywords"));
    }

    #[test]
    fn test_v13_non_string_email_no_missing_report() {
        let val = json!({
            "description": "desc",
            "author": {"email": true},
            "keywords": ["lint"]
        });
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_plugin_enriched(&ctx, &mut diag);
        assert_eq!(
            diag.error_count(),
            0,
            "non-string email should not fire M011"
        );
    }

    #[test]
    fn test_v13_skips_when_not_parsed() {
        let ctx = make_ctx(ManifestState::Missing, ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_plugin_enriched(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    // ── M003: empty and whitespace-only name ────────────────────────

    #[test]
    fn test_m003_empty_string_name_fires() {
        let val = json!({"name": "", "version": "1.0.0"});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_plugin_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("missing required field: name"));
    }

    #[test]
    fn test_m003_whitespace_only_name_fires() {
        let val = json!({"name": "   ", "version": "1.0.0"});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_plugin_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("missing required field: name"));
    }

    #[test]
    fn test_m003_non_string_name_fires() {
        let val = json!({"name": 42, "version": "1.0.0"});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_plugin_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("missing required field: name"));
    }

    // ── M013: component-path-unsafe ─────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_m013_absolute_path_fires() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let val = json!({"name": "p", "version": "1.0.0", "commands": "/etc/commands"});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_component_paths(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("must be relative"));
    }

    #[test]
    #[serial_test::serial]
    fn test_m013_windows_drive_path_fires() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let val = json!({"name": "p", "version": "1.0.0", "agents": "C:\\agents"});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_component_paths(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("must be relative"));
    }

    #[test]
    #[serial_test::serial]
    fn test_m013_traversal_escape_fires() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        // Acceptance-criteria edge case: a path that escapes the plugin root.
        let val = json!({"name": "p", "version": "1.0.0", "skills": "foo/../../etc"});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_component_paths(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("'..' traversal"));
    }

    #[test]
    #[serial_test::serial]
    fn test_m013_inner_traversal_fires() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        // Resolves back inside the root, but '..' is still rejected: write "bar".
        let val = json!({"name": "p", "version": "1.0.0", "skills": "foo/../bar"});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_component_paths(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("'..' traversal"));
    }

    #[test]
    #[serial_test::serial]
    fn test_m013_array_paths_checked_individually() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let val = json!({"name": "p", "version": "1.0.0", "agents": ["agents", "/abs", "../up"]});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_component_paths(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 2);
    }

    #[test]
    #[serial_test::serial]
    fn test_m013_relative_paths_pass() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let val = json!({
            "name": "p",
            "version": "1.0.0",
            "commands": "./commands",
            "agents": ["agents", "extra/agents"],
            "skills": "skills"
        });
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_component_paths(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    // ── M012: component-path-nested ─────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_m012_manifest_path_inside_plugin_dir_fires() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let val = json!({"name": "p", "version": "1.0.0", "skills": ".claude-plugin/skills"});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_component_paths(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("must not point inside .claude-plugin/"));
    }

    #[test]
    #[serial_test::serial]
    fn test_m012_dot_slash_prefix_still_detected() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let val = json!({"name": "p", "version": "1.0.0", "agents": "./.claude-plugin/agents"});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_component_paths(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("must not point inside .claude-plugin/"));
    }

    #[test]
    #[serial_test::serial]
    fn test_m012_physical_layout_fires() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude-plugin/skills").unwrap();
        let val = json!({"name": "p", "version": "1.0.0"});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_component_paths(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains(".claude-plugin/skills/ must not live inside"));
    }

    #[test]
    #[serial_test::serial]
    fn test_m012_physical_layout_reported_without_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude-plugin/agents").unwrap();
        // The on-disk layout is checked even when plugin.json is unusable.
        let ctx = make_ctx(ManifestState::Missing, ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_component_paths(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains(".claude-plugin/agents/ must not live inside"));
    }

    #[test]
    #[serial_test::serial]
    fn test_m012_manifests_beside_components_pass() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        // A correct layout: manifests inside .claude-plugin/, components outside.
        std::fs::create_dir_all(".claude-plugin").unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(".claude-plugin/plugin.json", "{}").unwrap();
        let val = json!({"name": "p", "version": "1.0.0"});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_component_paths(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_m012_inline_hooks_object_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        // An inline hooks object declares no path, so there is nothing to check.
        let val = json!({"name": "p", "version": "1.0.0", "hooks": {"PreToolUse": []}});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_component_paths(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    // ── M014: author-name-missing ───────────────────────────────────

    #[test]
    fn test_m014_author_object_without_name_fires() {
        let val = json!({"name": "p", "version": "1.0.0", "author": {"email": "a@b.com"}});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_plugin_metadata(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("author.name"));
    }

    #[test]
    fn test_m014_author_object_with_name_passes() {
        let val = json!({"name": "p", "author": {"name": "Ada", "email": "a@b.com"}});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_plugin_metadata(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_m014_blank_and_non_string_name_fire() {
        for author in [json!({"name": "   "}), json!({"name": 42})] {
            let val = json!({"name": "p", "author": author});
            let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_plugin_metadata(&ctx, &mut diag);
            assert_eq!(diag.error_count(), 1);
        }
    }

    #[test]
    fn test_m014_author_string_and_absent_pass() {
        for val in [
            json!({"name": "p", "author": "Ada Lovelace"}),
            json!({"name": "p"}),
        ] {
            let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_plugin_metadata(&ctx, &mut diag);
            assert_eq!(diag.error_count(), 0);
        }
    }

    // ── M015: homepage-url-invalid ──────────────────────────────────

    #[test]
    fn test_m015_valid_urls_pass() {
        for url in [
            "https://example.com",
            "http://example.com",
            "https://example.com/a/b?c=d#e",
        ] {
            let val = json!({"name": "p", "homepage": url});
            let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_plugin_metadata(&ctx, &mut diag);
            assert_eq!(diag.error_count(), 0, "expected {url} to be accepted");
        }
    }

    #[test]
    fn test_m015_invalid_urls_fire() {
        for url in [
            json!("ftp://example.com"),
            json!("example.com"),
            json!("https://"),
            json!("not a url"),
            json!(""),
            json!(42),
        ] {
            let val = json!({"name": "p", "homepage": url});
            let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_plugin_metadata(&ctx, &mut diag);
            assert_eq!(diag.error_count(), 1, "expected {url} to be rejected");
        }
    }

    #[test]
    fn test_m015_absent_homepage_passes() {
        let val = json!({"name": "p", "version": "1.0.0"});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_plugin_metadata(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    // ── M016: lsp-server-invalid ────────────────────────────────────

    #[test]
    fn test_m016_valid_lsp_server_passes() {
        let val = json!({
            "lspServers": {
                "rust-analyzer": {
                    "command": "rust-analyzer",
                    "extensionToLanguage": {".rs": "rust"}
                }
            }
        });
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_lsp_servers(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_m016_missing_command_fires() {
        let val = json!({
            "lspServers": {"pyright": {"extensionToLanguage": {".py": "python"}}}
        });
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_lsp_servers(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("lspServers.pyright"));
    }

    #[test]
    fn test_m016_missing_extension_map_fires() {
        let val = json!({"lspServers": {"pyright": {"command": "pyright-langserver"}}});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_lsp_servers(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("extensionToLanguage"));
    }

    #[test]
    fn test_m016_one_report_per_entry() {
        // Both fields bad in one entry still reports once; a valid sibling is quiet.
        let val = json!({
            "lspServers": {
                "bad": {"command": "  ", "extensionToLanguage": []},
                "good": {"command": "ok", "extensionToLanguage": {".x": "x"}}
            }
        });
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_lsp_servers(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("lspServers.bad"));
    }

    #[test]
    fn test_m016_absent_lsp_servers_passes() {
        let val = json!({"name": "p", "version": "1.0.0"});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_lsp_servers(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    // ── M017: channel-server-missing ────────────────────────────────

    #[test]
    fn test_m017_object_channels() {
        let val = json!({"channels": {"alerts": {"server": "my-server"}}});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_channels(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);

        let val = json!({"channels": {"alerts": {"topic": "x"}}});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_channels(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("channels.alerts"));
    }

    #[test]
    fn test_m017_array_channels() {
        let val = json!({"channels": [{"server": "s"}, {"name": "no-server"}]});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_channels(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("channels[1]"));
    }

    #[test]
    fn test_m017_blank_server_fires() {
        let val = json!({"channels": {"alerts": {"server": "   "}}});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_channels(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
    }

    #[test]
    fn test_m017_absent_channels_passes() {
        let val = json!({"name": "p", "version": "1.0.0"});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_channels(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_new_validators_skip_when_not_parsed() {
        let ctx = make_ctx(ManifestState::Missing, ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_plugin_metadata(&ctx, &mut diag);
        validate_lsp_servers(&ctx, &mut diag);
        validate_channels(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }
}
