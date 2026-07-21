use serde_json::Value;
use std::collections::HashSet;
use std::ops::Deref;
use std::path::{Path, PathBuf};

/// Three-state manifest parse result.
#[derive(Debug)]
pub enum ManifestState {
    Missing,
    Invalid(ManifestError),
    Parsed(ParsedManifest),
}

/// A JSON manifest together with the exact source used to parse it.
///
/// Validators consume the parsed value through `Deref`; validators that need
/// a precise source span can use `source` without re-reading the manifest.
#[derive(Debug)]
pub struct ParsedManifest {
    value: Value,
    source: Option<String>,
}

impl ParsedManifest {
    pub fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }
}

impl Deref for ParsedManifest {
    type Target = Value;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

/// A hook configuration declared by `.claude-plugin/plugin.json`.
///
/// File-backed configurations retain their loader result, including a missing
/// state, so H001 can distinguish an absent optional default from a declared
/// path that cannot be resolved. Inline configurations are wrapped in the
/// normal top-level `{ "hooks": ... }` shape before they reach validators.
#[derive(Debug)]
pub struct DeclaredHookConfig {
    pub subject_path: PathBuf,
    pub state: ManifestState,
    pub kind: DeclaredHookConfigKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclaredHookConfigKind {
    File,
    Inline,
}

/// A manifest loading failure and, when JSON parsing reached a source point,
/// its one-based location. Keeping this parse state with the loaded manifest
/// lets every consuming validator report the same structured fact.
#[derive(Debug)]
pub struct ManifestError {
    message: String,
    location: Option<ManifestErrorLocation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManifestErrorLocation {
    line: usize,
    column: Option<usize>,
}

impl ManifestError {
    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn location(&self) -> Option<ManifestErrorLocation> {
        self.location
    }
}

impl ManifestErrorLocation {
    pub fn line(self) -> usize {
        self.line
    }

    pub fn column(self) -> Option<usize> {
        self.column
    }
}

impl ManifestState {
    /// Construct a parsed manifest for tests and callers that only have a
    /// semantic JSON value. Source locations are unavailable in this form.
    #[cfg(test)]
    pub fn parsed(value: Value) -> Self {
        Self::Parsed(ParsedManifest {
            value,
            source: None,
        })
    }

    /// Construct a synthetic invalid state for unit tests that do not exercise
    /// the filesystem loader.
    #[cfg(test)]
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(ManifestError {
            message: message.into(),
            location: None,
        })
    }

    pub fn load(path: &Path, subject_path: &Path) -> Self {
        if !path.is_file() {
            return ManifestState::Missing;
        }
        match std::fs::read_to_string(path) {
            Err(e) => ManifestState::Invalid(ManifestError {
                message: format!("cannot read {}: {e}", subject_path.display()),
                location: None,
            }),
            Ok(content) => match serde_json::from_str::<Value>(&content) {
                Ok(value) => ManifestState::Parsed(ParsedManifest {
                    value,
                    source: Some(content),
                }),
                Err(e) => ManifestState::Invalid(ManifestError {
                    message: format!("{} is not valid JSON: {e}", subject_path.display()),
                    location: Some(ManifestErrorLocation {
                        line: e.line(),
                        // serde_json uses column zero when an error has no
                        // concrete point (for example an empty document).
                        column: (e.column() > 0).then_some(e.column()),
                    }),
                }),
            },
        }
    }
}

/// Recursively collect all string values from a JSON value.
/// Equivalent to jq '.. | strings'.
pub(crate) fn collect_json_strings(value: &Value) -> Vec<String> {
    let mut result = Vec::new();
    collect_json_strings_inner(value, &mut result);
    result
}

fn collect_json_strings_inner(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(s) => out.push(s.clone()),
        Value::Array(arr) => {
            for item in arr {
                collect_json_strings_inner(item, out);
            }
        }
        Value::Object(map) => {
            for (_, v) in map {
                collect_json_strings_inner(v, out);
            }
        }
        _ => {}
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LintMode {
    Basic,
    Plugin,
}

pub struct LintContext {
    #[allow(dead_code)]
    pub base_path: PathBuf,
    pub mode: LintMode,
    pub plugin_json: ManifestState,
    pub marketplace_json: ManifestState,
    pub hooks_json: ManifestState,
    pub declared_hook_configs: Vec<DeclaredHookConfig>,
    pub settings_json: ManifestState,
    pub settings_local_json: ManifestState,
}

impl LintContext {
    pub fn new(base_path: &Path, mode: LintMode) -> Self {
        // The legacy default and Claude settings surfaces are always loaded
        // regardless of mode. Plugin-declared hook surfaces are loaded only in
        // Plugin mode after plugin.json has been parsed.
        let hooks_json = ManifestState::load(
            &base_path.join("hooks/hooks.json"),
            Path::new("hooks/hooks.json"),
        );
        let settings_json = ManifestState::load(
            &base_path.join(".claude/settings.json"),
            Path::new(".claude/settings.json"),
        );
        let settings_local_json = ManifestState::load(
            &base_path.join(".claude/settings.local.json"),
            Path::new(".claude/settings.local.json"),
        );

        // plugin_json and marketplace_json are only loaded in Plugin mode.
        // In Basic mode, they are set to Missing since run_basic never accesses them.
        let (plugin_json, marketplace_json) = if mode == LintMode::Plugin {
            (
                ManifestState::load(
                    &base_path.join(".claude-plugin/plugin.json"),
                    Path::new(".claude-plugin/plugin.json"),
                ),
                ManifestState::load(
                    &base_path.join(".claude-plugin/marketplace.json"),
                    Path::new(".claude-plugin/marketplace.json"),
                ),
            )
        } else {
            (ManifestState::Missing, ManifestState::Missing)
        };

        let declared_hook_configs = if mode == LintMode::Plugin {
            collect_declared_hook_configs(base_path, &plugin_json, &hooks_json)
        } else {
            Vec::new()
        };

        Self {
            base_path: base_path.to_path_buf(),
            mode,
            plugin_json,
            marketplace_json,
            hooks_json,
            declared_hook_configs,
            settings_json,
            settings_local_json,
        }
    }
}

/// Load hook configurations declared by the parsed plugin manifest. Component
/// paths use the same lexical safety contract as M013: they must be relative
/// and must not contain a parent-directory segment. Unsafe raw values are left
/// to M013 and are never probed on disk here.
fn collect_declared_hook_configs(
    base_path: &Path,
    plugin_json: &ManifestState,
    default_hooks_json: &ManifestState,
) -> Vec<DeclaredHookConfig> {
    let ManifestState::Parsed(plugin) = plugin_json else {
        return Vec::new();
    };
    let Some(hooks) = plugin.get("hooks") else {
        return Vec::new();
    };

    let mut configs = Vec::new();
    let mut seen = HashSet::new();
    if !matches!(default_hooks_json, ManifestState::Missing) {
        seen.insert(PathBuf::from("hooks/hooks.json"));
    }

    match hooks {
        Value::String(path) => push_declared_hook_file(base_path, path, &mut seen, &mut configs),
        Value::Array(paths) => {
            for path in paths.iter().filter_map(Value::as_str) {
                push_declared_hook_file(base_path, path, &mut seen, &mut configs);
            }
        }
        Value::Object(inline) => configs.push(DeclaredHookConfig {
            subject_path: PathBuf::from(".claude-plugin/plugin.json"),
            state: ManifestState::Parsed(Value::Object(serde_json::Map::from_iter([(
                "hooks".to_owned(),
                Value::Object(inline.clone()),
            )]))),
            kind: DeclaredHookConfigKind::Inline,
        }),
        _ => {}
    }

    configs
}

fn push_declared_hook_file(
    base_path: &Path,
    raw_path: &str,
    seen: &mut HashSet<PathBuf>,
    configs: &mut Vec<DeclaredHookConfig>,
) {
    let Some(subject_path) = safe_component_path(raw_path) else {
        return;
    };
    if !seen.insert(subject_path.clone()) {
        return;
    }
    configs.push(DeclaredHookConfig {
        state: ManifestState::load(&base_path.join(&subject_path), &subject_path),
        subject_path,
        kind: DeclaredHookConfigKind::File,
    });
}

fn safe_component_path(raw_path: &str) -> Option<PathBuf> {
    let is_absolute = raw_path.starts_with('/')
        || raw_path.starts_with('\\')
        || raw_path
            .as_bytes()
            .get(1)
            .is_some_and(|separator| *separator == b':')
            && raw_path
                .as_bytes()
                .get(2)
                .is_some_and(|separator| matches!(*separator, b'/' | b'\\'));
    if is_absolute {
        return None;
    }

    let mut path = PathBuf::new();
    for segment in raw_path.split(['/', '\\']) {
        match segment {
            "" | "." => {}
            ".." => return None,
            segment => path.push(segment),
        }
    }
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    // ── ManifestState::load ──────────────────────────────────────────

    #[test]
    fn load_missing_file_returns_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.json");
        let state = ManifestState::load(&path, Path::new("nonexistent.json"));
        assert!(matches!(state, ManifestState::Missing));
    }

    #[test]
    fn load_valid_json_returns_parsed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("valid.json");
        std::fs::write(&path, r#"{"name": "test"}"#).unwrap();

        let state = ManifestState::load(&path, Path::new("valid.json"));
        match state {
            ManifestState::Parsed(val) => {
                assert_eq!(val["name"], "test");
            }
            other => panic!("expected Parsed, got {other:?}"),
        }
    }

    #[test]
    fn load_invalid_json_returns_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, "not json at all {{{").unwrap();

        let state = ManifestState::load(&path, Path::new("bad.json"));
        match state {
            ManifestState::Invalid(error) => {
                assert!(error.message().contains("not valid JSON"));
                assert_eq!(error.location().unwrap().line(), 1);
                assert!(error.location().unwrap().column().is_some());
            }
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn load_empty_file_returns_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.json");
        std::fs::write(&path, "").unwrap();

        let state = ManifestState::load(&path, Path::new("empty.json"));
        assert!(matches!(state, ManifestState::Invalid(_)));
    }

    #[test]
    fn load_directory_path_returns_missing() {
        let dir = tempfile::tempdir().unwrap();
        // Path::is_file() returns false for directories
        let state = ManifestState::load(dir.path(), Path::new("directory.json"));
        assert!(matches!(state, ManifestState::Missing));
    }

    // ── LintContext::new ─────────────────────────────────────────────

    #[test]
    fn new_context_loads_manifests_from_base_path() {
        let tmp = tempfile::tempdir().unwrap();

        // Create plugin.json only; the rest stay Missing.
        std::fs::create_dir_all(tmp.path().join(".claude-plugin")).unwrap();
        std::fs::write(
            tmp.path().join(".claude-plugin/plugin.json"),
            r#"{"name": "test-plugin"}"#,
        )
        .unwrap();

        let ctx = LintContext::new(tmp.path(), LintMode::Plugin);

        assert_eq!(ctx.mode, LintMode::Plugin);
        assert!(matches!(ctx.plugin_json, ManifestState::Parsed(_)));
        assert!(matches!(ctx.marketplace_json, ManifestState::Missing));
        assert!(matches!(ctx.hooks_json, ManifestState::Missing));
        assert!(matches!(ctx.settings_json, ManifestState::Missing));
    }

    #[test]
    fn new_context_all_manifests_present_plugin_mode() {
        let tmp = tempfile::tempdir().unwrap();

        std::fs::create_dir_all(tmp.path().join(".claude-plugin")).unwrap();
        std::fs::create_dir_all(tmp.path().join(".claude")).unwrap();
        std::fs::create_dir_all(tmp.path().join("hooks")).unwrap();

        std::fs::write(tmp.path().join(".claude-plugin/plugin.json"), r#"{"a":1}"#).unwrap();
        std::fs::write(
            tmp.path().join(".claude-plugin/marketplace.json"),
            r#"{"b":2}"#,
        )
        .unwrap();
        std::fs::write(tmp.path().join("hooks/hooks.json"), r#"{"c":3}"#).unwrap();
        std::fs::write(tmp.path().join(".claude/settings.json"), r#"{"d":4}"#).unwrap();

        let ctx = LintContext::new(tmp.path(), LintMode::Plugin);

        assert_eq!(ctx.mode, LintMode::Plugin);
        assert!(matches!(ctx.plugin_json, ManifestState::Parsed(_)));
        assert!(matches!(ctx.marketplace_json, ManifestState::Parsed(_)));
        assert!(matches!(ctx.hooks_json, ManifestState::Parsed(_)));
        assert!(matches!(ctx.settings_json, ManifestState::Parsed(_)));
    }

    #[test]
    fn new_context_basic_mode_skips_plugin_manifests() {
        let tmp = tempfile::tempdir().unwrap();

        std::fs::create_dir_all(tmp.path().join(".claude-plugin")).unwrap();
        std::fs::create_dir_all(tmp.path().join(".claude")).unwrap();
        std::fs::create_dir_all(tmp.path().join("hooks")).unwrap();

        std::fs::write(tmp.path().join(".claude-plugin/plugin.json"), r#"{"a":1}"#).unwrap();
        std::fs::write(
            tmp.path().join(".claude-plugin/marketplace.json"),
            r#"{"b":2}"#,
        )
        .unwrap();
        std::fs::write(tmp.path().join("hooks/hooks.json"), r#"{"c":3}"#).unwrap();
        std::fs::write(tmp.path().join(".claude/settings.json"), r#"{"d":4}"#).unwrap();

        let ctx = LintContext::new(tmp.path(), LintMode::Basic);

        assert_eq!(ctx.mode, LintMode::Basic);
        // In Basic mode, plugin_json and marketplace_json are always Missing
        assert!(matches!(ctx.plugin_json, ManifestState::Missing));
        assert!(matches!(ctx.marketplace_json, ManifestState::Missing));
        // hooks_json and settings_json are always loaded regardless of mode
        assert!(matches!(ctx.hooks_json, ManifestState::Parsed(_)));
        assert!(matches!(ctx.settings_json, ManifestState::Parsed(_)));
    }

    #[test]
    fn new_context_with_invalid_json() {
        let tmp = tempfile::tempdir().unwrap();

        std::fs::create_dir_all(tmp.path().join(".claude-plugin")).unwrap();
        std::fs::write(tmp.path().join(".claude-plugin/plugin.json"), "broken!!!").unwrap();

        let ctx = LintContext::new(tmp.path(), LintMode::Plugin);

        assert!(matches!(ctx.plugin_json, ManifestState::Invalid(_)));
    }

    #[test]
    fn new_context_base_path_independent_of_cwd() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".claude")).unwrap();
        std::fs::write(
            tmp.path().join(".claude/settings.json"),
            r#"{"key": "value"}"#,
        )
        .unwrap();

        // Construct LintContext with a base path that is NOT the CWD
        // This verifies manifest loading uses base_path, not process CWD
        let ctx = LintContext::new(tmp.path(), LintMode::Basic);
        assert!(matches!(ctx.settings_json, ManifestState::Parsed(_)));
    }

    // ── collect_json_strings ────────────────────────────────────────

    #[test]
    fn collect_json_strings_flat_string() {
        let val = serde_json::json!("hello");
        assert_eq!(collect_json_strings(&val), vec!["hello"]);
    }

    #[test]
    fn collect_json_strings_nested_object() {
        let val = serde_json::json!({"a": "one", "b": {"c": "two"}});
        let mut strings = collect_json_strings(&val);
        strings.sort();
        assert_eq!(strings, vec!["one", "two"]);
    }

    #[test]
    fn collect_json_strings_array() {
        let val = serde_json::json!(["a", "b", "c"]);
        assert_eq!(collect_json_strings(&val), vec!["a", "b", "c"]);
    }

    #[test]
    fn collect_json_strings_deeply_nested() {
        let val = serde_json::json!({"x": [{"y": [{"z": "deep"}]}]});
        assert_eq!(collect_json_strings(&val), vec!["deep"]);
    }

    #[test]
    fn collect_json_strings_no_strings() {
        let val = serde_json::json!({"a": 1, "b": true, "c": null});
        assert!(collect_json_strings(&val).is_empty());
    }
}
