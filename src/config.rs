use crate::rules::{ALL_RULES, LintRule};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::Path;

/// CLI strictness mode. Applied as a one-shot transformation to LintConfig
/// before creating DiagnosticCollector. Not configurable via TOML.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CliMode {
    #[default]
    Normal,
    /// Promotes warn-listed rules to errors (except too-long rules).
    /// Respects ignore list. Default-suppressed rules stay suppressed.
    Pedantic,
    /// All 104 rules fire as errors. Ignores all TOML severity config.
    All,
}

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
    error: Vec<String>,
    #[serde(default)]
    warn: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
}

/// Resolved lint configuration. Rules in `ignore` are completely suppressed.
/// Rules in `error` are promoted to errors (overriding default severity).
/// Rules in `warn` are downgraded to warnings. Priority: ignore > error > warn.
/// Rules not in any set fall back to `LintRule::default_severity()`.
#[derive(Debug, Default, Clone)]
pub struct LintConfig {
    pub ignore: HashSet<LintRule>,
    pub error: HashSet<LintRule>,
    pub warn: HashSet<LintRule>,
    pub exclude: Vec<String>,
}

/// Compiled glob set for file exclusion. Wraps `globset::GlobSet` and provides
/// path normalization. Use `ExcludeSet::default()` for an empty set that matches
/// nothing.
pub struct ExcludeSet {
    globs: GlobSet,
}

impl Default for ExcludeSet {
    fn default() -> Self {
        Self {
            globs: GlobSet::empty(),
        }
    }
}

impl ExcludeSet {
    /// Build an `ExcludeSet` from raw glob pattern strings.
    /// Returns `Err` if any pattern is invalid.
    pub fn new(patterns: &[String]) -> Result<Self, String> {
        if patterns.is_empty() {
            return Ok(Self::default());
        }
        let mut builder = GlobSetBuilder::new();
        for pattern in patterns {
            let glob = GlobBuilder::new(pattern)
                .literal_separator(true)
                .build()
                .map_err(|e| format!("invalid exclude glob pattern '{pattern}': {e}"))?;
            builder.add(glob);
        }
        let globs = builder
            .build()
            .map_err(|e| format!("failed to compile exclude patterns: {e}"))?;
        Ok(Self { globs })
    }

    /// Check whether a path should be excluded from linting.
    /// Normalizes the path before matching: strips leading `./` and
    /// converts backslashes to forward slashes.
    pub fn is_excluded(&self, path: &str) -> bool {
        let normalized = normalize_path(path);
        self.globs.is_match(&normalized)
    }
}

/// Normalize a path for consistent glob matching: strip leading `./`,
/// convert `\` to `/`.
pub fn normalize_path(path: &str) -> String {
    let s = path.replace('\\', "/");
    s.strip_prefix("./").unwrap_or(&s).to_string()
}

impl LintConfig {
    /// Load configuration from `agent-lint.toml` in the given repo root.
    ///
    /// - Missing file → default (empty) config.
    /// - Malformed TOML or unknown rule code/name → `Err(msg)`.
    pub fn load(repo_root: &str) -> Result<Self, String> {
        let path = Path::new(repo_root).join("agent-lint.toml");
        if !path.is_file() {
            let legacy = Path::new(repo_root).join("claude-lint.toml");
            if legacy.is_file() {
                eprintln!(
                    "warning: found 'claude-lint.toml' which is no longer read; rename it to 'agent-lint.toml'"
                );
            }
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;

        let raw: RawConfig =
            toml::from_str(&content).map_err(|e| format!("{}: {e}", path.display()))?;

        let section = raw.lint.unwrap_or_default();

        // Parse error list first (user-explicit error promotions).
        let mut error = HashSet::new();
        for entry in &section.error {
            let rule = LintRule::from_code_or_name(entry).ok_or_else(|| {
                format!(
                    "{}: unknown rule in error list: '{entry}'. Use a valid code (e.g. M001) or name (e.g. plugin-json-missing).",
                    path.display()
                )
            })?;
            error.insert(rule);
        }

        // Parse warn list. error wins over warn.
        let mut warn = HashSet::new();
        for entry in &section.warn {
            let rule = LintRule::from_code_or_name(entry).ok_or_else(|| {
                format!(
                    "{}: unknown rule in warn list: '{entry}'. Use a valid code (e.g. M001) or name (e.g. plugin-json-missing).",
                    path.display()
                )
            })?;
            if !error.contains(&rule) {
                warn.insert(rule);
            }
        }

        // Parse ignore list. ignore wins over error and warn.
        let mut ignore = HashSet::new();
        for entry in &section.ignore {
            let rule = LintRule::from_code_or_name(entry).ok_or_else(|| {
                format!(
                    "{}: unknown rule in ignore list: '{entry}'. Use a valid code (e.g. M001) or name (e.g. plugin-json-missing).",
                    path.display()
                )
            })?;
            error.remove(&rule);
            warn.remove(&rule);
            ignore.insert(rule);
        }

        // Validate exclude patterns at load time (compile a throwaway GlobSet).
        ExcludeSet::new(&section.exclude).map_err(|e| format!("{}: {e}", path.display()))?;

        Ok(Self {
            ignore,
            error,
            warn,
            exclude: section.exclude,
        })
    }

    /// Apply CLI strictness mode. Transforms the ignore/error/warn sets
    /// so that `DiagnosticCollector::report()` needs no changes.
    ///
    /// - `Pedantic`: moves warn entries to error (except too-long rules).
    ///   Respects ignore list. Default-suppressed rules stay suppressed.
    /// - `All`: clears ignore/warn, fills error with all rules. Overrides
    ///   all TOML severity config. File exclusions (`exclude`) are not
    ///   affected — `--all` changes rule severity, not file selection.
    pub fn apply_cli_mode(&mut self, mode: CliMode) {
        match mode {
            CliMode::Normal => {}
            CliMode::Pedantic => {
                let to_promote: Vec<_> = self
                    .warn
                    .iter()
                    .filter(|r| !r.is_too_long())
                    .copied()
                    .collect();
                for r in to_promote {
                    self.warn.remove(&r);
                    self.error.insert(r);
                }
            }
            CliMode::All => {
                self.ignore.clear();
                self.warn.clear();
                self.error.clear();
                for r in ALL_RULES {
                    self.error.insert(*r);
                }
            }
        }
    }

    /// Build a compiled `ExcludeSet` from this config's exclude patterns.
    /// This should be called once after loading and passed through to validators.
    pub fn build_exclude_set(&self) -> ExcludeSet {
        // Patterns were already validated in load(), so unwrap is safe.
        ExcludeSet::new(&self.exclude).expect("exclude patterns were validated at load time")
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
        assert!(config.error.is_empty());
        assert!(config.warn.is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn valid_config_by_code() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
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
            tmp.path().join("agent-lint.toml"),
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
            tmp.path().join("agent-lint.toml"),
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
            tmp.path().join("agent-lint.toml"),
            "[lint]\nignore = [\"X999\"]\n",
        )
        .unwrap();
        let err = LintConfig::load(tmp.path().to_str().unwrap()).unwrap_err();
        assert!(
            err.contains("unknown rule"),
            "Expected unknown rule error, got: {err}"
        );
        assert!(err.contains("X999"));
    }

    #[test]
    #[serial_test::serial]
    fn malformed_toml_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("agent-lint.toml"), "not valid toml {{{\n").unwrap();
        let err = LintConfig::load(tmp.path().to_str().unwrap()).unwrap_err();
        assert!(!err.is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn empty_lint_section_is_valid() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("agent-lint.toml"), "[lint]\n").unwrap();
        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert!(config.ignore.is_empty());
        assert!(config.error.is_empty());
        assert!(config.warn.is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn no_lint_section_is_valid() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("agent-lint.toml"), "# empty config\n").unwrap();
        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert!(config.ignore.is_empty());
        assert!(config.error.is_empty());
        assert!(config.warn.is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn typo_in_section_name_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lnt]\nignore = [\"M001\"]\n",
        )
        .unwrap();
        let err = LintConfig::load(tmp.path().to_str().unwrap()).unwrap_err();
        assert!(
            err.contains("unknown field"),
            "Expected unknown field error, got: {err}"
        );
    }

    #[test]
    #[serial_test::serial]
    fn typo_in_field_name_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\nwran = [\"M001\"]\n",
        )
        .unwrap();
        let err = LintConfig::load(tmp.path().to_str().unwrap()).unwrap_err();
        assert!(
            err.contains("unknown field"),
            "Expected unknown field error, got: {err}"
        );
    }

    // ── Error list ──────────────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn error_list_parsed() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\nerror = [\"S033\", \"G005\"]\n",
        )
        .unwrap();
        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert!(config.error.contains(&LintRule::NameVague));
        assert!(config.error.contains(&LintRule::SecurityMdMissing));
    }

    #[test]
    #[serial_test::serial]
    fn error_list_by_name() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\nerror = [\"name-vague\"]\n",
        )
        .unwrap();
        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert!(config.error.contains(&LintRule::NameVague));
    }

    #[test]
    #[serial_test::serial]
    fn unknown_error_code_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\nerror = [\"X999\"]\n",
        )
        .unwrap();
        let err = LintConfig::load(tmp.path().to_str().unwrap()).unwrap_err();
        assert!(
            err.contains("unknown rule"),
            "Expected unknown rule error, got: {err}"
        );
        assert!(err.contains("X999"));
    }

    #[test]
    #[serial_test::serial]
    fn error_wins_over_warn() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\nerror = [\"S033\"]\nwarn = [\"S033\"]\n",
        )
        .unwrap();
        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert!(config.error.contains(&LintRule::NameVague));
        assert!(!config.warn.contains(&LintRule::NameVague));
    }

    #[test]
    #[serial_test::serial]
    fn ignore_wins_over_error() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\nignore = [\"S033\"]\nerror = [\"S033\"]\n",
        )
        .unwrap();
        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert!(config.ignore.contains(&LintRule::NameVague));
        assert!(!config.error.contains(&LintRule::NameVague));
    }

    #[test]
    #[serial_test::serial]
    fn ignore_wins_over_error_and_warn() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\nignore = [\"S033\"]\nerror = [\"S033\"]\nwarn = [\"S033\"]\n",
        )
        .unwrap();
        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert!(config.ignore.contains(&LintRule::NameVague));
        assert!(!config.error.contains(&LintRule::NameVague));
        assert!(!config.warn.contains(&LintRule::NameVague));
    }

    #[test]
    #[serial_test::serial]
    fn missing_error_defaults_to_empty() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\nignore = [\"M001\"]\n",
        )
        .unwrap();
        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert!(config.error.is_empty());
    }

    // ── Exclude patterns ────────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn exclude_parsed_from_config() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\nexclude = [\"docs/*.md\", \"skills/internal/**\"]\n",
        )
        .unwrap();
        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert_eq!(config.exclude.len(), 2);
        assert_eq!(config.exclude[0], "docs/*.md");
        assert_eq!(config.exclude[1], "skills/internal/**");
    }

    #[test]
    #[serial_test::serial]
    fn empty_exclude_is_valid() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("agent-lint.toml"), "[lint]\nexclude = []\n").unwrap();
        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert!(config.exclude.is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn missing_exclude_defaults_to_empty() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\nignore = [\"M001\"]\n",
        )
        .unwrap();
        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert!(config.exclude.is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn invalid_exclude_pattern_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\nexclude = [\"[invalid\"]\n",
        )
        .unwrap();
        let err = LintConfig::load(tmp.path().to_str().unwrap()).unwrap_err();
        assert!(
            err.contains("invalid exclude glob"),
            "Expected invalid glob error, got: {err}"
        );
    }

    #[test]
    #[serial_test::serial]
    fn exclude_not_array_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\nexclude = \"not-an-array\"\n",
        )
        .unwrap();
        let err = LintConfig::load(tmp.path().to_str().unwrap()).unwrap_err();
        assert!(!err.is_empty());
    }

    // ── ExcludeSet ──────────────────────────────────────────────────

    #[test]
    fn exclude_set_empty_matches_nothing() {
        let set = ExcludeSet::default();
        assert!(!set.is_excluded("skills/foo/SKILL.md"));
        assert!(!set.is_excluded("anything"));
    }

    #[test]
    fn exclude_set_star_matches_single_level() {
        let set = ExcludeSet::new(&["docs/*.md".to_string()]).unwrap();
        assert!(set.is_excluded("docs/readme.md"));
        assert!(set.is_excluded("docs/architecture.md"));
        // * does NOT match across path separators
        assert!(!set.is_excluded("docs/sub/nested.md"));
    }

    #[test]
    fn exclude_set_double_star_matches_recursive() {
        let set = ExcludeSet::new(&["docs/**/*.md".to_string()]).unwrap();
        assert!(set.is_excluded("docs/readme.md"));
        assert!(set.is_excluded("docs/sub/nested.md"));
        assert!(set.is_excluded("docs/a/b/c.md"));
    }

    #[test]
    fn exclude_set_skill_pattern() {
        let set = ExcludeSet::new(&["skills/my-skill/**".to_string()]).unwrap();
        assert!(set.is_excluded("skills/my-skill/SKILL.md"));
        assert!(set.is_excluded("skills/my-skill/scripts/helper.sh"));
        assert!(!set.is_excluded("skills/other-skill/SKILL.md"));
    }

    #[test]
    fn exclude_set_normalizes_dot_slash() {
        let set = ExcludeSet::new(&["skills/*/SKILL.md".to_string()]).unwrap();
        // With leading ./
        assert!(set.is_excluded("./skills/my-skill/SKILL.md"));
        // Without leading ./
        assert!(set.is_excluded("skills/my-skill/SKILL.md"));
    }

    #[test]
    fn exclude_set_normalizes_backslashes() {
        let set = ExcludeSet::new(&["skills/*/SKILL.md".to_string()]).unwrap();
        assert!(set.is_excluded("skills\\my-skill\\SKILL.md"));
    }

    #[test]
    fn exclude_set_multiple_patterns() {
        let set = ExcludeSet::new(&[
            "agents/internal.md".to_string(),
            "skills/deprecated-*/**".to_string(),
        ])
        .unwrap();
        assert!(set.is_excluded("agents/internal.md"));
        assert!(set.is_excluded("skills/deprecated-old/SKILL.md"));
        assert!(!set.is_excluded("agents/general.md"));
        assert!(!set.is_excluded("skills/active/SKILL.md"));
    }

    #[test]
    fn exclude_set_exact_file() {
        let set = ExcludeSet::new(&["CLAUDE.md".to_string()]).unwrap();
        assert!(set.is_excluded("CLAUDE.md"));
        assert!(!set.is_excluded("README.md"));
    }

    #[test]
    fn exclude_set_invalid_pattern_error() {
        let result = ExcludeSet::new(&["[invalid".to_string()]);
        assert!(result.is_err());
    }

    // ── normalize_path ──────────────────────────────────────────────

    #[test]
    fn normalize_strips_dot_slash() {
        assert_eq!(
            normalize_path("./skills/foo/SKILL.md"),
            "skills/foo/SKILL.md"
        );
    }

    #[test]
    fn normalize_no_dot_slash_unchanged() {
        assert_eq!(normalize_path("skills/foo/SKILL.md"), "skills/foo/SKILL.md");
    }

    #[test]
    fn normalize_backslash_to_forward() {
        assert_eq!(
            normalize_path("skills\\foo\\SKILL.md"),
            "skills/foo/SKILL.md"
        );
    }

    #[test]
    fn normalize_mixed_separators() {
        assert_eq!(
            normalize_path(".\\skills/foo\\SKILL.md"),
            "skills/foo/SKILL.md"
        );
    }

    // ── apply_cli_mode ─────────────────────────────────────────────

    #[test]
    fn apply_normal_no_change() {
        let mut config = LintConfig {
            ignore: HashSet::from([LintRule::PluginJsonMissing]),
            error: HashSet::from([LintRule::NameVague]),
            warn: HashSet::from([LintRule::SecurityMdMissing]),
            exclude: vec![],
        };
        config.apply_cli_mode(CliMode::Normal);
        assert!(config.ignore.contains(&LintRule::PluginJsonMissing));
        assert!(config.error.contains(&LintRule::NameVague));
        assert!(config.warn.contains(&LintRule::SecurityMdMissing));
    }

    #[test]
    fn apply_pedantic_moves_warn_to_error() {
        let mut config = LintConfig {
            ignore: HashSet::new(),
            error: HashSet::new(),
            warn: HashSet::from([LintRule::SecurityMdMissing, LintRule::TodoInSkill]),
            exclude: vec![],
        };
        config.apply_cli_mode(CliMode::Pedantic);
        assert!(config.error.contains(&LintRule::SecurityMdMissing));
        assert!(config.error.contains(&LintRule::TodoInSkill));
        assert!(config.warn.is_empty());
    }

    #[test]
    fn apply_pedantic_skips_too_long() {
        let mut config = LintConfig {
            ignore: HashSet::new(),
            error: HashSet::new(),
            warn: HashSet::from([
                LintRule::SecurityMdMissing,
                LintRule::BodyTooLong,
                LintRule::CompatTooLong,
            ]),
            exclude: vec![],
        };
        config.apply_cli_mode(CliMode::Pedantic);
        // Non-too-long rule promoted to error
        assert!(config.error.contains(&LintRule::SecurityMdMissing));
        // Too-long rules remain in warn
        assert!(config.warn.contains(&LintRule::BodyTooLong));
        assert!(config.warn.contains(&LintRule::CompatTooLong));
        assert!(!config.error.contains(&LintRule::BodyTooLong));
        assert!(!config.error.contains(&LintRule::CompatTooLong));
    }

    #[test]
    fn apply_pedantic_leaves_ignore_intact() {
        let mut config = LintConfig {
            ignore: HashSet::from([LintRule::PluginJsonMissing]),
            error: HashSet::new(),
            warn: HashSet::from([LintRule::SecurityMdMissing]),
            exclude: vec![],
        };
        config.apply_cli_mode(CliMode::Pedantic);
        assert!(config.ignore.contains(&LintRule::PluginJsonMissing));
        assert!(config.error.contains(&LintRule::SecurityMdMissing));
    }

    #[test]
    fn apply_pedantic_default_error_stays_error() {
        let mut config = LintConfig {
            ignore: HashSet::new(),
            error: HashSet::new(),
            warn: HashSet::new(),
            exclude: vec![],
        };
        config.apply_cli_mode(CliMode::Pedantic);
        // Default-error rules like PluginJsonMissing aren't in the error set,
        // but they fire as errors via default_severity() in report().
        // Pedantic doesn't need to touch them.
        assert!(config.error.is_empty());
    }

    #[test]
    fn apply_all_enables_everything() {
        let mut config = LintConfig {
            ignore: HashSet::from([LintRule::PluginJsonMissing]),
            error: HashSet::new(),
            warn: HashSet::from([LintRule::SecurityMdMissing]),
            exclude: vec!["docs/*.md".to_string()],
        };
        config.apply_cli_mode(CliMode::All);
        assert!(config.ignore.is_empty());
        assert!(config.warn.is_empty());
        assert_eq!(config.error.len(), 104);
        // Exclude is NOT cleared — it's about file paths, not rule severity
        assert_eq!(config.exclude.len(), 1);
    }

    #[test]
    fn apply_all_overrides_ignore() {
        let mut config = LintConfig {
            ignore: HashSet::from([LintRule::PluginJsonMissing, LintRule::NameVague]),
            error: HashSet::new(),
            warn: HashSet::new(),
            exclude: vec![],
        };
        config.apply_cli_mode(CliMode::All);
        assert!(config.ignore.is_empty());
        assert!(config.error.contains(&LintRule::PluginJsonMissing));
        assert!(config.error.contains(&LintRule::NameVague));
    }

    #[test]
    fn apply_all_includes_too_long_rules() {
        let mut config = LintConfig::default();
        config.apply_cli_mode(CliMode::All);
        assert!(config.error.contains(&LintRule::NameTooLong));
        assert!(config.error.contains(&LintRule::DescTooLong));
        assert!(config.error.contains(&LintRule::BodyTooLong));
        assert!(config.error.contains(&LintRule::CompatTooLong));
    }
}
