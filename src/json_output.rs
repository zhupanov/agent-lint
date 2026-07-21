use std::io::Write;

use serde::Serialize;

use crate::config::{CliMode, RunPolicy};
use crate::context::LintMode;
use crate::diagnostic::{Diagnostic, DiagnosticCollector, Severity, SourcePosition, SourceSpan};
use crate::platforms::ValidationTargets;

pub const SCHEMA_ID: &str = "https://raw.githubusercontent.com/zhupanov/agent-lint/main/schemas/diagnostic-output-v1.schema.json";

#[derive(Debug, Clone, Serialize)]
pub struct Notice {
    pub kind: &'static str,
    pub severity: NoticeSeverity,
    pub message: String,
}

impl Notice {
    pub fn warning(kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            severity: NoticeSeverity::Warning,
            message: message.into(),
        }
    }

    pub fn error(kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            severity: NoticeSeverity::Error,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NoticeSeverity {
    Warning,
    Error,
}

#[derive(Serialize)]
struct Report {
    #[serde(rename = "$schema")]
    schema: &'static str,
    schema_version: u8,
    agent_lint_version: &'static str,
    analysis_root: &'static str,
    mode: Option<OutputLintMode>,
    strictness: OutputStrictness,
    selected_rules: Option<Vec<OutputRule>>,
    active_platforms: Vec<&'static str>,
    status: OutputStatus,
    counts: Counts,
    diagnostics: Vec<OutputDiagnostic>,
    notices: Vec<Notice>,
}

#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
enum OutputLintMode {
    Basic,
    Plugin,
}

#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
enum OutputStrictness {
    Normal,
    Pedantic,
    All,
}

#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
enum OutputStatus {
    Clean,
    Warnings,
    Errors,
    UsageError,
}

#[derive(Serialize)]
struct OutputRule {
    code: &'static str,
    name: &'static str,
}

#[derive(Serialize)]
struct Counts {
    errors: usize,
    warnings: usize,
    suppressed: usize,
    notices: usize,
}

#[derive(Serialize)]
struct OutputDiagnostic {
    code: &'static str,
    name: &'static str,
    severity: OutputSeverity,
    #[serde(skip_serializing_if = "Option::is_none")]
    subject_path: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    related_subjects: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    location: Option<OutputLocation>,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    evidence: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    suggestion: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
enum OutputSeverity {
    Error,
    Warning,
}

#[derive(Serialize)]
struct OutputLocation {
    start: OutputPosition,
    #[serde(skip_serializing_if = "Option::is_none")]
    end: Option<OutputPosition>,
}

#[derive(Serialize)]
struct OutputPosition {
    line: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    column: Option<usize>,
}

impl From<LintMode> for OutputLintMode {
    fn from(mode: LintMode) -> Self {
        match mode {
            LintMode::Basic => Self::Basic,
            LintMode::Plugin => Self::Plugin,
        }
    }
}

impl From<CliMode> for OutputStrictness {
    fn from(mode: CliMode) -> Self {
        match mode {
            CliMode::Normal => Self::Normal,
            CliMode::Pedantic => Self::Pedantic,
            CliMode::All => Self::All,
        }
    }
}

impl From<&Diagnostic> for OutputDiagnostic {
    fn from(diagnostic: &Diagnostic) -> Self {
        Self {
            code: diagnostic.rule.code(),
            name: diagnostic.rule.name(),
            severity: match diagnostic.severity {
                Severity::Error => OutputSeverity::Error,
                Severity::Warning => OutputSeverity::Warning,
            },
            subject_path: diagnostic
                .subject_path
                .as_ref()
                .map(|path| path.to_string_lossy().replace('\\', "/")),
            related_subjects: diagnostic
                .related_subjects
                .iter()
                .map(|path| path.to_string_lossy().replace('\\', "/"))
                .collect(),
            location: diagnostic.location.map(OutputLocation::from),
            message: diagnostic.message.clone(),
            evidence: diagnostic.evidence.clone(),
            suggestion: diagnostic.suggestion.clone(),
        }
    }
}

impl From<SourceSpan> for OutputLocation {
    fn from(span: SourceSpan) -> Self {
        Self {
            start: span.start().into(),
            end: span.end().map(Into::into),
        }
    }
}

impl From<SourcePosition> for OutputPosition {
    fn from(position: SourcePosition) -> Self {
        Self {
            line: position.line_number(),
            column: position.column_number(),
        }
    }
}

pub fn write_report(
    mode: Option<LintMode>,
    strictness: CliMode,
    run_policy: &RunPolicy,
    targets: ValidationTargets,
    diagnostics: Option<&DiagnosticCollector>,
    notices: Vec<Notice>,
) {
    write(build_report(
        mode,
        strictness,
        run_policy,
        targets,
        diagnostics,
        notices,
    ));
}

fn build_report(
    mode: Option<LintMode>,
    strictness: CliMode,
    run_policy: &RunPolicy,
    targets: ValidationTargets,
    diagnostics: Option<&DiagnosticCollector>,
    notices: Vec<Notice>,
) -> Report {
    let errors = diagnostics.map_or(0, DiagnosticCollector::error_count);
    let warnings = diagnostics.map_or(0, DiagnosticCollector::warning_count);
    let suppressed = diagnostics.map_or(0, DiagnosticCollector::suppressed_count);
    let status = if errors > 0 {
        OutputStatus::Errors
    } else if warnings > 0
        || notices
            .iter()
            .any(|notice| matches!(notice.severity, NoticeSeverity::Warning))
    {
        OutputStatus::Warnings
    } else {
        OutputStatus::Clean
    };
    let diagnostics = diagnostics
        .map(|collector| {
            collector
                .diagnostics()
                .iter()
                .map(OutputDiagnostic::from)
                .collect()
        })
        .unwrap_or_default();
    Report {
        schema: SCHEMA_ID,
        schema_version: 1,
        agent_lint_version: env!("CARGO_PKG_VERSION"),
        analysis_root: ".",
        mode: mode.map(Into::into),
        strictness: strictness.into(),
        selected_rules: selected_rules(run_policy),
        active_platforms: active_platforms(mode, targets),
        status,
        counts: Counts {
            errors,
            warnings,
            suppressed,
            notices: notices.len(),
        },
        diagnostics,
        notices,
    }
}

pub fn write_usage_error(
    strictness: CliMode,
    run_policy: &RunPolicy,
    kind: &'static str,
    message: impl Into<String>,
    mut notices: Vec<Notice>,
) {
    notices.push(Notice::error(kind, message));
    write(Report {
        schema: SCHEMA_ID,
        schema_version: 1,
        agent_lint_version: env!("CARGO_PKG_VERSION"),
        analysis_root: ".",
        mode: None,
        strictness: strictness.into(),
        selected_rules: selected_rules(run_policy),
        active_platforms: Vec::new(),
        status: OutputStatus::UsageError,
        counts: Counts {
            errors: 0,
            warnings: 0,
            suppressed: 0,
            notices: notices.len(),
        },
        diagnostics: Vec::new(),
        notices,
    });
}

fn active_platforms(mode: Option<LintMode>, targets: ValidationTargets) -> Vec<&'static str> {
    let mut platforms = Vec::new();
    if mode.is_some() {
        platforms.push("claude");
    }
    if targets.cursor {
        platforms.push("cursor");
    }
    if targets.codex {
        platforms.push("codex");
    }
    platforms
}

fn selected_rules(run_policy: &RunPolicy) -> Option<Vec<OutputRule>> {
    run_policy.is_focused().then(|| {
        run_policy
            .effective_rules()
            .iter()
            .map(|rule| OutputRule {
                code: rule.code(),
                name: rule.name(),
            })
            .collect()
    })
}

fn write(report: Report) {
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    serde_json::to_writer_pretty(&mut stdout, &report).expect("JSON report serializes");
    writeln!(stdout).expect("JSON report terminates with a newline");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::{DiagnosticMetadata, SourceSpan};
    use crate::rules::LintRule;

    #[test]
    fn serializes_range_evidence_and_suggestion_from_structured_fields() {
        let mut diagnostics = DiagnosticCollector::new();
        diagnostics.report_at_with(
            LintRule::PluginJsonMissing,
            "config/plugin.json",
            "structured diagnostic",
            DiagnosticMetadata::default()
                .with_location(SourceSpan::range(2, 3, 4, 5))
                .with_evidence("safe evidence")
                .with_suggestion("add the missing manifest"),
        );
        let report = build_report(
            Some(LintMode::Basic),
            CliMode::Normal,
            &RunPolicy::default(),
            ValidationTargets::default(),
            Some(&diagnostics),
            Vec::new(),
        );
        let value = serde_json::to_value(report).unwrap();
        let diagnostic = &value["diagnostics"][0];

        assert_eq!(diagnostic["subject_path"], "config/plugin.json");
        assert_eq!(diagnostic["location"]["start"]["line"], 2);
        assert_eq!(diagnostic["location"]["start"]["column"], 3);
        assert_eq!(diagnostic["location"]["end"]["line"], 4);
        assert_eq!(diagnostic["location"]["end"]["column"], 5);
        assert_eq!(diagnostic["evidence"], "safe evidence");
        assert_eq!(diagnostic["suggestion"], "add the missing manifest");

        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../schemas/diagnostic-output-v1.schema.json"))
                .unwrap();
        assert!(jsonschema::validator_for(&schema).unwrap().is_valid(&value));
    }

    #[test]
    fn serializes_related_subjects_for_pathless_multi_source_findings() {
        let mut diagnostics = DiagnosticCollector::new();
        diagnostics.report_with(
            LintRule::AgentDescOverlap,
            "agents/a.md and agents/b.md have overlapping routing descriptions (similarity 1.00)",
            DiagnosticMetadata::default().with_related_subjects(["agents/a.md", "agents/b.md"]),
        );
        let report = build_report(
            Some(LintMode::Basic),
            CliMode::Normal,
            &RunPolicy::default(),
            ValidationTargets::default(),
            Some(&diagnostics),
            Vec::new(),
        );
        let value = serde_json::to_value(report).unwrap();
        let diagnostic = &value["diagnostics"][0];

        assert!(diagnostic.get("subject_path").is_none());
        assert_eq!(
            diagnostic["related_subjects"],
            serde_json::json!(["agents/a.md", "agents/b.md"])
        );

        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../schemas/diagnostic-output-v1.schema.json"))
                .unwrap();
        assert!(jsonschema::validator_for(&schema).unwrap().is_valid(&value));
    }
}
