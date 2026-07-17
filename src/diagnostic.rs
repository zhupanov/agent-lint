use std::collections::HashSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::config::LintConfig;
use crate::rules::{DefaultSeverity, LintRule};

/// Diagnostic severity after config resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// A single lint diagnostic with rule identity and resolved severity.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub rule: LintRule,
    pub severity: Severity,
    #[allow(dead_code)] // consumed by autofix and available through diagnostics()
    pub subject_path: Option<PathBuf>,
    #[allow(dead_code)] // read by #[cfg(test)] accessors and available via diagnostics()
    pub message: String,
}

/// Collects lint diagnostics, applying configuration-based filtering.
///
/// Priority: `config.suppress` (suppress with count) > `config.error` (promote
/// to error) > `config.warn` (downgrade to warning) > `default_severity()`
/// (compiled-in default: error or silently skipped).
pub struct DiagnosticCollector {
    config: LintConfig,
    diagnostics: Vec<Diagnostic>,
    suppressed_count: usize,
    used_overrides: HashSet<(usize, LintRule)>,
    current_subject_path: Option<PathBuf>,
    writer: Box<dyn Write>,
}

impl DiagnosticCollector {
    pub fn config(&self) -> &LintConfig {
        &self.config
    }
    /// Create a collector with default config. Rules fall through to their
    /// compiled-in `default_severity()`: default-error rules fire as errors,
    /// default-warning rules fire as warnings, default-suppressed rules are
    /// silently skipped.
    #[cfg(test)]
    pub fn new() -> Self {
        Self {
            config: LintConfig::default(),
            diagnostics: Vec::new(),
            suppressed_count: 0,
            used_overrides: HashSet::new(),
            current_subject_path: None,
            writer: Box::new(io::sink()),
        }
    }

    /// Create a collector with all rules enabled as errors, including
    /// default-suppressed and default-warning rules. Use this in tests
    /// that need to verify non-default-error rules fire correctly.
    #[cfg(test)]
    pub fn new_all_enabled() -> Self {
        use crate::rules::{ALL_RULES, DefaultSeverity};
        let error: std::collections::HashSet<crate::rules::LintRule> = ALL_RULES
            .iter()
            .filter(|r| {
                matches!(
                    r.default_severity(),
                    DefaultSeverity::Suppressed | DefaultSeverity::Warning
                )
            })
            .copied()
            .collect();
        let config = LintConfig {
            error,
            ..LintConfig::default()
        };
        Self {
            config,
            diagnostics: Vec::new(),
            suppressed_count: 0,
            used_overrides: HashSet::new(),
            current_subject_path: None,
            writer: Box::new(io::sink()),
        }
    }

    /// Create a collector with the given configuration.
    pub fn with_config(config: LintConfig) -> Self {
        Self {
            config,
            diagnostics: Vec::new(),
            suppressed_count: 0,
            used_overrides: HashSet::new(),
            current_subject_path: None,
            writer: Box::new(io::stderr()),
        }
    }

    /// Create a collector that collects diagnostics silently (no stderr output).
    /// Used by the autofix loop to re-validate without spamming stderr.
    pub fn with_config_silent(config: LintConfig) -> Self {
        Self {
            config,
            diagnostics: Vec::new(),
            suppressed_count: 0,
            used_overrides: HashSet::new(),
            current_subject_path: None,
            writer: Box::new(io::sink()),
        }
    }

    /// Report a diagnostic for the given rule. Checks config and default
    /// severity to determine disposition. Priority: user suppress > user error >
    /// user warn > compiled default severity.
    pub fn report(&mut self, rule: LintRule, msg: &str) {
        let path = self.current_subject_path.clone();
        self.report_inner(rule, path.as_deref(), msg);
    }

    /// Report a diagnostic owned by one concrete repository path. The path is
    /// normalized and matched against per-file overrides before severity is
    /// resolved; display text remains unchanged.
    pub fn report_at(&mut self, rule: LintRule, path: impl AsRef<Path>, msg: &str) {
        self.report_inner(rule, Some(path.as_ref()), msg);
    }

    /// Run a file validator with an explicit diagnostic subject. Calls to
    /// `report` inside the closure are equivalent to `report_at` for this path;
    /// nested scopes restore the prior subject when they return.
    pub fn with_subject_path<R>(
        &mut self,
        path: impl AsRef<Path>,
        validate: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let previous = self
            .current_subject_path
            .replace(path.as_ref().to_path_buf());
        let result = validate(self);
        self.current_subject_path = previous;
        result
    }

    fn report_inner(&mut self, rule: LintRule, path: Option<&Path>, msg: &str) {
        // User suppress always wins — suppress and count.
        if self.config.suppress.contains(&rule) {
            self.suppressed_count += 1;
            return;
        }

        if let Some(path) = path {
            let matching = self.config.matching_override_indexes(rule, path);
            if !matching.is_empty() {
                self.used_overrides
                    .extend(matching.into_iter().map(|index| (index, rule)));
                self.suppressed_count += 1;
                return;
            }
        }

        // User error promotes to error (overrides default severity).
        // User warn downgrades to warning.
        // Otherwise, fall back to compiled-in default severity.
        let severity = if self.config.error.contains(&rule) {
            Severity::Error
        } else if self.config.warn.contains(&rule) {
            Severity::Warning
        } else {
            match rule.default_severity() {
                DefaultSeverity::Error => Severity::Error,
                DefaultSeverity::Warning => Severity::Warning,
                // Default-suppressed: silently skip (no count, no output).
                DefaultSeverity::Suppressed => return,
            }
        };

        let label = match severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };

        let _ = writeln!(
            self.writer,
            "{label}[{}/{}]: {msg}",
            rule.code(),
            rule.name()
        );

        self.diagnostics.push(Diagnostic {
            rule,
            severity,
            subject_path: path.map(|path| PathBuf::from(self.config.normalize_subject_path(path))),
            message: msg.to_string(),
        });
    }

    /// Emit one non-failing warning for each configured `(override, rule)`
    /// pair that suppressed no diagnostic in this visible lint pass.
    pub fn emit_unused_override_warnings(&mut self) {
        for warning in self.unused_override_warnings() {
            let _ = writeln!(self.writer, "{warning}");
        }
    }

    pub fn unused_override_warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        if !self.config.per_file_overrides_enabled() {
            return warnings;
        }
        for (index, entry) in self.config.overrides.iter().enumerate() {
            let mut rules: Vec<_> = entry.suppress.iter().copied().collect();
            rules.sort_by_key(|rule| rule.code());
            for rule in rules {
                if self.used_overrides.contains(&(index, rule)) {
                    continue;
                }
                let patterns = entry.files.join(", ");
                let reason = entry
                    .reason
                    .as_deref()
                    .map(|reason| format!("; reason: {reason}"))
                    .unwrap_or_default();
                warnings.push(format!(
                    "warning[config/unused-override]: {}/{} for [{}] suppressed no diagnostics{}",
                    rule.code(),
                    rule.name(),
                    patterns,
                    reason
                ));
            }
        }
        warnings
    }

    /// Number of diagnostics recorded as errors.
    pub fn error_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count()
    }

    /// Number of diagnostics recorded as warnings.
    pub fn warning_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .count()
    }

    /// Number of diagnostics that were completely suppressed by config.
    pub fn suppressed_count(&self) -> usize {
        self.suppressed_count
    }

    /// Return collected diagnostics for programmatic access (e.g., autofix).
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Return collected error messages for test assertions.
    /// Returns the human-readable message (without the code prefix) so that
    /// existing `contains()` assertions continue to work.
    #[cfg(test)]
    pub fn errors(&self) -> Vec<String> {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .map(|d| d.message.clone())
            .collect()
    }

    /// Return collected warning messages for test assertions.
    #[cfg(test)]
    #[allow(dead_code)]
    pub fn warnings(&self) -> Vec<String> {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .map(|d| d.message.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn config_with_overrides(source: &str) -> LintConfig {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("agent-lint.toml"), source).unwrap();
        LintConfig::load(tmp.path()).unwrap()
    }

    #[test]
    fn default_collector_treats_all_as_errors() {
        let mut diag = DiagnosticCollector::new();
        diag.report(LintRule::PluginJsonMissing, "test message");
        assert_eq!(diag.error_count(), 1);
        assert_eq!(diag.warning_count(), 0);
        assert_eq!(diag.suppressed_count(), 0);
    }

    #[test]
    fn suppressed_rule_is_suppressed() {
        let config = LintConfig {
            suppress: HashSet::from([LintRule::PluginJsonMissing]),
            error: HashSet::new(),
            warn: HashSet::new(),
            exclude: vec![],
            ..LintConfig::default()
        };
        let mut diag = DiagnosticCollector::with_config(config);
        diag.report(LintRule::PluginJsonMissing, "test message");
        assert_eq!(diag.error_count(), 0);
        assert_eq!(diag.warning_count(), 0);
        assert_eq!(diag.suppressed_count(), 1);
    }

    #[test]
    fn warned_rule_is_warning() {
        let config = LintConfig {
            suppress: HashSet::new(),
            error: HashSet::new(),
            warn: HashSet::from([LintRule::SecurityMdMissing]),
            exclude: vec![],
            ..LintConfig::default()
        };
        let mut diag = DiagnosticCollector::with_config(config);
        // SecurityMdMissing is default-warning; user warn still takes priority.
        diag.report(LintRule::SecurityMdMissing, "SECURITY.md missing");
        assert_eq!(diag.error_count(), 0);
        assert_eq!(diag.warning_count(), 1);
        assert_eq!(diag.suppressed_count(), 0);
    }

    #[test]
    fn errors_accessor_returns_messages() {
        let mut diag = DiagnosticCollector::new();
        diag.report(LintRule::PluginJsonMissing, "plugin.json is missing");
        let errors = diag.errors();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("is missing"));
    }

    #[test]
    fn mixed_severities() {
        let config = LintConfig {
            suppress: HashSet::from([LintRule::PluginJsonMissing]),
            error: HashSet::new(),
            warn: HashSet::from([LintRule::SecurityMdMissing]),
            exclude: vec![],
            ..LintConfig::default()
        };
        let mut diag = DiagnosticCollector::with_config(config);
        diag.report(LintRule::PluginJsonMissing, "suppressed");
        diag.report(LintRule::SecurityMdMissing, "warned");
        diag.report(LintRule::HooksJsonMissing, "errored");
        assert_eq!(diag.error_count(), 1);
        assert_eq!(diag.warning_count(), 1);
        assert_eq!(diag.suppressed_count(), 1);
    }

    #[test]
    fn error_set_promotes_to_error() {
        let config = LintConfig {
            suppress: HashSet::new(),
            error: HashSet::from([LintRule::NameVague]),
            warn: HashSet::new(),
            exclude: vec![],
            ..LintConfig::default()
        };
        let mut diag = DiagnosticCollector::with_config(config);
        // NameVague is default-warning; user error overrides to error.
        diag.report(LintRule::NameVague, "vague name");
        assert_eq!(diag.error_count(), 1);
        assert_eq!(diag.warning_count(), 0);
        assert_eq!(diag.suppressed_count(), 0);
    }

    #[test]
    fn default_suppressed_rule_is_silently_skipped() {
        let config = LintConfig {
            suppress: HashSet::new(),
            error: HashSet::new(),
            warn: HashSet::new(),
            exclude: vec![],
            ..LintConfig::default()
        };
        let mut diag = DiagnosticCollector::with_config(config);
        // NameNotGerund is default-suppressed — silently skipped, no count.
        diag.report(LintRule::NameNotGerund, "not gerund");
        assert_eq!(diag.error_count(), 0);
        assert_eq!(diag.warning_count(), 0);
        assert_eq!(diag.suppressed_count(), 0);
    }

    #[test]
    fn default_warning_rule_fires_as_warning() {
        let config = LintConfig {
            suppress: HashSet::new(),
            error: HashSet::new(),
            warn: HashSet::new(),
            exclude: vec![],
            ..LintConfig::default()
        };
        let mut diag = DiagnosticCollector::with_config(config);
        // NameVague is default-warning — fires as warning without config.
        diag.report(LintRule::NameVague, "vague name");
        assert_eq!(diag.error_count(), 0);
        assert_eq!(diag.warning_count(), 1);
        assert_eq!(diag.suppressed_count(), 0);
    }

    #[test]
    fn default_error_rule_fires_without_config() {
        let config = LintConfig {
            suppress: HashSet::new(),
            error: HashSet::new(),
            warn: HashSet::new(),
            exclude: vec![],
            ..LintConfig::default()
        };
        let mut diag = DiagnosticCollector::with_config(config);
        // PluginJsonMissing is default-error — fires as error.
        diag.report(LintRule::PluginJsonMissing, "missing");
        assert_eq!(diag.error_count(), 1);
    }

    #[test]
    fn override_suppresses_only_matching_rule_and_path() {
        let config = config_with_overrides(
            r#"
[lint]
[[lint.overrides]]
files = ["legacy/*.md"]
suppress = ["M001"]
"#,
        );
        let mut diag = DiagnosticCollector::with_config(config);
        diag.report_at(LintRule::PluginJsonMissing, "legacy/one.md", "suppressed");
        diag.report_at(LintRule::HooksJsonMissing, "legacy/one.md", "other rule");
        diag.report_at(LintRule::PluginJsonMissing, "current/one.md", "other path");

        assert_eq!(diag.suppressed_count(), 1);
        assert_eq!(diag.error_count(), 2);
        assert_eq!(
            diag.diagnostics()[0].subject_path.as_deref(),
            Some(Path::new("legacy/one.md"))
        );
        assert_eq!(
            diag.diagnostics()[1].subject_path.as_deref(),
            Some(Path::new("current/one.md"))
        );
    }

    #[test]
    fn scoped_subject_is_structured_and_pathless_diagnostic_does_not_match() {
        let config = config_with_overrides(
            "[lint]\n[[lint.overrides]]\nfiles = [\"docs/*.md\"]\nsuppress = [\"M001\"]\n",
        );
        let mut diag = DiagnosticCollector::with_config(config);
        diag.with_subject_path("docs/one.md", |diag| {
            diag.report(LintRule::PluginJsonMissing, "scoped");
        });
        diag.report(LintRule::PluginJsonMissing, "repository-wide");

        assert_eq!(diag.suppressed_count(), 1);
        assert_eq!(diag.error_count(), 1);
        assert_eq!(diag.diagnostics()[0].subject_path, None);
    }

    #[test]
    fn overlapping_blocks_are_each_marked_used() {
        let config = config_with_overrides(
            r#"
[lint]
[[lint.overrides]]
files = ["docs/*.md"]
suppress = ["M001"]
[[lint.overrides]]
files = ["docs/one.md"]
suppress = ["M001"]
"#,
        );
        let mut diag = DiagnosticCollector::with_config(config);
        diag.report_at(LintRule::PluginJsonMissing, "docs/one.md", "suppressed");

        assert_eq!(diag.suppressed_count(), 1);
        assert!(diag.unused_override_warnings().is_empty());
    }

    #[test]
    fn unused_reporting_tracks_each_rule_and_includes_audit_context() {
        let config = config_with_overrides(
            r#"
[lint]
[[lint.overrides]]
files = ["docs/*.md"]
suppress = ["M001", "H001"]
reason = "legacy publication contract"
"#,
        );
        let mut diag = DiagnosticCollector::with_config(config);
        diag.report_at(LintRule::PluginJsonMissing, "docs/one.md", "suppressed");

        let warnings = diag.unused_override_warnings();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("H001/hooks-json-missing"));
        assert!(warnings[0].contains("docs/*.md"));
        assert!(warnings[0].contains("legacy publication contract"));
    }

    #[test]
    fn global_suppression_wins_without_marking_override_used() {
        let config = config_with_overrides(
            r#"
[lint]
suppress = ["M001"]
[[lint.overrides]]
files = ["docs/*.md"]
suppress = ["M001"]
"#,
        );
        let mut diag = DiagnosticCollector::with_config(config);
        diag.report_at(
            LintRule::PluginJsonMissing,
            "docs/one.md",
            "globally suppressed",
        );

        assert_eq!(diag.suppressed_count(), 1);
        assert_eq!(diag.unused_override_warnings().len(), 1);
    }

    #[test]
    fn override_precedes_global_error_and_warn_lists() {
        let config = config_with_overrides(
            r#"
[lint]
error = ["M001"]
warn = ["H001"]
[[lint.overrides]]
files = ["docs/*.md"]
suppress = ["M001", "H001"]
"#,
        );
        let mut diag = DiagnosticCollector::with_config(config);
        diag.report_at(
            LintRule::PluginJsonMissing,
            "docs/one.md",
            "promoted elsewhere",
        );
        diag.report_at(
            LintRule::HooksJsonMissing,
            "docs/one.md",
            "warned elsewhere",
        );

        assert_eq!(diag.suppressed_count(), 2);
        assert_eq!(diag.error_count(), 0);
        assert_eq!(diag.warning_count(), 0);
    }
}
