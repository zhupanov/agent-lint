use std::io::{self, Write};

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
    writer: Box<dyn Write>,
}

impl DiagnosticCollector {
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
            writer: Box::new(io::sink()),
        }
    }

    /// Create a collector with the given configuration.
    pub fn with_config(config: LintConfig) -> Self {
        Self {
            config,
            diagnostics: Vec::new(),
            suppressed_count: 0,
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
            writer: Box::new(io::sink()),
        }
    }

    /// Report a diagnostic for the given rule. Checks config and default
    /// severity to determine disposition. Priority: user suppress > user error >
    /// user warn > compiled default severity.
    pub fn report(&mut self, rule: LintRule, msg: &str) {
        // User suppress always wins — suppress and count.
        if self.config.suppress.contains(&rule) {
            self.suppressed_count += 1;
            return;
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
            message: msg.to_string(),
        });
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
        };
        let mut diag = DiagnosticCollector::with_config(config);
        // PluginJsonMissing is default-error — fires as error.
        diag.report(LintRule::PluginJsonMissing, "missing");
        assert_eq!(diag.error_count(), 1);
    }
}
