use crate::context::{LintContext, ManifestState};
use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata};
use crate::rules::LintRule;
use crate::sensitive::contains_sensitive_evidence;
use regex::Regex;
use serde_json::{Map, Value};
use std::sync::LazyLock;

const PLUGIN_JSON: &str = ".claude-plugin/plugin.json";

/// Exact Claude Code / SchemaStore identifier grammar for userConfig keys.
static RE_CONFIG_KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*$").unwrap());

const VALID_TYPES: &[&str] = &["string", "number", "boolean", "directory", "file"];

const KNOWN_OPTION_FIELDS: &[&str] = &[
    "type",
    "title",
    "description",
    "required",
    "default",
    "multiple",
    "sensitive",
    "min",
    "max",
];

/// Validate top-level and channel `userConfig` surfaces (U001–U002, U004–U009).
///
/// U003 (scripts-only env-var mapping) was removed: option use is not inferred
/// from repository text. Title and description enforce a non-empty-after-trim
/// usability policy that is intentionally stricter than the JSON schema.
/// U009 forbids a manifest-committed `default` that ships a secret; it is also
/// stricter than the schema (see [`validate_default_secret`]).
pub fn validate_user_config(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    let val = match &ctx.plugin_json {
        ManifestState::Parsed(v) => v,
        _ => return,
    };

    if let Some(user_config) = val.get("userConfig") {
        validate_container(diag, "userConfig", "/userConfig", user_config);
    }

    match val.get("channels") {
        Some(Value::Array(entries)) => {
            for (index, entry) in entries.iter().enumerate() {
                if let Some(user_config) = entry.get("userConfig") {
                    validate_container(
                        diag,
                        &format!("channels[{index}].userConfig"),
                        &format!("/channels/{index}/userConfig"),
                        user_config,
                    );
                }
            }
        }
        Some(Value::Object(entries)) => {
            let mut names: Vec<&String> = entries.keys().collect();
            names.sort();
            for name in names {
                if let Some(user_config) = entries[name].get("userConfig") {
                    validate_container(
                        diag,
                        &format!("channels.{name}.userConfig"),
                        &format!("/channels/{}/userConfig", json_pointer_escape(name)),
                        user_config,
                    );
                }
            }
        }
        _ => {}
    }
}

fn validate_container(
    diag: &mut DiagnosticCollector,
    display: &str,
    pointer: &str,
    user_config: &Value,
) {
    let Some(map) = user_config.as_object() else {
        report(
            diag,
            LintRule::UserconfigNotObject,
            &format!("{PLUGIN_JSON} {display} must be an object"),
            pointer,
            &format!("change {display} to a JSON object of option entries"),
        );
        return;
    };

    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort();
    for key in keys {
        validate_option(diag, display, pointer, key, &map[key]);
    }
}

fn validate_option(
    diag: &mut DiagnosticCollector,
    container_display: &str,
    container_pointer: &str,
    key: &str,
    entry: &Value,
) {
    let option_display = format!("{container_display}.{key}");
    let option_pointer = format!("{container_pointer}/{}", json_pointer_escape(key));

    if !RE_CONFIG_KEY.is_match(key) {
        report(
            diag,
            LintRule::UserconfigKeyInvalid,
            &format!(
                "{PLUGIN_JSON} {option_display} key is not a valid identifier (must match ^[A-Za-z_][A-Za-z0-9_]*$)"
            ),
            &option_pointer,
            "rename the key to an ASCII identifier starting with a letter or underscore, containing only letters, digits, and underscores",
        );
    }

    let Some(fields) = entry.as_object() else {
        report(
            diag,
            LintRule::UserconfigOptionInvalid,
            &format!("{PLUGIN_JSON} {option_display} must be an object"),
            &option_pointer,
            "replace the entry with an object that declares type, title, and description",
        );
        return;
    };

    let type_name = validate_type(diag, &option_display, &option_pointer, fields);
    validate_title(diag, &option_display, &option_pointer, fields);
    validate_description(diag, &option_display, &option_pointer, fields);
    validate_sensitive(diag, &option_display, &option_pointer, fields);
    validate_optional_shapes(diag, &option_display, &option_pointer, fields);
    if let Some(type_name) = type_name {
        validate_semantic_combinations(diag, &option_display, &option_pointer, fields, type_name);
    }
    validate_default_secret(diag, &option_display, &option_pointer, fields);
}

/// U009: a `userConfig` option `default` must not commit a secret. At most one
/// diagnostic per option, first matching branch:
///
/// (a) `sensitive: true` declares the value secret (masked input), so any
/// manifest-committed `default` of any shape contradicts it — the shared default
/// sits in exactly the public file `sensitive` exists to keep the value out of.
/// A non-boolean `sensitive` is U004's concern and is skipped here (no cascade).
///
/// (b) otherwise a `default` that is a string, or an array containing a string,
/// for which the shared possible-secret heuristic
/// ([`contains_sensitive_evidence`]) matches is a committed credential injected
/// into every `${user_config.KEY}` consumer, even without `sensitive`.
/// Non-string/non-array `default` shapes are U008's concern and are skipped.
///
/// This convention is stricter than the manifest schema. No output channel
/// (message, evidence, or suggestion) contains any character of the default.
fn validate_default_secret(
    diag: &mut DiagnosticCollector,
    option_display: &str,
    option_pointer: &str,
    fields: &Map<String, Value>,
) {
    let default_pointer = format!("{option_pointer}/default");
    let suggestion =
        "remove the default and let each user supply the value through plugin configuration";

    // (a) sensitive:true with any committed default.
    if fields.get("sensitive") == Some(&Value::Bool(true)) && fields.contains_key("default") {
        report(
            diag,
            LintRule::UserconfigDefaultSecret,
            &format!(
                "{PLUGIN_JSON} {option_display}.default must not be declared for a sensitive option"
            ),
            &default_pointer,
            suggestion,
        );
        return;
    }

    // (b) secret-shaped string or string-array default, regardless of sensitive.
    let Some(default) = fields.get("default") else {
        return;
    };
    let is_secret_shaped = match default {
        Value::String(value) => contains_sensitive_evidence(value),
        Value::Array(items) => items
            .iter()
            .any(|item| item.as_str().is_some_and(contains_sensitive_evidence)),
        _ => false,
    };
    if is_secret_shaped {
        report(
            diag,
            LintRule::UserconfigDefaultSecret,
            &format!("{PLUGIN_JSON} {option_display}.default is a secret-shaped literal"),
            &default_pointer,
            suggestion,
        );
    }
}

fn validate_type(
    diag: &mut DiagnosticCollector,
    option_display: &str,
    option_pointer: &str,
    fields: &Map<String, Value>,
) -> Option<&'static str> {
    let pointer = format!("{option_pointer}/type");
    match fields.get("type") {
        Some(Value::String(value)) => {
            if let Some(valid) = VALID_TYPES
                .iter()
                .copied()
                .find(|candidate| candidate == value)
            {
                Some(valid)
            } else {
                report(
                    diag,
                    LintRule::UserconfigTypeMissing,
                    &format!(
                        "{PLUGIN_JSON} {option_display}.type must be one of string, number, boolean, directory, or file"
                    ),
                    &pointer,
                    "set type to string, number, boolean, directory, or file",
                );
                None
            }
        }
        Some(_) => {
            report(
                diag,
                LintRule::UserconfigTypeMissing,
                &format!(
                    "{PLUGIN_JSON} {option_display}.type must be one of string, number, boolean, directory, or file"
                ),
                &pointer,
                "set type to a string enum value: string, number, boolean, directory, or file",
            );
            None
        }
        None => {
            report(
                diag,
                LintRule::UserconfigTypeMissing,
                &format!(
                    "{PLUGIN_JSON} {option_display} missing type (must be string, number, boolean, directory, or file)"
                ),
                option_pointer,
                "add a type field set to string, number, boolean, directory, or file",
            );
            None
        }
    }
}

fn validate_title(
    diag: &mut DiagnosticCollector,
    option_display: &str,
    option_pointer: &str,
    fields: &Map<String, Value>,
) {
    let pointer = format!("{option_pointer}/title");
    match fields.get("title") {
        Some(Value::String(value)) if !value.trim().is_empty() => {}
        Some(Value::String(_)) => report(
            diag,
            LintRule::UserconfigTitleMissing,
            &format!(
                "{PLUGIN_JSON} {option_display}.title must be a non-empty string after trimming whitespace"
            ),
            &pointer,
            "provide a non-empty title label for the config dialog",
        ),
        Some(_) => report(
            diag,
            LintRule::UserconfigTitleMissing,
            &format!("{PLUGIN_JSON} {option_display}.title must be a non-empty string"),
            &pointer,
            "set title to a non-empty string",
        ),
        None => report(
            diag,
            LintRule::UserconfigTitleMissing,
            &format!("{PLUGIN_JSON} {option_display} missing title (must be a non-empty string)"),
            option_pointer,
            "add a non-empty title string",
        ),
    }
}

fn validate_description(
    diag: &mut DiagnosticCollector,
    option_display: &str,
    option_pointer: &str,
    fields: &Map<String, Value>,
) {
    let pointer = format!("{option_pointer}/description");
    match fields.get("description") {
        Some(Value::String(value)) if !value.trim().is_empty() => {}
        Some(Value::String(_)) => report(
            diag,
            LintRule::UserconfigDescMissing,
            &format!(
                "{PLUGIN_JSON} {option_display}.description must be a non-empty string after trimming whitespace"
            ),
            &pointer,
            "provide a non-empty description for the config dialog",
        ),
        Some(_) => report(
            diag,
            LintRule::UserconfigDescMissing,
            &format!("{PLUGIN_JSON} {option_display}.description must be a non-empty string"),
            &pointer,
            "set description to a non-empty string",
        ),
        None => report(
            diag,
            LintRule::UserconfigDescMissing,
            &format!(
                "{PLUGIN_JSON} {option_display} missing description (must be a non-empty string)"
            ),
            option_pointer,
            "add a non-empty description string",
        ),
    }
}

fn validate_sensitive(
    diag: &mut DiagnosticCollector,
    option_display: &str,
    option_pointer: &str,
    fields: &Map<String, Value>,
) {
    if let Some(sensitive) = fields.get("sensitive")
        && !sensitive.is_boolean()
    {
        report(
            diag,
            LintRule::UserconfigSensitiveType,
            &format!("{PLUGIN_JSON} {option_display}.sensitive must be a boolean"),
            &format!("{option_pointer}/sensitive"),
            "set sensitive to true or false",
        );
    }
}

fn validate_optional_shapes(
    diag: &mut DiagnosticCollector,
    option_display: &str,
    option_pointer: &str,
    fields: &Map<String, Value>,
) {
    let mut names: Vec<&String> = fields.keys().collect();
    names.sort();
    for name in names {
        let field_pointer = format!("{option_pointer}/{}", json_pointer_escape(name));
        let value = &fields[name];
        if !KNOWN_OPTION_FIELDS.contains(&name.as_str()) {
            report(
                diag,
                LintRule::UserconfigOptionInvalid,
                &format!("{PLUGIN_JSON} {option_display} has unknown field '{name}'"),
                &field_pointer,
                "remove the unknown field; allowed fields are type, title, description, required, default, multiple, sensitive, min, and max",
            );
            continue;
        }
        match name.as_str() {
            "required" | "multiple" if !value.is_boolean() => report(
                diag,
                LintRule::UserconfigOptionInvalid,
                &format!("{PLUGIN_JSON} {option_display}.{name} must be a boolean"),
                &field_pointer,
                &format!("set {name} to true or false"),
            ),
            "min" | "max" if !is_finite_number(value) => report(
                diag,
                LintRule::UserconfigOptionInvalid,
                &format!("{PLUGIN_JSON} {option_display}.{name} must be a finite JSON number"),
                &field_pointer,
                &format!("set {name} to a finite number"),
            ),
            "default" if !is_allowed_default_shape(value) => report(
                diag,
                LintRule::UserconfigOptionInvalid,
                &format!(
                    "{PLUGIN_JSON} {option_display}.default must be a string, finite number, boolean, or array of strings"
                ),
                &field_pointer,
                "set default to a string, finite number, boolean, or string array that matches the option type",
            ),
            _ => {}
        }
    }
}

fn validate_semantic_combinations(
    diag: &mut DiagnosticCollector,
    option_display: &str,
    option_pointer: &str,
    fields: &Map<String, Value>,
    type_name: &str,
) {
    if matches!(fields.get("multiple"), Some(Value::Bool(_))) && type_name != "string" {
        report(
            diag,
            LintRule::UserconfigOptionInvalid,
            &format!(
                "{PLUGIN_JSON} {option_display}.multiple is only permitted when type is string"
            ),
            &format!("{option_pointer}/multiple"),
            "remove multiple or change type to string",
        );
    }

    for bound in ["min", "max"] {
        match fields.get(bound) {
            Some(value) if is_finite_number(value) && type_name != "number" => report(
                diag,
                LintRule::UserconfigOptionInvalid,
                &format!(
                    "{PLUGIN_JSON} {option_display}.{bound} is only permitted when type is number"
                ),
                &format!("{option_pointer}/{bound}"),
                "remove the bound or change type to number",
            ),
            _ => {}
        }
    }

    if type_name == "number"
        && let (Some(min), Some(max)) = (fields.get("min"), fields.get("max"))
        && let (Some(min_n), Some(max_n)) = (min.as_f64(), max.as_f64())
        && min_n.is_finite()
        && max_n.is_finite()
        && min_n > max_n
    {
        report(
            diag,
            LintRule::UserconfigOptionInvalid,
            &format!("{PLUGIN_JSON} {option_display}.min must be less than or equal to max"),
            option_pointer,
            "swap or adjust min and max so min <= max",
        );
    }

    if let Some(default) = fields.get("default")
        && is_allowed_default_shape(default)
        && !default_matches_type(default, type_name, fields.get("multiple"))
    {
        report(
            diag,
            LintRule::UserconfigOptionInvalid,
            &format!(
                "{PLUGIN_JSON} {option_display}.default does not match the declared type '{type_name}'"
            ),
            &format!("{option_pointer}/default"),
            "set default to a value that matches the option type (string arrays require type string with multiple true)",
        );
    }
}

fn default_matches_type(default: &Value, type_name: &str, multiple: Option<&Value>) -> bool {
    let multiple = matches!(multiple, Some(Value::Bool(true)));
    match type_name {
        "string" => match default {
            Value::String(_) => true,
            Value::Array(items) => multiple && items.iter().all(Value::is_string),
            _ => false,
        },
        "number" => is_finite_number(default),
        "boolean" => default.is_boolean(),
        "directory" | "file" => default.is_string(),
        _ => false,
    }
}

fn is_allowed_default_shape(value: &Value) -> bool {
    match value {
        Value::String(_) | Value::Bool(_) => true,
        Value::Number(_) => is_finite_number(value),
        Value::Array(items) => items.iter().all(Value::is_string),
        _ => false,
    }
}

fn is_finite_number(value: &Value) -> bool {
    value.as_f64().is_some_and(f64::is_finite)
}

fn json_pointer_escape(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

fn report(
    diag: &mut DiagnosticCollector,
    rule: LintRule,
    message: &str,
    evidence: &str,
    suggestion: &str,
) {
    diag.report_with(
        rule,
        message,
        DiagnosticMetadata::default()
            .with_evidence(evidence)
            .with_suggestion(suggestion),
    );
}

/// ASCII-uppercase environment name for a valid userConfig key.
#[cfg(test)]
fn env_option_name(key: &str) -> String {
    format!("CLAUDE_PLUGIN_OPTION_{}", key.to_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::Severity;

    fn make_ctx(plugin: ManifestState) -> LintContext {
        LintContext {
            base_path: std::path::PathBuf::new(),
            mode: crate::context::LintMode::Plugin,
            plugin_json: plugin,
            marketplace_json: ManifestState::Missing,
            hooks_json: ManifestState::Missing,
            declared_hook_configs: vec![],
            settings_json: ManifestState::Missing,
            settings_local_json: ManifestState::Missing,
        }
    }

    fn run(val: Value) -> DiagnosticCollector {
        let ctx = make_ctx(ManifestState::parsed(val));
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_user_config(&ctx, &mut diag);
        diag
    }

    fn codes(diag: &DiagnosticCollector) -> Vec<&str> {
        diag.diagnostics().iter().map(|d| d.rule.code()).collect()
    }

    fn valid_option() -> Value {
        serde_json::json!({
            "type": "string",
            "title": "Token",
            "description": "Bot token"
        })
    }

    #[test]
    fn env_option_name_is_ascii_uppercase_only() {
        assert_eq!(
            env_option_name("slackBotToken"),
            "CLAUDE_PLUGIN_OPTION_SLACKBOTTOKEN"
        );
        assert_eq!(env_option_name("api_key"), "CLAUDE_PLUGIN_OPTION_API_KEY");
    }

    #[test]
    fn absent_userconfig_is_silent() {
        let diag = run(serde_json::json!({"name": "p"}));
        assert!(diag.diagnostics().is_empty());
    }

    #[test]
    fn top_level_non_object_container_is_u001() {
        let diag = run(serde_json::json!({"userConfig": []}));
        assert_eq!(codes(&diag), ["U001"]);
        assert_eq!(
            diag.diagnostics()[0].evidence.as_deref(),
            Some("/userConfig")
        );
    }

    #[test]
    fn channel_array_and_object_surfaces_are_validated() {
        let diag = run(serde_json::json!({
            "channels": [
                {"server": "a", "userConfig": "bad"},
                {
                    "server": "b",
                    "userConfig": {
                        "nested_bad": {
                            "type": "bogus",
                            "title": 42,
                            "description": false,
                            "sensitive": "yes"
                        }
                    }
                }
            ]
        }));
        let reported = codes(&diag);
        assert!(reported.contains(&"U001"), "{reported:?}");
        assert!(reported.contains(&"U006"), "{reported:?}");
        assert!(reported.contains(&"U005"), "{reported:?}");
        assert!(reported.contains(&"U002"), "{reported:?}");
        assert!(reported.contains(&"U004"), "{reported:?}");

        let diag = run(serde_json::json!({
            "channels": {
                "alerts": {
                    "server": "a",
                    "userConfig": {
                        "ok": {
                            "type": "boolean",
                            "title": "On",
                            "description": "Enable"
                        }
                    }
                }
            }
        }));
        assert!(diag.diagnostics().is_empty());
    }

    #[test]
    fn non_object_entry_emits_u008_without_cascading_required_fields() {
        let diag = run(serde_json::json!({
            "userConfig": {
                "token": "not-an-object"
            }
        }));
        assert_eq!(codes(&diag), ["U008"]);
    }

    #[test]
    fn required_fields_and_trim_policy() {
        let diag = run(serde_json::json!({
            "userConfig": {
                "token": {
                    "type": "string",
                    "title": "  ",
                    "description": "\u{00a0}"
                }
            }
        }));
        let reported = codes(&diag);
        assert!(reported.contains(&"U005"), "{reported:?}");
        assert!(reported.contains(&"U002"), "{reported:?}");
        assert!(!reported.contains(&"U006"), "{reported:?}");
    }

    #[test]
    fn all_five_types_pass_and_invalid_type_is_u006() {
        for type_name in VALID_TYPES {
            let diag = run(serde_json::json!({
                "userConfig": {
                    "opt": {
                        "type": type_name,
                        "title": "T",
                        "description": "D"
                    }
                }
            }));
            assert!(
                diag.diagnostics().is_empty(),
                "type {type_name} should pass: {:?}",
                codes(&diag)
            );
        }

        let diag = run(serde_json::json!({
            "userConfig": {
                "opt": {"type": "enum", "title": "T", "description": "D"}
            }
        }));
        assert_eq!(codes(&diag), ["U006"]);
    }

    #[test]
    fn key_grammar_boundaries() {
        for key in ["a", "_a", "a0", "slackBotToken", "api_key"] {
            let diag = run(serde_json::json!({
                "userConfig": { key: valid_option() }
            }));
            assert!(
                !codes(&diag).contains(&"U007"),
                "key {key} should be accepted"
            );
        }
        for key in [
            "9lives",
            "hyphen-key",
            "dot.key",
            "has space",
            "with/slash",
            "café",
            "",
        ] {
            let diag = run(serde_json::json!({
                "userConfig": { key: valid_option() }
            }));
            assert!(
                codes(&diag).contains(&"U007"),
                "key {key:?} should be rejected"
            );
            assert_eq!(diag.diagnostics()[0].severity, Severity::Error);
        }
    }

    #[test]
    fn u008_unknown_fields_and_shapes() {
        let diag = run(serde_json::json!({
            "userConfig": {
                "opt": {
                    "type": "string",
                    "title": "T",
                    "description": "D",
                    "required": "yes",
                    "multiple": 1,
                    "extra": true,
                    "default": {"nested": true}
                }
            }
        }));
        let reported = codes(&diag);
        assert_eq!(reported.iter().filter(|c| **c == "U008").count(), 4);
    }

    #[test]
    fn u008_semantic_combinations() {
        let diag = run(serde_json::json!({
            "userConfig": {
                "opt": {
                    "type": "number",
                    "title": "T",
                    "description": "D",
                    "multiple": true,
                    "min": 5,
                    "max": 1,
                    "default": "nope"
                }
            }
        }));
        let u008 = diag
            .diagnostics()
            .iter()
            .filter(|d| d.rule.code() == "U008")
            .count();
        assert_eq!(u008, 3, "{:?}", codes(&diag));

        let diag = run(serde_json::json!({
            "userConfig": {
                "opt": {
                    "type": "string",
                    "title": "T",
                    "description": "D",
                    "multiple": true,
                    "default": ["a", "b"]
                }
            }
        }));
        assert!(diag.diagnostics().is_empty(), "{:?}", codes(&diag));
    }

    #[test]
    fn u008_multiple_boolean_is_string_only_across_all_userconfig_surfaces() {
        for type_name in VALID_TYPES {
            for multiple in [true, false] {
                let option = serde_json::json!({
                    "type": type_name,
                    "title": "T",
                    "description": "D",
                    "multiple": multiple,
                });
                let surfaces = [
                    (
                        "top-level",
                        serde_json::json!({"userConfig": {"opt": option}}),
                        "/userConfig/opt/multiple",
                    ),
                    (
                        "array channel",
                        serde_json::json!({"channels": [{"userConfig": {"opt": option}}]}),
                        "/channels/0/userConfig/opt/multiple",
                    ),
                    (
                        "object channel",
                        serde_json::json!({"channels": {"alerts": {"userConfig": {"opt": option}}}}),
                        "/channels/alerts/userConfig/opt/multiple",
                    ),
                ];
                for (surface, value, pointer) in surfaces {
                    let diag = run(value);
                    let u008: Vec<_> = diag
                        .diagnostics()
                        .iter()
                        .filter(|diagnostic| diagnostic.rule == LintRule::UserconfigOptionInvalid)
                        .collect();
                    if *type_name == "string" {
                        assert!(
                            u008.is_empty(),
                            "{surface} {type_name} {multiple}: {u008:?}"
                        );
                    } else {
                        assert_eq!(u008.len(), 1, "{surface} {type_name} {multiple}: {u008:?}");
                        assert_eq!(u008[0].evidence.as_deref(), Some(pointer));
                    }
                }
            }
        }
    }

    #[test]
    fn invalid_min_shape_does_not_also_emit_type_combination_u008() {
        let diag = run(serde_json::json!({
            "userConfig": {
                "opt": {
                    "type": "string",
                    "title": "T",
                    "description": "D",
                    "min": "1"
                }
            }
        }));
        assert_eq!(codes(&diag), ["U008"]);
        assert!(
            diag.diagnostics()[0]
                .message
                .contains("must be a finite JSON number"),
            "{}",
            diag.diagnostics()[0].message
        );
    }

    #[test]
    fn type_dependent_semantics_skip_when_type_invalid() {
        let diag = run(serde_json::json!({
            "userConfig": {
                "opt": {
                    "type": "enum",
                    "title": "T",
                    "description": "D",
                    "multiple": true,
                    "min": 1,
                    "default": true
                }
            }
        }));
        assert_eq!(codes(&diag), ["U006"]);
    }

    #[test]
    fn never_exposes_default_values_in_diagnostics() {
        let diag = run(serde_json::json!({
            "userConfig": {
                "opt": {
                    "type": "number",
                    "title": "T",
                    "description": "D",
                    "default": "secret-default-value"
                }
            }
        }));
        let text = format!("{:?}", diag.diagnostics());
        assert!(!text.contains("secret-default-value"), "{text}");
    }

    #[test]
    fn reproduction_manifest_diagnoses_schema_errors() {
        let diag = run(serde_json::json!({
            "name": "p",
            "userConfig": {
                "hyphen-key": {"type": "enum", "title": "T", "description": "D"}
            },
            "channels": [{
                "server": "slack",
                "userConfig": {
                    "nested-bad": {
                        "type": "bogus",
                        "title": 42,
                        "description": false,
                        "sensitive": "yes"
                    }
                }
            }],
            "mcpServers": {"slack": {"command": "slack-server"}}
        }));
        let reported = codes(&diag);
        assert!(reported.contains(&"U007"), "{reported:?}");
        assert!(reported.contains(&"U006"), "{reported:?}");
        assert!(reported.contains(&"U005"), "{reported:?}");
        assert!(reported.contains(&"U002"), "{reported:?}");
        assert!(reported.contains(&"U004"), "{reported:?}");
        assert!(!reported.contains(&"U003"), "{reported:?}");
    }

    fn u009_count(diag: &DiagnosticCollector) -> usize {
        diag.diagnostics()
            .iter()
            .filter(|d| d.rule.code() == "U009")
            .count()
    }

    #[test]
    fn u009_sensitive_true_with_any_default_shape_fires_once() {
        for default in [
            serde_json::json!("value"),
            serde_json::json!(""),
            serde_json::json!(5),
            serde_json::json!(true),
            serde_json::json!(["a", "b"]),
        ] {
            let diag = run(serde_json::json!({
                "userConfig": {
                    "opt": {
                        "type": "string",
                        "title": "T",
                        "description": "D",
                        "sensitive": true,
                        "default": default
                    }
                }
            }));
            assert_eq!(
                u009_count(&diag),
                1,
                "default {default:?}: {:?}",
                codes(&diag)
            );
        }
    }

    #[test]
    fn u009_non_sensitive_benign_defaults_are_clean() {
        for opt in [
            serde_json::json!({"type": "string", "title": "T", "description": "D", "default": "plain-value"}),
            serde_json::json!({"type": "string", "title": "T", "description": "D", "sensitive": false, "default": "plain-value"}),
            serde_json::json!({"type": "number", "title": "T", "description": "D", "default": 3}),
            serde_json::json!({"type": "string", "title": "T", "description": "D", "multiple": true, "default": ["alpha", "beta"]}),
        ] {
            let diag = run(serde_json::json!({"userConfig": {"opt": opt}}));
            assert!(!codes(&diag).contains(&"U009"), "{:?}", codes(&diag));
        }
    }

    #[test]
    fn u009_secret_shaped_default_fires_without_sensitive() {
        let diag = run(serde_json::json!({
            "userConfig": {
                "opt": {"type": "string", "title": "T", "description": "D", "default": "xoxb-1abcdefghij"}
            }
        }));
        assert_eq!(u009_count(&diag), 1, "{:?}", codes(&diag));

        // A string element inside an array default is enough (type string +
        // multiple:true keeps the shape U008-clean so only U009 fires).
        let diag = run(serde_json::json!({
            "userConfig": {
                "opt": {
                    "type": "string", "title": "T", "description": "D", "multiple": true,
                    "default": ["ok", "xoxb-1abcdefghij"]
                }
            }
        }));
        assert_eq!(u009_count(&diag), 1, "{:?}", codes(&diag));
        assert!(!codes(&diag).contains(&"U008"), "{:?}", codes(&diag));
    }

    #[test]
    fn u009_non_boolean_sensitive_defers_to_u004_then_still_checks_signature() {
        // Benign default: non-boolean sensitive is U004's; U009 does not cascade.
        let diag = run(serde_json::json!({
            "userConfig": {
                "opt": {"type": "string", "title": "T", "description": "D", "sensitive": "yes", "default": "plain"}
            }
        }));
        assert!(codes(&diag).contains(&"U004"), "{:?}", codes(&diag));
        assert!(!codes(&diag).contains(&"U009"), "{:?}", codes(&diag));

        // Secret-shaped default: U004 (non-boolean) plus U009 branch (b).
        let diag = run(serde_json::json!({
            "userConfig": {
                "opt": {"type": "string", "title": "T", "description": "D", "sensitive": "yes", "default": "sk-abcdefghijklmnopqrstuvwxyz"}
            }
        }));
        assert!(codes(&diag).contains(&"U004"), "{:?}", codes(&diag));
        assert_eq!(u009_count(&diag), 1, "{:?}", codes(&diag));
    }

    #[test]
    fn u009_non_object_default_stays_u008_only() {
        let diag = run(serde_json::json!({
            "userConfig": {
                "opt": {"type": "string", "title": "T", "description": "D", "default": {"nested": true}}
            }
        }));
        assert!(codes(&diag).contains(&"U008"), "{:?}", codes(&diag));
        assert!(!codes(&diag).contains(&"U009"), "{:?}", codes(&diag));
    }

    #[test]
    fn u009_covers_both_channel_surfaces() {
        let secret_option = serde_json::json!({
            "type": "string", "title": "T", "description": "D", "sensitive": true, "default": "x"
        });

        let diag = run(serde_json::json!({
            "channels": [{"server": "slack", "userConfig": {"opt": secret_option}}]
        }));
        assert_eq!(u009_count(&diag), 1, "array form: {:?}", codes(&diag));

        let secret_option = serde_json::json!({
            "type": "string", "title": "T", "description": "D", "sensitive": true, "default": "x"
        });
        let diag = run(serde_json::json!({
            "channels": {"alerts": {"server": "slack", "userConfig": {"opt": secret_option}}}
        }));
        assert_eq!(u009_count(&diag), 1, "object form: {:?}", codes(&diag));
    }

    #[test]
    fn u009_reproduction_manifest_reports_pointer_and_message() {
        let diag = run(serde_json::json!({
            "userConfig": {
                "botToken": {
                    "type": "string",
                    "title": "Bot token",
                    "description": "Slack bot token",
                    "sensitive": true,
                    "default": "committed"
                }
            }
        }));
        let u009: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|d| d.rule.code() == "U009")
            .collect();
        assert_eq!(u009.len(), 1, "{:?}", codes(&diag));
        assert_eq!(
            u009[0].evidence.as_deref(),
            Some("/userConfig/botToken/default")
        );
        assert_eq!(
            u009[0].message,
            ".claude-plugin/plugin.json userConfig.botToken.default must not be declared for a sensitive option"
        );
    }

    #[test]
    fn u009_never_exposes_the_default_value() {
        let secret = "xoxb-1supersecretliteral";
        let diag = run(serde_json::json!({
            "userConfig": {
                "opt": {"type": "string", "title": "T", "description": "D", "default": secret}
            }
        }));
        assert_eq!(u009_count(&diag), 1, "{:?}", codes(&diag));
        let text = format!("{:?}", diag.diagnostics());
        assert!(!text.contains(secret), "{text}");
        assert!(!text.contains("supersecret"), "{text}");
    }
}
