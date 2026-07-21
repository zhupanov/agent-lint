//! Validators for optional Claude Code configuration surfaces.
//!
//! These surfaces are intentionally optional: their validators are silent when
//! the corresponding directory or settings file is absent.

use crate::config::ExcludeSet;
use crate::diagnostic::DiagnosticCollector;
use crate::frontmatter;
use crate::rules::LintRule;
use crate::traversal;
use crate::yaml::{Mapping, Value as YamlValue};
use globset::Glob;
use serde_json::Value as JsonValue;
use std::fs;
use std::path::Path;

const RULES_FIELDS: &[&str] = &["paths"];
const OUTPUT_STYLE_FIELDS: &[&str] = &["name", "description", "keep-coding-instructions"];
const PR_URL_TEMPLATE_PLACEHOLDERS: &[&str] = &["{host}", "{owner}", "{repo}", "{number}", "{url}"];

/// Validate every optional private Claude configuration surface.
pub fn validate_private_config(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    validate_rules(diag, exclude);
    validate_output_styles(diag, exclude);
    validate_typed_settings(diag);
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

fn validate_typed_settings(diag: &mut DiagnosticCollector) {
    for path in [".claude/settings.json", ".claude/settings.local.json"] {
        let Ok(content) = fs::read_to_string(path) else {
            continue;
        };
        let Ok(settings) = serde_json::from_str::<JsonValue>(&content) else {
            continue;
        };
        diag.with_subject_path(path, |diag| {
            validate_typed_settings_file(diag, path, &settings);
        });
    }
}

/// Validate one parsed settings file. New typed fields belong here.
fn validate_typed_settings_file(diag: &mut DiagnosticCollector, path: &str, settings: &JsonValue) {
    if let Some(value) = settings.get("prUrlTemplate") {
        let valid = value.as_str().is_some_and(|template| {
            !template.trim().is_empty()
                && PR_URL_TEMPLATE_PLACEHOLDERS
                    .iter()
                    .any(|placeholder| template.contains(placeholder))
        });
        if !valid {
            diag.report(
                LintRule::SettingsPrUrlTemplateInvalid,
                &format!("{path}: 'prUrlTemplate' must be a non-empty string containing a documented placeholder"),
            );
        }
    }

    if let Some(value) = settings.get("channelsEnabled")
        && !value.is_boolean()
    {
        diag.report(
            LintRule::SettingsChannelsEnabledInvalid,
            &format!("{path}: 'channelsEnabled' must be a boolean"),
        );
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
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_private_config(&mut diag, &ExcludeSet::default());
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
            fs::write(".claude/settings.json", r#"{"prUrlTemplate":"https://{host}/{owner}/{repo}/pull/{number}","channelsEnabled":true}"#).unwrap();
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
