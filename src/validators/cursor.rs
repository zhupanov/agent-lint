//! Validation for Cursor project configuration.

use std::fs;
use std::path::Path;

use globset::GlobBuilder;
use serde_json::Value as JsonValue;
use serde_yaml::{Mapping, Value as YamlValue};

use crate::config::ExcludeSet;
use crate::diagnostic::DiagnosticCollector;
use crate::frontmatter;
use crate::rules::LintRule;
use crate::traversal;

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
];

/// Validate Cursor surfaces when they are present. Every validator is
/// file-gated, so Claude-only repositories receive no Cursor diagnostics.
pub fn validate(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    validate_legacy_rules(diag, exclude);
    validate_project_rules(diag, exclude);
    validate_hooks(diag, exclude);
    validate_agents(diag, exclude);
    validate_environment(diag, exclude);
    validate_skills(diag, exclude);
}

fn report(diag: &mut DiagnosticCollector, rule: LintRule, path: &str, message: &str) {
    diag.report(rule, &format!("{path}: {message}"));
}

fn yaml_key(name: &str) -> YamlValue {
    YamlValue::String(name.to_string())
}

fn yaml_string<'a>(map: &'a Mapping, name: &str) -> Option<&'a str> {
    map.get(yaml_key(name)).and_then(YamlValue::as_str)
}

fn validate_legacy_rules(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
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
    if fs::read_to_string(PATH).is_ok_and(|content| content.trim().is_empty()) {
        report(
            diag,
            LintRule::CursorRuleEmpty,
            PATH,
            "rule file has no instructions",
        );
    }
}

fn validate_project_rules(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let root = Path::new(".cursor/rules");
    if !root.is_dir() {
        return;
    }
    for entry in traversal::recursive_files(root, Path::new("."), Some(exclude)).entries {
        if entry.path.extension().is_none_or(|ext| ext != "mdc") {
            continue;
        }
        let path = entry.display;
        if let Ok(content) = fs::read_to_string(&entry.path) {
            validate_rule_file(diag, &path, &content);
        }
    }
}

fn validate_rule_file(diag: &mut DiagnosticCollector, path: &str, content: &str) {
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
    let yaml = match serde_yaml::from_str::<YamlValue>(&raw) {
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
    for key in yaml.keys().filter_map(YamlValue::as_str) {
        if !RULE_KEYS.contains(&key) {
            report(
                diag,
                LintRule::CursorRuleFieldUnknown,
                path,
                &format!("unknown frontmatter field '{key}'"),
            );
        }
    }
    let globs = yaml.get(yaml_key("globs"));
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
    let always_apply = yaml.get(yaml_key("alwaysApply"));
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
    if root.get("version").and_then(JsonValue::as_i64).is_none() {
        report(
            diag,
            LintRule::CursorHooksSchemaInvalid,
            PATH,
            "'version' must be an integer",
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
            if entry
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
            if entry.get("type").and_then(JsonValue::as_str) == Some("prompt") {
                if !entry.contains_key("prompt") {
                    report(
                        diag,
                        LintRule::CursorPromptHookPromptMissing,
                        PATH,
                        &format!("{label} is missing 'prompt'"),
                    );
                }
                if entry.get("model").is_some_and(|value| !value.is_string()) {
                    report(
                        diag,
                        LintRule::CursorPromptHookModelInvalid,
                        PATH,
                        &format!("{label}.model must be a string"),
                    );
                }
            }
        }
    }
}

fn validate_agents(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
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
            validate_agent_file(diag, &path, &content);
        }
    }
}

fn validate_agent_file(diag: &mut DiagnosticCollector, path: &str, content: &str) {
    let Some(lines) = frontmatter::extract_frontmatter(content) else {
        report(
            diag,
            LintRule::CursorAgentFrontmatterInvalid,
            path,
            "missing or malformed YAML frontmatter",
        );
        return;
    };
    let yaml = match serde_yaml::from_str::<YamlValue>(&lines.join("\n")) {
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
    if yaml
        .get(yaml_key("model"))
        .is_some_and(|value| !value.is_string())
    {
        report(
            diag,
            LintRule::CursorAgentFrontmatterInvalid,
            path,
            "'model' must be a string",
        );
    }
    for field in ["readonly", "is_background"] {
        if yaml
            .get(yaml_key(field))
            .is_some_and(|value| !value.is_bool())
        {
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
    let Some(root) = value.as_object() else {
        report(
            diag,
            LintRule::CursorEnvironmentInvalid,
            PATH,
            "top level must be an object",
        );
        return;
    };
    if root.get("install").and_then(JsonValue::as_str).is_none() {
        report(
            diag,
            LintRule::CursorEnvironmentInvalid,
            PATH,
            "'install' must be a string",
        );
    }
    for field in ["start", "update"] {
        if root.get(field).is_some_and(|value| !value.is_string()) {
            report(
                diag,
                LintRule::CursorEnvironmentInvalid,
                PATH,
                &format!("'{field}' must be a string"),
            );
        }
    }
    if let Some(build) = root.get("build") {
        let Some(build) = build.as_object() else {
            report(
                diag,
                LintRule::CursorEnvironmentInvalid,
                PATH,
                "'build' must be an object",
            );
            return;
        };
        for field in ["dockerfile", "context"] {
            if build.get(field).and_then(JsonValue::as_str).is_none() {
                report(
                    diag,
                    LintRule::CursorEnvironmentInvalid,
                    PATH,
                    &format!("'build.{field}' must be a string"),
                );
            }
        }
    }
    if let Some(terminals) = root.get("terminals") {
        let Some(terminals) = terminals.as_array() else {
            report(
                diag,
                LintRule::CursorEnvironmentInvalid,
                PATH,
                "'terminals' must be an array",
            );
            return;
        };
        for (index, terminal) in terminals.iter().enumerate() {
            let Some(terminal) = terminal.as_object() else {
                report(
                    diag,
                    LintRule::CursorEnvironmentInvalid,
                    PATH,
                    &format!("terminals[{}] must be an object", index + 1),
                );
                continue;
            };
            for field in ["name", "command"] {
                if terminal.get(field).and_then(JsonValue::as_str).is_none() {
                    report(
                        diag,
                        LintRule::CursorEnvironmentInvalid,
                        PATH,
                        &format!("terminals[{}].{field} must be a string", index + 1),
                    );
                }
            }
        }
    }
}

fn validate_skills(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
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
        let Some(lines) = frontmatter::extract_frontmatter(&content) else {
            continue;
        };
        let Ok(YamlValue::Mapping(yaml)) = serde_yaml::from_str::<YamlValue>(&lines.join("\n"))
        else {
            continue;
        };
        for key in yaml.keys().filter_map(YamlValue::as_str) {
            if !CURSOR_SKILL_KEYS.contains(&key) {
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
    fn hooks_validate_schema_events_and_prompt_fields() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".cursor")).unwrap();
        std::fs::write(tmp.path().join(".cursor/hooks.json"), r#"{"version":"1","hooks":{"unknown":[{"type":"prompt","timeout":"fast","loop_limit":false,"failClosed":"yes","model":1},{"type":"agent","command":"echo invalid"}]}}"#).unwrap();
        let codes = codes_for(tmp.path());
        for expected in [
            "CU010", "CU011", "CU012", "CU013", "CU017", "CU018", "CU019",
        ] {
            assert!(codes.contains(&expected), "missing {expected}: {codes:?}");
        }
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
}
