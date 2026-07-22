//! Validation for Cursor project configuration.

use std::fs;
use std::path::Path;
use std::sync::LazyLock;

use globset::GlobBuilder;
use jsonschema::error::ValidationErrorKind;
use serde_json::Value as JsonValue;
#[cfg(test)]
use serde_json::json;

use crate::config::ExcludeSet;
use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata};
use crate::frontmatter;
use crate::live_instructions::{InstructionSurfaceKind, LiveInstructionDocument};
use crate::markdown::MarkdownDocument;
use crate::platforms;
use crate::rules::LintRule;
use crate::traversal;
use crate::yaml::{Mapping, Value as YamlValue};

const RULE_KEYS: &[&str] = &["description", "globs", "alwaysApply"];
const CURSOR_SKILL_KEYS: &[&str] = &[
    "name",
    "description",
    "disable-model-invocation",
    "user-invocable",
];
const HOOK_EVENTS: &[&str] = &[
    "sessionStart",
    "sessionEnd",
    "preToolUse",
    "postToolUse",
    "postToolUseFailure",
    "subagentStart",
    "subagentStop",
    "beforeShellExecution",
    "afterShellExecution",
    "beforeMCPExecution",
    "afterMCPExecution",
    "beforeReadFile",
    "afterFileEdit",
    "beforeSubmitPrompt",
    "preCompact",
    "stop",
    "afterAgentResponse",
    "afterAgentThought",
    "beforeTabFileRead",
    "afterTabFileEdit",
    "workspaceOpen",
];

/// Vendored Cursor cloud-environment schema. It is compiled from the checked-in
/// snapshot, so linting neither reads a local schema path nor fetches a network
/// resource. See `schemas/cursor-environment.schema.md` for provenance.
static CURSOR_ENVIRONMENT_VALIDATOR: LazyLock<jsonschema::Validator> = LazyLock::new(|| {
    let schema = serde_json::from_str(include_str!("../../schemas/cursor-environment.schema.json"))
        .expect("checked-in Cursor environment schema parses");
    jsonschema::validator_for(&schema).expect("embedded Cursor environment schema is valid")
});

/// Validate Cursor surfaces when they are present. Every validator is
/// file-gated, so Claude-only repositories receive no Cursor diagnostics.
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
    validate_legacy_rules(diag, exclude, prompt_pass);
    validate_project_rules(diag, exclude, prompt_pass);
    validate_hooks(diag, exclude);
    validate_agents(diag, exclude, prompt_pass);
    validate_environment(diag, exclude);
    validate_skills(diag, exclude, prompt_pass);
}

fn report(diag: &mut DiagnosticCollector, rule: LintRule, path: &str, message: &str) {
    diag.report_at(rule, path, &format!("{path}: {message}"));
}

fn yaml_string<'a>(map: &'a Mapping, name: &str) -> Option<&'a str> {
    map.get(name).and_then(YamlValue::as_str)
}

fn validate_legacy_rules(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    prompt_pass: &mut super::prompt_content::PromptContentPass,
) {
    const PATH: &str = ".cursorrules";
    if exclude.is_excluded(PATH) || !Path::new(PATH).is_file() {
        return;
    }
    report(
        diag,
        LintRule::CursorLegacyRules,
        PATH,
        "legacy .cursorrules file is present; migrate to .cursor/rules/*.mdc",
    );
    if let Ok(content) = fs::read_to_string(PATH) {
        if content.trim().is_empty() {
            report(
                diag,
                LintRule::CursorRuleEmpty,
                PATH,
                "rule file has no instructions",
            );
        }
        let markdown = MarkdownDocument::parse_body(&content);
        let document = LiveInstructionDocument::new(
            Path::new(PATH),
            InstructionSurfaceKind::CursorLegacyRule,
            &markdown,
        );
        prompt_pass.validate(&document, diag);
    }
}

fn validate_project_rules(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    prompt_pass: &mut super::prompt_content::PromptContentPass,
) {
    for entry in platforms::cursor_rule_candidates(exclude) {
        let extension = entry.path.extension().and_then(|ext| ext.to_str());
        let path = entry.display;
        if extension == Some("md") {
            let renamed_path = path
                .strip_suffix(".md")
                .map(|stem| format!("{stem}.mdc"))
                .expect("Cursor candidate filtering only returns .md or .mdc files");
            diag.report_at_with(
                LintRule::CursorRuleExtension,
                &path,
                &format!("{path}: Cursor project rules must use the .mdc extension"),
                DiagnosticMetadata::default().with_suggestion(format!("rename to {renamed_path}")),
            );
        } else if let Ok(content) = fs::read_to_string(&entry.path) {
            if extension == Some("mdc") {
                validate_rule_file(diag, &path, &content, prompt_pass);
            }
        }
    }
}

fn validate_rule_file(
    diag: &mut DiagnosticCollector,
    path: &str,
    content: &str,
    prompt_pass: &mut super::prompt_content::PromptContentPass,
) {
    let markdown = MarkdownDocument::parse(content);
    let document = LiveInstructionDocument::new(
        Path::new(path),
        InstructionSurfaceKind::CursorRule,
        &markdown,
    );
    // Cursor still loads a rule body when its frontmatter is absent or
    // malformed, so structural failures must not exempt that live prose.
    prompt_pass.validate(&document, diag);
    if content.trim().is_empty() {
        report(
            diag,
            LintRule::CursorRuleEmpty,
            path,
            "rule file has no instructions",
        );
        return;
    }
    if !content.starts_with("---") {
        report(
            diag,
            LintRule::CursorRuleFrontmatterMissing,
            path,
            "missing YAML frontmatter",
        );
        return;
    }
    let Some(lines) = frontmatter::extract_frontmatter(content) else {
        report(
            diag,
            LintRule::CursorRuleFrontmatterInvalid,
            path,
            "frontmatter must have a closing '---' delimiter",
        );
        return;
    };
    if frontmatter::extract_body(content).trim().is_empty() {
        report(
            diag,
            LintRule::CursorRuleEmpty,
            path,
            "rule file has no instructions after frontmatter",
        );
    }
    let raw = lines.join("\n");
    let yaml = match crate::yaml::parse(&raw) {
        Ok(YamlValue::Mapping(map)) => map,
        // Empty YAML frontmatter is a valid manual/agent-requested rule shape;
        // CU009 reports its missing description when no targeting fields exist.
        Ok(YamlValue::Null) => Mapping::new(),
        Ok(_) => {
            report(
                diag,
                LintRule::CursorRuleFrontmatterInvalid,
                path,
                "frontmatter must be a YAML object",
            );
            return;
        }
        Err(error) => {
            report(
                diag,
                LintRule::CursorRuleFrontmatterInvalid,
                path,
                &format!("frontmatter is not valid YAML: {error}"),
            );
            return;
        }
    };
    for key in yaml.keys() {
        if !RULE_KEYS.contains(&key.as_str()) {
            report(
                diag,
                LintRule::CursorRuleFieldUnknown,
                path,
                &format!("unknown frontmatter field '{key}'"),
            );
        }
    }
    let globs = yaml.get("globs");
    if let Some(globs) = globs {
        let patterns: Option<Vec<&str>> = match globs {
            YamlValue::String(pattern) => Some(vec![pattern]),
            YamlValue::Sequence(items) => items.iter().map(YamlValue::as_str).collect(),
            _ => None,
        };
        match patterns {
            Some(patterns) => {
                for pattern in patterns {
                    if let Err(error) = GlobBuilder::new(pattern).build() {
                        report(
                            diag,
                            LintRule::CursorRuleGlobInvalid,
                            path,
                            &format!("invalid glob '{pattern}': {error}"),
                        );
                    }
                }
            }
            None => report(
                diag,
                LintRule::CursorRuleGlobInvalid,
                path,
                "'globs' must be a string or list of strings",
            ),
        }
    }
    let always_apply = yaml.get("alwaysApply");
    if let Some(value) = always_apply
        && !value.is_bool()
    {
        report(
            diag,
            LintRule::CursorAlwaysApplyInvalid,
            path,
            "'alwaysApply' must be a boolean",
        );
    }
    if always_apply.and_then(YamlValue::as_bool) == Some(true) && globs.is_some() {
        report(
            diag,
            LintRule::CursorAlwaysApplyGlobs,
            path,
            "'globs' is ignored when 'alwaysApply' is true",
        );
    }
    let description = yaml_string(&yaml, "description").unwrap_or("");
    if always_apply.is_none() && globs.is_none() && description.trim().is_empty() {
        report(
            diag,
            LintRule::CursorRuleDescriptionMissing,
            path,
            "agent-requested rule needs a non-empty 'description'",
        );
    }
}

fn validate_hooks(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    const PATH: &str = ".cursor/hooks.json";
    if exclude.is_excluded(PATH) {
        return;
    }
    let Ok(content) = fs::read_to_string(PATH) else {
        return;
    };
    let value = match serde_json::from_str::<JsonValue>(&content) {
        Ok(value) => value,
        Err(error) => {
            report(
                diag,
                LintRule::CursorHooksSchemaInvalid,
                PATH,
                &format!("invalid JSON: {error}"),
            );
            return;
        }
    };
    let Some(root) = value.as_object() else {
        report(
            diag,
            LintRule::CursorHooksSchemaInvalid,
            PATH,
            "top level must be an object",
        );
        return;
    };
    if !root
        .get("version")
        .is_some_and(|version| version.is_number() && version.as_f64() == Some(1.0))
    {
        report(
            diag,
            LintRule::CursorHooksSchemaInvalid,
            PATH,
            "'version' must be numeric schema version 1",
        );
    }
    let Some(hooks) = root.get("hooks").and_then(JsonValue::as_object) else {
        report(
            diag,
            LintRule::CursorHooksSchemaInvalid,
            PATH,
            "'hooks' must be an object",
        );
        return;
    };
    for (event, entries) in hooks {
        if !HOOK_EVENTS.contains(&event.as_str()) {
            report(
                diag,
                LintRule::CursorHookEventUnknown,
                PATH,
                &format!("unknown hook event '{event}'"),
            );
        }
        let Some(entries) = entries.as_array() else {
            report(
                diag,
                LintRule::CursorHooksSchemaInvalid,
                PATH,
                &format!("hooks.{event} must be an array"),
            );
            continue;
        };
        for (index, entry) in entries.iter().enumerate() {
            let label = format!("hooks.{event}[{}]", index + 1);
            let Some(entry) = entry.as_object() else {
                report(
                    diag,
                    LintRule::CursorHooksSchemaInvalid,
                    PATH,
                    &format!("{label} must be an object"),
                );
                continue;
            };
            let type_value = entry.get("type");
            let hook_type = type_value.and_then(JsonValue::as_str);
            if (type_value.is_none() || hook_type == Some("command"))
                && entry
                    .get("command")
                    .and_then(JsonValue::as_str)
                    .is_none_or(|value| value.trim().is_empty())
            {
                report(
                    diag,
                    LintRule::CursorHookCommandMissing,
                    PATH,
                    &format!("{label} is missing a non-empty 'command'"),
                );
            }
            if let Some(kind) = entry.get("type")
                && !matches!(kind.as_str(), Some("command" | "prompt"))
            {
                report(
                    diag,
                    LintRule::CursorHookTypeInvalid,
                    PATH,
                    &format!("{label}.type must be 'command' or 'prompt'"),
                );
            }
            for (field, valid) in [
                (
                    "timeout",
                    entry.get("timeout").is_none_or(JsonValue::is_number),
                ),
                (
                    "loop_limit",
                    entry
                        .get("loop_limit")
                        .is_none_or(|value| value.is_null() || value.is_number()),
                ),
                (
                    "failClosed",
                    entry.get("failClosed").is_none_or(JsonValue::is_boolean),
                ),
                (
                    "matcher",
                    entry.get("matcher").is_none_or(JsonValue::is_string),
                ),
            ] {
                if !valid {
                    report(
                        diag,
                        LintRule::CursorHookFieldTypeInvalid,
                        PATH,
                        &format!("{label}.{field} has an invalid type"),
                    );
                }
            }
            if hook_type == Some("prompt") {
                if entry
                    .get("prompt")
                    .and_then(JsonValue::as_str)
                    .is_none_or(|value| value.trim().is_empty())
                {
                    report(
                        diag,
                        LintRule::CursorPromptHookPromptMissing,
                        PATH,
                        &format!("{label} is missing a non-empty 'prompt'"),
                    );
                }
                if entry
                    .get("model")
                    .is_some_and(|value| value.as_str().is_none_or(|model| model.trim().is_empty()))
                {
                    report(
                        diag,
                        LintRule::CursorPromptHookModelInvalid,
                        PATH,
                        &format!("{label}.model must be a non-empty string"),
                    );
                }
            }
        }
    }
}

fn validate_agents(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    prompt_pass: &mut super::prompt_content::PromptContentPass,
) {
    let root = Path::new(".cursor/agents");
    if !root.is_dir() {
        return;
    }
    for entry in traversal::recursive_files(root, Path::new("."), Some(exclude)).entries {
        if entry.path.extension().is_none_or(|ext| ext != "md") {
            continue;
        }
        let path = entry.display;
        if let Ok(content) = fs::read_to_string(&entry.path) {
            validate_agent_file(diag, &path, &content, prompt_pass);
        }
    }
}

fn validate_agent_file(
    diag: &mut DiagnosticCollector,
    path: &str,
    content: &str,
    prompt_pass: &mut super::prompt_content::PromptContentPass,
) {
    let markdown = MarkdownDocument::parse(content);
    let document =
        LiveInstructionDocument::new(Path::new(path), InstructionSurfaceKind::Agent, &markdown);
    prompt_pass.validate(&document, diag);
    let Some(lines) = frontmatter::extract_frontmatter(content) else {
        report(
            diag,
            LintRule::CursorAgentFrontmatterInvalid,
            path,
            "missing or malformed YAML frontmatter",
        );
        return;
    };
    let yaml = match crate::yaml::parse(&lines.join("\n")) {
        Ok(YamlValue::Mapping(map)) => map,
        _ => {
            report(
                diag,
                LintRule::CursorAgentFrontmatterInvalid,
                path,
                "frontmatter must be a YAML object",
            );
            return;
        }
    };
    for field in ["name", "description"] {
        if yaml_string(&yaml, field).is_none_or(|value| value.trim().is_empty()) {
            report(
                diag,
                LintRule::CursorAgentFrontmatterInvalid,
                path,
                &format!("'{field}' must be a non-empty string"),
            );
        }
    }
    if yaml.get("model").is_some_and(|value| !value.is_string()) {
        report(
            diag,
            LintRule::CursorAgentFrontmatterInvalid,
            path,
            "'model' must be a string",
        );
    }
    for field in ["readonly", "is_background"] {
        if yaml.get(field).is_some_and(|value| !value.is_bool()) {
            report(
                diag,
                LintRule::CursorAgentFrontmatterInvalid,
                path,
                &format!("'{field}' must be a boolean"),
            );
        }
    }
    if frontmatter::extract_body(content).trim().is_empty() {
        report(
            diag,
            LintRule::CursorAgentBodyEmpty,
            path,
            "subagent body is empty",
        );
    }
}

fn validate_environment(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    const PATH: &str = ".cursor/environment.json";
    if exclude.is_excluded(PATH) {
        return;
    }
    let Ok(content) = fs::read_to_string(PATH) else {
        return;
    };
    let value = match serde_json::from_str::<JsonValue>(&content) {
        Ok(value) => value,
        Err(error) => {
            report(
                diag,
                LintRule::CursorEnvironmentInvalid,
                PATH,
                &format!("invalid JSON: {error}"),
            );
            return;
        }
    };
    for error in CURSOR_ENVIRONMENT_VALIDATOR.iter_errors(&value) {
        let property_path = cursor_environment_property_path(&error);
        report(
            diag,
            LintRule::CursorEnvironmentInvalid,
            PATH,
            // The configuration may contain commands or credentials. Retain
            // the actionable path and constraint while masking its value.
            &format!("{property_path}: {}", error.masked()),
        );
    }
}

/// Convert JSON Pointer locations into the property paths shown in agent-lint
/// diagnostics. Array indices are one-based, matching the validator's prior
/// Cursor environment output.
fn cursor_environment_property_path(error: &jsonschema::ValidationError<'_>) -> String {
    let mut pointer = error.instance_path().as_str().to_string();
    if let ValidationErrorKind::Required { property } = error.kind()
        && let Some(property) = property.as_str()
    {
        pointer.push('/');
        pointer.push_str(&property.replace('~', "~0").replace('/', "~1"));
    }

    let mut path = String::new();
    for segment in pointer.trim_start_matches('/').split('/') {
        if segment.is_empty() {
            continue;
        }
        let segment = segment.replace("~1", "/").replace("~0", "~");
        if let Ok(index) = segment.parse::<usize>() {
            path.push_str(&format!("[{}]", index + 1));
        } else if path.is_empty() {
            path.push_str(&segment);
        } else {
            path.push('.');
            path.push_str(&segment);
        }
    }
    if path.is_empty() {
        "top level".to_string()
    } else {
        path
    }
}

fn validate_skills(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    prompt_pass: &mut super::prompt_content::PromptContentPass,
) {
    let root = Path::new(".cursor/skills");
    if !root.is_dir() {
        return;
    }
    for entry in traversal::shallow_directories(root, Path::new("."), None).entries {
        let path = entry.path.join("SKILL.md");
        if !path.is_file() {
            continue;
        }
        let display_path = path.to_string_lossy().replace('\\', "/");
        if exclude.is_excluded(&display_path) {
            continue;
        }
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let markdown = MarkdownDocument::parse(&content);
        let document = LiveInstructionDocument::new(
            Path::new(&display_path),
            InstructionSurfaceKind::CursorSkill,
            &markdown,
        );
        prompt_pass.validate(&document, diag);
        let Some(lines) = frontmatter::extract_frontmatter(&content) else {
            continue;
        };
        let Ok(YamlValue::Mapping(yaml)) = crate::yaml::parse(&lines.join("\n")) else {
            continue;
        };
        for key in yaml.keys() {
            if !CURSOR_SKILL_KEYS.contains(&key.as_str()) {
                report(
                    diag,
                    LintRule::CursorSkillFieldUnsupported,
                    &display_path,
                    &format!("'{key}' is not supported by Cursor skills"),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ExcludeSet;
    use crate::test_helpers::CwdGuard;

    fn codes_for(root: &Path) -> Vec<&'static str> {
        let _guard = CwdGuard::new();
        std::env::set_current_dir(root).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate(&mut diag, &ExcludeSet::default());
        diag.diagnostics()
            .iter()
            .map(|item| item.rule.code())
            .collect()
    }

    fn environment_messages_for(content: &str) -> Vec<String> {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".cursor")).unwrap();
        std::fs::write(tmp.path().join(".cursor/environment.json"), content).unwrap();

        let _guard = CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate(&mut diag, &ExcludeSet::default());
        diag.diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::CursorEnvironmentInvalid)
            .map(|item| item.message.clone())
            .collect()
    }

    #[test]
    #[serial_test::serial]
    fn no_cursor_files_produce_no_cursor_diagnostics() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(codes_for(tmp.path()).is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn project_rules_cover_frontmatter_and_legacy_rules() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".cursor/rules/nested")).unwrap();
        std::fs::write(
            tmp.path().join(".cursor/rules/nested/rule.mdc"),
            "---\nalwaysApply: \"true\"\nglobs: '[unclosed'\nunknown: value\n---\n",
        )
        .unwrap();
        std::fs::write(tmp.path().join(".cursor/rules/missing.mdc"), "# Rule\n").unwrap();
        std::fs::write(
            tmp.path().join(".cursor/rules/malformed.mdc"),
            "---\nglobs: [\n---\nBody\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join(".cursor/rules/always.mdc"),
            "---\nalwaysApply: true\nglobs: '*.rs'\n---\nBody\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join(".cursor/rules/requested.mdc"),
            "---\n---\nBody\n",
        )
        .unwrap();
        std::fs::write(tmp.path().join(".cursorrules"), "").unwrap();
        let codes = codes_for(tmp.path());
        for expected in [
            "CU001", "CU002", "CU003", "CU004", "CU005", "CU006", "CU007", "CU008", "CU009",
        ] {
            assert!(codes.contains(&expected), "missing {expected}: {codes:?}");
        }
    }

    #[test]
    #[serial_test::serial]
    fn nested_mdc_rules_are_live_and_md_rules_only_report_extension() {
        let tmp = tempfile::tempdir().unwrap();
        let mdc = tmp.path().join("packages/api/.cursor/rules/api.mdc");
        let md = tmp.path().join("packages/web/.cursor/rules/not-a-rule.md");
        std::fs::create_dir_all(mdc.parent().unwrap()).unwrap();
        std::fs::create_dir_all(md.parent().unwrap()).unwrap();
        std::fs::write(&mdc, "---\nalwaysApply: true\n---\nRetry until success.\n").unwrap();
        std::fs::write(&md, "Retry until success.\n").unwrap();

        let _guard = CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate(&mut diag, &ExcludeSet::default());

        let identities: Vec<_> = diag
            .diagnostics()
            .iter()
            .map(|item| {
                (
                    item.rule.code(),
                    item.subject_path.as_ref().unwrap().display().to_string(),
                )
            })
            .collect();
        assert_eq!(
            identities,
            vec![
                ("Q005", "packages/api/.cursor/rules/api.mdc".to_string()),
                (
                    "CU020",
                    "packages/web/.cursor/rules/not-a-rule.md".to_string()
                ),
            ]
        );
    }

    #[test]
    #[serial_test::serial]
    fn documented_hook_events_are_accepted() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".cursor")).unwrap();
        let documented_events = [
            "sessionStart",
            "sessionEnd",
            "preToolUse",
            "postToolUse",
            "postToolUseFailure",
            "subagentStart",
            "subagentStop",
            "beforeShellExecution",
            "afterShellExecution",
            "beforeMCPExecution",
            "afterMCPExecution",
            "beforeReadFile",
            "afterFileEdit",
            "beforeSubmitPrompt",
            "preCompact",
            "stop",
            "afterAgentResponse",
            "afterAgentThought",
            "beforeTabFileRead",
            "afterTabFileEdit",
            "workspaceOpen",
        ];
        assert_eq!(HOOK_EVENTS, documented_events);

        let mut hooks = serde_json::Map::new();
        for event in documented_events {
            hooks.insert(event.to_string(), json!([{"command": "echo hook"}]));
        }
        hooks.insert(
            "beforeShellExecution".to_string(),
            json!([{
                "type": "prompt",
                "prompt": "Allow read-only commands?",
                "timeout": 10,
                "futureCursorField": {"accepted": true}
            }]),
        );
        std::fs::write(
            tmp.path().join(".cursor/hooks.json"),
            serde_json::to_string(&json!({"version": 1.0, "hooks": hooks})).unwrap(),
        )
        .unwrap();
        assert!(codes_for(tmp.path()).is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn hooks_validate_schema_and_entry_shapes() {
        let cases = [
            ("null", vec!["CU010"]),
            ("{", vec!["CU010"]),
            (r#"{"hooks": {}}"#, vec!["CU010"]),
            (r#"{"version": 1}"#, vec!["CU010"]),
            (r#"{"version": 2, "hooks": {}}"#, vec!["CU010"]),
            (r#"{"version": "1", "hooks": {}}"#, vec!["CU010"]),
            (r#"{"version": 1.5, "hooks": {}}"#, vec!["CU010"]),
            (r#"{"version": 1, "hooks": {"stop": {}}}"#, vec!["CU010"]),
            (
                r#"{"version": 1, "hooks": {"stop": [null]}}"#,
                vec!["CU010"],
            ),
            (r#"{"version": 1, "hooks": {}}"#, vec![]),
            (r#"{"version": 1.0, "hooks": {}}"#, vec![]),
        ];
        for (content, expected) in cases {
            let tmp = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(tmp.path().join(".cursor")).unwrap();
            std::fs::write(tmp.path().join(".cursor/hooks.json"), content).unwrap();
            assert_eq!(codes_for(tmp.path()), expected, "content: {content}");
        }
    }

    #[test]
    #[serial_test::serial]
    fn hooks_apply_command_prompt_and_field_contracts() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".cursor")).unwrap();
        std::fs::write(
            tmp.path().join(".cursor/hooks.json"),
            r#"{
                "version": 1,
                "hooks": {
                    "unknown": [{"command": "echo hook"}],
                    "beforeShellExecution": [
                        {"type": "prompt"},
                        {"type": "prompt", "prompt": null},
                        {"type": "prompt", "prompt": 7},
                        {"type": "prompt", "prompt": "   "},
                        {"type": "prompt", "prompt": "Continue?", "model": null},
                        {"type": "prompt", "prompt": "Continue?", "model": 7},
                        {"type": "prompt", "prompt": "Continue?", "model": "  "},
                        {"command": ""},
                        {"type": "command", "command": 7},
                        {"type": "agent", "command": "echo ignored"},
                        {"type": 7},
                        {"command": "echo valid", "timeout": 1.5, "loop_limit": null, "failClosed": true, "matcher": "Bash"},
                        {"command": "echo invalid", "timeout": "fast", "loop_limit": false, "failClosed": "yes", "matcher": 7}
                    ]
                }
            }"#,
        )
        .unwrap();
        assert_eq!(
            codes_for(tmp.path()),
            vec![
                "CU018", "CU018", "CU018", "CU018", "CU019", "CU019", "CU019", "CU012", "CU012",
                "CU013", "CU013", "CU017", "CU017", "CU017", "CU017", "CU011",
            ]
        );
    }

    #[test]
    #[serial_test::serial]
    fn agents_environment_and_skills_are_validated() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".cursor/agents/nested")).unwrap();
        std::fs::create_dir_all(tmp.path().join(".cursor/skills/example")).unwrap();
        std::fs::write(
            tmp.path().join(".cursor/agents/nested/reviewer.md"),
            "---\nname: 1\ndescription: reviewer\nreadonly: \"true\"\n---\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join(".cursor/environment.json"),
            r#"{"install":1,"build":{"dockerfile":1},"terminals":[{"name":"app"}]}"#,
        )
        .unwrap();
        std::fs::write(tmp.path().join(".cursor/skills/example/SKILL.md"), "---\nname: example\ndescription: Test\nmodel: opus\ncontext: fork\nhooks: {}\n---\nBody\n").unwrap();
        let codes = codes_for(tmp.path());
        for expected in ["CU014", "CU015", "CU016", "CR-SK-001"] {
            assert!(codes.contains(&expected), "missing {expected}: {codes:?}");
        }
        assert_eq!(codes.iter().filter(|code| **code == "CR-SK-001").count(), 3);
    }

    #[test]
    #[serial_test::serial]
    fn cursor_environment_schema_is_checked_in_and_compiles() {
        let schema: JsonValue =
            serde_json::from_str(include_str!("../../schemas/cursor-environment.schema.json"))
                .expect("checked-in Cursor environment schema parses");
        assert!(schema.get("allOf").is_some());
        assert!(jsonschema::validator_for(&schema).is_ok());

        let provenance = include_str!("../../schemas/cursor-environment.schema.md");
        assert!(provenance.contains("https://www.cursor.com/schemas/environment.schema.json"));
        assert!(provenance.contains("Retrieved: 2026-07-21"));
        assert!(
            provenance.contains("62b13994164f4186198b1f002ff957605df37ba5eee803e6afe69c981af001d6")
        );
        assert!(provenance.contains("curl --fail"));
    }

    #[test]
    #[serial_test::serial]
    fn cursor_environment_schema_accepts_every_current_shape() {
        let valid = [
            ("empty", r#"{}"#),
            (
                "snapshot only",
                r#"{"snapshot":"snapshot-20260212-00000000"}"#,
            ),
            ("install only", r#"{"install":"npm ci"}"#),
            (
                "dockerfile without context",
                r#"{"build":{"dockerfile":"Dockerfile"}}"#,
            ),
            (
                "terminal object",
                r#"{"terminals":[{"command":"npm start"}]}"#,
            ),
            (
                "terminal array",
                r#"{"terminals":[[{"command":"npm start","name":"web","description":"serve"}]]}"#,
            ),
            ("lowest port", r#"{"ports":[{"port":1}]}"#),
            ("highest port", r#"{"ports":[{"port":65535,"name":"web"}]}"#),
            (
                "all common and container properties",
                r#"{"name":"development","user":"agent","install":"npm ci","start":"npm start","repositoryDependencies":["github.com/acme/api"],"ports":[{"port":3000,"name":"web"}],"terminals":[{"command":"npm start","name":"web","description":"serve"}],"build":{"dockerfile":"Dockerfile","context":"."},"snapshot":"snapshot-20260212-00000000","agentCanUpdateSnapshot":true}"#,
            ),
        ];

        for (name, content) in valid {
            let messages = environment_messages_for(content);
            assert!(messages.is_empty(), "{name}: {messages:?}");
        }
    }

    #[test]
    #[serial_test::serial]
    fn cursor_environment_schema_rejects_every_current_constraint() {
        let invalid = [
            (
                "invalid JSON",
                r#"{"install":"unterminated"#,
                "invalid JSON:",
            ),
            ("root type", r#"[]"#, "top level:"),
            (
                "obsolete update",
                r#"{"update":"npm update"}"#,
                "top level:",
            ),
            ("unknown top level", r#"{"futureField":true}"#, "top level:"),
            ("name type", r#"{"name":false}"#, "name:"),
            ("user type", r#"{"user":false}"#, "user:"),
            ("install type", r#"{"install":false}"#, "install:"),
            ("start type", r#"{"start":false}"#, "start:"),
            (
                "repository dependency type",
                r#"{"repositoryDependencies":[false]}"#,
                "repositoryDependencies[1]:",
            ),
            (
                "repository dependencies type",
                r#"{"repositoryDependencies":false}"#,
                "repositoryDependencies:",
            ),
            ("ports type", r#"{"ports":false}"#, "ports:"),
            ("port entry type", r#"{"ports":[false]}"#, "ports[1]:"),
            ("port required", r#"{"ports":[{}]}"#, "ports[1].port:"),
            (
                "port name type",
                r#"{"ports":[{"port":1,"name":false}]}"#,
                "ports[1].name:",
            ),
            ("port zero", r#"{"ports":[{"port":0}]}"#, "ports[1].port:"),
            (
                "port too high",
                r#"{"ports":[{"port":65536}]}"#,
                "ports[1].port:",
            ),
            ("terminal type", r#"{"terminals":false}"#, "terminals:"),
            (
                "terminal command required",
                r#"{"terminals":[{}]}"#,
                "terminals[1]:",
            ),
            (
                "terminal command type",
                r#"{"terminals":[{"command":false}]}"#,
                "terminals[1]:",
            ),
            (
                "terminal name type",
                r#"{"terminals":[{"command":"run","name":false}]}"#,
                "terminals[1]:",
            ),
            (
                "terminal description type",
                r#"{"terminals":[{"command":"run","description":false}]}"#,
                "terminals[1]:",
            ),
            (
                "nested terminal command required",
                r#"{"terminals":[[{}]]}"#,
                "terminals[1]:",
            ),
            (
                "nested terminal field type",
                r#"{"terminals":[[{"command":"run","name":false,"description":false}]]}"#,
                "terminals[1]:",
            ),
            ("build type", r#"{"build":false}"#, "build:"),
            (
                "dockerfile required",
                r#"{"build":{}}"#,
                "build.dockerfile:",
            ),
            (
                "dockerfile type",
                r#"{"build":{"dockerfile":false}}"#,
                "build.dockerfile:",
            ),
            (
                "context type",
                r#"{"build":{"dockerfile":"Dockerfile","context":false}}"#,
                "build.context:",
            ),
            (
                "unknown build property",
                r#"{"build":{"dockerfile":"Dockerfile","image":"base"}}"#,
                "build:",
            ),
            (
                "agent can update snapshot type",
                r#"{"agentCanUpdateSnapshot":"yes"}"#,
                "agentCanUpdateSnapshot:",
            ),
            ("snapshot type", r#"{"snapshot":false}"#, "snapshot:"),
        ];

        for (name, content, path) in invalid {
            let messages = environment_messages_for(content);
            assert!(!messages.is_empty(), "{name} produced no CU016 diagnostic");
            assert!(
                messages.iter().any(|message| message.contains(path)),
                "{name} did not report {path}: {messages:?}"
            );
            assert!(
                messages
                    .iter()
                    .all(|message| message.starts_with(".cursor/environment.json: ")),
                "{name} has an unstable subject: {messages:?}"
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn cursor_environment_schema_never_echoes_invalid_instance_values() {
        let invalid = [
            (r#"{"install":"unclosed-secret"#, "unclosed-secret"),
            (r#"{"install":["type-secret"]}"#, "type-secret"),
            (
                r#"{"build":{"context":"required-secret"}}"#,
                "required-secret",
            ),
            (
                r#"{"ports":[{"port":0,"name":"range-secret"}]}"#,
                "range-secret",
            ),
            (
                r#"{"terminals":[{"command":["one-of-secret"]}]}"#,
                "one-of-secret",
            ),
            (r#"{"build":"object-secret"}"#, "object-secret"),
            (r#"{"unknown":"unevaluated-secret"}"#, "unevaluated-secret"),
        ];

        for (content, secret) in invalid {
            let messages = environment_messages_for(content);
            assert!(
                !messages.is_empty(),
                "{secret} produced no CU016 diagnostic"
            );
            assert!(
                messages.iter().all(|message| !message.contains(secret)),
                "a diagnostic exposed {secret}: {messages:?}"
            );
        }
    }
}
