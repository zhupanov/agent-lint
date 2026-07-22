use std::borrow::Cow;
use std::collections::HashSet;
use std::io::Write;
use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::config::{LintConfig, RunPolicy};
use crate::rules::{DefaultSeverity, LintRule};

/// Replace every Unicode control character with a Rust-style `\u{…}` escape so
/// human-readable terminal output cannot interpret ESC/BEL/C1 sequences from
/// repository-controlled diagnostic text. Non-control input is returned borrowed.
pub(crate) fn sanitize_for_terminal(text: &str) -> Cow<'_, str> {
    if !text.chars().any(char::is_control) {
        return Cow::Borrowed(text);
    }
    let mut sanitized = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_control() {
            sanitized.extend(ch.escape_unicode());
        } else {
            sanitized.push(ch);
        }
    }
    Cow::Owned(sanitized)
}

/// Diagnostic severity after config resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// A one-based source position.
///
/// A missing column means that only the line is known. Unknown coordinates are
/// never inferred from the human message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourcePosition {
    line: usize,
    column: Option<usize>,
}

impl SourcePosition {
    pub fn line(line: usize) -> Self {
        assert!(line > 0, "source lines are one-based");
        Self { line, column: None }
    }

    pub fn point(line: usize, column: usize) -> Self {
        assert!(line > 0, "source lines are one-based");
        assert!(column > 0, "source columns are one-based");
        Self {
            line,
            column: Some(column),
        }
    }

    #[allow(dead_code)] // public output access for renderer leaves
    pub fn line_number(self) -> usize {
        self.line
    }

    #[allow(dead_code)] // public output access for renderer leaves
    pub fn column_number(self) -> Option<usize> {
        self.column
    }
}

/// A source span whose start is inclusive and whose optional end is exclusive.
///
/// Positions use one-based Unicode scalar columns. A point has no end. A range
/// always has columns at both ends so its exclusive boundary is unambiguous.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceSpan {
    start: SourcePosition,
    end: Option<SourcePosition>,
}

impl SourceSpan {
    pub fn line(line: usize) -> Self {
        Self {
            start: SourcePosition::line(line),
            end: None,
        }
    }

    #[allow(dead_code)] // reporting API for validators as they migrate
    pub fn point(line: usize, column: usize) -> Self {
        Self {
            start: SourcePosition::point(line, column),
            end: None,
        }
    }

    #[allow(dead_code)] // reporting API for validators as they migrate
    pub fn range(
        start_line: usize,
        start_column: usize,
        end_line: usize,
        end_column: usize,
    ) -> Self {
        let start = SourcePosition::point(start_line, start_column);
        let end = SourcePosition::point(end_line, end_column);
        assert!(
            (end_line, end_column) >= (start_line, start_column),
            "source range end precedes its start"
        );
        Self {
            start,
            end: Some(end),
        }
    }

    /// Convert a UTF-8 byte range into a one-based, end-exclusive source span.
    /// Returns `None` for out-of-bounds offsets or non-character boundaries.
    pub fn from_byte_range(source: &str, range: Range<usize>) -> Option<Self> {
        if range.start > range.end
            || range.end > source.len()
            || !source.is_char_boundary(range.start)
            || !source.is_char_boundary(range.end)
        {
            return None;
        }
        let start = position_at_offset(source, range.start);
        let end = position_at_offset(source, range.end);
        Some(Self {
            start,
            end: Some(end),
        })
    }

    #[allow(dead_code)] // public output access for renderer leaves
    pub fn start(self) -> SourcePosition {
        self.start
    }

    #[allow(dead_code)] // public output access for renderer leaves
    pub fn end(self) -> Option<SourcePosition> {
        self.end
    }
}

fn position_at_offset(source: &str, offset: usize) -> SourcePosition {
    let prefix = &source[..offset];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit_once('\n')
        .map_or(prefix, |(_, current_line)| current_line)
        .chars()
        .count()
        + 1;
    SourcePosition::point(line, column)
}

#[allow(dead_code)] // used by the incremental evidence reporting API
const MAX_EVIDENCE_BYTES: usize = 512;
#[allow(dead_code)] // used by the incremental evidence reporting API
const REDACTED_EVIDENCE: &str = "[redacted: possible secret]";

/// Optional renderer-independent details supplied by a validator.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiagnosticMetadata {
    location: Option<SourceSpan>,
    evidence: Option<String>,
    suggestion: Option<String>,
    related_subjects: Vec<PathBuf>,
}

impl DiagnosticMetadata {
    pub fn at_line(line: usize) -> Self {
        Self::default().with_location(SourceSpan::line(line))
    }

    #[allow(dead_code)] // reporting API for validators as they migrate
    pub fn at_point(line: usize, column: usize) -> Self {
        Self::default().with_location(SourceSpan::point(line, column))
    }

    pub fn with_location(mut self, location: SourceSpan) -> Self {
        self.location = Some(location);
        self
    }

    /// Attach bounded display evidence. Values matching the shared possible-
    /// secret heuristic are replaced with a stable redaction marker.
    #[allow(dead_code)] // reporting API for validators as they migrate
    pub fn with_evidence(mut self, evidence: impl AsRef<str>) -> Self {
        self.evidence = Some(safe_evidence(evidence.as_ref()));
        self
    }

    /// Attach the stable evidence redaction marker without consulting source
    /// content. Use this when the very fact being diagnosed is sensitive.
    pub fn with_redacted_evidence(mut self) -> Self {
        self.evidence = Some(REDACTED_EVIDENCE.to_string());
        self
    }

    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }

    /// Attach additional repository-relative subjects for multi-source findings.
    ///
    /// Related subjects are structured identity only. They do not become the
    /// diagnostic's `subject_path` and therefore do not participate in
    /// per-file override matching (I-Diag-2).
    pub fn with_related_subjects<I, P>(mut self, subjects: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        self.related_subjects = subjects
            .into_iter()
            .map(|path| path.as_ref().to_path_buf())
            .collect();
        self
    }
}

#[allow(dead_code)] // used by the incremental evidence reporting API
fn safe_evidence(evidence: &str) -> String {
    if crate::sensitive::contains_sensitive_evidence(evidence) {
        return REDACTED_EVIDENCE.to_string();
    }
    let evidence = evidence.trim();
    if evidence.len() <= MAX_EVIDENCE_BYTES {
        return evidence.to_string();
    }
    let mut end = MAX_EVIDENCE_BYTES - '…'.len_utf8();
    while !evidence.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &evidence[..end])
}

/// A single lint diagnostic with rule identity, resolved severity, and
/// renderer-independent structured metadata.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub rule: LintRule,
    pub severity: Severity,
    #[allow(dead_code)] // consumed by autofix and available through diagnostics()
    pub subject_path: Option<PathBuf>,
    /// Additional repository-relative subjects for multi-source findings.
    /// Empty for ordinary single-subject diagnostics. Never used for per-file
    /// override matching; only `subject_path` participates in that policy.
    pub related_subjects: Vec<PathBuf>,
    #[allow(dead_code)] // read by #[cfg(test)] accessors and available via diagnostics()
    pub message: String,
    #[allow(dead_code)] // consumed by renderer leaves through diagnostics()
    pub location: Option<SourceSpan>,
    #[allow(dead_code)] // consumed by renderer leaves through diagnostics()
    pub evidence: Option<String>,
    #[allow(dead_code)] // consumed by renderer leaves through diagnostics()
    pub suggestion: Option<String>,
}

/// Collects lint diagnostics, applying configuration-based filtering.
///
/// Priority: `config.suppress` (suppress with count) > `config.error` (promote
/// to error) > `config.warn` (downgrade to warning) > `default_severity()`
/// (compiled-in default: error or silently skipped).
pub struct DiagnosticCollector {
    config: LintConfig,
    run_policy: RunPolicy,
    diagnostics: Vec<Diagnostic>,
    suppressed_count: usize,
    used_overrides: HashSet<(usize, LintRule)>,
    current_subject_path: Option<PathBuf>,
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
            run_policy: RunPolicy::default(),
            diagnostics: Vec::new(),
            suppressed_count: 0,
            used_overrides: HashSet::new(),
            current_subject_path: None,
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
            run_policy: RunPolicy::default(),
            diagnostics: Vec::new(),
            suppressed_count: 0,
            used_overrides: HashSet::new(),
            current_subject_path: None,
        }
    }

    /// Create a collector with the given configuration.
    #[cfg(test)]
    pub fn with_config(config: LintConfig) -> Self {
        Self::with_run_policy(config, RunPolicy::default())
    }

    /// Create a collector with resolved configuration and invocation policy.
    pub fn with_run_policy(config: LintConfig, run_policy: RunPolicy) -> Self {
        Self {
            config,
            run_policy,
            diagnostics: Vec::new(),
            suppressed_count: 0,
            used_overrides: HashSet::new(),
            current_subject_path: None,
        }
    }

    /// Create a collector for a validation pass that will not be rendered.
    /// Used by the autofix loop to re-validate without spamming stderr.
    #[cfg(test)]
    pub fn with_config_silent(config: LintConfig) -> Self {
        Self::with_config(config)
    }

    /// Report a diagnostic for the given rule. Checks config and default
    /// severity to determine disposition. Priority: user suppress > user error >
    /// user warn > compiled default severity.
    pub fn report(&mut self, rule: LintRule, msg: &str) {
        let path = self.current_subject_path.clone();
        self.report_inner(rule, path.as_deref(), msg, DiagnosticMetadata::default());
    }

    /// Report a diagnostic owned by one concrete repository path. The path is
    /// normalized and matched against per-file overrides before severity is
    /// resolved; display text remains unchanged.
    pub fn report_at(&mut self, rule: LintRule, path: impl AsRef<Path>, msg: &str) {
        self.report_inner(
            rule,
            Some(path.as_ref()),
            msg,
            DiagnosticMetadata::default(),
        );
    }

    /// Report within the current subject scope with optional structured data.
    pub fn report_with(&mut self, rule: LintRule, msg: &str, metadata: DiagnosticMetadata) {
        let path = self.current_subject_path.clone();
        self.report_inner(rule, path.as_deref(), msg, metadata);
    }

    /// Report for a concrete path with optional structured data.
    pub fn report_at_with(
        &mut self,
        rule: LintRule,
        path: impl AsRef<Path>,
        msg: &str,
        metadata: DiagnosticMetadata,
    ) {
        self.report_inner(rule, Some(path.as_ref()), msg, metadata);
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

    fn report_inner(
        &mut self,
        rule: LintRule,
        path: Option<&Path>,
        msg: &str,
        metadata: DiagnosticMetadata,
    ) {
        // Selection is independent of severity and suppression. Unselected
        // reports do not participate in any diagnostic accounting.
        if !self.run_policy.selects(rule) {
            return;
        }

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

        let related_subjects = metadata
            .related_subjects
            .into_iter()
            .map(|related| PathBuf::from(self.config.normalize_subject_path(&related)))
            .collect();
        let diagnostic = Diagnostic {
            rule,
            severity,
            subject_path: path.map(|path| PathBuf::from(self.config.normalize_subject_path(path))),
            related_subjects,
            message: msg.to_string(),
            location: metadata.location,
            evidence: metadata.evidence,
            suggestion: metadata.suggestion,
        };
        if let Some(rank) = self.run_policy.registry_rank(rule) {
            let insert_at = self
                .diagnostics
                .iter()
                .position(|existing| {
                    self.run_policy
                        .registry_rank(existing.rule)
                        .is_some_and(|existing_rank| existing_rank > rank)
                })
                .unwrap_or(self.diagnostics.len());
            self.diagnostics.insert(insert_at, diagnostic);
        } else {
            self.diagnostics.push(diagnostic);
        }
    }

    /// Render collected diagnostics in the stable human-readable format.
    /// Structured metadata is deliberately not appended, preserving the
    /// existing text contract while other renderers consume `diagnostics()`.
    /// Control characters in messages are escaped for terminal safety; stored
    /// diagnostics and JSON output keep the original strings.
    pub fn render_text(&self, writer: &mut impl Write) {
        for diagnostic in &self.diagnostics {
            let label = match diagnostic.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
            };
            let message = sanitize_for_terminal(&diagnostic.message);
            let _ = writeln!(
                writer,
                "{label}[{}/{}]: {message}",
                diagnostic.rule.code(),
                diagnostic.rule.name(),
            );
        }
    }

    /// Emit one non-failing warning for each configured `(override, rule)`
    /// pair that suppressed no diagnostic in this visible lint pass.
    /// Escapes control characters at the text choke point only; callers that
    /// need the raw warning strings (JSON notices) use
    /// [`Self::unused_override_warnings`].
    pub fn emit_unused_override_warnings(&self, writer: &mut impl Write) {
        for warning in self.unused_override_warnings() {
            let _ = writeln!(writer, "{}", sanitize_for_terminal(&warning));
        }
    }

    pub fn unused_override_warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        if !self.config.per_file_overrides_enabled() {
            return warnings;
        }
        for (index, entry) in self.config.overrides.iter().enumerate() {
            let mut rules: Vec<_> = if self.run_policy.is_focused() {
                self.run_policy
                    .selected_rules()
                    .iter()
                    .copied()
                    .filter(|rule| entry.suppress.contains(rule))
                    .collect()
            } else {
                entry.suppress.iter().copied().collect()
            };
            if !self.run_policy.is_focused() {
                rules.sort_by_key(|rule| rule.code());
            }
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
    fn focused_collector_filters_before_suppression_and_orders_by_registry() {
        let config = LintConfig {
            suppress: HashSet::from([LintRule::PluginJsonMissing]),
            ..LintConfig::default()
        };
        let policy =
            RunPolicy::resolve(crate::config::CliMode::Normal, &["H001,G005".to_string()]).unwrap();
        let mut diag = DiagnosticCollector::with_run_policy(config, policy);

        diag.report(LintRule::SecurityMdMissing, "security");
        diag.report(LintRule::PluginJsonMissing, "unselected and suppressed");
        diag.report(LintRule::HooksJsonMissing, "hooks");

        assert_eq!(diag.suppressed_count(), 0);
        assert_eq!(
            diag.diagnostics()
                .iter()
                .map(|diagnostic| diagnostic.rule)
                .collect::<Vec<_>>(),
            vec![LintRule::HooksJsonMissing, LintRule::SecurityMdMissing]
        );
    }

    #[test]
    fn focused_unused_override_reporting_ignores_unselected_entries() {
        let config = config_with_overrides(
            "[lint]\n[[lint.overrides]]\nfiles = [\"missing.md\"]\nsuppress = [\"M001\", \"H001\"]\n",
        );
        let policy =
            RunPolicy::resolve(crate::config::CliMode::Normal, &["H001".to_string()]).unwrap();
        let diag = DiagnosticCollector::with_run_policy(config, policy);

        let warnings = diag.unused_override_warnings();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("H001/hooks-json-missing"));
        assert!(!warnings[0].contains("M001"));
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
        diag.report(LintRule::BodyNoExamples, "no examples");
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

    #[test]
    fn structured_metadata_covers_path_point_range_evidence_and_suggestion() {
        let mut diag = DiagnosticCollector::new();
        diag.report_at(LintRule::PluginJsonMissing, "path-only.json", "path only");
        diag.report_at_with(
            LintRule::PluginJsonMissing,
            "point.json",
            "point",
            DiagnosticMetadata::at_point(3, 7),
        );
        diag.report_at_with(
            LintRule::PluginJsonMissing,
            "range.json",
            "range",
            DiagnosticMetadata::default()
                .with_location(SourceSpan::range(4, 2, 4, 6))
                .with_evidence("bad value")
                .with_suggestion("use a supported value"),
        );

        let path_only = &diag.diagnostics()[0];
        assert_eq!(
            path_only.subject_path.as_deref(),
            Some(Path::new("path-only.json"))
        );
        assert_eq!(path_only.location, None);

        let point = diag.diagnostics()[1].location.unwrap();
        assert_eq!(point.start().line_number(), 3);
        assert_eq!(point.start().column_number(), Some(7));
        assert_eq!(point.end(), None);

        let ranged = &diag.diagnostics()[2];
        let range = ranged.location.unwrap();
        assert_eq!(range.start(), SourcePosition::point(4, 2));
        assert_eq!(range.end(), Some(SourcePosition::point(4, 6)));
        assert_eq!(ranged.evidence.as_deref(), Some("bad value"));
        assert_eq!(ranged.suggestion.as_deref(), Some("use a supported value"));
    }

    #[test]
    fn related_subjects_are_structured_and_do_not_match_per_file_overrides() {
        let config = config_with_overrides(
            "[lint]\n[[lint.overrides]]\nfiles = [\"agents/*.md\"]\nsuppress = [\"A030\"]\n",
        );
        let mut diag = DiagnosticCollector::with_config(config);
        diag.report_with(
            LintRule::AgentDescOverlap,
            "agents/a.md and agents/b.md overlap",
            DiagnosticMetadata::default().with_related_subjects(["agents/a.md", "agents/b.md"]),
        );

        assert_eq!(diag.suppressed_count(), 0);
        assert_eq!(diag.warning_count(), 1);
        let diagnostic = &diag.diagnostics()[0];
        assert_eq!(diagnostic.subject_path, None);
        assert_eq!(
            diagnostic.related_subjects,
            vec![PathBuf::from("agents/a.md"), PathBuf::from("agents/b.md")]
        );
    }

    #[test]
    fn no_location_is_not_inferred_from_path_like_message_text() {
        let mut diag = DiagnosticCollector::new();
        diag.report(
            LintRule::PluginJsonMissing,
            "other/place.md:91:4 looks path-like",
        );

        let diagnostic = &diag.diagnostics()[0];
        assert_eq!(diagnostic.subject_path, None);
        assert_eq!(diagnostic.location, None);
    }

    #[test]
    fn byte_ranges_use_unicode_columns_and_exclusive_ends() {
        let span = SourceSpan::from_byte_range("αx\nz", 0..2).unwrap();
        assert_eq!(span.start(), SourcePosition::point(1, 1));
        assert_eq!(span.end(), Some(SourcePosition::point(1, 2)));

        let multiline = SourceSpan::from_byte_range("αx\nz", 2..4).unwrap();
        assert_eq!(multiline.start(), SourcePosition::point(1, 2));
        assert_eq!(multiline.end(), Some(SourcePosition::point(2, 1)));
    }

    #[test]
    fn diagnostic_evidence_is_bounded_redacted_and_utf8_safe() {
        let ordinary = DiagnosticMetadata::default().with_evidence("safe evidence");
        assert_eq!(ordinary.evidence.as_deref(), Some("safe evidence"));

        for length in [511, 512, 513] {
            let input = "x".repeat(length);
            let evidence = DiagnosticMetadata::default()
                .with_evidence(&input)
                .evidence
                .unwrap();
            assert!(evidence.len() <= MAX_EVIDENCE_BYTES, "length {length}");
            assert!(evidence.is_char_boundary(evidence.len()), "length {length}");
            if length <= MAX_EVIDENCE_BYTES {
                assert_eq!(evidence, input, "length {length}");
            } else {
                assert!(evidence.ends_with('…'), "length {length}: {evidence}");
            }
        }

        for scalar in ['a', 'é', '€', '𐍈'] {
            let input = scalar.to_string().repeat(600);
            let first = DiagnosticMetadata::default()
                .with_evidence(&input)
                .evidence
                .unwrap();
            let second = DiagnosticMetadata::default()
                .with_evidence(&input)
                .evidence
                .unwrap();
            assert!(first.len() <= MAX_EVIDENCE_BYTES, "{scalar}");
            assert!(first.is_char_boundary(first.len()), "{scalar}");
            assert!(first.ends_with('…'), "{scalar}: {first}");
            assert_eq!(first, second, "{scalar}");
        }

        let secret = "token = 'this-is-a-sensitive-value'";
        let metadata = DiagnosticMetadata::default().with_evidence(secret);
        assert_eq!(metadata.evidence.as_deref(), Some(REDACTED_EVIDENCE));
        assert!(!metadata.evidence.as_deref().unwrap().contains(secret));
        let long_secret = format!("{} {secret}", "x".repeat(MAX_EVIDENCE_BYTES * 2));
        assert_eq!(
            DiagnosticMetadata::default()
                .with_evidence(long_secret)
                .evidence
                .as_deref(),
            Some(REDACTED_EVIDENCE),
            "redaction must happen before truncation"
        );
        assert_eq!(
            DiagnosticMetadata::default()
                .with_redacted_evidence()
                .evidence
                .as_deref(),
            Some(REDACTED_EVIDENCE)
        );

        let control_evidence = "shown\\u{1b}[31mtext";
        let metadata = DiagnosticMetadata::default()
            .with_evidence(control_evidence)
            .with_suggestion("safe suggestion")
            .with_related_subjects(["other.md"]);
        assert_eq!(metadata.evidence.as_deref(), Some(control_evidence));
        let mut diag = DiagnosticCollector::new();
        diag.report_with(LintRule::PluginJsonMissing, "safe message", metadata);
        let diagnostic = &diag.diagnostics()[0];
        assert_eq!(diagnostic.evidence.as_deref(), Some(control_evidence));
        assert_eq!(diagnostic.suggestion.as_deref(), Some("safe suggestion"));
        assert_eq!(diagnostic.related_subjects, vec![PathBuf::from("other.md")]);

        let mut rendered = Vec::new();
        diag.render_text(&mut rendered);
        assert!(!String::from_utf8(rendered).unwrap().contains(secret));
    }

    #[test]
    fn text_is_rendered_after_collection_in_the_stable_format() {
        let mut diag = DiagnosticCollector::new();
        diag.report_at_with(
            LintRule::PluginJsonMissing,
            "config.json",
            "canonical message",
            DiagnosticMetadata::at_line(8)
                .with_evidence("safe evidence")
                .with_suggestion("fix it"),
        );

        let mut rendered = Vec::new();
        diag.render_text(&mut rendered);
        assert_eq!(
            String::from_utf8(rendered).unwrap(),
            "error[M001/plugin-json-missing]: canonical message\n"
        );
    }

    #[test]
    fn sanitize_for_terminal_borrows_clean_text_and_escapes_controls() {
        assert!(matches!(
            sanitize_for_terminal("plain message"),
            std::borrow::Cow::Borrowed("plain message")
        ));
        assert_eq!(
            sanitize_for_terminal("a\u{1b}b\u{7}c\u{9b}d\u{7f}e"),
            r"a\u{1b}b\u{7}c\u{9b}d\u{7f}e"
        );
    }

    #[test]
    fn render_text_escapes_control_characters_without_mutating_stored_diagnostics() {
        let message = "evil\u{1b}[31mred\u{7}bell\u{9b}csi";
        let mut diag = DiagnosticCollector::new();
        diag.report(LintRule::PluginJsonMissing, message);

        assert_eq!(diag.diagnostics()[0].message, message);

        let mut rendered = Vec::new();
        diag.render_text(&mut rendered);
        let text = String::from_utf8(rendered.clone()).unwrap();
        assert!(text.contains(r"\u{1b}"));
        assert!(text.contains(r"\u{7}"));
        assert!(text.contains(r"\u{9b}"));
        assert_no_raw_controls_except_line_terminators(&rendered);

        let json = serde_json::to_string(&diag.diagnostics()[0].message).unwrap();
        let roundtrip: String = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtrip, message);

        let mut rendered_again = Vec::new();
        diag.render_text(&mut rendered_again);
        assert_eq!(rendered, rendered_again);
    }

    #[test]
    fn unused_override_emit_escapes_control_characters_in_reason() {
        let config = config_with_overrides(
            "[lint]\n[[lint.overrides]]\nfiles = [\"missing.md\"]\nsuppress = [\"M001\"]\nreason = \"legacy\\u001breason\"\n",
        );
        let diag = DiagnosticCollector::with_config(config);

        let warnings = diag.unused_override_warnings();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains('\u{1b}'));
        assert!(!warnings[0].contains(r"\u{1b}"));

        let mut rendered = Vec::new();
        diag.emit_unused_override_warnings(&mut rendered);
        let text = String::from_utf8(rendered.clone()).unwrap();
        assert!(text.contains(r"\u{1b}"));
        assert!(!text.contains('\u{1b}'));
        assert_no_raw_controls_except_line_terminators(&rendered);

        let mut rendered_again = Vec::new();
        diag.emit_unused_override_warnings(&mut rendered_again);
        assert_eq!(rendered, rendered_again);
    }

    fn assert_no_raw_controls_except_line_terminators(bytes: &[u8]) {
        let text = std::str::from_utf8(bytes).unwrap();
        for ch in text.chars() {
            if ch == '\n' {
                continue;
            }
            assert!(
                !ch.is_control(),
                "unexpected raw control U+{:04X} in {text:?}",
                ch as u32
            );
        }
    }
}
