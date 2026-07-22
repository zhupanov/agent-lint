use crate::config::normalize_path;
use crate::context::{LintContext, ManifestState};
use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata, SourceSpan};
use crate::plugin_paths::{
    ComponentPathSafety, classify_component_path, declared_component_paths,
    has_normalized_path_segment, is_absolute_path, path_segments, plugin_root_is_safe,
    safe_component_path,
};
use crate::rules::LintRule;
use crate::validators::codex_surfaces::{JsonScanner, Seg};
use crate::validators::common::{is_valid_http_url, manifest_error_metadata};
use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
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

/// Marketplace / plugin entry name kebab-case: `[a-z0-9]+(-[a-z0-9]+)*`.
static RE_MARKETPLACE_KEBAB: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9]+(-[a-z0-9]+)*$").unwrap());

/// The plugin manifest directory. Components must never live under it.
const PLUGIN_DIR: &str = ".claude-plugin";

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

/// Whether an optional JSON value is a string with non-whitespace content.
fn is_non_empty_string(v: Option<&Value>) -> bool {
    v.and_then(Value::as_str)
        .is_some_and(|s| !s.trim().is_empty())
}

/// The private Claude configuration directory. Plugin component paths that point
/// into it belong to a separately scanned surface, not to the plugin's own tree.
const PRIVATE_CLAUDE_DIR: &str = ".claude";

/// Repository-safe agent root paths explicitly declared in plugin.json `agents`.
///
/// Returns each distinct safe path once, normalized, in declaration order, for
/// the shared agent-file collector. Safety (absolute, `..`-escaping, missing
/// `./` prefix, `.claude-plugin/`-nested) is owned by M012/M013 via the shared
/// [`safe_component_path`] classifier, so only declarations that pass it reach
/// discovery — discovery never escapes the repository nor double-reports a
/// manifest path defect. Paths inside the private `.claude/` tree are also
/// dropped: that tree is a distinct surface with its own recursive scan and
/// `Private` per-agent semantics, so admitting it here would validate the same
/// file twice under two surfaces. Non-string shapes declare no filesystem root
/// and yield nothing. An absent or invalid manifest, or an undeclared `agents`
/// field, yields an empty list; the implicit default `agents/` directory is
/// added by the collector's caller, not here.
pub(crate) fn declared_agent_roots(ctx: &LintContext) -> Vec<String> {
    let ManifestState::Parsed(val) = &ctx.plugin_json else {
        return Vec::new();
    };

    let mut roots = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for declared in declared_component_paths(val) {
        // Only the `agents` field declares agent roots: label "agents" (scalar)
        // or "agents[N]" (array element).
        if declared.label != "agents" && !declared.label.starts_with("agents[") {
            continue;
        }
        // The shared classifier rejects unsafe shapes before any filesystem probe.
        let Some(path) = safe_component_path(declared.raw) else {
            continue;
        };
        let canonical = normalize_path(&path.to_string_lossy());
        // The private `.claude/` tree is scanned separately under its own surface.
        if canonical.is_empty()
            || canonical == PRIVATE_CLAUDE_DIR
            || canonical.starts_with(&format!("{PRIVATE_CLAUDE_DIR}/"))
        {
            continue;
        }
        if seen.insert(canonical.clone()) {
            roots.push(canonical);
        }
    }
    roots
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
    } else if mp_name.chars().any(char::is_whitespace) {
        diag.report_with(
            LintRule::MarketplaceNameWhitespace,
            &format!("{f} name contains whitespace"),
            marketplace_name_metadata(val.source(), None, "whitespace-containing marketplace name"),
        );
    } else if !RE_MARKETPLACE_KEBAB.is_match(mp_name) {
        // Match the raw value: upstream kebab-case checks do not trim first.
        diag.report(
            LintRule::MarketplaceNameFormat,
            &format!("{f} name '{mp_name}' is not kebab-case ([a-z0-9]+(-[a-z0-9]+)*)"),
        );
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
            let plugin_root = val.get("metadata").and_then(|m| m.get("pluginRoot"));
            let invalid_plugin_root =
                plugin_root.is_some_and(|root| !root.as_str().is_some_and(plugin_root_is_safe));
            let mut name_indexes: HashMap<String, Vec<usize>> = HashMap::new();

            for (i, plugin) in arr.iter().enumerate() {
                let pname_raw = plugin.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let pname = pname_raw.trim();
                let name_missing = pname.is_empty();
                if !name_missing {
                    // Duplicate detection keeps trimmed-key semantics (M009).
                    name_indexes.entry(pname.to_string()).or_default().push(i);
                    if pname_raw.chars().any(char::is_whitespace) {
                        diag.report_with(
                            LintRule::MarketplaceNameWhitespace,
                            &format!("{f} plugins[{i}].name contains whitespace"),
                            marketplace_name_metadata(
                                val.source(),
                                Some(i),
                                "whitespace-containing plugin name",
                            ),
                        );
                    // Match the raw value: upstream kebab-case checks do not trim first.
                    } else if !RE_MARKETPLACE_KEBAB.is_match(pname_raw) {
                        diag.report(
                            LintRule::MarketplaceNameFormat,
                            &format!(
                                "{f} plugins[{i}] name '{pname_raw}' is not kebab-case ([a-z0-9]+(-[a-z0-9]+)*)"
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
                            diag.report_with(
                                LintRule::MarketplacePluginInvalid,
                                &format!(
                                    "{f} plugins[{i}].source path '{s}' must be relative, not absolute"
                                ),
                                marketplace_value_metadata(
                                    val.source(),
                                    &Value::String(s.to_owned()),
                                    &format!("plugins[{i}].source"),
                                    "use a repository-relative marketplace source",
                                ),
                            );
                        } else if path_segments(s).any(|seg| seg == "..") {
                            diag.report_with(
                                LintRule::MarketplacePluginInvalid,
                                &format!(
                                    "{f} plugins[{i}].source path '{s}' must not use '..' traversal"
                                ),
                                marketplace_value_metadata(
                                    val.source(),
                                    &Value::String(s.to_owned()),
                                    &format!("plugins[{i}].source"),
                                    "remove '..' traversal from the marketplace source",
                                ),
                            );
                        } else if !s.starts_with("./") && invalid_plugin_root {
                            diag.report_with(
                                LintRule::MarketplacePluginInvalid,
                                &format!(
                                    "{f} plugins[{i}].source depends on invalid metadata.pluginRoot"
                                ),
                                marketplace_value_metadata(
                                    val.source(),
                                    plugin_root.expect("invalid_plugin_root requires pluginRoot"),
                                    "metadata.pluginRoot",
                                    "use a non-empty plugin-root-relative './' pluginRoot",
                                ),
                            );
                        } else if !s.starts_with("./") && plugin_root.is_none() {
                            diag.report(
                                LintRule::MarketplaceBarePath,
                                &format!(
                                    "{f} plugins[{i}].source '{s}' should start with './' (or set metadata.pluginRoot)"
                                ),
                            );
                        }
                    }
                    Some(Value::Object(obj)) => {
                        validate_object_plugin_source(f, i, obj, val.source(), diag);
                    }
                    _ => {}
                }
                validate_declared_component_paths(
                    plugin,
                    &format!("{f} plugins[{i}]"),
                    val.source(),
                    diag,
                );
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
    source: Option<&str>,
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
    if source_type == "git-subdir"
        && obj
            .get("path")
            .and_then(Value::as_str)
            .is_some_and(|path| path_segments(path).any(|segment| segment == ".."))
    {
        diag.report_with(
            LintRule::MarketplacePluginInvalid,
            &format!("{f} plugins[{i}].source git-subdir path must not use '..' traversal"),
            marketplace_value_metadata(
                source,
                obj.get("path")
                    .expect("the preceding condition requires path"),
                &format!("plugins[{i}].source.path"),
                "remove '..' traversal from the git-subdir path",
            ),
        );
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

    validate_declared_component_paths(val, f, val.source(), diag);
}

/// Report M012/M013 for every path-bearing component declaration. The shared
/// extractor is also used by marketplace validation and discovery consumers.
fn validate_declared_component_paths(
    value: &Value,
    owner: &str,
    source: Option<&str>,
    diag: &mut DiagnosticCollector,
) {
    for path in declared_component_paths(value) {
        // H026 owns hook declarations that normalize to no path. Keep M013
        // focused on unsafe paths; absolute and traversal forms retain its
        // established precedence and never reach H026.
        if (path.label == "hooks" || path.label.starts_with("hooks["))
            && !is_absolute_path(path.raw)
            && !path_segments(path.raw).any(|segment| segment == "..")
            && !has_normalized_path_segment(path.raw)
        {
            continue;
        }
        let (rule, requirement) = match classify_component_path(path.raw) {
            ComponentPathSafety::Safe => continue,
            ComponentPathSafety::Absolute => (
                LintRule::ComponentPathUnsafe,
                "must be relative, not absolute",
            ),
            ComponentPathSafety::Traversal => {
                (LintRule::ComponentPathUnsafe, "must not use '..' traversal")
            }
            ComponentPathSafety::MissingPrefix => {
                (LintRule::ComponentPathUnsafe, "must start with './'")
            }
            ComponentPathSafety::NestedPluginDir => (
                LintRule::ComponentPathNested,
                "must not point inside .claude-plugin/",
            ),
        };
        report_component_path(
            diag,
            rule,
            owner,
            &path.label,
            path.raw,
            requirement,
            source,
        );
    }
}

fn report_component_path(
    diag: &mut DiagnosticCollector,
    rule: LintRule,
    owner: &str,
    label: &str,
    raw: &str,
    requirement: &str,
    source: Option<&str>,
) {
    let mut metadata = DiagnosticMetadata::default()
        .with_evidence(label)
        .with_suggestion("use a plugin-root-relative './' component path");
    if let Some(location) = source
        .and_then(|source| json_value_range(source, &Value::String(raw.to_owned())))
        .and_then(|range| source.and_then(|source| SourceSpan::from_byte_range(source, range)))
    {
        metadata = metadata.with_location(location);
    }
    diag.report_with(
        rule,
        &format!("{owner} {label} path '{raw}' {requirement}"),
        metadata,
    );
}

/// The manifest has already parsed, so this maps an escaped JSON string back
/// to a source token without executing or probing repository-controlled data.
fn marketplace_value_metadata(
    source: Option<&str>,
    value: &Value,
    evidence: &str,
    suggestion: &str,
) -> DiagnosticMetadata {
    let mut metadata = DiagnosticMetadata::default()
        .with_evidence(evidence)
        .with_suggestion(suggestion);
    if let Some(location) = source
        .and_then(|source| json_value_range(source, value))
        .and_then(|range| source.and_then(|source| SourceSpan::from_byte_range(source, range)))
    {
        metadata = metadata.with_location(location);
    }
    metadata
}

/// Metadata for a marketplace name value. Unlike generic value lookup, this
/// follows the owning JSON path so repeated string values retain their exact
/// source token span.
fn marketplace_name_metadata(
    source: Option<&str>,
    plugin_index: Option<usize>,
    evidence: &str,
) -> DiagnosticMetadata {
    let mut metadata = DiagnosticMetadata::default()
        .with_evidence(evidence)
        .with_suggestion("replace whitespace with hyphens and use a whitespace-free identifier");
    if let Some(location) = source.and_then(|source| {
        marketplace_name_value_range(source, plugin_index)
            .and_then(|range| SourceSpan::from_byte_range(source, range))
    }) {
        metadata = metadata.with_location(location);
    }
    metadata
}

/// Finds the source token for either the marketplace `name` or a particular
/// `plugins[index].name`. The manifest has already parsed as JSON, so this
/// small cursor only maps semantic ownership back to the original bytes; it
/// never accepts or executes repository-controlled input.
fn marketplace_name_value_range(
    source: &str,
    plugin_index: Option<usize>,
) -> Option<std::ops::Range<usize>> {
    let root = skip_json_whitespace(source, 0);
    let root_name = |object_start| json_object_field_value_range(source, object_start, "name");
    match plugin_index {
        None => root_name(root),
        Some(index) => {
            let plugins = json_object_field_value_range(source, root, "plugins")?;
            let plugin = json_array_item_value_range(source, plugins.start, index)?;
            root_name(plugin.start)
        }
    }
}

fn skip_json_whitespace(source: &str, mut offset: usize) -> usize {
    while source
        .as_bytes()
        .get(offset)
        .is_some_and(u8::is_ascii_whitespace)
    {
        offset += 1;
    }
    offset
}

fn json_object_field_value_range(
    source: &str,
    object_start: usize,
    wanted_key: &str,
) -> Option<std::ops::Range<usize>> {
    if source.as_bytes().get(object_start) != Some(&b'{') {
        return None;
    }
    let mut offset = skip_json_whitespace(source, object_start + 1);
    while source.as_bytes().get(offset) != Some(&b'}') {
        let key_start = offset;
        let key_end = json_string_end(source, key_start)?;
        let key: String = serde_json::from_str(&source[key_start..key_end]).ok()?;
        offset = skip_json_whitespace(source, key_end);
        if source.as_bytes().get(offset) != Some(&b':') {
            return None;
        }
        let value_start = skip_json_whitespace(source, offset + 1);
        let value_end = json_value_end(source, value_start)?;
        if key == wanted_key {
            return Some(value_start..value_end);
        }
        offset = skip_json_whitespace(source, value_end);
        match source.as_bytes().get(offset) {
            Some(b',') => offset = skip_json_whitespace(source, offset + 1),
            Some(b'}') => break,
            _ => return None,
        }
    }
    None
}

fn json_array_item_value_range(
    source: &str,
    array_start: usize,
    wanted_index: usize,
) -> Option<std::ops::Range<usize>> {
    if source.as_bytes().get(array_start) != Some(&b'[') {
        return None;
    }
    let mut offset = skip_json_whitespace(source, array_start + 1);
    let mut index = 0;
    while source.as_bytes().get(offset) != Some(&b']') {
        let value_start = offset;
        let value_end = json_value_end(source, value_start)?;
        if index == wanted_index {
            return Some(value_start..value_end);
        }
        index += 1;
        offset = skip_json_whitespace(source, value_end);
        match source.as_bytes().get(offset) {
            Some(b',') => offset = skip_json_whitespace(source, offset + 1),
            Some(b']') => break,
            _ => return None,
        }
    }
    None
}

fn json_value_end(source: &str, offset: usize) -> Option<usize> {
    match source.as_bytes().get(offset)? {
        b'"' => json_string_end(source, offset),
        b'{' => {
            let mut cursor = skip_json_whitespace(source, offset + 1);
            while source.as_bytes().get(cursor) != Some(&b'}') {
                cursor = json_string_end(source, cursor)?;
                cursor = skip_json_whitespace(source, cursor);
                if source.as_bytes().get(cursor) != Some(&b':') {
                    return None;
                }
                cursor = json_value_end(source, skip_json_whitespace(source, cursor + 1))?;
                cursor = skip_json_whitespace(source, cursor);
                match source.as_bytes().get(cursor) {
                    Some(b',') => cursor = skip_json_whitespace(source, cursor + 1),
                    Some(b'}') => break,
                    _ => return None,
                }
            }
            Some(cursor + 1)
        }
        b'[' => {
            let mut cursor = skip_json_whitespace(source, offset + 1);
            while source.as_bytes().get(cursor) != Some(&b']') {
                cursor = json_value_end(source, cursor)?;
                cursor = skip_json_whitespace(source, cursor);
                match source.as_bytes().get(cursor) {
                    Some(b',') => cursor = skip_json_whitespace(source, cursor + 1),
                    Some(b']') => break,
                    _ => return None,
                }
            }
            Some(cursor + 1)
        }
        _ => {
            let end = source[offset..]
                .find(|character: char| {
                    character.is_whitespace() || matches!(character, ',' | ']' | '}')
                })
                .map_or(source.len(), |relative| offset + relative);
            (end > offset).then_some(end)
        }
    }
}

fn json_string_end(source: &str, start: usize) -> Option<usize> {
    if source.as_bytes().get(start) != Some(&b'"') {
        return None;
    }
    let mut offset = start + 1;
    while let Some(byte) = source.as_bytes().get(offset) {
        match byte {
            b'"' => return Some(offset + 1),
            b'\\' => offset += 2,
            _ => offset += 1,
        }
    }
    None
}

fn json_value_range(source: &str, value: &Value) -> Option<std::ops::Range<usize>> {
    let token = serde_json::to_string(value).expect("JSON values always serialize");
    source.find(&token).map(|start| start..start + token.len())
}

/// V30–V32: validate plugin-manifest fields on both directly lintable surfaces.
/// Marketplace entries are inline metadata, not unavailable remote manifests.
pub fn validate_plugin_fields(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    if let ManifestState::Parsed(value) = &ctx.plugin_json {
        diag.with_subject_path(".claude-plugin/plugin.json", |diag| {
            validate_plugin_fields_surface(
                ctx,
                value,
                value.source(),
                ".claude-plugin/plugin.json",
                "",
                &[],
                diag,
            );
        });
    }
    if let ManifestState::Parsed(marketplace) = &ctx.marketplace_json
        && let Some(entries) = marketplace.get("plugins").and_then(Value::as_array)
    {
        diag.with_subject_path(".claude-plugin/marketplace.json", |diag| {
            for (index, entry) in entries.iter().enumerate() {
                if entry.is_object() {
                    let prefix = format!("plugins[{index}]");
                    validate_plugin_fields_surface(
                        ctx,
                        entry,
                        marketplace.source(),
                        ".claude-plugin/marketplace.json",
                        &prefix,
                        &[Seg::Key("plugins"), Seg::Index(index)],
                        diag,
                    );
                }
            }
        });
    }
}

/// Compatibility entry point retained for focused unit tests.
#[cfg(test)]
pub fn validate_plugin_metadata(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    let f = ".claude-plugin/plugin.json";
    let val = match &ctx.plugin_json {
        ManifestState::Parsed(v) => v,
        _ => return, // Missing/invalid already reported by V1
    };

    validate_plugin_fields_surface(ctx, val, val.source(), f, "", &[], diag);
}

/// V31: Validate plugin.json lspServers entries (M016).
#[cfg(test)]
pub fn validate_lsp_servers(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    let f = ".claude-plugin/plugin.json";
    let val = match &ctx.plugin_json {
        ManifestState::Parsed(v) => v,
        _ => return, // Missing/invalid already reported by V1
    };

    validate_plugin_fields_surface(ctx, val, val.source(), f, "", &[], diag);
}

/// V32: Validate plugin.json channels entries (M017).
///
#[cfg(test)]
pub fn validate_channels(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    let f = ".claude-plugin/plugin.json";
    let val = match &ctx.plugin_json {
        ManifestState::Parsed(v) => v,
        _ => return, // Missing/invalid already reported by V1
    };

    validate_plugin_fields_surface(ctx, val, val.source(), f, "", &[], diag);
}

fn validate_plugin_fields_surface(
    ctx: &LintContext,
    value: &Value,
    source: Option<&str>,
    document: &str,
    prefix: &str,
    path_prefix: &[Seg<'_>],
    diag: &mut DiagnosticCollector,
) {
    validate_author(value, source, document, prefix, path_prefix, diag);
    validate_homepage(value, source, document, prefix, path_prefix, diag);
    validate_lsp_servers_value(value, source, document, prefix, path_prefix, diag);
    validate_channels_value(ctx, value, source, document, prefix, path_prefix, diag);
}

fn field_label(prefix: &str, field: &str) -> String {
    if prefix.is_empty() {
        field.to_owned()
    } else {
        format!("{prefix}.{field}")
    }
}

fn extend_path<'a>(prefix: &[Seg<'a>], tail: &[Seg<'a>]) -> Vec<Seg<'a>> {
    prefix.iter().chain(tail).copied().collect()
}

fn metadata_at(
    source: Option<&str>,
    path: &[Seg<'_>],
    evidence: &str,
    suggestion: &str,
    redact: bool,
) -> DiagnosticMetadata {
    let mut metadata = DiagnosticMetadata::default().with_suggestion(suggestion);
    metadata = if redact {
        metadata.with_redacted_evidence()
    } else {
        metadata.with_evidence(evidence)
    };
    if let Some(span) = source
        .and_then(|s| JsonScanner::locate(s, path))
        .and_then(|range| source.and_then(|s| SourceSpan::from_byte_range(s, range)))
    {
        metadata = metadata.with_location(span);
    }
    metadata
}

fn validate_author(
    value: &Value,
    source: Option<&str>,
    document: &str,
    prefix: &str,
    path_prefix: &[Seg<'_>],
    diag: &mut DiagnosticCollector,
) {
    let Some(author) = value.get("author") else {
        return;
    };
    let author_path = extend_path(path_prefix, &[Seg::Key("author")]);
    let label = field_label(prefix, "author");
    if !author.is_object() {
        diag.report_with(
            LintRule::AuthorTypeInvalid,
            &format!(
                "{document} {label} must be an object (found {})",
                json_type(author)
            ),
            metadata_at(
                source,
                &author_path,
                &label,
                "use an author object with a non-empty name",
                false,
            ),
        );
    } else if !is_non_empty_string(author.get("name")) {
        let name_path = author.get("name").map_or_else(
            || author_path.clone(),
            |_| extend_path(&author_path, &[Seg::Key("name")]),
        );
        diag.report_with(
            LintRule::AuthorNameMissing,
            &format!(
                "{document} {} missing or invalid (must be a non-empty string)",
                field_label(prefix, "author.name")
            ),
            metadata_at(
                source,
                &name_path,
                &label,
                "set author.name to a non-empty string",
                false,
            ),
        );
    }
}

fn validate_homepage(
    value: &Value,
    source: Option<&str>,
    document: &str,
    prefix: &str,
    path_prefix: &[Seg<'_>],
    diag: &mut DiagnosticCollector,
) {
    let Some(homepage) = value.get("homepage") else {
        return;
    };
    let path = extend_path(path_prefix, &[Seg::Key("homepage")]);
    let label = field_label(prefix, "homepage");
    match homepage {
        Value::String(url) if is_valid_http_url(url) => {}
        Value::String(_) => diag.report_with(
            LintRule::HomepageUrlInvalid,
            &format!("{document} {label} must be an absolute http(s) URL with a host"),
            metadata_at(
                source,
                &path,
                &label,
                "use an absolute http:// or https:// homepage URL",
                true,
            ),
        ),
        _ => diag.report_with(
            LintRule::HomepageTypeInvalid,
            &format!(
                "{document} {label} must be a string (found {})",
                json_type(homepage)
            ),
            metadata_at(
                source,
                &path,
                &label,
                "use an absolute http:// or https:// homepage URL string",
                true,
            ),
        ),
    }
}

fn validate_lsp_servers_value(
    value: &Value,
    source: Option<&str>,
    document: &str,
    prefix: &str,
    path_prefix: &[Seg<'_>],
    diag: &mut DiagnosticCollector,
) {
    let Some(servers) = value.get("lspServers") else {
        return;
    };
    let root_path = extend_path(path_prefix, &[Seg::Key("lspServers")]);
    let root_label = field_label(prefix, "lspServers");
    match servers {
        Value::String(_) => {}
        Value::Object(map) => {
            validate_lsp_map(map, source, document, &root_path, &root_label, diag)
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                let path = extend_path(&root_path, &[Seg::Index(index)]);
                let label = format!("{root_label}[{index}]");
                match item {
                    Value::String(_) => {}
                    Value::Object(map) => {
                        validate_lsp_map(map, source, document, &path, &label, diag)
                    }
                    _ => report_lsp_invalid(
                        source,
                        &path,
                        document,
                        &label,
                        "must be a string path or inline server-map object",
                        diag,
                    ),
                }
            }
        }
        _ => report_lsp_invalid(
            source,
            &root_path,
            document,
            &root_label,
            "must be a string path, inline server-map object, or array of those forms",
            diag,
        ),
    }
}

fn validate_lsp_map(
    map: &serde_json::Map<String, Value>,
    source: Option<&str>,
    document: &str,
    map_path: &[Seg<'_>],
    map_label: &str,
    diag: &mut DiagnosticCollector,
) {
    for (name, server) in map {
        let path = extend_path(map_path, &[Seg::Key(name)]);
        let label = format!("{map_label}.{name}");
        let Some(server) = server.as_object() else {
            report_lsp_invalid(source, &path, document, &label, "must be an object", diag);
            continue;
        };
        let command_bad = !is_non_empty_string(server.get("command"));
        let extensions_bad = match server.get("extensionToLanguage").and_then(Value::as_object) {
            Some(map) if !map.is_empty() => map.iter().any(|(extension, language)| {
                extension.trim().is_empty()
                    || extension.chars().count() < 2
                    || !is_non_empty_string(Some(language))
            }),
            _ => true,
        };
        if command_bad || extensions_bad {
            let mut defects = Vec::new();
            if command_bad {
                defects.push("non-empty command");
            }
            if extensions_bad {
                defects.push(
                    "non-empty extensionToLanguage object with extension keys and language strings",
                );
            }
            report_lsp_invalid(
                source,
                &path,
                document,
                &label,
                &format!("requires {}", defects.join(" and ")),
                diag,
            );
        }
    }
}

fn report_lsp_invalid(
    source: Option<&str>,
    path: &[Seg<'_>],
    document: &str,
    label: &str,
    detail: &str,
    diag: &mut DiagnosticCollector,
) {
    diag.report_with(
        LintRule::LspServerInvalid,
        &format!("{document} {label} {detail}"),
        metadata_at(
            source,
            path,
            label,
            "use an inline server object with command and extensionToLanguage",
            false,
        ),
    );
}

enum KnownMcpServers {
    Known(BTreeSet<String>),
    Unknown,
}

fn known_mcp_servers(ctx: &LintContext, value: &Value) -> KnownMcpServers {
    let Some(declaration) = value.get("mcpServers") else {
        return KnownMcpServers::Known(BTreeSet::new());
    };
    let mut names = BTreeSet::new();
    match declaration {
        Value::Object(map) => names.extend(map.keys().cloned()),
        Value::String(path) => match mcp_names_from_path(ctx, path) {
            Some(found) => names.extend(found),
            None => return KnownMcpServers::Unknown,
        },
        Value::Array(items) => {
            for item in items {
                match item {
                    Value::Object(map) => names.extend(map.keys().cloned()),
                    Value::String(path) => match mcp_names_from_path(ctx, path) {
                        Some(found) => names.extend(found),
                        None => return KnownMcpServers::Unknown,
                    },
                    _ => return KnownMcpServers::Unknown,
                }
            }
        }
        _ => return KnownMcpServers::Unknown,
    }
    KnownMcpServers::Known(names)
}

fn mcp_names_from_path(ctx: &LintContext, path: &str) -> Option<BTreeSet<String>> {
    let relative = safe_component_path(path)?;
    let candidate = ctx.base_path.join(relative);
    let base = std::fs::canonicalize(&ctx.base_path).ok()?;
    let resolved = std::fs::canonicalize(candidate).ok()?;
    if !resolved.starts_with(base) {
        return None;
    }
    let content = std::fs::read_to_string(resolved).ok()?;
    let config = serde_json::from_str::<Value>(&content).ok()?;
    config
        .get("mcpServers")
        .and_then(Value::as_object)
        .map(|map| map.keys().cloned().collect())
}

fn validate_channels_value(
    ctx: &LintContext,
    value: &Value,
    source: Option<&str>,
    document: &str,
    prefix: &str,
    path_prefix: &[Seg<'_>],
    diag: &mut DiagnosticCollector,
) {
    let Some(channels) = value.get("channels") else {
        return;
    };
    let root_path = extend_path(path_prefix, &[Seg::Key("channels")]);
    let root_label = field_label(prefix, "channels");
    let Value::Array(entries) = channels else {
        report_channel_invalid(
            source,
            &root_path,
            document,
            &root_label,
            "must be an array",
            diag,
        );
        return;
    };
    let known_servers = known_mcp_servers(ctx, value);
    for (index, entry) in entries.iter().enumerate() {
        let path = extend_path(&root_path, &[Seg::Index(index)]);
        let label = format!("{root_label}[{index}]");
        let Some(entry) = entry.as_object() else {
            report_channel_invalid(
                source,
                &path,
                document,
                &label,
                "must be an object with a non-empty server",
                diag,
            );
            continue;
        };
        let Some(server) = entry
            .get("server")
            .and_then(Value::as_str)
            .filter(|server| !server.trim().is_empty())
        else {
            report_channel_invalid(
                source,
                &path,
                document,
                &label,
                "requires a non-empty string server",
                diag,
            );
            continue;
        };
        if let KnownMcpServers::Known(names) = &known_servers
            && !names.contains(server)
        {
            let server_path = extend_path(&path, &[Seg::Key("server")]);
            report_channel_invalid(
                source,
                &server_path,
                document,
                &label,
                "server does not reference a known mcpServers entry",
                diag,
            );
        }
    }
}

fn report_channel_invalid(
    source: Option<&str>,
    path: &[Seg<'_>],
    document: &str,
    label: &str,
    detail: &str,
    diag: &mut DiagnosticCollector,
) {
    diag.report_with(
        LintRule::ChannelServerMissing,
        &format!("{document} {label} {detail}"),
        metadata_at(
            source,
            path,
            label,
            "use a channels array entry with a server declared by mcpServers",
            false,
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::LintMode;
    use crate::plugin_paths::{COMPONENT_PATH_FIELDS, ComponentPathField};
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

    // ── #321: declared_agent_roots ──────────────────────────────────
    fn agent_roots(agents: Value) -> Vec<String> {
        let val = json!({"name": "p", "version": "1.0.0", "agents": agents});
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
        declared_agent_roots(&ctx)
    }

    #[test]
    fn declared_agent_roots_accepts_scalar_and_array_in_declaration_order() {
        // Component paths must be `./`-prefixed (M013); the prefix is stripped in
        // the returned canonical root.
        assert_eq!(agent_roots(json!("./custom")), vec!["custom".to_string()]);
        assert_eq!(
            agent_roots(json!(["./alpha", "./beta"])),
            vec!["alpha".to_string(), "beta".to_string()]
        );
    }

    #[test]
    fn declared_agent_roots_canonicalize_and_deduplicate_spellings() {
        // `./custom` and `./custom/` are one normalized root; only safe
        // (`./`-prefixed) spellings are considered.
        assert_eq!(
            agent_roots(json!(["./custom", "./custom/", "./custom//sub"])),
            vec!["custom".to_string(), "custom/sub".to_string()]
        );
    }

    #[test]
    fn declared_agent_roots_exclude_unsafe_and_private_paths() {
        // Absolute, traversal, missing-`./`-prefix, and `.claude-plugin/`-nested
        // paths are owned by M012/M013; the private `.claude/` tree is scanned
        // under its own surface. Only the plugin-relative safe path survives.
        assert_eq!(
            agent_roots(json!([
                "/abs",
                "../up",
                "no-prefix",
                "./.claude-plugin/agents",
                "./.claude/agents",
                "./.claude",
                "./safe"
            ])),
            vec!["safe".to_string()]
        );
    }

    #[test]
    fn declared_agent_roots_ignore_non_string_shapes() {
        assert!(agent_roots(json!({"inline": "object"})).is_empty());
        assert!(agent_roots(json!(42)).is_empty());
        // A non-string array entry is skipped; the safe string survives.
        assert_eq!(agent_roots(json!(["./ok", 7])), vec!["ok".to_string()]);
    }

    #[test]
    fn declared_agent_roots_empty_without_field_or_manifest() {
        let no_field = json!({"name": "p", "version": "1.0.0"});
        let ctx = make_ctx(ManifestState::parsed(no_field), ManifestState::Missing);
        assert!(declared_agent_roots(&ctx).is_empty());

        let missing = make_ctx(ManifestState::Missing, ManifestState::Missing);
        assert!(declared_agent_roots(&missing).is_empty());

        let invalid = make_ctx(ManifestState::invalid("bad json"), ManifestState::Missing);
        assert!(declared_agent_roots(&invalid).is_empty());
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
    fn test_v2_name_rules_are_mutually_exclusive() {
        let cases: &[(&str, serde_json::Value, &[LintRule])] = &[
            (
                "top_ok",
                json!({"name": "a", "owner": {"name": "o"}, "plugins": [{"name": "a-b2", "source": "./p"}]}),
                &[],
            ),
            (
                "top_ok_my_plugin",
                json!({"name": "my-plugin", "owner": {"name": "o"}, "plugins": [{"name": "my-plugin", "source": "./p"}]}),
                &[],
            ),
            (
                "top_uppercase",
                json!({"name": "My_Plugin", "owner": {"name": "o"}, "plugins": [{"name": "p", "source": "./p"}]}),
                &[LintRule::MarketplaceNameFormat],
            ),
            (
                "entry_upper",
                json!({"name": "mp", "owner": {"name": "o"}, "plugins": [{"name": "UPPER", "source": "./p"}]}),
                &[LintRule::MarketplaceNameFormat],
            ),
            (
                "entry_double_hyphen",
                json!({"name": "mp", "owner": {"name": "o"}, "plugins": [{"name": "a--b", "source": "./p"}]}),
                &[LintRule::MarketplaceNameFormat],
            ),
            (
                "entry_edge_hyphen",
                json!({"name": "mp", "owner": {"name": "o"}, "plugins": [{"name": "-a", "source": "./p"}]}),
                &[LintRule::MarketplaceNameFormat],
            ),
            (
                "entry_non_ascii",
                json!({"name": "mp", "owner": {"name": "o"}, "plugins": [{"name": "café", "source": "./p"}]}),
                &[LintRule::MarketplaceNameFormat],
            ),
            (
                "top_ascii_whitespace",
                json!({"name": " my-market ", "owner": {"name": "o"}, "plugins": [{"name": "p", "source": "./p"}]}),
                &[LintRule::MarketplaceNameWhitespace],
            ),
            (
                "top_tab",
                json!({"name": "my\tmarket", "owner": {"name": "o"}, "plugins": [{"name": "p", "source": "./p"}]}),
                &[LintRule::MarketplaceNameWhitespace],
            ),
            (
                "entry_newline",
                json!({"name": "mp", "owner": {"name": "o"}, "plugins": [{"name": "my\nplugin", "source": "./p"}]}),
                &[LintRule::MarketplaceNameWhitespace],
            ),
            (
                "entry_non_breaking_space",
                json!({"name": "mp", "owner": {"name": "o"}, "plugins": [{"name": "my\u{00a0}plugin", "source": "./p"}]}),
                &[LintRule::MarketplaceNameWhitespace],
            ),
            (
                "entry_unicode_space",
                json!({"name": "mp", "owner": {"name": "o"}, "plugins": [{"name": "my\u{2003}plugin", "source": "./p"}]}),
                &[LintRule::MarketplaceNameWhitespace],
            ),
            (
                "top_blank",
                json!({"name": "  ", "owner": {"name": "o"}, "plugins": [{"name": "p", "source": "./p"}]}),
                &[LintRule::MarketplaceFieldMissing],
            ),
            (
                "top_non_string",
                json!({"name": 42, "owner": {"name": "o"}, "plugins": [{"name": "p", "source": "./p"}]}),
                &[LintRule::MarketplaceFieldMissing],
            ),
            (
                "entry_blank",
                json!({"name": "mp", "owner": {"name": "o"}, "plugins": [{"name": "  ", "source": "./p"}]}),
                &[LintRule::MarketplacePluginInvalid],
            ),
            (
                "entry_non_string",
                json!({"name": "mp", "owner": {"name": "o"}, "plugins": [{"name": 42, "source": "./p"}]}),
                &[LintRule::MarketplacePluginInvalid],
            ),
        ];
        for (label, val, expected) in cases {
            let ctx = make_ctx(ManifestState::Missing, ManifestState::parsed(val.clone()));
            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_marketplace_json(&ctx, &mut diag);
            let got: Vec<_> = diag
                .diagnostics()
                .iter()
                .map(|diagnostic| diagnostic.rule)
                .collect();
            assert_eq!(got, *expected, "{label}: {:?}", diag.diagnostics());
            assert!(
                !(got.contains(&LintRule::MarketplaceNameFormat)
                    && got.contains(&LintRule::MarketplaceNameWhitespace)),
                "{label}: M021 and M024 must be mutually exclusive"
            );
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
        let val = json!({"name": "p", "version": "1.0.0", "agents": ["./agents", "/abs", "../up"]});
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
            "agents": ["./agents", "./extra/agents"],
            "skills": "./skills"
        });
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_component_paths(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn component_paths_require_prefix_and_cover_command_object_sources() {
        let val = json!({
            "name": "p",
            "commands": {"unsafe": {"source": "commands/unsafe.md"}},
            "skills": "skills",
            "agents": ["./agents", "agents/extra"],
        });
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_component_paths(&ctx, &mut diag);
        let labels = diag
            .diagnostics()
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>();
        assert_eq!(labels.len(), 3);
        assert!(
            labels
                .iter()
                .all(|message| message.contains("must start with './'"))
        );
        assert!(
            labels
                .iter()
                .any(|message| message.contains("commands.unsafe.source"))
        );
        assert!(labels.iter().any(|message| message.contains("skills")));
        assert!(labels.iter().any(|message| message.contains("agents[1]")));
    }

    #[test]
    fn marketplace_roots_sources_and_components_share_path_policy() {
        let val = json!({
            "name": "mp",
            "owner": {"name": "o"},
            "metadata": {"pluginRoot": "../outside"},
            "plugins": [
                {
                    "name": "entry",
                    "source": "plugins/entry",
                    "commands": {"bad": {"source": "../../outside.md"}},
                    "skills": "skills"
                },
                {
                    "name": "subdir",
                    "source": {"source": "git-subdir", "url": "https://example.com/x.git", "path": "packages/../escape"}
                }
            ]
        });
        let ctx = make_ctx(ManifestState::Missing, ManifestState::parsed(val));
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_marketplace_json(&ctx, &mut diag);
        let rules = diag
            .diagnostics()
            .iter()
            .map(|diagnostic| diagnostic.rule)
            .collect::<Vec<_>>();
        assert_eq!(
            rules,
            vec![
                LintRule::MarketplacePluginInvalid,
                LintRule::ComponentPathUnsafe,
                LintRule::ComponentPathUnsafe,
                LintRule::MarketplacePluginInvalid,
            ]
        );
        assert!(diag.errors()[0].contains("invalid metadata.pluginRoot"));
        assert!(diag.errors()[1].contains("plugins[0] commands.bad.source"));
        assert!(diag.errors()[2].contains("plugins[0] skills"));
        assert!(diag.errors()[3].contains("git-subdir path must not use '..' traversal"));
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
                "./.claude-plugin/components",
                LintRule::ComponentPathNested,
                "must not point inside .claude-plugin/",
            ),
        ];

        for field in COMPONENT_PATH_FIELDS {
            for (case_name, path, rule, expected_message) in cases {
                for (shape, value) in [
                    ("string", json!(path)),
                    ("array", json!(["./components", path])),
                ] {
                    let ctx = make_ctx(
                        ManifestState::parsed(manifest_with_component_path(field, value)),
                        ManifestState::Missing,
                    );
                    let mut diag = DiagnosticCollector::new_all_enabled();
                    validate_component_paths(&ctx, &mut diag);

                    assert_eq!(diag.error_count(), 1, "{} {shape} {case_name}", field.label);
                    assert_eq!(diag.diagnostics()[0].rule, rule);
                    assert!(diag.errors()[0].contains(path));
                    assert!(diag.errors()[0].contains(expected_message));
                }
            }

            for (shape, value) in [
                ("string", json!("./components")),
                ("array", json!(["./components", "./other-components"])),
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
                    .enumerate()
                    .map(move |(index, path)| format!("{}[{index}] path '{path}'", field.label))
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
        let val = json!({"name": "p", "version": "1.0.0", "skills": "./.claude-plugin/skills"});
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

    #[test]
    fn test_m016_array_forms_and_extension_mapping_contract() {
        let valid = json!({
            "lspServers": [
                "./lsp.json",
                {"rust": {"command": "rust-analyzer", "extensionToLanguage": {".rs": "rust"}}}
            ]
        });
        let ctx = make_ctx(ManifestState::parsed(valid), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_lsp_servers(&ctx, &mut diag);
        assert!(diag.diagnostics().is_empty(), "{:?}", diag.diagnostics());

        let invalid = json!({
            "lspServers": [
                {"bad-key": {"command": "ok", "extensionToLanguage": {"x": "", ".ok": "lang"}}},
                7
            ]
        });
        let ctx = make_ctx(ManifestState::parsed(invalid), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_lsp_servers(&ctx, &mut diag);
        assert_eq!(diag.diagnostics().len(), 2, "{:?}", diag.diagnostics());
        assert!(
            diag.diagnostics()[0]
                .message
                .contains("lspServers[0].bad-key")
        );
        assert!(diag.diagnostics()[1].message.contains("lspServers[1]"));
    }

    // ── M017: channel-server-missing ────────────────────────────────

    #[test]
    fn test_m017_object_channels_are_rejected() {
        let val = json!({"channels": {"alerts": {"server": "my-server"}}});
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_channels(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("channels must be an array"));
    }

    #[test]
    fn test_m017_array_channels() {
        let val = json!({"channels": [{"server": "s"}, {"name": "no-server"}, "not-an-object"]});
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_channels(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 3);
        assert!(diag.errors()[0].contains("channels[0]"));
        assert!(diag.errors()[1].contains("channels[1]"));
        assert!(diag.errors()[2].contains("channels[2]"));
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
            "channels": [{"server": "missing"}]
        });
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_channels(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert_eq!(diag.diagnostics()[0].rule, LintRule::ChannelServerMissing);
        assert!(diag.diagnostics()[0].message.contains("known mcpServers"));
    }

    #[test]
    fn test_m017_matching_or_external_mcp_servers_skip_cross_check() {
        for val in [
            json!({
                "mcpServers": {"existing": {"command": "server"}},
                "channels": [{"server": "existing"}]
            }),
            json!({
                "mcpServers": "./servers.json",
                "channels": [{"server": "external"}]
            }),
        ] {
            let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_channels(&ctx, &mut diag);
            assert_eq!(diag.error_count(), 0);
        }
    }

    #[test]
    fn test_m017_resolves_safe_local_mcp_config_names() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("servers.json"),
            r#"{"mcpServers":{"known":{"command":"server"}}}"#,
        )
        .unwrap();
        let ctx = LintContext {
            base_path: tmp.path().to_path_buf(),
            mode: LintMode::Plugin,
            plugin_json: ManifestState::parsed(json!({
                "mcpServers": "./servers.json",
                "channels": [{"server": "known"}]
            })),
            marketplace_json: ManifestState::Missing,
            hooks_json: ManifestState::Missing,
            declared_hook_configs: vec![],
            settings_json: ManifestState::Missing,
            settings_local_json: ManifestState::Missing,
        };
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_channels(&ctx, &mut diag);
        assert!(diag.diagnostics().is_empty(), "{:?}", diag.diagnostics());
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
