use crate::context::{LintContext, ManifestState};
use crate::diagnostic::DiagnosticCollector;
use crate::rules::LintRule;
use crate::validators::common::{is_valid_http_url, manifest_error_metadata};
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

/// Semantic Versioning 2.0.0, including optional pre-release and build
/// metadata. Numeric identifiers cannot have leading zeroes.
static RE_SEMVER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)(?:-(?:0|[1-9][0-9]*|[0-9A-Za-z-]*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9][0-9]*|[0-9A-Za-z-]*[A-Za-z-][0-9A-Za-z-]*))*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$",
    )
    .unwrap()
});

/// Windows drive-letter path prefix, e.g. `C:\` or `c:/`.
static RE_WIN_DRIVE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z]:[\\/]").unwrap());

/// Marketplace / plugin entry name kebab-case: `[a-z0-9]+(-[a-z0-9]+)*`.
static RE_MARKETPLACE_KEBAB: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9]+(-[a-z0-9]+)*$").unwrap());

/// The plugin manifest directory. Components must never live under it.
const PLUGIN_DIR: &str = ".claude-plugin";

/// A path-bearing plugin.json field and its location in the JSON object.
struct ComponentPathField {
    label: &'static str,
    keys: &'static [&'static str],
}

/// plugin.json fields that may point at plugin components. Inline object forms
/// are intentionally excluded: they configure a component rather than declare
/// a component path.
const COMPONENT_PATH_FIELDS: &[ComponentPathField] = &[
    ComponentPathField {
        label: "commands",
        keys: &["commands"],
    },
    ComponentPathField {
        label: "agents",
        keys: &["agents"],
    },
    ComponentPathField {
        label: "skills",
        keys: &["skills"],
    },
    ComponentPathField {
        label: "hooks",
        keys: &["hooks"],
    },
    ComponentPathField {
        label: "mcpServers",
        keys: &["mcpServers"],
    },
    ComponentPathField {
        label: "outputStyles",
        keys: &["outputStyles"],
    },
    ComponentPathField {
        label: "lspServers",
        keys: &["lspServers"],
    },
    ComponentPathField {
        label: "experimental.themes",
        keys: &["experimental", "themes"],
    },
    ComponentPathField {
        label: "experimental.monitors",
        keys: &["experimental", "monitors"],
    },
];

/// Directories that Claude Code requires at the plugin root rather than under
/// the manifest directory.
const COMPONENT_DIRECTORIES: &[&str] = &[
    "commands",
    "agents",
    "skills",
    "hooks",
    "output-styles",
    "themes",
    "monitors",
];

/// Known marketplace plugin object-source types and their required fields.
const OBJECT_SOURCE_REQUIRED: &[(&str, &[&str])] = &[
    ("github", &["repo"]),
    ("url", &["url"]),
    ("git-subdir", &["url", "path"]),
    ("npm", &["package"]),
];

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
fn component_paths<'a>(val: &'a Value, field: &ComponentPathField) -> Vec<&'a str> {
    let value = field
        .keys
        .iter()
        .try_fold(val, |value, key| value.get(*key));

    match value {
        Some(Value::String(s)) => vec![s],
        Some(Value::Array(arr)) => arr.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

/// V1: Validate .claude-plugin/plugin.json
pub fn validate_plugin_json(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    let f = ".claude-plugin/plugin.json";
    let val = match &ctx.plugin_json {
        ManifestState::Missing => {
            if matches!(
                &ctx.marketplace_json,
                ManifestState::Parsed(_) | ManifestState::Invalid(_)
            ) {
                return;
            }
            diag.report(LintRule::PluginJsonMissing, &format!("{f} is missing"));
            return;
        }
        ManifestState::Invalid(e) => {
            diag.report_with(
                LintRule::PluginJsonInvalid,
                e.message(),
                manifest_error_metadata(e),
            );
            return;
        }
        ManifestState::Parsed(v) => v,
    };

    let name = val.get("name").and_then(|v| v.as_str()).unwrap_or("");

    // An absent, empty, or whitespace-only name all mean "no name".
    if name.trim().is_empty() {
        diag.report(
            LintRule::PluginFieldMissing,
            &format!("{f} missing required field: name"),
        );
    }
    match val.get("version") {
        None => diag.report(
            LintRule::PluginVersionMissing,
            &format!("{f} omits optional field: version"),
        ),
        Some(Value::String(version)) if RE_SEMVER.is_match(version) => {}
        Some(_) => {
            diag.report(
                LintRule::PluginVersionFormat,
                &format!("{f} version is not valid Semantic Versioning 2.0.0"),
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
            diag.report_with(
                LintRule::MarketplaceJsonInvalid,
                e.message(),
                manifest_error_metadata(e),
            );
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

    if mp_name.trim().is_empty() {
        diag.report(
            LintRule::MarketplaceFieldMissing,
            &format!("{f} missing required field: name"),
        );
    } else {
        let trimmed = mp_name.trim();
        if !trimmed.is_empty() && !RE_MARKETPLACE_KEBAB.is_match(trimmed) {
            diag.report(
                LintRule::MarketplaceNameFormat,
                &format!("{f} name '{mp_name}' is not kebab-case ([a-z0-9]+(-[a-z0-9]+)*)"),
            );
        }
    }
    if mp_owner.trim().is_empty() {
        diag.report(
            LintRule::MarketplaceFieldMissing,
            &format!("{f} missing required field: owner.name"),
        );
    }

    match val.get("plugins") {
        None => {
            diag.report(
                LintRule::MarketplaceFieldMissing,
                &format!("{f} missing required field: plugins"),
            );
        }
        Some(plugins) if !plugins.is_array() => {
            diag.report(
                LintRule::MarketplacePluginsEmpty,
                &format!(
                    "{f} plugins must be an array (found {})",
                    json_type(plugins)
                ),
            );
        }
        Some(plugins) if plugins.as_array().is_some_and(|arr| arr.is_empty()) => {
            diag.report(
                LintRule::MarketplacePluginsEmpty,
                &format!("{f} has empty plugins array"),
            );
        }
        Some(plugins) => {
            let arr = plugins
                .as_array()
                .expect("the preceding branch established plugins is an array");
            let has_plugin_root = val
                .get("metadata")
                .and_then(|m| m.get("pluginRoot"))
                .and_then(|v| v.as_str())
                .is_some();
            let mut name_indexes: HashMap<String, Vec<usize>> = HashMap::new();

            for (i, plugin) in arr.iter().enumerate() {
                let pname = plugin
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let name_missing = pname.is_empty();
                if !name_missing {
                    name_indexes.entry(pname.to_string()).or_default().push(i);
                    if !RE_MARKETPLACE_KEBAB.is_match(pname) {
                        diag.report(
                            LintRule::MarketplaceNameFormat,
                            &format!(
                                "{f} plugins[{i}] name '{pname}' is not kebab-case ([a-z0-9]+(-[a-z0-9]+)*)"
                            ),
                        );
                    }
                }

                let source_missing_or_wrong = match plugin.get("source") {
                    None => true,
                    Some(Value::String(s)) => s.trim().is_empty(),
                    Some(Value::Object(_)) => false,
                    Some(_) => true,
                };
                if name_missing || source_missing_or_wrong {
                    diag.report(
                        LintRule::MarketplacePluginInvalid,
                        &format!(
                            "{f} has plugin entry with missing/invalid name or source (plugins[{i}])"
                        ),
                    );
                }

                match plugin.get("source") {
                    Some(Value::String(s)) => {
                        let s = s.trim();
                        if s.is_empty() {
                            // Already reported as missing/invalid above.
                        } else if is_absolute_path(s) {
                            diag.report(
                                LintRule::MarketplacePluginInvalid,
                                &format!(
                                    "{f} plugins[{i}].source path '{s}' must be relative, not absolute"
                                ),
                            );
                        } else if path_segments(s).any(|seg| seg == "..") {
                            diag.report(
                                LintRule::MarketplacePluginInvalid,
                                &format!(
                                    "{f} plugins[{i}].source path '{s}' must not use '..' traversal"
                                ),
                            );
                        } else if !s.starts_with("./") && !has_plugin_root {
                            diag.report(
                                LintRule::MarketplaceBarePath,
                                &format!(
                                    "{f} plugins[{i}].source '{s}' should start with './' (or set metadata.pluginRoot)"
                                ),
                            );
                        }
                    }
                    Some(Value::Object(obj)) => {
                        validate_object_plugin_source(f, i, obj, diag);
                    }
                    _ => {}
                }
            }

            let mut duplicates: Vec<(String, Vec<usize>)> = name_indexes
                .into_iter()
                .filter(|(_, idxs)| idxs.len() > 1)
                .collect();
            duplicates.sort_by(|a, b| a.1[0].cmp(&b.1[0]).then_with(|| a.0.cmp(&b.0)));
            for (name, idxs) in duplicates {
                let indexes = idxs
                    .iter()
                    .map(|i| format!("plugins[{i}]"))
                    .collect::<Vec<_>>()
                    .join(", ");
                diag.report(
                    LintRule::MarketplacePluginInvalid,
                    &format!("{f} duplicate plugin name \"{name}\" ({indexes})"),
                );
            }
        }
    }
}

/// Validate a marketplace plugin entry whose `source` is a JSON object.
fn validate_object_plugin_source(
    f: &str,
    i: usize,
    obj: &serde_json::Map<String, Value>,
    diag: &mut DiagnosticCollector,
) {
    let source_type = obj.get("source").and_then(|v| v.as_str()).unwrap_or("");
    if source_type.is_empty() {
        diag.report(
            LintRule::MarketplacePluginInvalid,
            &format!("{f} plugins[{i}].source missing required field: source"),
        );
        return;
    }
    let Some((_, required)) = OBJECT_SOURCE_REQUIRED
        .iter()
        .find(|(ty, _)| *ty == source_type)
    else {
        diag.report(
            LintRule::MarketplacePluginInvalid,
            &format!(
                "{f} plugins[{i}].source unknown source type '{source_type}' (expected github, url, git-subdir, or npm)"
            ),
        );
        return;
    };
    for field in *required {
        if !is_non_empty_string(obj.get(*field)) {
            diag.report(
                LintRule::MarketplacePluginInvalid,
                &format!(
                    "{f} plugins[{i}].source {source_type} source requires non-empty \"{field}\""
                ),
            );
        }
    }
}

fn json_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
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
    for field in COMPONENT_DIRECTORIES {
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

    for field in COMPONENT_PATH_FIELDS {
        for p in component_paths(val, field) {
            // M013: an absolute or escaping path is rejected outright — where it
            // would land is not meaningful, so M012 is not also evaluated.
            if is_absolute_path(p) {
                diag.report(
                    LintRule::ComponentPathUnsafe,
                    &format!(
                        "{f} {} path '{p}' must be relative, not absolute",
                        field.label
                    ),
                );
            } else if path_segments(p).any(|s| s == "..") {
                diag.report(
                    LintRule::ComponentPathUnsafe,
                    &format!("{f} {} path '{p}' must not use '..' traversal", field.label),
                );
            } else if path_segments(p).next() == Some(PLUGIN_DIR) {
                diag.report(
                    LintRule::ComponentPathNested,
                    &format!(
                        "{f} {} path '{p}' must not point inside {PLUGIN_DIR}/",
                        field.label
                    ),
                );
            }
        }
    }
}

/// V30: Validate optional plugin.json metadata (M014, M015, M020).
pub fn validate_plugin_metadata(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    let f = ".claude-plugin/plugin.json";
    let val = match &ctx.plugin_json {
        ManifestState::Parsed(v) => v,
        _ => return, // Missing/invalid already reported by V1
    };

    // M014 owns an incomplete author object; M020 owns every other present
    // author shape because Claude Code accepts only an object here.
    if let Some(author) = val.get("author") {
        if !author.is_object() {
            diag.report(
                LintRule::AuthorTypeInvalid,
                &format!("{f} author must be an object (found {})", json_type(author)),
            );
        } else if !is_non_empty_string(author.get("name")) {
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
/// array of entries; either way every entry must name a `server`. When the
/// manifest declares inline MCP servers, that name must refer to one of them.
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
    let inline_servers = val.get("mcpServers").and_then(Value::as_object);

    for (label, entry) in entries {
        let server = entry
            .get("server")
            .and_then(Value::as_str)
            .filter(|server| !server.trim().is_empty());
        let Some(server) = server else {
            diag.report(
                LintRule::ChannelServerMissing,
                &format!("{f} {label} missing required field: server"),
            );
            continue;
        };
        if inline_servers.is_some_and(|servers| !servers.contains_key(server)) {
            diag.report(
                LintRule::ChannelServerMissing,
                &format!("{f} {label} server '{server}' does not reference an mcpServers entry"),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::LintMode;
    use serde_json::{Value, json};

    fn make_ctx(plugin: ManifestState, marketplace: ManifestState) -> LintContext {
        LintContext {
            base_path: std::path::PathBuf::new(),
            mode: LintMode::Plugin,
            plugin_json: plugin,
            marketplace_json: marketplace,
            hooks_json: ManifestState::Missing,
            declared_hook_configs: vec![],
            settings_json: ManifestState::Missing,
            settings_local_json: ManifestState::Missing,
        }
    }

    fn manifest_with_component_path(field: &ComponentPathField, path: Value) -> Value {
        let mut manifest = json!({"name": "p", "version": "1.0.0"});
        match field.keys {
            [key] => manifest[*key] = path,
            [parent, key] => manifest[*parent] = json!({*key: path}),
            _ => unreachable!("component path fields have one or two keys"),
        }
        manifest
    }

    // V1: validate_plugin_json
    #[test]
    fn test_v1_valid_plugin_json() {
        let val = json!({"name": "my-plugin", "version": "1.2.3"});
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
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
    fn test_v1_missing_plugin_json_with_marketplace_does_not_report() {
        for marketplace in [
            ManifestState::parsed(json!({})),
            ManifestState::invalid("parse error"),
        ] {
            let ctx = make_ctx(ManifestState::Missing, marketplace);
            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_plugin_json(&ctx, &mut diag);
            assert!(diag.diagnostics().is_empty());
        }
    }

    #[test]
    fn test_v1_invalid_plugin_json() {
        let ctx = make_ctx(
            ManifestState::invalid("parse error"),
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
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_plugin_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("name"));
    }

    #[test]
    fn test_v1_semver_2_0_0_versions_are_accepted() {
        for version in ["1.0.0", "1.0.0-beta.1", "1.0.0+build.5", "1.0.0-rc.1+build"] {
            let val = json!({"name": "p", "version": version});
            let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_plugin_json(&ctx, &mut diag);
            assert!(
                diag.diagnostics().is_empty(),
                "expected {version} to be accepted"
            );
        }
    }

    #[test]
    fn test_v1_invalid_semver_2_0_0_versions_are_rejected() {
        for version in ["01.2.3", "1.2", "v1.2.3", "1.0.0-", " 1.0.0 "] {
            let val = json!({"name": "p", "version": version});
            let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_plugin_json(&ctx, &mut diag);
            assert_eq!(diag.error_count(), 1, "expected {version} to be rejected");
            assert_eq!(diag.diagnostics()[0].rule, LintRule::PluginVersionFormat);
        }
    }

    #[test]
    fn test_v1_missing_version_is_a_warning() {
        let val = json!({"name": "p"});
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new();
        validate_plugin_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
        assert_eq!(diag.warning_count(), 1);
        assert_eq!(diag.diagnostics()[0].rule, LintRule::PluginVersionMissing);
    }

    // V2: validate_marketplace_json
    #[test]
    fn test_v2_valid_marketplace_json() {
        let val = json!({
            "name": "mp",
            "owner": {"name": "owner-name"},
            "plugins": [{"name": "p1", "source": "./plugins/p1"}]
        });
        let ctx = make_ctx(ManifestState::Missing, ManifestState::parsed(val));
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_marketplace_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_v2_missing_marketplace_json() {
        let ctx = make_ctx(ManifestState::Missing, ManifestState::Missing);
        let mut diag = DiagnosticCollector::new();
        validate_marketplace_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
        assert_eq!(diag.warning_count(), 1);
        assert_eq!(diag.diagnostics()[0].rule, LintRule::MarketplaceJsonMissing);
    }

    #[test]
    fn test_v2_plugins_shape_failures_have_distinct_diagnostics() {
        let cases = [
            (
                json!({"name": "mp", "owner": {"name": "o"}}),
                LintRule::MarketplaceFieldMissing,
                "missing required field: plugins",
            ),
            (
                json!({"name": "mp", "owner": {"name": "o"}, "plugins": {}}),
                LintRule::MarketplacePluginsEmpty,
                "plugins must be an array (found object)",
            ),
            (
                json!({"name": "mp", "owner": {"name": "o"}, "plugins": []}),
                LintRule::MarketplacePluginsEmpty,
                "has empty plugins array",
            ),
        ];
        for (value, rule, message) in cases {
            let ctx = make_ctx(ManifestState::Missing, ManifestState::parsed(value));
            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_marketplace_json(&ctx, &mut diag);
            assert_eq!(diag.diagnostics().len(), 1);
            assert_eq!(diag.diagnostics()[0].rule, rule);
            assert!(diag.diagnostics()[0].message.contains(message));
        }
    }

    #[test]
    fn test_v2_empty_plugins_array_is_a_warning() {
        let val = json!({"name": "mp", "owner": {"name": "o"}, "plugins": []});
        let ctx = make_ctx(ManifestState::Missing, ManifestState::parsed(val));
        let mut diag = DiagnosticCollector::new();
        validate_marketplace_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
        assert_eq!(diag.warning_count(), 1);
        assert_eq!(
            diag.diagnostics()[0].rule,
            LintRule::MarketplacePluginsEmpty
        );
    }

    #[test]
    fn test_v2_missing_owner_name() {
        let val = json!({
            "name": "mp",
            "owner": {},
            "plugins": [{"name": "p", "source": "./p"}]
        });
        let ctx = make_ctx(ManifestState::Missing, ManifestState::parsed(val));
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
        let ctx = make_ctx(ManifestState::Missing, ManifestState::parsed(val));
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_marketplace_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("plugins[0]"));
    }

    #[test]
    fn test_v2_plugin_source_shapes_table() {
        type Expect<'a> = &'a [(LintRule, &'a str)];
        let cases: &[(&str, serde_json::Value, Expect<'_>)] = &[
            (
                "relative_ok",
                json!({"name": "p", "source": "./plugins/x"}),
                &[],
            ),
            (
                "github_ok",
                json!({"name": "p", "source": {"source": "github", "repo": "org/repo"}}),
                &[],
            ),
            (
                "url_ok",
                json!({"name": "p", "source": {"source": "url", "url": "https://example.com/p.git"}}),
                &[],
            ),
            (
                "git_subdir_ok",
                json!({"name": "p", "source": {"source": "git-subdir", "url": "https://example.com/m.git", "path": "plugins/p"}}),
                &[],
            ),
            (
                "npm_ok",
                json!({"name": "p", "source": {"source": "npm", "package": "@scope/pkg"}}),
                &[],
            ),
            (
                "github_with_optional",
                json!({"name": "p", "source": {"source": "github", "repo": "org/repo", "ref": "main", "sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}),
                &[],
            ),
            (
                "github_missing_repo",
                json!({"name": "p", "source": {"source": "github"}}),
                &[(
                    LintRule::MarketplacePluginInvalid,
                    "requires non-empty \"repo\"",
                )],
            ),
            (
                "url_missing_url",
                json!({"name": "p", "source": {"source": "url"}}),
                &[(
                    LintRule::MarketplacePluginInvalid,
                    "requires non-empty \"url\"",
                )],
            ),
            (
                "git_subdir_missing_path",
                json!({"name": "p", "source": {"source": "git-subdir", "url": "https://example.com/m.git"}}),
                &[(
                    LintRule::MarketplacePluginInvalid,
                    "requires non-empty \"path\"",
                )],
            ),
            (
                "npm_missing_package",
                json!({"name": "p", "source": {"source": "npm"}}),
                &[(
                    LintRule::MarketplacePluginInvalid,
                    "requires non-empty \"package\"",
                )],
            ),
            (
                "unknown_type",
                json!({"name": "p", "source": {"source": "nonsense-type"}}),
                &[(LintRule::MarketplacePluginInvalid, "unknown source type")],
            ),
            (
                "missing_source_type",
                json!({"name": "p", "source": {"repo": "org/repo"}}),
                &[(
                    LintRule::MarketplacePluginInvalid,
                    "missing required field: source",
                )],
            ),
            (
                "non_object_non_string",
                json!({"name": "p", "source": 42}),
                &[(
                    LintRule::MarketplacePluginInvalid,
                    "missing/invalid name or source",
                )],
            ),
            (
                "traversal",
                json!({"name": "p", "source": "../outside"}),
                &[(LintRule::MarketplacePluginInvalid, "'..' traversal")],
            ),
            (
                "posix_absolute",
                json!({"name": "p", "source": "/abs"}),
                &[(LintRule::MarketplacePluginInvalid, "must be relative")],
            ),
            (
                "windows_absolute",
                json!({"name": "p", "source": "C:\\x"}),
                &[(LintRule::MarketplacePluginInvalid, "must be relative")],
            ),
            (
                "bare_without_root",
                json!({"name": "p", "source": "plugins/x"}),
                &[(LintRule::MarketplaceBarePath, "should start with './'")],
            ),
            (
                "whitespace_name",
                json!({"name": "  ", "source": "./p"}),
                &[(
                    LintRule::MarketplacePluginInvalid,
                    "missing/invalid name or source",
                )],
            ),
            (
                "whitespace_source",
                json!({"name": "p", "source": "  "}),
                &[(
                    LintRule::MarketplacePluginInvalid,
                    "missing/invalid name or source",
                )],
            ),
        ];

        for (label, plugin, expected) in cases {
            let val = json!({
                "name": "mp",
                "owner": {"name": "o"},
                "plugins": [plugin]
            });
            let ctx = make_ctx(ManifestState::Missing, ManifestState::parsed(val));
            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_marketplace_json(&ctx, &mut diag);
            let got: Vec<_> = diag
                .diagnostics()
                .iter()
                .map(|d| (d.rule, d.message.clone()))
                .collect();
            assert_eq!(
                got.len(),
                expected.len(),
                "{label}: unexpected diagnostics {got:?}"
            );
            for (rule, needle) in *expected {
                assert!(
                    got.iter().any(|(r, msg)| r == rule && msg.contains(needle)),
                    "{label}: missing {rule:?} containing {needle:?} in {got:?}"
                );
            }
        }
    }

    #[test]
    fn test_v2_bare_path_clean_with_plugin_root() {
        let val = json!({
            "name": "mp",
            "owner": {"name": "o"},
            "metadata": {"pluginRoot": "./plugins"},
            "plugins": [{"name": "p", "source": "plugins/x"}]
        });
        let ctx = make_ctx(ManifestState::Missing, ManifestState::parsed(val));
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_marketplace_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0, "{:?}", diag.diagnostics());
    }

    #[test]
    fn test_v2_duplicate_plugin_names() {
        let val = json!({
            "name": "mp",
            "owner": {"name": "o"},
            "plugins": [
                {"name": "a", "source": "./a"},
                {"name": "a", "source": "./a2"},
                {"name": "b", "source": "./b"},
                {"name": "b", "source": "./b2"},
                {"name": "b", "source": "./b3"}
            ]
        });
        let ctx = make_ctx(ManifestState::Missing, ManifestState::parsed(val));
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_marketplace_json(&ctx, &mut diag);
        let dups: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|d| {
                d.rule == LintRule::MarketplacePluginInvalid && d.message.contains("duplicate")
            })
            .map(|d| d.message.as_str())
            .collect();
        assert_eq!(dups.len(), 2, "{dups:?}");
        assert!(dups[0].contains("\"a\"") && dups[0].contains("plugins[0], plugins[1]"));
        assert!(
            dups[1].contains("\"b\"") && dups[1].contains("plugins[2], plugins[3], plugins[4]")
        );
    }

    #[test]
    fn test_v2_evidence_manifest_catches_four_conditions() {
        let val = json!({
            "name": "m",
            "owner": {"name": "o"},
            "plugins": [
                {"name": "a", "source": "../outside"},
                {"name": "a", "source": {"source": "github"}},
                {"name": "b", "source": {"source": "nonsense-type"}}
            ]
        });
        let ctx = make_ctx(ManifestState::Missing, ManifestState::parsed(val));
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_marketplace_json(&ctx, &mut diag);
        let msgs: Vec<_> = diag
            .diagnostics()
            .iter()
            .map(|d| d.message.as_str())
            .collect();
        assert!(
            msgs.iter().any(|m| m.contains("'..' traversal")),
            "{msgs:?}"
        );
        assert!(
            msgs.iter()
                .any(|m| m.contains("duplicate plugin name \"a\"")),
            "{msgs:?}"
        );
        assert!(
            msgs.iter()
                .any(|m| m.contains("requires non-empty \"repo\"")),
            "{msgs:?}"
        );
        assert!(
            msgs.iter().any(|m| m.contains("unknown source type")),
            "{msgs:?}"
        );
    }

    #[test]
    fn test_v2_name_format_warnings() {
        let cases: &[(&str, serde_json::Value, bool)] = &[
            (
                "top_ok",
                json!({"name": "a", "owner": {"name": "o"}, "plugins": [{"name": "a-b2", "source": "./p"}]}),
                false,
            ),
            (
                "top_bad",
                json!({"name": "My_Plugin", "owner": {"name": "o"}, "plugins": [{"name": "p", "source": "./p"}]}),
                true,
            ),
            (
                "entry_upper",
                json!({"name": "mp", "owner": {"name": "o"}, "plugins": [{"name": "UPPER", "source": "./p"}]}),
                true,
            ),
            (
                "entry_double_hyphen",
                json!({"name": "mp", "owner": {"name": "o"}, "plugins": [{"name": "a--b", "source": "./p"}]}),
                true,
            ),
        ];
        for (label, val, expect_warn) in cases {
            let ctx = make_ctx(ManifestState::Missing, ManifestState::parsed(val.clone()));
            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_marketplace_json(&ctx, &mut diag);
            let has = diag
                .diagnostics()
                .iter()
                .any(|d| d.rule == LintRule::MarketplaceNameFormat);
            assert_eq!(has, *expect_warn, "{label}");
        }
    }

    // V12: validate_marketplace_enriched
    #[test]
    fn test_v12_valid_enriched() {
        let val = json!({
            "name": "mp",
            "owner": {"name": "o", "email": "a@b.com"},
            "plugins": [{"name": "p", "source": "s", "category": "lint"}]
        });
        let ctx = make_ctx(ManifestState::Missing, ManifestState::parsed(val));
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
        let ctx = make_ctx(ManifestState::Missing, ManifestState::parsed(val));
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
        let ctx = make_ctx(ManifestState::Missing, ManifestState::parsed(val));
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
        let ctx = make_ctx(ManifestState::Missing, ManifestState::parsed(val));
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
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
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
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
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
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
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
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
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
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_plugin_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("missing required field: name"));
    }

    #[test]
    fn test_m003_whitespace_only_name_fires() {
        let val = json!({"name": "   ", "version": "1.0.0"});
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_plugin_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("missing required field: name"));
    }

    #[test]
    fn test_m003_non_string_name_fires() {
        let val = json!({"name": 42, "version": "1.0.0"});
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
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
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
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
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
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
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
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
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
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
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
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
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_component_paths(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_component_path_fields_check_string_and_array_forms() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        let cases = [
            (
                "absolute takes precedence over traversal and nesting",
                "/etc/../.claude-plugin/components",
                LintRule::ComponentPathUnsafe,
                "must be relative, not absolute",
            ),
            (
                "Windows drive",
                r"C:\\components",
                LintRule::ComponentPathUnsafe,
                "must be relative, not absolute",
            ),
            (
                "leading backslash",
                r"\\components",
                LintRule::ComponentPathUnsafe,
                "must be relative, not absolute",
            ),
            (
                "escaping traversal",
                "components/../../outside",
                LintRule::ComponentPathUnsafe,
                "must not use '..' traversal",
            ),
            (
                "non-escaping traversal",
                "components/../other",
                LintRule::ComponentPathUnsafe,
                "must not use '..' traversal",
            ),
            (
                "nested manifest directory",
                ".claude-plugin/components",
                LintRule::ComponentPathNested,
                "must not point inside .claude-plugin/",
            ),
        ];

        for field in COMPONENT_PATH_FIELDS {
            for (case_name, path, rule, expected_message) in cases {
                for (shape, value) in [
                    ("string", json!(path)),
                    ("array", json!(["components", path])),
                ] {
                    let ctx = make_ctx(
                        ManifestState::parsed(manifest_with_component_path(field, value)),
                        ManifestState::Missing,
                    );
                    let mut diag = DiagnosticCollector::new_all_enabled();
                    validate_component_paths(&ctx, &mut diag);

                    assert_eq!(diag.error_count(), 1, "{} {shape} {case_name}", field.label);
                    assert_eq!(diag.diagnostics()[0].rule, rule);
                    assert!(diag.errors()[0].contains(&format!("{} path '{path}'", field.label)));
                    assert!(diag.errors()[0].contains(expected_message));
                }
            }

            for (shape, value) in [
                ("string", json!("components")),
                ("array", json!(["components", "other-components"])),
            ] {
                let ctx = make_ctx(
                    ManifestState::parsed(manifest_with_component_path(field, value)),
                    ManifestState::Missing,
                );
                let mut diag = DiagnosticCollector::new_all_enabled();
                validate_component_paths(&ctx, &mut diag);
                assert_eq!(diag.error_count(), 0, "{} clean {shape}", field.label);
            }
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_component_path_diagnostics_are_ordered_by_field_then_index() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        let mut manifest = json!({"name": "p", "version": "1.0.0"});
        for field in COMPONENT_PATH_FIELDS {
            let paths = json!(["/first", "../second"]);
            match field.keys {
                [key] => manifest[*key] = paths,
                [parent, key] => manifest[*parent][*key] = paths,
                _ => unreachable!("component path fields have one or two keys"),
            }
        }

        let ctx = make_ctx(ManifestState::parsed(manifest), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_component_paths(&ctx, &mut diag);

        let expected: Vec<String> = COMPONENT_PATH_FIELDS
            .iter()
            .flat_map(|field| {
                ["/first", "../second"]
                    .into_iter()
                    .map(move |path| format!("{} path '{path}'", field.label))
            })
            .collect();
        assert_eq!(diag.error_count(), expected.len());
        for (diagnostic, expected) in diag.errors().iter().zip(expected) {
            assert!(diagnostic.contains(&expected), "{diagnostic}");
        }
    }

    // ── M012: component-path-nested ─────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_m012_manifest_path_inside_plugin_dir_fires() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let val = json!({"name": "p", "version": "1.0.0", "skills": ".claude-plugin/skills"});
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
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
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
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
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
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
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
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
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_component_paths(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_component_path_inline_objects_are_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let val = json!({
            "name": "p",
            "version": "1.0.0",
            "mcpServers": {"local": {"command": "server"}},
            "lspServers": {"rust": {"command": "rust-analyzer"}},
            "experimental": {
                "themes": {"dark": {"name": "dark"}},
                "monitors": [{"matcher": "Bash"}]
            }
        });
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_component_paths(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_m012_new_forbidden_component_directories_fire_once() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        for directory in ["output-styles", "themes", "monitors"] {
            std::fs::create_dir_all(Path::new(PLUGIN_DIR).join(directory)).unwrap();
        }

        let ctx = make_ctx(
            ManifestState::parsed(json!({"name": "p", "version": "1.0.0"})),
            ManifestState::Missing,
        );
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_component_paths(&ctx, &mut diag);

        assert_eq!(diag.error_count(), 3);
        for directory in ["output-styles", "themes", "monitors"] {
            assert_eq!(
                diag.errors()
                    .iter()
                    .filter(|message| message.contains(&format!("{PLUGIN_DIR}/{directory}/")))
                    .count(),
                1
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_m012_new_component_directories_at_plugin_root_pass() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        for directory in ["output-styles", "themes", "monitors"] {
            std::fs::create_dir_all(directory).unwrap();
        }

        let ctx = make_ctx(
            ManifestState::parsed(json!({"name": "p", "version": "1.0.0"})),
            ManifestState::Missing,
        );
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_component_paths(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    // ── M014: author-name-missing ───────────────────────────────────

    #[test]
    fn test_m014_author_object_without_name_fires() {
        let val = json!({"name": "p", "version": "1.0.0", "author": {"email": "a@b.com"}});
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_plugin_metadata(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("author.name"));
    }

    #[test]
    fn test_m014_author_object_with_name_passes() {
        let val = json!({"name": "p", "author": {"name": "Ada", "email": "a@b.com"}});
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_plugin_metadata(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_m014_blank_and_non_string_name_fire() {
        for author in [json!({"name": "   "}), json!({"name": 42})] {
            let val = json!({"name": "p", "author": author});
            let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_plugin_metadata(&ctx, &mut diag);
            assert_eq!(diag.error_count(), 1);
        }
    }

    #[test]
    fn test_m020_non_object_authors_are_errors() {
        for author in [
            json!("Ada Lovelace <ada@example.com>"),
            json!(42),
            json!([]),
            json!(true),
            json!(null),
        ] {
            let val = json!({"name": "p", "author": author});
            let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
            let mut diag = DiagnosticCollector::new();
            validate_plugin_metadata(&ctx, &mut diag);
            assert_eq!(diag.error_count(), 1);
            assert_eq!(diag.diagnostics()[0].rule, LintRule::AuthorTypeInvalid);
        }
    }

    #[test]
    fn test_m020_absent_author_passes() {
        let ctx = make_ctx(
            ManifestState::parsed(json!({"name": "p"})),
            ManifestState::Missing,
        );
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_plugin_metadata(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_m007_and_m009_whitespace_only_values_fire() {
        let val = json!({
            "name": "   ",
            "owner": {"name": "\t"},
            "plugins": [{"name": " ", "source": "\n"}]
        });
        let ctx = make_ctx(ManifestState::Missing, ManifestState::parsed(val));
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_marketplace_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 3);
        assert_eq!(
            diag.diagnostics()
                .iter()
                .filter(|diagnostic| diagnostic.rule == LintRule::MarketplaceFieldMissing)
                .count(),
            2
        );
        assert_eq!(
            diag.diagnostics()[2].rule,
            LintRule::MarketplacePluginInvalid
        );
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
            let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
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
            let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_plugin_metadata(&ctx, &mut diag);
            assert_eq!(diag.error_count(), 1, "expected {url} to be rejected");
        }
    }

    #[test]
    fn test_m015_absent_homepage_passes() {
        let val = json!({"name": "p", "version": "1.0.0"});
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
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
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_lsp_servers(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_m016_missing_command_fires() {
        let val = json!({
            "lspServers": {"pyright": {"extensionToLanguage": {".py": "python"}}}
        });
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_lsp_servers(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("lspServers.pyright"));
    }

    #[test]
    fn test_m016_missing_extension_map_fires() {
        let val = json!({"lspServers": {"pyright": {"command": "pyright-langserver"}}});
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
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
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_lsp_servers(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("lspServers.bad"));
    }

    #[test]
    fn test_m016_absent_lsp_servers_passes() {
        let val = json!({"name": "p", "version": "1.0.0"});
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_lsp_servers(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    // ── M017: channel-server-missing ────────────────────────────────

    #[test]
    fn test_m017_object_channels() {
        let val = json!({"channels": {"alerts": {"server": "my-server"}}});
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_channels(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);

        let val = json!({"channels": {"alerts": {"topic": "x"}}});
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_channels(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("channels.alerts"));
    }

    #[test]
    fn test_m017_array_channels() {
        let val = json!({"channels": [{"server": "s"}, {"name": "no-server"}, "not-an-object"]});
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_channels(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 2);
        assert!(diag.errors()[0].contains("channels[1]"));
        assert!(diag.errors()[1].contains("channels[2]"));
    }

    #[test]
    fn test_m017_blank_server_fires() {
        let val = json!({"channels": {"alerts": {"server": "   "}}});
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_channels(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
    }

    #[test]
    fn test_m017_inline_mcp_server_reference_must_exist() {
        let val = json!({
            "mcpServers": {"existing": {"command": "server"}},
            "channels": {"alerts": {"server": "missing"}}
        });
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_channels(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert_eq!(diag.diagnostics()[0].rule, LintRule::ChannelServerMissing);
        assert!(diag.diagnostics()[0].message.contains("does not reference"));
    }

    #[test]
    fn test_m017_matching_or_external_mcp_servers_skip_cross_check() {
        for val in [
            json!({
                "mcpServers": {"existing": {"command": "server"}},
                "channels": {"alerts": {"server": "existing"}}
            }),
            json!({
                "mcpServers": "./servers.json",
                "channels": {"alerts": {"server": "external"}}
            }),
        ] {
            let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_channels(&ctx, &mut diag);
            assert_eq!(diag.error_count(), 0);
        }
    }

    #[test]
    fn test_m017_absent_channels_passes() {
        let val = json!({"name": "p", "version": "1.0.0"});
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
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
