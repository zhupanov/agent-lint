use crate::config::LintConfig;
use crate::rules::LintRule;

/// Diagnostic severity after config resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// A single lint diagnostic with rule identity and resolved severity.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    #[allow(dead_code)] // stored for future use (e.g., JSON output, --list-rules)
    pub rule: LintRule,
    pub severity: Severity,
    #[allow(dead_code)] // read by #[cfg(test)] accessors (errors(), warnings())
    pub message: String,
}

/// Collects lint diagnostics, applying configuration-based filtering.
///
/// - Rules in `config.ignore` are completely suppressed (no output, no count).
/// - Rules in `config.warn` are downgraded to warnings (printed, but do not
///   contribute to the error count or exit code 1).
/// - All other rules are errors.
pub struct DiagnosticCollector {
    config: LintConfig,
    diagnostics: Vec<Diagnostic>,
    suppressed_count: usize,
}

impl DiagnosticCollector {
    /// Create a collector with default config (all rules enabled as errors).
    /// Used by tests and when no config file is present.
    #[cfg(test)]
    pub fn new() -> Self {
        Self {
            config: LintConfig::default(),
            diagnostics: Vec::new(),
            suppressed_count: 0,
        }
    }

    /// Create a collector with the given configuration.
    pub fn with_config(config: LintConfig) -> Self {
        Self {
            config,
            diagnostics: Vec::new(),
            suppressed_count: 0,
        }
    }

    /// Report a diagnostic for the given rule. Checks config to determine
    /// disposition: suppress (ignore), downgrade to warning, or record as error.
    /// Non-suppressed diagnostics are printed immediately to stderr.
    pub fn report(&mut self, rule: LintRule, msg: &str) {
        if self.config.ignore.contains(&rule) {
            self.suppressed_count += 1;
            return;
        }

        let severity = if self.config.warn.contains(&rule) {
            Severity::Warning
        } else {
            Severity::Error
        };

        let label = match severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };

        eprintln!("{label}[{}/{}]: {msg}", rule.code(), rule.name());

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
    fn ignored_rule_is_suppressed() {
        let config = LintConfig {
            ignore: HashSet::from([LintRule::PluginJsonMissing]),
            warn: HashSet::new(),
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
            ignore: HashSet::new(),
            warn: HashSet::from([LintRule::SecurityMdMissing]),
        };
        let mut diag = DiagnosticCollector::with_config(config);
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
            ignore: HashSet::from([LintRule::PluginJsonMissing]),
            warn: HashSet::from([LintRule::SecurityMdMissing]),
        };
        let mut diag = DiagnosticCollector::with_config(config);
        diag.report(LintRule::PluginJsonMissing, "suppressed");
        diag.report(LintRule::SecurityMdMissing, "warned");
        diag.report(LintRule::HooksJsonMissing, "errored");
        assert_eq!(diag.error_count(), 1);
        assert_eq!(diag.warning_count(), 1);
        assert_eq!(diag.suppressed_count(), 1);
    }
}
