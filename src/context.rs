use serde_json::Value;
use std::path::{Path, PathBuf};

/// Three-state manifest parse result.
#[derive(Debug)]
pub enum ManifestState {
    Missing,
    Invalid(String),
    Parsed(Value),
}

impl ManifestState {
    pub fn load(path: &Path) -> Self {
        if !path.is_file() {
            return ManifestState::Missing;
        }
        match std::fs::read_to_string(path) {
            Err(e) => ManifestState::Invalid(format!("cannot read {}: {e}", path.display())),
            Ok(content) => match serde_json::from_str::<Value>(&content) {
                Ok(val) => ManifestState::Parsed(val),
                Err(e) => {
                    ManifestState::Invalid(format!("{} is not valid JSON: {e}", path.display()))
                }
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
    pub settings_json: ManifestState,
}

impl LintContext {
    pub fn new(base_path: &Path, mode: LintMode) -> Self {
        // hooks_json and settings_json are always loaded regardless of mode.
        let hooks_json = ManifestState::load(&base_path.join("hooks/hooks.json"));
        let settings_json = ManifestState::load(&base_path.join(".claude/settings.json"));

        // plugin_json and marketplace_json are only loaded in Plugin mode.
        // In Basic mode, they are set to Missing since run_basic never accesses them.
        let (plugin_json, marketplace_json) = if mode == LintMode::Plugin {
            (
                ManifestState::load(&base_path.join(".claude-plugin/plugin.json")),
                ManifestState::load(&base_path.join(".claude-plugin/marketplace.json")),
            )
        } else {
            (ManifestState::Missing, ManifestState::Missing)
        };

        Self {
            base_path: base_path.to_path_buf(),
            mode,
            plugin_json,
            marketplace_json,
            hooks_json,
            settings_json,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // ── ManifestState::load ──────────────────────────────────────────

    #[test]
    fn load_missing_file_returns_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.json");
        let state = ManifestState::load(&path);
        assert!(matches!(state, ManifestState::Missing));
    }

    #[test]
    fn load_valid_json_returns_parsed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("valid.json");
        std::fs::write(&path, r#"{"name": "test"}"#).unwrap();

        let state = ManifestState::load(&path);
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

        let state = ManifestState::load(&path);
        match state {
            ManifestState::Invalid(msg) => {
                assert!(msg.contains("not valid JSON"), "msg was: {msg}");
            }
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn load_empty_file_returns_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.json");
        std::fs::write(&path, "").unwrap();

        let state = ManifestState::load(&path);
        assert!(matches!(state, ManifestState::Invalid(_)));
    }

    #[test]
    fn load_directory_path_returns_missing() {
        let dir = tempfile::tempdir().unwrap();
        // Path::is_file() returns false for directories
        let state = ManifestState::load(dir.path());
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
