//! Validation for Codex override files, plugin manifests, and skills.

use crate::config::ExcludeSet;
use crate::diagnostic::DiagnosticCollector;
use crate::frontmatter;
use crate::live_instructions::{InstructionSurfaceKind, LiveInstructionDocument};
use crate::markdown::MarkdownDocument;
use crate::rules::LintRule;
use crate::traversal;
use crate::validators::common::is_valid_http_url;
use serde_json::Value;
use std::path::{Component, Path};

// Verified against openai/codex commit 18110b810f0a328147f6cd85e6f1ab6414927366
// (`codex-rs/core-plugins/src/manifest.rs`) on 2026-07-16.
const MAX_DEFAULT_PROMPT_COUNT: usize = 3;
const MAX_DEFAULT_PROMPT_LEN: usize = 128;
const CODEX_SKILL_UNSUPPORTED_FIELDS: &[&str] = &["context", "agent", "hooks"];

#[cfg(test)]
pub fn validate(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let mut prompt_pass = super::prompt_content::PromptContentPass::default();
    validate_with_prompt_pass(diag, exclude, &mut prompt_pass);
}

pub(crate) fn validate_with_prompt_pass(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    prompt_pass: &mut super::prompt_content::PromptContentPass,
) {
    validate_override_tracking(diag, exclude, prompt_pass);
    validate_plugin_manifests(diag, exclude);
    validate_codex_skill_frontmatter(diag, exclude);
}

fn validate_override_tracking(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    prompt_pass: &mut super::prompt_content::PromptContentPass,
) {
    let path = "AGENTS.override.md";
    if exclude.is_excluded(path) || !Path::new(path).is_file() {
        return;
    }
    if let Ok(content) = std::fs::read_to_string(path) {
        let markdown = MarkdownDocument::parse_body(content);
        let document = LiveInstructionDocument::new(
            Path::new(path),
            InstructionSurfaceKind::CodexAgentsOverride,
            &markdown,
        );
        prompt_pass.validate(&document, diag);
    }
    if !is_git_tracked(path) {
        return;
    }
    diag.report_at(LintRule::CodexAgentsOverrideTracked, path, "AGENTS.override.md is tracked by Git; add it to .gitignore because it holds user-specific overrides");
}

fn is_git_tracked(path: &str) -> bool {
    std::process::Command::new("git")
        .args(["ls-files", "--error-unmatch", "--", path])
        .output()
        .is_ok_and(|output| output.status.success())
}

fn validate_plugin_manifests(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let canonical = ".codex-plugin/plugin.json";
    for entry in traversal::recursive_files(Path::new("."), Path::new("."), Some(exclude)).entries {
        let path = &entry.path;
        let display = entry.display;
        if path.file_name().is_none_or(|name| name != "plugin.json")
            || !display.ends_with(".codex-plugin/plugin.json")
        {
            continue;
        }
        if display != canonical {
            diag.report_at(
                LintRule::CodexPluginManifestPath,
                &display,
                &format!("{display} must be located at .codex-plugin/plugin.json"),
            );
            continue;
        }
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        let value: Value = match serde_json::from_str(&content) {
            Ok(value) => value,
            Err(error) => {
                diag.report_at(
                    LintRule::CodexPluginManifestInvalid,
                    &display,
                    &format!("{display} is not valid JSON: {error}"),
                );
                continue;
            }
        };
        diag.with_subject_path(&display, |diag| {
            validate_plugin_manifest_value(diag, &display, &value);
        });
    }
}

fn validate_plugin_manifest_value(diag: &mut DiagnosticCollector, display: &str, value: &Value) {
    let Some(root) = value.as_object() else {
        diag.report(
            LintRule::CodexPluginNameMissing,
            &format!("{display}: plugin manifest must be a JSON object with a non-empty name"),
        );
        return;
    };
    match root.get("name").and_then(Value::as_str).map(str::trim) {
        Some(name) if !name.is_empty() => {
            if !name
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
            {
                diag.report(LintRule::CodexPluginNameInvalid, &format!("{display}: name must contain only ASCII alphanumerics, hyphens, or underscores"));
            }
        }
        _ => diag.report(
            LintRule::CodexPluginNameMissing,
            &format!("{display}: name must be present and non-empty"),
        ),
    }
    for field in ["skills", "mcpServers", "apps"] {
        if let Some(value) = root.get(field) {
            validate_component_paths(diag, display, field, value);
        }
    }
    if root.contains_key("hooks") {
        diag.report(
            LintRule::CodexPluginHooksUnsupported,
            &format!("{display}: hooks is not supported in Codex plugin manifests"),
        );
    }
    if root
        .get("description")
        .and_then(Value::as_str)
        .is_none_or(|value| value.trim().is_empty())
    {
        diag.report(
            LintRule::CodexPluginDescriptionMissing,
            &format!("{display}: description is missing or empty"),
        );
    }
    if let Some(interface) = root.get("interface").and_then(Value::as_object) {
        validate_default_prompts(
            diag,
            display,
            interface
                .get("defaultPrompt")
                .or_else(|| interface.get("default_prompts")),
        );
        validate_interface_urls(diag, display, interface);
        validate_interface_assets(diag, display, interface);
    }
}

fn validate_component_paths(
    diag: &mut DiagnosticCollector,
    display: &str,
    field: &str,
    value: &Value,
) {
    let paths: Vec<&str> = match value {
        Value::String(path) => vec![path],
        Value::Array(paths) => paths.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    for path in paths {
        let trimmed = path.trim();
        if path_has_traversal(trimmed) {
            diag.report(
                LintRule::CodexPluginPathTraversal,
                &format!("{display}: {field} path `{trimmed}` must not contain `..` segments"),
            );
        } else if trimmed == "./" {
            diag.report(
                LintRule::CodexPluginPathBare,
                &format!(
                    "{display}: {field} path must reference a file or directory, not bare `./`"
                ),
            );
        } else if !trimmed.starts_with("./") {
            diag.report(
                LintRule::CodexPluginPathPrefix,
                &format!("{display}: {field} path `{trimmed}` must start with `./`"),
            );
        }
    }
}

fn path_has_traversal(path: &str) -> bool {
    Path::new(path)
        .components()
        .any(|component| component == Component::ParentDir)
        || path.split('\\').any(|component| component == "..")
}

fn validate_default_prompts(diag: &mut DiagnosticCollector, display: &str, value: Option<&Value>) {
    let Some(value) = value else { return };
    let prompts: Vec<&str> = match value {
        Value::String(prompt) => vec![prompt],
        Value::Array(prompts) => prompts.iter().filter_map(Value::as_str).collect(),
        _ => return,
    };
    if prompts.len() > MAX_DEFAULT_PROMPT_COUNT {
        diag.report(LintRule::CodexPluginDefaultPromptCount, &format!("{display}: interface.defaultPrompt has {} entries; Codex supports at most {MAX_DEFAULT_PROMPT_COUNT}", prompts.len()));
    }
    for prompt in prompts {
        let normalized = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
        if normalized.is_empty() {
            diag.report(
                LintRule::CodexPluginDefaultPromptEmpty,
                &format!("{display}: interface.defaultPrompt must not contain an empty entry"),
            );
        } else if normalized.chars().count() > MAX_DEFAULT_PROMPT_LEN {
            diag.report(LintRule::CodexPluginDefaultPromptLength, &format!("{display}: interface.defaultPrompt entry exceeds Codex's {MAX_DEFAULT_PROMPT_LEN}-character limit"));
        }
    }
}

fn validate_interface_urls(
    diag: &mut DiagnosticCollector,
    display: &str,
    interface: &serde_json::Map<String, Value>,
) {
    for field in ["websiteUrl", "privacyPolicyUrl", "termsOfServiceUrl"] {
        let Some(value) = interface.get(field) else {
            continue;
        };
        if !value.as_str().is_some_and(is_valid_http_url) {
            diag.report(
                LintRule::CodexPluginInterfaceUrl,
                &format!("{display}: interface.{field} must be a valid http(s) URL"),
            );
        }
    }
}

fn validate_interface_assets(
    diag: &mut DiagnosticCollector,
    display: &str,
    interface: &serde_json::Map<String, Value>,
) {
    for field in ["composerIcon", "logo"] {
        if let Some(value) = interface.get(field) {
            validate_asset_path(diag, display, field, value);
        }
    }
    if let Some(screenshots) = interface.get("screenshots").and_then(Value::as_array) {
        for (index, value) in screenshots.iter().enumerate() {
            validate_asset_path(diag, display, &format!("screenshots[{index}]"), value);
        }
    }
}

fn validate_asset_path(diag: &mut DiagnosticCollector, display: &str, field: &str, value: &Value) {
    if !value
        .as_str()
        .is_some_and(|path| path.starts_with("./") && !path_has_traversal(path))
    {
        diag.report(
            LintRule::CodexPluginInterfaceAssetPath,
            &format!(
                "{display}: interface.{field} must start with `./` and must not contain traversal"
            ),
        );
    }
}

fn validate_codex_skill_frontmatter(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let root = Path::new(".agents/skills");
    if !root.is_dir() {
        return;
    }
    for entry in traversal::recursive_files(root, Path::new("."), Some(exclude)).entries {
        if entry.path.file_name().is_none_or(|name| name != "SKILL.md") {
            continue;
        }
        let path = &entry.path;
        let display = entry.display;
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        let Some(lines) = frontmatter::extract_frontmatter(&content) else {
            continue;
        };
        for line in lines {
            let Some((field, _)) = line.split_once(':') else {
                continue;
            };
            let field = field.trim();
            if CODEX_SKILL_UNSUPPORTED_FIELDS.contains(&field) {
                diag.report_at(LintRule::CodexSkillUnsupportedFrontmatter, &display, &format!("{display}: `{field}` is Claude-only skill frontmatter unsupported by Codex CLI"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[serial_test::serial]
    fn validates_tracked_agents_override() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::write("AGENTS.override.md", "personal settings\n").unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q"])
                .status()
                .unwrap()
                .success()
        );
        assert!(
            std::process::Command::new("git")
                .args(["add", "AGENTS.override.md"])
                .status()
                .unwrap()
                .success()
        );

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate(&mut diag, &ExcludeSet::default());
        assert!(
            diag.diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::CodexAgentsOverrideTracked)
        );
    }

    #[test]
    #[serial_test::serial]
    fn validates_codex_plugin_manifest_rules() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".codex-plugin").unwrap();
        std::fs::write(".codex-plugin/plugin.json", r#"{
          "name": "bad name!",
          "skills": "../skills",
          "hooks": {},
          "interface": {
            "defaultPrompt": [" ", "x", "x", "x", "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"],
            "websiteUrl": "ftp://example.com",
            "logo": "../logo.svg"
          }
        }"#).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate(&mut diag, &ExcludeSet::default());
        let errors = diag.errors();
        for needle in [
            "name must contain",
            "must not contain `..`",
            "hooks is not supported",
            "at most 3",
            "empty entry",
            "128-character",
            "valid http(s)",
            "must start with `./`",
            "description is missing",
        ] {
            assert!(
                errors.iter().any(|error| error.contains(needle)),
                "missing {needle}: {errors:?}"
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn validates_manifest_location_parse_name_and_component_path_rules() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("nested/.codex-plugin").unwrap();
        std::fs::write("nested/.codex-plugin/plugin.json", "{}").unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate(&mut diag, &ExcludeSet::default());
        assert!(
            diag.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.rule == LintRule::CodexPluginManifestPath)
        );

        std::fs::remove_dir_all("nested").unwrap();
        std::fs::create_dir_all(".codex-plugin").unwrap();
        std::fs::write(".codex-plugin/plugin.json", "{").unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate(&mut diag, &ExcludeSet::default());
        assert!(
            diag.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.rule == LintRule::CodexPluginManifestInvalid)
        );

        std::fs::write(
            ".codex-plugin/plugin.json",
            r#"{"skills":"skills", "mcpServers":"./", "apps":"./apps"}"#,
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate(&mut diag, &ExcludeSet::default());
        for rule in [
            LintRule::CodexPluginNameMissing,
            LintRule::CodexPluginPathPrefix,
            LintRule::CodexPluginPathBare,
        ] {
            assert!(
                diag.diagnostics()
                    .iter()
                    .any(|diagnostic| diagnostic.rule == rule),
                "missing {}: {:?}",
                rule.code(),
                diag.errors()
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn rejects_claude_only_codex_skill_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".agents/skills/example").unwrap();
        std::fs::write(".agents/skills/example/SKILL.md", "---\nname: example\ndescription: Example\ncontext: fork\nagent: Explore\nhooks: {}\n---\nbody\n").unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate(&mut diag, &ExcludeSet::default());
        assert_eq!(
            diag.errors()
                .iter()
                .filter(|error| error.contains("Claude-only"))
                .count(),
            3
        );
    }
}
