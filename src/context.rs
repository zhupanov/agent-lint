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
