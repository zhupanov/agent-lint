use crate::rules::LintRule;
use serde::Deserialize;
use std::collections::HashSet;
use std::path::Path;

/// Raw TOML structure for deserialization.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    lint: Option<RawLintSection>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawLintSection {
    #[serde(default)]
    ignore: Vec<String>,
    #[serde(default)]
    warn: Vec<String>,
}

/// Resolved lint configuration. Rules in `ignore` are completely suppressed.
/// Rules in `warn` are downgraded from errors to warnings. If a rule appears
/// in both, `ignore` wins.
#[derive(Debug, Default)]
pub struct LintConfig {
    pub ignore: HashSet<LintRule>,
    pub warn: HashSet<LintRule>,
}

impl LintConfig {
    /// Load configuration from `claude-lint.toml` in the given repo root.
    ///
    /// - Missing file → default (empty) config.
    /// - Malformed TOML or unknown rule code/name → `Err(msg)`.
    pub fn load(repo_root: &str) -> Result<Self, String> {
        let path = Path::new(repo_root).join("claude-lint.toml");
        if !path.is_file() {
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;

        let raw: RawConfig = toml::from_str(&content)
            .map_err(|e| format!("{}: {e}", path.display()))?;

        let section = raw.lint.unwrap_or_default();

        let mut ignore = HashSet::new();
        for entry in &section.ignore {
            let rule = LintRule::from_code_or_name(entry).ok_or_else(|| {
                format!(
                    "{}: unknown rule in ignore list: '{entry}'. Use a valid code (e.g. M001) or name (e.g. plugin-json-missing).",
                    path.display()
                )
            })?;
            ignore.insert(rule);
        }

        let mut warn = HashSet::new();
        for entry in &section.warn {
            let rule = LintRule::from_code_or_name(entry).ok_or_else(|| {
                format!(
                    "{}: unknown rule in warn list: '{entry}'. Use a valid code (e.g. M001) or name (e.g. plugin-json-missing).",
                    path.display()
                )
            })?;
            // ignore wins over warn
            if !ignore.contains(&rule) {
                warn.insert(rule);
            }
        }

        Ok(Self { ignore, warn })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[serial_test::serial]
    fn missing_config_file_returns_default() {
        let tmp = tempfile::tempdir().unwrap();
        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert!(config.ignore.is_empty());
        assert!(config.warn.is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn valid_config_by_code() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("claude-lint.toml"),
            "[lint]\nignore = [\"M001\"]\nwarn = [\"G005\"]\n",
        )
        .unwrap();
        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert!(config.ignore.contains(&LintRule::PluginJsonMissing));
        assert!(config.warn.contains(&LintRule::SecurityMdMissing));
    }

    #[test]
    #[serial_test::serial]
    fn valid_config_by_name() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("claude-lint.toml"),
            "[lint]\nignore = [\"plugin-json-missing\"]\nwarn = [\"security-md-missing\"]\n",
        )
        .unwrap();
        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert!(config.ignore.contains(&LintRule::PluginJsonMissing));
        assert!(config.warn.contains(&LintRule::SecurityMdMissing));
    }

    #[test]
    #[serial_test::serial]
    fn ignore_wins_over_warn() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("claude-lint.toml"),
            "[lint]\nignore = [\"M001\"]\nwarn = [\"M001\"]\n",
        )
        .unwrap();
        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert!(config.ignore.contains(&LintRule::PluginJsonMissing));
        assert!(!config.warn.contains(&LintRule::PluginJsonMissing));
    }

    #[test]
    #[serial_test::serial]
    fn unknown_code_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("claude-lint.toml"),
            "[lint]\nignore = [\"X999\"]\n",
        )
        .unwrap();
        let err = LintConfig::load(tmp.path().to_str().unwrap()).unwrap_err();
        assert!(err.contains("unknown rule"), "Expected unknown rule error, got: {err}");
        assert!(err.contains("X999"));
    }

    #[test]
    #[serial_test::serial]
    fn malformed_toml_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("claude-lint.toml"),
            "not valid toml {{{\n",
        )
        .unwrap();
        let err = LintConfig::load(tmp.path().to_str().unwrap()).unwrap_err();
        assert!(!err.is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn empty_lint_section_is_valid() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("claude-lint.toml"),
            "[lint]\n",
        )
        .unwrap();
        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert!(config.ignore.is_empty());
        assert!(config.warn.is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn no_lint_section_is_valid() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("claude-lint.toml"),
            "# empty config\n",
        )
        .unwrap();
        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert!(config.ignore.is_empty());
        assert!(config.warn.is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn typo_in_section_name_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("claude-lint.toml"),
            "[lnt]\nignore = [\"M001\"]\n",
        )
        .unwrap();
        let err = LintConfig::load(tmp.path().to_str().unwrap()).unwrap_err();
        assert!(err.contains("unknown field"), "Expected unknown field error, got: {err}");
    }

    #[test]
    #[serial_test::serial]
    fn typo_in_field_name_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("claude-lint.toml"),
            "[lint]\nwran = [\"M001\"]\n",
        )
        .unwrap();
        let err = LintConfig::load(tmp.path().to_str().unwrap()).unwrap_err();
        assert!(err.contains("unknown field"), "Expected unknown field error, got: {err}");
    }
}
