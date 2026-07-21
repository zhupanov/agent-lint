//! Validators for optional Claude Code configuration surfaces.
//!
//! These surfaces are intentionally optional: their validators are silent when
//! the corresponding directory or settings file is absent.

use crate::config::ExcludeSet;
use crate::context::{LintContext, ManifestState, ParsedManifest};
use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata, SourceSpan};
use crate::frontmatter;
use crate::rules::LintRule;
use crate::traversal;
use crate::yaml::{Mapping, Value as YamlValue};
use globset::Glob;
use serde_json::Value as JsonValue;
use std::fs;
use std::ops::Range;
use std::path::Path;
use std::sync::LazyLock;

const RULES_FIELDS: &[&str] = &["paths"];
const OUTPUT_STYLE_FIELDS: &[&str] = &["name", "description", "keep-coding-instructions"];
const PR_URL_TEMPLATE_PLACEHOLDERS: &[&str] = &["{host}", "{owner}", "{repo}", "{number}", "{url}"];
const CHANNELS_ENABLED_SUGGESTION: &str =
    "remove channelsEnabled here and configure it through managed organization policy";

static RE_BRACE_TOKEN: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"\{[A-Za-z][A-Za-z0-9_-]*\}").expect("valid placeholder-token regex")
});

/// Validate every optional private Claude configuration surface.
pub fn validate_private_config(
    ctx: &LintContext,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
) {
    validate_rules(diag, exclude);
    validate_output_styles(diag, exclude);
    validate_typed_settings(ctx, diag);
}

fn validate_rules(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    validate_markdown_directory(".claude/rules", diag, exclude, |path, content, diag| {
        let Some(frontmatter) = parse_frontmatter(content) else {
            return;
        };
        report_unknown_fields(
            diag,
            path,
            &frontmatter,
            RULES_FIELDS,
            LintRule::RulesFieldUnknown,
        );
        validate_rule_paths(diag, path, &frontmatter);
    });
}

fn validate_output_styles(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    validate_markdown_directory(
        ".claude/output-styles",
        diag,
        exclude,
        |path, content, diag| {
            let Some(frontmatter) = parse_frontmatter(content) else {
                diag.report(
                    LintRule::OutputStyleFrontmatterInvalid,
                    &format!("{path}: frontmatter must be valid YAML between '---' delimiters"),
                );
                return;
            };

            report_unknown_fields(
                diag,
                path,
                &frontmatter,
                OUTPUT_STYLE_FIELDS,
                LintRule::OutputStyleFieldUnknown,
            );
            validate_output_style_fields(diag, path, &frontmatter);
            if frontmatter::extract_body(content).trim().is_empty() {
                diag.report(
                    LintRule::OutputStyleBodyEmpty,
                    &format!("{path}: body is empty after frontmatter"),
                );
            }
        },
    );
}

fn validate_markdown_directory<F>(
    directory: &str,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    mut validate: F,
) where
    F: FnMut(&str, &str, &mut DiagnosticCollector),
{
    let dir = Path::new(directory);
    if !dir.is_dir() {
        return;
    }

    for entry in traversal::shallow_files(dir, Path::new("."), Some(exclude)).entries {
        let path = entry.path;
        if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }
        let display = entry.display;
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        diag.with_subject_path(&display, |diag| {
            validate(&display, &content, diag);
        });
    }
}

fn parse_frontmatter(content: &str) -> Option<Mapping> {
    let lines = frontmatter::extract_frontmatter(content)?;
    let yaml = crate::yaml::parse(&lines.join("\n")).ok()?;
    Some(yaml.as_mapping().cloned().unwrap_or_default())
}

fn report_unknown_fields(
    diag: &mut DiagnosticCollector,
    path: &str,
    frontmatter: &Mapping,
    known_fields: &[&str],
    rule: LintRule,
) {
    for key in frontmatter.keys() {
        if !known_fields.contains(&key.as_str()) {
            diag.report(rule, &format!("{path}: unknown frontmatter field '{key}'"));
        }
    }
}

fn validate_rule_paths(diag: &mut DiagnosticCollector, path: &str, frontmatter: &Mapping) {
    let Some(value) = frontmatter.get("paths") else {
        return;
    };
    let values: Vec<&str> = match value {
        YamlValue::String(value) => vec![value],
        YamlValue::Sequence(values) => values.iter().filter_map(YamlValue::as_str).collect(),
        _ => Vec::new(),
    };
    for value in values {
        if let Err(error) = Glob::new(value) {
            diag.report(
                LintRule::RulesGlobInvalid,
                &format!("{path}: invalid paths glob '{value}': {error}"),
            );
        }
    }
}

fn validate_output_style_fields(diag: &mut DiagnosticCollector, path: &str, frontmatter: &Mapping) {
    let description = frontmatter.get("description").and_then(YamlValue::as_str);
    if description.is_none_or(|value| value.trim().is_empty()) {
        diag.report(
            LintRule::OutputStyleDescriptionMissing,
            &format!("{path}: missing or blank 'description' frontmatter field"),
        );
    }

    if let Some(value) = frontmatter.get("keep-coding-instructions")
        && !value.is_bool()
    {
        diag.report(
            LintRule::OutputStyleKeepCodingInstructionsInvalid,
            &format!("{path}: 'keep-coding-instructions' must be a YAML boolean"),
        );
    }

    if let Some(name) = frontmatter.get("name").and_then(YamlValue::as_str)
        && name.chars().count() > 64
    {
        diag.report(
            LintRule::OutputStyleNameTooLong,
            &format!(
                "{path}: 'name' exceeds 64 characters ({})",
                name.chars().count()
            ),
        );
    }
}

fn validate_typed_settings(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    for (path, state) in [
        (".claude/settings.json", &ctx.settings_json),
        (".claude/settings.local.json", &ctx.settings_local_json),
    ] {
        if let ManifestState::Parsed(settings) = state {
            validate_typed_settings_file(diag, path, settings);
        }
    }
}

/// Validate one parsed settings file. New typed fields belong here.
fn validate_typed_settings_file(
    diag: &mut DiagnosticCollector,
    path: &str,
    settings: &ParsedManifest,
) {
    if let Some(value) = settings.get("prUrlTemplate") {
        if let Some(category) = pr_url_template_category(value) {
            report_typed_setting(
                diag,
                LintRule::SettingsPrUrlTemplateInvalid,
                path,
                settings,
                TypedSettingIssue {
                    key: "prUrlTemplate",
                    evidence: category.evidence(),
                    suggestion: "replace prUrlTemplate with a documented-placeholder http(s) URL template",
                    message: "prUrlTemplate is not a usable PR URL template",
                },
            );
        }
    }

    if settings.get("channelsEnabled").is_some() {
        report_typed_setting(
            diag,
            LintRule::SettingsChannelsEnabledInvalid,
            path,
            settings,
            TypedSettingIssue {
                key: "channelsEnabled",
                evidence: "channelsEnabled: unsupported scope",
                suggestion: CHANNELS_ENABLED_SUGGESTION,
                message: "channelsEnabled is managed-policy-only and ignored in project/local settings",
            },
        );
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PrUrlTemplateCategory {
    WrongType,
    BlankOrSurroundingWhitespace,
    NoDocumentedPlaceholder,
    UnknownPlaceholder,
    InvalidRenderedUrl,
}

impl PrUrlTemplateCategory {
    fn evidence(self) -> &'static str {
        match self {
            Self::WrongType => "prUrlTemplate: wrong type",
            Self::BlankOrSurroundingWhitespace => "prUrlTemplate: blank or surrounding whitespace",
            Self::NoDocumentedPlaceholder => "prUrlTemplate: no documented placeholder",
            Self::UnknownPlaceholder => "prUrlTemplate: unknown placeholder",
            Self::InvalidRenderedUrl => "prUrlTemplate: invalid rendered URL",
        }
    }
}

fn pr_url_template_category(value: &JsonValue) -> Option<PrUrlTemplateCategory> {
    let Some(template) = value.as_str() else {
        return Some(PrUrlTemplateCategory::WrongType);
    };
    if template.is_empty() || template.trim() != template {
        return Some(PrUrlTemplateCategory::BlankOrSurroundingWhitespace);
    }
    if !PR_URL_TEMPLATE_PLACEHOLDERS
        .iter()
        .any(|placeholder| template.contains(placeholder))
    {
        return Some(PrUrlTemplateCategory::NoDocumentedPlaceholder);
    }
    if RE_BRACE_TOKEN
        .find_iter(template)
        .any(|token| !PR_URL_TEMPLATE_PLACEHOLDERS.contains(&token.as_str()))
    {
        return Some(PrUrlTemplateCategory::UnknownPlaceholder);
    }

    let rendered = render_pr_url_template(template);
    let is_http_url = url::Url::parse(&rendered)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host().is_some());
    (rendered.chars().any(char::is_whitespace) || !is_http_url)
        .then_some(PrUrlTemplateCategory::InvalidRenderedUrl)
}

fn render_pr_url_template(template: &str) -> String {
    template
        .replace("{host}", "github.com")
        .replace("{owner}", "owner")
        .replace("{repo}", "repo")
        .replace("{number}", "1")
        .replace("{url}", "https://github.com/owner/repo/pull/1")
}

struct TypedSettingIssue {
    key: &'static str,
    evidence: &'static str,
    suggestion: &'static str,
    message: &'static str,
}

fn report_typed_setting(
    diag: &mut DiagnosticCollector,
    rule: LintRule,
    path: &str,
    settings: &ParsedManifest,
    issue: TypedSettingIssue,
) {
    let metadata = typed_setting_metadata(settings, issue.key, issue.evidence, issue.suggestion);
    diag.report_at_with(rule, path, &format!("{path}: {}", issue.message), metadata);
}

fn typed_setting_metadata(
    settings: &ParsedManifest,
    key: &str,
    evidence: &str,
    suggestion: &str,
) -> DiagnosticMetadata {
    let mut metadata = DiagnosticMetadata::default()
        .with_evidence(evidence)
        .with_suggestion(suggestion);
    if let Some(span) = settings
        .source()
        .and_then(|source| json_top_level_member_range(source, key))
        .and_then(|range| {
            settings
                .source()
                .and_then(|source| SourceSpan::from_byte_range(source, range))
        })
    {
        metadata = metadata.with_location(span);
    }
    metadata
}

/// Locate the final effective top-level JSON member from its key through its
/// value. The manifest was already parsed, so this only maps source offsets.
fn json_top_level_member_range(source: &str, wanted_key: &str) -> Option<Range<usize>> {
    let mut scanner = TopLevelMemberScanner::new(source);
    scanner.scan_root(wanted_key);
    scanner.found
}

struct TopLevelMemberScanner<'a> {
    input: &'a [u8],
    position: usize,
    found: Option<Range<usize>>,
}

impl<'a> TopLevelMemberScanner<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            input: source.as_bytes(),
            position: 0,
            found: None,
        }
    }

    fn scan_root(&mut self, wanted_key: &str) {
        self.skip_whitespace();
        if self.consume(b'{') {
            loop {
                self.skip_whitespace();
                if self.consume(b'}') {
                    return;
                }
                let member_start = self.position;
                let key = self.scan_string();
                self.skip_whitespace();
                self.consume(b':');
                self.skip_whitespace();
                self.scan_value();
                if key == wanted_key {
                    self.found = Some(member_start..self.position);
                }
                self.skip_whitespace();
                self.consume(b',');
            }
        }
    }

    fn scan_value(&mut self) {
        self.skip_whitespace();
        match self.input.get(self.position) {
            Some(b'{') => self.scan_object(),
            Some(b'[') => self.scan_array(),
            Some(b'"') => {
                self.scan_string();
            }
            Some(_) => self.scan_scalar(),
            None => {}
        }
    }

    fn scan_object(&mut self) {
        self.position += 1;
        loop {
            self.skip_whitespace();
            if self.consume(b'}') {
                return;
            }
            self.scan_string();
            self.skip_whitespace();
            self.consume(b':');
            self.scan_value();
            self.skip_whitespace();
            self.consume(b',');
        }
    }

    fn scan_array(&mut self) {
        self.position += 1;
        loop {
            self.skip_whitespace();
            if self.consume(b']') {
                return;
            }
            self.scan_value();
            self.skip_whitespace();
            self.consume(b',');
        }
    }

    fn scan_string(&mut self) -> String {
        let start = self.position;
        self.position += 1;
        while self.position < self.input.len() {
            match self.input[self.position] {
                b'\\' => self.position += 2,
                b'"' => {
                    self.position += 1;
                    break;
                }
                _ => self.position += 1,
            }
        }
        serde_json::from_slice(&self.input[start..self.position])
            .expect("scanner only runs after successful JSON parsing")
    }

    fn scan_scalar(&mut self) {
        while self.position < self.input.len()
            && !matches!(
                self.input[self.position],
                b',' | b']' | b'}' | b' ' | b'\n' | b'\r' | b'\t'
            )
        {
            self.position += 1;
        }
    }

    fn skip_whitespace(&mut self) {
        while self.position < self.input.len() && self.input[self.position].is_ascii_whitespace() {
            self.position += 1;
        }
    }

    fn consume(&mut self, byte: u8) -> bool {
        if self.input.get(self.position) == Some(&byte) {
            self.position += 1;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_temp_dir(test: impl FnOnce()) {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        test();
    }

    fn validate() -> DiagnosticCollector {
        let ctx = LintContext::new(
            &std::env::current_dir().expect("current directory is readable"),
            crate::context::LintMode::Basic,
        );
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_private_config(&ctx, &mut diag, &ExcludeSet::default());
        diag
    }

    #[test]
    #[serial_test::serial]
    fn rules_invalid_glob_reports_r001() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude/rules").unwrap();
            fs::write(".claude/rules/rule.md", "---\npaths: '['\n---\nRule body\n").unwrap();
            let diag = validate();
            assert!(
                diag.diagnostics()
                    .iter()
                    .any(|d| d.rule == LintRule::RulesGlobInvalid)
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn rules_unknown_field_reports_r002() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude/rules").unwrap();
            fs::write(
                ".claude/rules/rule.md",
                "---\nunknown: value\n---\nRule body\n",
            )
            .unwrap();
            let diag = validate();
            assert!(
                diag.diagnostics()
                    .iter()
                    .any(|d| d.rule == LintRule::RulesFieldUnknown)
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn output_style_invalid_frontmatter_reports_o006() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude/output-styles").unwrap();
            fs::write(
                ".claude/output-styles/style.md",
                "---\ndescription: [\n---\nBody\n",
            )
            .unwrap();
            let diag = validate();
            assert!(
                diag.diagnostics()
                    .iter()
                    .any(|d| d.rule == LintRule::OutputStyleFrontmatterInvalid)
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn output_style_description_reports_o001() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude/output-styles").unwrap();
            fs::write(
                ".claude/output-styles/style.md",
                "---\ndescription: '   '\n---\nBody\n",
            )
            .unwrap();
            let diag = validate();
            assert!(
                diag.diagnostics()
                    .iter()
                    .any(|d| d.rule == LintRule::OutputStyleDescriptionMissing)
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn output_style_keep_coding_type_reports_o002() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude/output-styles").unwrap();
            fs::write(
                ".claude/output-styles/style.md",
                "---\ndescription: Good\nkeep-coding-instructions: 'true'\n---\nBody\n",
            )
            .unwrap();
            let diag = validate();
            assert!(
                diag.diagnostics()
                    .iter()
                    .any(|d| d.rule == LintRule::OutputStyleKeepCodingInstructionsInvalid)
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn output_style_unknown_field_reports_o003() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude/output-styles").unwrap();
            fs::write(
                ".claude/output-styles/style.md",
                "---\ndescription: Good\nextra: value\n---\nBody\n",
            )
            .unwrap();
            let diag = validate();
            assert!(
                diag.diagnostics()
                    .iter()
                    .any(|d| d.rule == LintRule::OutputStyleFieldUnknown)
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn output_style_empty_body_reports_o004() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude/output-styles").unwrap();
            fs::write(
                ".claude/output-styles/style.md",
                "---\ndescription: Good\n---\n \n",
            )
            .unwrap();
            let diag = validate();
            assert!(
                diag.diagnostics()
                    .iter()
                    .any(|d| d.rule == LintRule::OutputStyleBodyEmpty)
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn output_style_long_name_reports_o005() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude/output-styles").unwrap();
            fs::write(
                ".claude/output-styles/style.md",
                format!(
                    "---\nname: {}\ndescription: Good\n---\nBody\n",
                    "a".repeat(65)
                ),
            )
            .unwrap();
            let diag = validate();
            assert!(
                diag.diagnostics()
                    .iter()
                    .any(|d| d.rule == LintRule::OutputStyleNameTooLong)
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn invalid_pr_url_template_reports_t001_for_local_settings() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude").unwrap();
            fs::write(
                ".claude/settings.local.json",
                r#"{"prUrlTemplate":"https://example.com"}"#,
            )
            .unwrap();
            let diag = validate();
            assert!(
                diag.diagnostics()
                    .iter()
                    .any(|d| d.rule == LintRule::SettingsPrUrlTemplateInvalid)
            );
        });
    }

    #[test]
    fn pr_url_template_validation_has_deterministic_categories_and_accepts_documented_shapes() {
        let invalid = [
            (serde_json::json!(null), PrUrlTemplateCategory::WrongType),
            (serde_json::json!(true), PrUrlTemplateCategory::WrongType),
            (serde_json::json!([]), PrUrlTemplateCategory::WrongType),
            (
                serde_json::json!(" "),
                PrUrlTemplateCategory::BlankOrSurroundingWhitespace,
            ),
            (
                serde_json::json!("https://example.test/static"),
                PrUrlTemplateCategory::NoDocumentedPlaceholder,
            ),
            (
                serde_json::json!("https://example.test/{number}/{ticket}"),
                PrUrlTemplateCategory::UnknownPlaceholder,
            ),
            (
                serde_json::json!("mailto:{number}"),
                PrUrlTemplateCategory::InvalidRenderedUrl,
            ),
            (
                serde_json::json!("https://example.test/{number} {ticket}"),
                PrUrlTemplateCategory::UnknownPlaceholder,
            ),
        ];
        for (value, expected) in invalid {
            assert_eq!(pr_url_template_category(&value), Some(expected), "{value}");
        }

        for template in [
            "{url}",
            "https://{host}:8443/{owner}/{repo}?pr={number}#review",
            "http://localhost/{number}",
            "https://127.0.0.1/{number}",
            "https://例え.テスト/{repo}/%7Bkept%7D",
            "https://reviews.example/?target={url}&again={url}",
        ] {
            assert_eq!(
                pr_url_template_category(&serde_json::json!(template)),
                None,
                "{template} should be valid"
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn invalid_channels_enabled_reports_t002() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude").unwrap();
            fs::write(".claude/settings.json", r#"{"channelsEnabled":"true"}"#).unwrap();
            let diag = validate();
            assert!(
                diag.diagnostics()
                    .iter()
                    .any(|d| d.rule == LintRule::SettingsChannelsEnabledInvalid)
            );
        });
    }

    #[test]
    fn channels_enabled_reports_once_for_every_json_type_on_each_settings_surface() {
        for path in [".claude/settings.json", ".claude/settings.local.json"] {
            for value in [
                serde_json::json!(null),
                serde_json::json!(true),
                serde_json::json!(1),
                serde_json::json!("true"),
                serde_json::json!([]),
                serde_json::json!({}),
            ] {
                let state = ManifestState::parsed(serde_json::json!({"channelsEnabled": value}));
                let ManifestState::Parsed(settings) = state else {
                    unreachable!("test state is parsed");
                };
                let mut diag = DiagnosticCollector::new_all_enabled();
                validate_typed_settings_file(&mut diag, path, &settings);
                assert_eq!(diag.diagnostics().len(), 1, "{path}: {value}");
                assert_eq!(
                    diag.diagnostics()[0].rule,
                    LintRule::SettingsChannelsEnabledInvalid,
                    "{path}: {value}"
                );
            }
        }
    }

    #[test]
    #[serial_test::serial]
    fn typed_settings_report_exact_member_metadata_without_exposing_values() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude").unwrap();
            let secret_like = "not-a-url/{number}?token=sk_this-value-must-not-appear";
            fs::write(
                ".claude/settings.json",
                format!(
                    "{{\n  \"prUrlTemplate\": \"{secret_like}\",\n  \"channelsEnabled\": true\n}}"
                ),
            )
            .unwrap();
            let diag = validate();
            let template = diag
                .diagnostics()
                .iter()
                .find(|diagnostic| diagnostic.rule == LintRule::SettingsPrUrlTemplateInvalid)
                .expect("T001 diagnostic");
            assert_eq!(
                template.evidence.as_deref(),
                Some("prUrlTemplate: invalid rendered URL")
            );
            assert_eq!(
                template.suggestion.as_deref(),
                Some("replace prUrlTemplate with a documented-placeholder http(s) URL template")
            );
            let location = template.location.expect("T001 has a member span");
            assert_eq!(location.start().line_number(), 2);
            assert_eq!(location.start().column_number(), Some(3));

            let channels = diag
                .diagnostics()
                .iter()
                .find(|diagnostic| diagnostic.rule == LintRule::SettingsChannelsEnabledInvalid)
                .expect("T002 diagnostic");
            assert_eq!(
                channels.evidence.as_deref(),
                Some("channelsEnabled: unsupported scope")
            );
            assert_eq!(
                channels
                    .location
                    .expect("T002 has a member span")
                    .start()
                    .line_number(),
                3
            );
            assert!(!template.message.contains(secret_like));
        });
    }

    #[test]
    #[serial_test::serial]
    fn typed_settings_use_the_context_snapshot_and_ignore_invalid_states() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude").unwrap();
            fs::write(
                ".claude/settings.json",
                r#"{"prUrlTemplate":"https://example.test/{number}/{ticket}"}"#,
            )
            .unwrap();
            let ctx = LintContext::new(
                &std::env::current_dir().unwrap(),
                crate::context::LintMode::Basic,
            );
            fs::write(".claude/settings.json", "{").unwrap();
            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_private_config(&ctx, &mut diag, &ExcludeSet::default());
            assert!(
                diag.diagnostics()
                    .iter()
                    .any(|diagnostic| diagnostic.rule == LintRule::SettingsPrUrlTemplateInvalid)
            );

            let invalid_ctx = LintContext::new(
                &std::env::current_dir().unwrap(),
                crate::context::LintMode::Basic,
            );
            let mut invalid_diag = DiagnosticCollector::new_all_enabled();
            validate_private_config(&invalid_ctx, &mut invalid_diag, &ExcludeSet::default());
            assert!(invalid_diag.diagnostics().is_empty());
        });
    }

    #[test]
    #[serial_test::serial]
    fn valid_optional_configuration_is_silent() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude/rules").unwrap();
            fs::create_dir_all(".claude/output-styles").unwrap();
            fs::write(
                ".claude/rules/rule.md",
                "---\npaths: ['src/**/*.rs']\n---\nRule body\n",
            )
            .unwrap();
            fs::write(".claude/output-styles/style.md", "---\nname: concise\ndescription: Keep answers concise\nkeep-coding-instructions: true\n---\nBody\n").unwrap();
            fs::write(
                ".claude/settings.json",
                r#"{"prUrlTemplate":"https://{host}/{owner}/{repo}/pull/{number}"}"#,
            )
            .unwrap();
            let diag = validate();
            assert_eq!(diag.error_count(), 0);
            assert_eq!(diag.warning_count(), 0);
        });
    }

    #[test]
    #[serial_test::serial]
    fn optional_surfaces_absent_are_silent() {
        with_temp_dir(|| {
            let diag = validate();
            assert_eq!(diag.error_count(), 0);
            assert_eq!(diag.warning_count(), 0);
        });
    }

    #[test]
    #[serial_test::serial]
    fn rules_are_dispatched_in_basic_and_plugin_modes() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude/rules").unwrap();
            fs::write(".claude/rules/rule.md", "---\npaths: '['\n---\nRule body\n").unwrap();

            for mode in [
                crate::context::LintMode::Basic,
                crate::context::LintMode::Plugin,
            ] {
                let ctx = crate::context::LintContext {
                    base_path: std::env::current_dir().unwrap(),
                    mode,
                    plugin_json: crate::context::ManifestState::Missing,
                    marketplace_json: crate::context::ManifestState::Missing,
                    hooks_json: crate::context::ManifestState::Missing,
                    declared_hook_configs: vec![],
                    settings_json: crate::context::ManifestState::Missing,
                    settings_local_json: crate::context::ManifestState::Missing,
                };
                let mut diag = DiagnosticCollector::new_all_enabled();
                super::super::run_all(&ctx, &mut diag, &ExcludeSet::default());
                assert!(
                    diag.diagnostics()
                        .iter()
                        .any(|d| d.rule == LintRule::RulesGlobInvalid),
                    "R001 was not dispatched in {mode:?} mode"
                );
            }
        });
    }
}
