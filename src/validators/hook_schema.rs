//! Hook object schema validation (H008-H024).
//!
//! One validation engine for a "hook object", applied to every JSON surface
//! where hooks appear: `hooks/hooks.json`, `.claude/settings.json`, and
//! `.claude/settings.local.json`.
//!
//! The event list and handler-type table below are the single source of truth
//! for hook schema knowledge and are expected to churn with Claude Code
//! releases — update them here and nowhere else.
//!
//! Source: Claude Code hooks reference (https://code.claude.com/docs/en/hooks.md),
//! retrieved 2026-07-16.
//!
//! Surface shape walked by this engine:
//!
//! ```json
//! {"hooks": {"<EventName>": [{"matcher": "<pat>", "hooks": [{"type": "command"}]}]}}
//! ```
//!
//! `hooks` object -> event name key -> array of matcher groups -> each group's
//! nested `hooks` array -> hook objects (handlers). The enclosing key supplies
//! the event context that H008/H009 need.

use crate::diagnostic::DiagnosticCollector;
use crate::rules::LintRule;
use crate::validators::common::VALID_SHELLS;
use regex::Regex;
use serde_json::{Map, Value};
use std::sync::LazyLock;

/// Recognized hook event names (case-sensitive).
const VALID_EVENTS: &[&str] = &[
    "SessionStart",
    "Setup",
    "SessionEnd",
    "UserPromptSubmit",
    "UserPromptExpansion",
    "Stop",
    "StopFailure",
    "PreToolUse",
    "PermissionRequest",
    "PermissionDenied",
    "PostToolUse",
    "PostToolUseFailure",
    "PostToolBatch",
    "PreCompact",
    "PostCompact",
    "SubagentStart",
    "SubagentStop",
    "TeammateIdle",
    "TaskCreated",
    "TaskCompleted",
    "Notification",
    "CwdChanged",
    "FileChanged",
    "MessageDisplay",
    "InstructionsLoaded",
    "ConfigChange",
    "Elicitation",
    "ElicitationResult",
    "WorktreeCreate",
    "WorktreeRemove",
];

/// Events that take no `matcher`. Only these fire H009.
///
/// This is deliberately an explicit allowlist rather than a "non-tool event"
/// test: several non-tool events take documented matchers (SessionStart,
/// Setup, Notification, SubagentStart, ConfigChange, FileChanged, StopFailure),
/// so a blanket non-tool check would produce false positives on valid configs.
const NO_MATCHER_EVENTS: &[&str] = &[
    "Stop",
    "SessionEnd",
    "UserPromptSubmit",
    "UserPromptExpansion",
    "PreCompact",
    "PostCompact",
    "SubagentStop",
    "TaskCreated",
    "TaskCompleted",
    "Elicitation",
    "ElicitationResult",
    "CwdChanged",
    "MessageDisplay",
    "InstructionsLoaded",
    "WorktreeCreate",
    "WorktreeRemove",
];

/// Recognized handler types (H011).
const VALID_HOOK_TYPES: &[&str] = &["command", "prompt", "agent", "http", "mcp_tool"];

/// H023: destructive patterns in hook commands.
static DANGEROUS_COMMAND_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        // rm with recursive+force flags in either order (-rf, -fr, -Rf, -vrf)
        Regex::new(r"\brm\s+-[a-zA-Z]*([rR][a-zA-Z]*f|f[a-zA-Z]*[rR])").unwrap(),
        Regex::new(r"\bgit\s+reset\s+--hard\b").unwrap(),
        Regex::new(r"\bgit\s+clean\s+-[a-zA-Z]*f").unwrap(),
        // curl/wget piped straight into a shell
        Regex::new(r"\b(curl|wget)\b[^|]*\|\s*(sudo\s+)?(ba|z|k)?sh\b").unwrap(),
    ]
});

/// H024: `$VAR` / `${VAR}` interpolation inside an HTTP header value.
static RE_ENV_INTERP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\$\{[A-Za-z_][A-Za-z0-9_]*\}|\$[A-Za-z_][A-Za-z0-9_]*").unwrap());

/// Validate every hook object reachable from `val`'s top-level `hooks` key.
///
/// Files whose `hooks` key is a legacy flat array carry no event context, so
/// event-dependent rules cannot be evaluated; such files are skipped entirely.
pub(super) fn validate_hook_schema(val: &Value, label: &str, diag: &mut DiagnosticCollector) {
    let events = match val.get("hooks").and_then(|h| h.as_object()) {
        Some(e) => e,
        None => return,
    };

    for (event, groups) in events {
        if !VALID_EVENTS.contains(&event.as_str()) {
            diag.report(
                LintRule::HookEventInvalid,
                &format!("{label}: unknown hook event '{event}'"),
            );
        }

        if let Some(groups) = groups.as_array() {
            for group in groups {
                validate_matcher_group(group, event, label, diag);
            }
        }
    }
}

/// Validate one matcher group: its `matcher` against the event, then each hook
/// object in its nested `hooks` array.
fn validate_matcher_group(group: &Value, event: &str, label: &str, diag: &mut DiagnosticCollector) {
    let group = match group.as_object() {
        Some(g) => g,
        None => return,
    };

    if group.contains_key("matcher") && NO_MATCHER_EVENTS.contains(&event) {
        diag.report(
            LintRule::HookMatcherInvalid,
            &format!("{label}: event '{event}' takes no 'matcher'"),
        );
    }

    if let Some(hooks) = group.get("hooks").and_then(|h| h.as_array()) {
        for hook in hooks {
            if let Some(hook) = hook.as_object() {
                validate_hook_object(hook, event, label, diag);
            }
        }
    }
}

/// Validate a single hook object (handler) against the schema.
fn validate_hook_object(
    hook: &Map<String, Value>,
    event: &str,
    label: &str,
    diag: &mut DiagnosticCollector,
) {
    let ctx = format!("{label}: {event} hook");

    // H010/H011: type identity gates every per-type check below.
    let hook_type = match hook.get("type") {
        None => {
            diag.report(
                LintRule::HookTypeMissing,
                &format!("{ctx} is missing required field 'type'"),
            );
            None
        }
        Some(Value::String(t)) if VALID_HOOK_TYPES.contains(&t.as_str()) => Some(t.as_str()),
        Some(other) => {
            diag.report(
                LintRule::HookTypeUnknown,
                &format!(
                    "{ctx} has unknown type {other}, must be one of {}",
                    VALID_HOOK_TYPES.join("/")
                ),
            );
            None
        }
    };

    // H012/H013/H014/H015/H016: per-type required fields.
    // `agent` has no required field.
    match hook_type {
        Some("command") => require_field(
            hook,
            "command",
            "command",
            &ctx,
            LintRule::HookCommandRequired,
            diag,
        ),
        Some("prompt") => require_field(
            hook,
            "prompt",
            "prompt",
            &ctx,
            LintRule::HookPromptRequired,
            diag,
        ),
        Some("http") => require_field(hook, "http", "url", &ctx, LintRule::HookUrlRequired, diag),
        Some("mcp_tool") => {
            require_field(
                hook,
                "mcp_tool",
                "server",
                &ctx,
                LintRule::HookServerRequired,
                diag,
            );
            require_field(
                hook,
                "mcp_tool",
                "tool",
                &ctx,
                LintRule::HookToolRequired,
                diag,
            );
        }
        _ => {}
    }

    // H017: timeout must be a positive integer.
    if let Some(timeout) = hook.get("timeout") {
        if timeout.as_u64().is_none_or(|t| t == 0) {
            diag.report(
                LintRule::HookTimeoutInvalid,
                &format!("{ctx} has 'timeout' {timeout}, must be a positive integer"),
            );
        }
    }

    // H018: async: true is only meaningful on command hooks. Skipped when the
    // type is missing or unknown — H010/H011 already covers that.
    if hook.get("async") == Some(&Value::Bool(true))
        && matches!(hook_type, Some(t) if t != "command")
    {
        diag.report(
            LintRule::HookAsyncInvalid,
            &format!("{ctx} sets 'async: true', which is only valid on type 'command'"),
        );
    }

    // H019: model is documented for prompt handlers only.
    if hook.contains_key("model") && matches!(hook_type, Some(t) if t != "prompt") {
        diag.report(
            LintRule::HookModelInvalid,
            &format!("{ctx} sets 'model', which is only valid on type 'prompt'"),
        );
    }

    // H020: once must be a boolean.
    if let Some(once) = hook.get("once") {
        if !once.is_boolean() {
            diag.report(
                LintRule::HookOnceInvalid,
                &format!("{ctx} has 'once' {once}, must be true or false"),
            );
        }
    }

    // H021: if must be a non-empty string.
    if let Some(cond) = hook.get("if") {
        if cond.as_str().is_none_or(|s| s.trim().is_empty()) {
            diag.report(
                LintRule::HookIfInvalid,
                &format!("{ctx} has 'if' {cond}, must be a non-empty string"),
            );
        }
    }

    // H022: shell must be one of the shared VALID_SHELLS (shared with S026).
    if let Some(shell) = hook.get("shell") {
        if !shell.as_str().is_some_and(|s| VALID_SHELLS.contains(&s)) {
            diag.report(
                LintRule::HookShellInvalid,
                &format!(
                    "{ctx} has 'shell' {shell}, must be {}",
                    VALID_SHELLS.join("/")
                ),
            );
        }
    }

    // H023: dangerous command patterns.
    if let Some(command) = hook.get("command").and_then(|c| c.as_str()) {
        for pattern in DANGEROUS_COMMAND_PATTERNS.iter() {
            if let Some(m) = pattern.find(command) {
                diag.report(
                    LintRule::HookCommandDangerous,
                    &format!("{ctx} command contains dangerous pattern '{}'", m.as_str()),
                );
                break;
            }
        }
    }

    // H024: interpolated HTTP headers need an allowedEnvVars declaration.
    if hook_type == Some("http") {
        check_http_headers(hook, &ctx, diag);
    }
}

/// Report `rule` unless `field` is present with a non-empty string value.
fn require_field(
    hook: &Map<String, Value>,
    type_name: &str,
    field: &str,
    ctx: &str,
    rule: LintRule,
    diag: &mut DiagnosticCollector,
) {
    let present = hook
        .get(field)
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.trim().is_empty());
    if !present {
        diag.report(
            rule,
            &format!("{ctx} of type '{type_name}' requires a non-empty '{field}'"),
        );
    }
}

/// H024: an HTTP hook whose headers interpolate `$VAR` must declare
/// `allowedEnvVars`, otherwise the variable silently resolves to nothing.
fn check_http_headers(hook: &Map<String, Value>, ctx: &str, diag: &mut DiagnosticCollector) {
    let headers = match hook.get("headers").and_then(|h| h.as_object()) {
        Some(h) => h,
        None => return,
    };

    let declared = hook
        .get("allowedEnvVars")
        .and_then(|v| v.as_array())
        .is_some_and(|a| !a.is_empty());
    if declared {
        return;
    }

    for (name, value) in headers {
        if let Some(found) = value.as_str().and_then(|v| RE_ENV_INTERP.find(v)) {
            diag.report(
                LintRule::HookHeadersInterpolated,
                &format!(
                    "{ctx} header '{name}' interpolates '{}' but 'allowedEnvVars' is not declared",
                    found.as_str()
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Run the engine over `val` with every rule promoted to error.
    fn check(val: Value) -> Vec<String> {
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_hook_schema(&val, "test", &mut diag);
        diag.errors()
    }

    /// Wrap a single hook object in the canonical event-keyed shape.
    fn wrap(event: &str, hook: Value) -> Value {
        json!({"hooks": {event: [{"hooks": [hook]}]}})
    }

    // ── Shape handling ──────────────────────────────────────────────

    #[test]
    fn legacy_array_shape_is_skipped_entirely() {
        // The shape H001-H007 model. No event context, so the engine must not
        // fire — notably not H010 on this fixture, which has no 'type'.
        let errors = check(json!({"hooks": [{"command": "echo test"}]}));
        assert!(
            errors.is_empty(),
            "legacy array must be skipped: {errors:?}"
        );
    }

    #[test]
    fn missing_hooks_key_is_skipped() {
        assert!(check(json!({"permissions": {}})).is_empty());
    }

    #[test]
    fn valid_event_keyed_config_passes() {
        let errors = check(json!({
            "hooks": {
                "PreToolUse": [{
                    "matcher": "Bash",
                    "hooks": [{"type": "command", "command": "echo hi", "timeout": 30}]
                }]
            }
        }));
        assert!(errors.is_empty(), "valid config must pass: {errors:?}");
    }

    #[test]
    fn malformed_inner_shapes_do_not_panic() {
        assert!(check(json!({"hooks": {"PreToolUse": "not-an-array"}})).is_empty());
        assert!(check(json!({"hooks": {"PreToolUse": ["not-an-object"]}})).is_empty());
        assert!(check(json!({"hooks": {"PreToolUse": [{"hooks": "nope"}]}})).is_empty());
    }

    // ── H008: event names ───────────────────────────────────────────

    #[test]
    fn h008_unknown_event_fires() {
        let errors = check(json!({"hooks": {"PreToolUsage": []}}));
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("unknown hook event 'PreToolUsage'"));
    }

    #[test]
    fn h008_is_case_sensitive() {
        let errors = check(json!({"hooks": {"pretooluse": []}}));
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("unknown hook event"));
    }

    #[test]
    fn h008_accepts_every_documented_event() {
        for event in VALID_EVENTS {
            let errors = check(json!({"hooks": {*event: []}}));
            assert!(errors.is_empty(), "{event} must be valid: {errors:?}");
        }
    }

    // ── H009: matcher placement ─────────────────────────────────────

    #[test]
    fn h009_matcher_on_no_matcher_event_fires() {
        let errors = check(json!({"hooks": {"Stop": [{"matcher": "Bash", "hooks": []}]}}));
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("event 'Stop' takes no 'matcher'"));
    }

    #[test]
    fn h009_fires_for_every_no_matcher_event() {
        for event in NO_MATCHER_EVENTS {
            let errors = check(json!({"hooks": {*event: [{"matcher": "x", "hooks": []}]}}));
            assert_eq!(errors.len(), 1, "{event} must reject a matcher");
        }
    }

    #[test]
    fn h009_does_not_fire_on_events_that_take_matchers() {
        // Regression guard: a blanket "non-tool event" check would false-positive
        // on every one of these. They all take documented matchers.
        for event in [
            "PreToolUse",
            "PostToolUse",
            "PostToolUseFailure",
            "PostToolBatch",
            "PermissionRequest",
            "PermissionDenied",
            "SessionStart",
            "Setup",
            "Notification",
            "SubagentStart",
            "ConfigChange",
            "FileChanged",
            "StopFailure",
        ] {
            let errors = check(json!({"hooks": {event: [{"matcher": "x", "hooks": []}]}}));
            assert!(
                errors.is_empty(),
                "{event} must accept a matcher: {errors:?}"
            );
        }
    }

    #[test]
    fn h009_absent_matcher_never_fires() {
        assert!(check(json!({"hooks": {"Stop": [{"hooks": []}]}})).is_empty());
    }

    // ── H010/H011: type identity ────────────────────────────────────

    #[test]
    fn h010_missing_type_fires() {
        let errors = check(wrap("PreToolUse", json!({"command": "echo hi"})));
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("missing required field 'type'"));
    }

    #[test]
    fn h011_unknown_type_fires() {
        let errors = check(wrap("PreToolUse", json!({"type": "bogus"})));
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("unknown type"));
    }

    #[test]
    fn h011_non_string_type_fires() {
        let errors = check(wrap("PreToolUse", json!({"type": 42})));
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("unknown type"));
    }

    #[test]
    fn h011_accepts_every_valid_type() {
        let complete = [
            json!({"type": "command", "command": "echo hi"}),
            json!({"type": "prompt", "prompt": "do it"}),
            json!({"type": "agent"}),
            json!({"type": "http", "url": "https://example.com"}),
            json!({"type": "mcp_tool", "server": "s", "tool": "t"}),
        ];
        for hook in complete {
            let errors = check(wrap("PreToolUse", hook.clone()));
            assert!(errors.is_empty(), "{hook} must pass: {errors:?}");
        }
    }

    // ── H012-H016: per-type required fields ─────────────────────────

    #[test]
    fn h012_command_without_command_fires() {
        let errors = check(wrap("PreToolUse", json!({"type": "command"})));
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("requires a non-empty 'command'"));
    }

    #[test]
    fn h013_prompt_without_prompt_fires() {
        let errors = check(wrap("PreToolUse", json!({"type": "prompt"})));
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("requires a non-empty 'prompt'"));
    }

    #[test]
    fn h014_http_without_url_fires() {
        let errors = check(wrap("PreToolUse", json!({"type": "http"})));
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("requires a non-empty 'url'"));
    }

    #[test]
    fn h015_h016_mcp_tool_without_server_or_tool_fires() {
        let errors = check(wrap("PreToolUse", json!({"type": "mcp_tool"})));
        assert_eq!(errors.len(), 2);
        assert!(errors.iter().any(|e| e.contains("'server'")));
        assert!(errors.iter().any(|e| e.contains("'tool'")));
    }

    #[test]
    fn h015_mcp_tool_with_empty_server_fires() {
        let errors = check(wrap(
            "PreToolUse",
            json!({"type": "mcp_tool", "server": "  ", "tool": "t"}),
        ));
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("'server'"));
    }

    #[test]
    fn agent_type_requires_no_field() {
        assert!(check(wrap("PreToolUse", json!({"type": "agent"}))).is_empty());
    }

    // ── H017: timeout ───────────────────────────────────────────────

    #[test]
    fn h017_rejects_negative_zero_float_and_string() {
        for bad in [json!(-5), json!(0), json!(1.5), json!("30")] {
            let errors = check(wrap(
                "PreToolUse",
                json!({"type": "command", "command": "x", "timeout": bad}),
            ));
            assert_eq!(errors.len(), 1, "timeout {bad} must fire");
            assert!(errors[0].contains("must be a positive integer"));
        }
    }

    #[test]
    fn h017_accepts_positive_integer() {
        let errors = check(wrap(
            "PreToolUse",
            json!({"type": "command", "command": "x", "timeout": 600}),
        ));
        assert!(errors.is_empty(), "{errors:?}");
    }

    // ── H018/H019: type-restricted fields ───────────────────────────

    #[test]
    fn h018_async_true_on_prompt_fires() {
        let errors = check(wrap(
            "PreToolUse",
            json!({"type": "prompt", "prompt": "p", "async": true}),
        ));
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("'async: true'"));
    }

    #[test]
    fn h018_async_true_on_command_passes() {
        let errors = check(wrap(
            "PreToolUse",
            json!({"type": "command", "command": "x", "async": true}),
        ));
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn h018_async_false_on_prompt_passes() {
        let errors = check(wrap(
            "PreToolUse",
            json!({"type": "prompt", "prompt": "p", "async": false}),
        ));
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn h019_model_on_prompt_passes_but_fires_elsewhere() {
        let ok = check(wrap(
            "PreToolUse",
            json!({"type": "prompt", "prompt": "p", "model": "sonnet"}),
        ));
        assert!(ok.is_empty(), "model on prompt must pass: {ok:?}");

        // Narrowed per the docs: 'agent' does NOT accept model.
        let errors = check(wrap(
            "PreToolUse",
            json!({"type": "agent", "model": "sonnet"}),
        ));
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("'model'"));
    }

    // ── H020/H021/H022: field typing ────────────────────────────────

    #[test]
    fn h020_non_boolean_once_fires() {
        let errors = check(wrap(
            "PreToolUse",
            json!({"type": "command", "command": "x", "once": "true"}),
        ));
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("'once'"));
    }

    #[test]
    fn h020_boolean_once_passes() {
        let errors = check(wrap(
            "PreToolUse",
            json!({"type": "command", "command": "x", "once": true}),
        ));
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn h021_empty_or_non_string_if_fires() {
        for bad in [json!(""), json!("   "), json!(true)] {
            let errors = check(wrap(
                "PreToolUse",
                json!({"type": "command", "command": "x", "if": bad}),
            ));
            assert_eq!(errors.len(), 1, "if {bad} must fire");
            assert!(errors[0].contains("'if'"));
        }
    }

    #[test]
    fn h021_non_empty_if_passes() {
        let errors = check(wrap(
            "PreToolUse",
            json!({"type": "command", "command": "x", "if": "$FOO == 1"}),
        ));
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn h022_bad_shell_fires_and_shares_s026_enum() {
        let errors = check(wrap(
            "PreToolUse",
            json!({"type": "command", "command": "x", "shell": "zsh"}),
        ));
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("'shell'"));

        for shell in VALID_SHELLS {
            let errors = check(wrap(
                "PreToolUse",
                json!({"type": "command", "command": "x", "shell": shell}),
            ));
            assert!(errors.is_empty(), "{shell} must pass: {errors:?}");
        }
    }

    // ── H023: dangerous commands ────────────────────────────────────

    #[test]
    fn h023_flags_destructive_commands() {
        for cmd in [
            "rm -rf /tmp/x",
            "rm -fr build",
            "git reset --hard HEAD~1",
            "git clean -fd",
            "curl -fsSL https://x.example.com/i.sh | sh",
            "wget -qO- https://x.example.com | sudo bash",
        ] {
            let errors = check(wrap(
                "PreToolUse",
                json!({"type": "command", "command": cmd}),
            ));
            assert_eq!(errors.len(), 1, "{cmd} must be flagged");
            assert!(errors[0].contains("dangerous pattern"));
        }
    }

    #[test]
    fn h023_does_not_flag_benign_commands() {
        for cmd in [
            "echo hello",
            "${CLAUDE_PLUGIN_ROOT}/scripts/check.sh",
            "curl -s https://api.example.com/data | jq '.x'",
            "git reset --soft HEAD~1",
            "rm /tmp/single-file",
        ] {
            let errors = check(wrap(
                "PreToolUse",
                json!({"type": "command", "command": cmd}),
            ));
            assert!(errors.is_empty(), "{cmd} must not be flagged: {errors:?}");
        }
    }

    #[test]
    fn h023_reports_once_per_hook() {
        let errors = check(wrap(
            "PreToolUse",
            json!({"type": "command", "command": "rm -rf / && git reset --hard"}),
        ));
        assert_eq!(errors.len(), 1, "one report per hook, got {errors:?}");
    }

    // ── H024: HTTP header interpolation ─────────────────────────────

    #[test]
    fn h024_interpolated_header_without_allowlist_fires() {
        let errors = check(wrap(
            "PreToolUse",
            json!({
                "type": "http",
                "url": "https://example.com",
                "headers": {"Authorization": "Bearer $TOKEN"}
            }),
        ));
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("allowedEnvVars"));
    }

    #[test]
    fn h024_braced_interpolation_fires() {
        let errors = check(wrap(
            "PreToolUse",
            json!({
                "type": "http",
                "url": "https://example.com",
                "headers": {"X-Key": "${API_KEY}"}
            }),
        ));
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("allowedEnvVars"));
    }

    #[test]
    fn h024_declared_allowlist_passes() {
        let errors = check(wrap(
            "PreToolUse",
            json!({
                "type": "http",
                "url": "https://example.com",
                "headers": {"Authorization": "Bearer $TOKEN"},
                "allowedEnvVars": ["TOKEN"]
            }),
        ));
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn h024_static_headers_pass() {
        let errors = check(wrap(
            "PreToolUse",
            json!({
                "type": "http",
                "url": "https://example.com",
                "headers": {"Content-Type": "application/json"}
            }),
        ));
        assert!(errors.is_empty(), "{errors:?}");
    }

    // ── Aggregation ─────────────────────────────────────────────────

    #[test]
    fn multiple_events_and_groups_all_validated() {
        let errors = check(json!({
            "hooks": {
                "PreToolUse": [{"hooks": [{"type": "command"}]}],
                "Stop": [{"matcher": "x", "hooks": [{"type": "bogus"}]}]
            }
        }));
        assert_eq!(
            errors.len(),
            3,
            "expected H012 + H009 + H011, got {errors:?}"
        );
    }
}
