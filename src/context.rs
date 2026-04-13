use serde_json::Value;
use std::path::Path;

/// Three-state manifest parse result.
#[derive(Debug)]
pub enum ManifestState {
    Missing,
    Invalid(String),
    Parsed(Value),
}

impl ManifestState {
    pub fn load(path: &str) -> Self {
        if !Path::new(path).is_file() {
            return ManifestState::Missing;
        }
        match std::fs::read_to_string(path) {
            Err(e) => ManifestState::Invalid(format!("cannot read {path}: {e}")),
            Ok(content) => match serde_json::from_str::<Value>(&content) {
                Ok(val) => ManifestState::Parsed(val),
                Err(e) => ManifestState::Invalid(format!("{path} is not valid JSON: {e}")),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LintMode {
    Basic,
    Plugin,
}

pub struct LintContext {
    #[allow(dead_code)]
    pub repo_root: String,
    pub mode: LintMode,
    pub plugin_json: ManifestState,
    pub marketplace_json: ManifestState,
    pub hooks_json: ManifestState,
    pub settings_json: ManifestState,
}

impl LintContext {
    pub fn new(repo_root: String, mode: LintMode) -> Self {
        let plugin_json = ManifestState::load(".claude-plugin/plugin.json");
        let marketplace_json = ManifestState::load(".claude-plugin/marketplace.json");
        let hooks_json = ManifestState::load("hooks/hooks.json");
        let settings_json = ManifestState::load(".claude/settings.json");

        Self {
            repo_root,
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
    use serial_test::serial;

    // ── ManifestState::load ──────────────────────────────────────────

    #[test]
    fn load_missing_file_returns_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.json");
        let state = ManifestState::load(path.to_str().unwrap());
        assert!(matches!(state, ManifestState::Missing));
    }

    #[test]
    fn load_valid_json_returns_parsed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("valid.json");
        std::fs::write(&path, r#"{"name": "test"}"#).unwrap();

        let state = ManifestState::load(path.to_str().unwrap());
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

        let state = ManifestState::load(path.to_str().unwrap());
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

        let state = ManifestState::load(path.to_str().unwrap());
        assert!(matches!(state, ManifestState::Invalid(_)));
    }

    #[test]
    fn load_directory_path_returns_missing() {
        let dir = tempfile::tempdir().unwrap();
        // Path::is_file() returns false for directories
        let state = ManifestState::load(dir.path().to_str().unwrap());
        assert!(matches!(state, ManifestState::Missing));
    }

    // ── LintContext::new ─────────────────────────────────────────────

    #[test]
    #[serial]
    fn new_context_loads_manifests_from_cwd() {
        let _guard = crate::test_helpers::CwdGuard::new();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        // Create plugin.json only; the rest stay Missing.
        std::fs::create_dir_all(".claude-plugin").unwrap();
        std::fs::write(".claude-plugin/plugin.json", r#"{"name": "test-plugin"}"#).unwrap();

        let ctx = LintContext::new(tmp.path().to_str().unwrap().to_string(), LintMode::Plugin);

        assert_eq!(ctx.mode, LintMode::Plugin);
        assert!(matches!(ctx.plugin_json, ManifestState::Parsed(_)));
        assert!(matches!(ctx.marketplace_json, ManifestState::Missing));
        assert!(matches!(ctx.hooks_json, ManifestState::Missing));
        assert!(matches!(ctx.settings_json, ManifestState::Missing));
    }

    #[test]
    #[serial]
    fn new_context_all_manifests_present() {
        let _guard = crate::test_helpers::CwdGuard::new();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude-plugin").unwrap();
        std::fs::create_dir_all(".claude").unwrap();
        std::fs::create_dir_all("hooks").unwrap();

        std::fs::write(".claude-plugin/plugin.json", r#"{"a":1}"#).unwrap();
        std::fs::write(".claude-plugin/marketplace.json", r#"{"b":2}"#).unwrap();
        std::fs::write("hooks/hooks.json", r#"{"c":3}"#).unwrap();
        std::fs::write(".claude/settings.json", r#"{"d":4}"#).unwrap();

        let ctx = LintContext::new(tmp.path().to_str().unwrap().to_string(), LintMode::Basic);

        assert_eq!(ctx.mode, LintMode::Basic);
        assert!(matches!(ctx.plugin_json, ManifestState::Parsed(_)));
        assert!(matches!(ctx.marketplace_json, ManifestState::Parsed(_)));
        assert!(matches!(ctx.hooks_json, ManifestState::Parsed(_)));
        assert!(matches!(ctx.settings_json, ManifestState::Parsed(_)));
    }

    #[test]
    #[serial]
    fn new_context_with_invalid_json() {
        let _guard = crate::test_helpers::CwdGuard::new();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude-plugin").unwrap();
        std::fs::write(".claude-plugin/plugin.json", "broken!!!").unwrap();

        let ctx = LintContext::new(tmp.path().to_str().unwrap().to_string(), LintMode::Plugin);

        assert!(matches!(ctx.plugin_json, ManifestState::Invalid(_)));
    }
}
