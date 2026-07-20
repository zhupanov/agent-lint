//! Validation for an optional project `.codex/config.toml`.

use crate::config::ExcludeSet;
use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata, SourceSpan};
use crate::rules::LintRule;
use crate::sensitive::contains_possible_secret;
use crate::validators::codex_constants::*;
use toml::Value;
use toml::value::Table;

const CONFIG_PATH: &str = ".codex/config.toml";

/// Validate project-local Codex TOML. The configuration is optional.
pub fn validate_config(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    if exclude.is_excluded(CONFIG_PATH) {
        return;
    }
    let Ok(content) = std::fs::read_to_string(CONFIG_PATH) else {
        return;
    };
    let value: Value = match content.parse() {
        Ok(value) => value,
        Err(error) => {
            let metadata = error
                .span()
                .and_then(|span| SourceSpan::from_byte_range(&content, span))
                .map(|location| DiagnosticMetadata::default().with_location(location))
                .unwrap_or_default();
            diag.report_at_with(
                LintRule::CodexTomlInvalid,
                CONFIG_PATH,
                &format!("{CONFIG_PATH} is not valid TOML: {error}"),
                metadata,
            );
            return;
        }
    };
    let Some(root) = value.as_table() else { return };
    validate_unknown_keys(
        diag,
        CONFIG_PATH,
        root,
        TOP_LEVEL_KEYS,
        LintRule::CodexTopLevelKey,
    );
    validate_project_docs(diag, root);
    validate_scalar_enums(diag, root);
    validate_types(diag, root);
    validate_nested(diag, root);
}

fn validate_project_docs(diag: &mut DiagnosticCollector, root: &Table) {
    if let Some(value) = root.get("project_doc_max_bytes")
        && (!value.is_integer()
            || value
                .as_integer()
                .is_none_or(|n| !(1..=65_536).contains(&n)))
    {
        diag.report(LintRule::CodexProjectDocMaxBytes, &format!("{CONFIG_PATH}: 'project_doc_max_bytes' must be a positive integer no greater than 65536"));
    }
    if let Some(value) = root.get("project_doc_fallback_filenames") {
        let valid = value.as_array().is_some_and(|items| {
            let mut seen = std::collections::HashSet::new();
            items.iter().all(|item| {
                item.as_str().is_some_and(|name| {
                    !name.trim().is_empty() && !name.contains(['/', '\\']) && seen.insert(name)
                })
            })
        });
        if !valid {
            diag.report(LintRule::CodexProjectDocFallbackNames, &format!("{CONFIG_PATH}: 'project_doc_fallback_filenames' must be an array of unique, non-empty bare filenames"));
        }
    }
}

fn validate_scalar_enums(diag: &mut DiagnosticCollector, root: &Table) {
    for (key, allowed, rule) in [
        (
            "approval_policy",
            APPROVAL_POLICIES,
            LintRule::CodexApprovalPolicy,
        ),
        ("sandbox_mode", SANDBOX_MODES, LintRule::CodexSandboxMode),
        (
            "model_reasoning_effort",
            REASONING_EFFORTS,
            LintRule::CodexReasoningEffort,
        ),
        (
            "model_verbosity",
            VERBOSITIES,
            LintRule::CodexModelVerbosity,
        ),
        ("personality", PERSONALITIES, LintRule::CodexPersonality),
        (
            "cli_auth_credentials_store",
            CREDENTIAL_STORES,
            LintRule::CodexCliCredentialsStore,
        ),
        (
            "mcp_oauth_credentials_store",
            MCP_CREDENTIAL_STORES,
            LintRule::CodexMcpCredentialsStore,
        ),
        (
            "model_reasoning_summary",
            REASONING_SUMMARIES,
            LintRule::CodexReasoningSummary,
        ),
        (
            "approvals_reviewer",
            APPROVAL_REVIEWERS,
            LintRule::CodexApprovalsReviewer,
        ),
        ("service_tier", SERVICE_TIERS, LintRule::CodexServiceTier),
    ] {
        if let Some(value) = root.get(key)
            && !(key == "approval_policy" && value.is_table())
            && !value.as_str().is_some_and(|value| allowed.contains(&value))
        {
            diag.report(
                rule,
                &format!(
                    "{CONFIG_PATH}: '{key}' must be one of: {}",
                    allowed.join(", ")
                ),
            );
        }
    }
    if root.get("sandbox_mode").and_then(Value::as_str) == Some("danger-full-access")
        && root
            .get("notice")
            .and_then(Value::as_table)
            .and_then(|notice| notice.get("hide_full_access_warning"))
            .and_then(Value::as_bool)
            != Some(true)
    {
        diag.report(
            LintRule::CodexFullAccessAcknowledgment,
            &format!(
                "{CONFIG_PATH}: danger-full-access requires notice.hide_full_access_warning = true"
            ),
        );
    }
}

fn validate_types(diag: &mut DiagnosticCollector, root: &Table) {
    for (key, rule) in [
        ("model", LintRule::CodexModelType),
        ("model_provider", LintRule::CodexModelProviderType),
        ("file_opener", LintRule::CodexFileOpenerType),
        ("profile", LintRule::CodexProfileType),
    ] {
        if let Some(value) = root.get(key)
            && !value.is_str()
        {
            diag.report(rule, &format!("{CONFIG_PATH}: '{key}' must be a string"));
        }
    }
    for (key, rule) in [
        ("history", LintRule::CodexHistoryType),
        ("tui", LintRule::CodexTuiType),
        ("skills", LintRule::CodexSkillsType),
    ] {
        if let Some(value) = root.get(key)
            && !value.is_table()
        {
            diag.report(
                rule,
                &format!("{CONFIG_PATH}: '{key}' must be a TOML table"),
            );
        }
    }
    for (key, rule) in [
        ("model_context_window", LintRule::CodexContextWindow),
        (
            "model_auto_compact_token_limit",
            LintRule::CodexAutoCompactLimit,
        ),
    ] {
        if let Some(value) = root.get(key)
            && (!value.is_integer() || value.as_integer().is_none_or(|number| number <= 0))
        {
            diag.report(
                rule,
                &format!("{CONFIG_PATH}: '{key}' must be a positive integer"),
            );
        }
    }
}

fn validate_nested(diag: &mut DiagnosticCollector, root: &Table) {
    if let Some(table) = root.get("features").and_then(Value::as_table) {
        validate_unknown_keys(
            diag,
            &format!("{CONFIG_PATH} [features]"),
            table,
            FEATURE_KEYS,
            LintRule::CodexFeatureKey,
        );
    }
    if let Some(table) = root.get("tui").and_then(Value::as_table) {
        validate_unknown_keys(
            diag,
            &format!("{CONFIG_PATH} [tui]"),
            table,
            TUI_KEYS,
            LintRule::CodexUnknownNestedKey,
        );
    }
    if let Some(table) = root
        .get("shell_environment_policy")
        .and_then(Value::as_table)
    {
        validate_unknown_keys(
            diag,
            &format!("{CONFIG_PATH} [shell_environment_policy]"),
            table,
            SHELL_POLICY_KEYS,
            LintRule::CodexUnknownNestedKey,
        );
        if let Some(value) = table.get("inherit")
            && !value
                .as_str()
                .is_some_and(|value| SHELL_INHERIT_VALUES.contains(&value))
        {
            diag.report(
                LintRule::CodexShellEnvironmentInherit,
                &format!(
                    "{CONFIG_PATH}: shell_environment_policy.inherit must be one of: {}",
                    SHELL_INHERIT_VALUES.join(", ")
                ),
            );
        }
    }
    if let Some(table) = root
        .get("sandbox_workspace_write")
        .and_then(Value::as_table)
        && let Some(value) = table.get("mode")
        && !value
            .as_str()
            .is_some_and(|value| WORKSPACE_WRITE_MODES.contains(&value))
    {
        diag.report(
            LintRule::CodexWorkspaceWriteMode,
            &format!(
                "{CONFIG_PATH}: sandbox_workspace_write.mode must be one of: {}",
                WORKSPACE_WRITE_MODES.join(", ")
            ),
        );
    }
    validate_mcp_servers(diag, root.get("mcp_servers"));
    validate_apps(diag, root.get("apps"));
    validate_approval_policy(diag, root.get("approval_policy"));
    validate_agent_thread_limit(diag, root);
    validate_suppressed_permissions(diag, root.get("permissions"));
    validate_suppressed_windows(diag, root.get("windows"));
}

fn validate_mcp_servers(diag: &mut DiagnosticCollector, value: Option<&Value>) {
    let Some(servers) = value.and_then(Value::as_table) else {
        return;
    };
    for (name, value) in servers {
        let label = format!("{CONFIG_PATH}: mcp_servers.{name}");
        let Some(server) = value.as_table() else {
            diag.report(
                LintRule::CodexMcpServerTransport,
                &format!("{label} must be an object with 'command' or 'url'"),
            );
            continue;
        };
        validate_unknown_keys(
            diag,
            &label,
            server,
            MCP_SERVER_KEYS,
            LintRule::CodexUnknownNestedKey,
        );
        if !server.get("command").is_some_and(Value::is_str)
            && !server.get("url").is_some_and(Value::is_str)
        {
            diag.report(
                LintRule::CodexMcpServerTransport,
                &format!("{label} must define a string 'command' or 'url'"),
            );
        }
        if server.contains_key("bearer_token") {
            diag.report(
                LintRule::CodexInlineBearerToken,
                &format!("{label}.bearer_token is forbidden; use bearer_token_env_var"),
            );
        }
        if table_contains_secret(server) {
            diag.report(
                LintRule::CodexHardcodedSecret,
                &format!("{label} contains a potential hardcoded secret"),
            );
        }
    }
}

fn validate_apps(diag: &mut DiagnosticCollector, value: Option<&Value>) {
    let Some(apps) = value.and_then(Value::as_table) else {
        return;
    };
    for (name, value) in apps {
        let label = format!("{CONFIG_PATH}: apps.{name}");
        let Some(app) = value.as_table() else {
            diag.report(
                LintRule::CodexUnknownNestedKey,
                &format!("{label} must be a table"),
            );
            continue;
        };
        validate_unknown_keys(
            diag,
            &label,
            app,
            if name == "_default" {
                APP_DEFAULT_KEYS
            } else {
                APP_KEYS
            },
            LintRule::CodexUnknownNestedKey,
        );
        if let Some(mode) = app.get("default_tools_approval_mode")
            && !mode
                .as_str()
                .is_some_and(|mode| APP_APPROVAL_MODES.contains(&mode))
        {
            diag.report(
                LintRule::CodexAppApprovalMode,
                &format!(
                    "{label}.default_tools_approval_mode must be one of: {}",
                    APP_APPROVAL_MODES.join(", ")
                ),
            );
        }
    }
}

fn validate_approval_policy(diag: &mut DiagnosticCollector, value: Option<&Value>) {
    let Some(table) = value.and_then(Value::as_table) else {
        return;
    };
    const KEYS: &[&str] = &[
        "sandbox_approval",
        "rules",
        "mcp_elicitations",
        "request_permissions",
        "skill_approval",
        "granular",
    ];
    validate_unknown_keys(
        diag,
        &format!("{CONFIG_PATH} [approval_policy]"),
        table,
        KEYS,
        LintRule::CodexApprovalPolicyField,
    );
}

fn validate_agent_thread_limit(diag: &mut DiagnosticCollector, root: &Table) {
    let Some(agents) = root.get("agents").and_then(Value::as_table) else {
        return;
    };
    if !agents.contains_key("max_threads") {
        return;
    }
    let enabled = root
        .get("features")
        .and_then(Value::as_table)
        .and_then(|features| features.get("multi_agent_v2"));
    if enabled.is_some_and(|value| {
        value.as_bool() == Some(true)
            || value
                .as_table()
                .is_some_and(|table| table.get("enabled").and_then(Value::as_bool) == Some(true))
    }) {
        diag.report(
            LintRule::CodexMultiAgentThreadLimit,
            &format!(
                "{CONFIG_PATH}: agents.max_threads cannot be set when multi_agent_v2 is enabled"
            ),
        );
    }
}

fn validate_unknown_keys(
    diag: &mut DiagnosticCollector,
    label: &str,
    table: &Table,
    allowed: &[&str],
    rule: LintRule,
) {
    for key in table.keys().filter(|key| !allowed.contains(&key.as_str())) {
        diag.report(rule, &format!("{label}: unknown key '{key}'"));
    }
}

fn validate_suppressed_permissions(diag: &mut DiagnosticCollector, value: Option<&Value>) {
    let Some(network) = value
        .and_then(Value::as_table)
        .and_then(|table| table.get("network"))
        .and_then(Value::as_table)
    else {
        return;
    };
    validate_unknown_keys(
        diag,
        &format!("{CONFIG_PATH} [permissions.network]"),
        network,
        NETWORK_PERMISSION_KEYS,
        LintRule::CodexNetworkPermissionField,
    );
}

fn validate_suppressed_windows(diag: &mut DiagnosticCollector, value: Option<&Value>) {
    if let Some(value) = value
        .and_then(Value::as_table)
        .and_then(|table| table.get("sandbox"))
        && !value
            .as_str()
            .is_some_and(|value| WINDOWS_SANDBOX_MODES.contains(&value))
    {
        diag.report(
            LintRule::CodexWindowsSandbox,
            &format!(
                "{CONFIG_PATH}: windows.sandbox must be one of: {}",
                WINDOWS_SANDBOX_MODES.join(", ")
            ),
        );
    }
}

fn table_contains_secret(table: &Table) -> bool {
    table.iter().any(|(key, value)| {
        contains_possible_secret(&format!("{key} = {value}")) || value_contains_secret(value)
    })
}

fn value_contains_secret(value: &Value) -> bool {
    match value {
        Value::String(value) => contains_possible_secret(value),
        Value::Array(values) => values.iter().any(value_contains_secret),
        Value::Table(values) => table_contains_secret(values),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_config(content: &str) -> DiagnosticCollector {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir(".codex").unwrap();
        std::fs::write(CONFIG_PATH, content).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_config(&mut diag, &ExcludeSet::default());
        diag
    }

    fn has(diag: &DiagnosticCollector, rule: LintRule) -> bool {
        diag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.rule == rule)
    }

    #[test]
    #[serial_test::serial]
    fn valid_config_is_clean() {
        let diag = with_config(
            "approval_policy = 'never'\nsandbox_mode = 'workspace-write'\nmodel_reasoning_effort = 'high'\nmodel_verbosity = 'medium'\npersonality = 'friendly'\ncli_auth_credentials_store = 'auto'\nmcp_oauth_credentials_store = 'keyring'\nmodel = 'gpt-5'\nmodel_provider = 'openai'\nmodel_reasoning_summary = 'auto'\nmodel_context_window = 1000\nmodel_auto_compact_token_limit = 500\nfile_opener = 'vscode'\nprofile = 'work'\nproject_doc_max_bytes = 1024\nproject_doc_fallback_filenames = ['TEAM.md']\n[history]\n[features]\nmulti_agent_v2 = false\n[tui]\n[shell_environment_policy]\ninherit = 'core'\n[skills]\n[apps.example]\ndefault_tools_approval_mode = 'auto'\n[mcp_servers.local]\ncommand = 'server'\n",
        );
        assert_eq!(diag.error_count(), 0, "{:?}", diag.errors());
    }

    #[test]
    #[serial_test::serial]
    fn malformed_toml_reports_cx001() {
        let diag = with_config("[broken");
        let diagnostic = diag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.rule == LintRule::CodexTomlInvalid)
            .unwrap();
        assert_eq!(
            diagnostic.subject_path.as_deref(),
            Some(std::path::Path::new(CONFIG_PATH))
        );
        assert!(diagnostic.location.is_some());
    }

    #[test]
    #[serial_test::serial]
    fn invalid_fields_report_their_rules() {
        let diag = with_config(
            "unknown = true\nproject_doc_max_bytes = 0\nproject_doc_fallback_filenames = ['a/b', 'a/b']\napproval_policy = 'bad'\nsandbox_mode = 'bad'\nmodel_reasoning_effort = 'bad'\nmodel_verbosity = 'bad'\npersonality = 'bad'\ncli_auth_credentials_store = 'bad'\nmcp_oauth_credentials_store = 'bad'\nmodel_reasoning_summary = 'bad'\napprovals_reviewer = 'bad'\nservice_tier = 'bad'\nmodel = 1\nmodel_provider = 1\nmodel_context_window = 0\nmodel_auto_compact_token_limit = 0\nhistory = 'bad'\ntui = 'bad'\nfile_opener = 1\nskills = 'bad'\nprofile = 1\n[features]\nunknown = true\nmulti_agent_v2 = true\n[agents]\nmax_threads = 2\n[shell_environment_policy]\ninherit = 'bad'\nunknown = true\n[sandbox_workspace_write]\nmode = 'bad'\n[mcp_servers.bad]\nbearer_token = 'secret'\n[mcp_servers.empty]\nenabled = true\n[apps.example]\ndefault_tools_approval_mode = 'bad'\nunknown = true\n",
        );
        for rule in [
            LintRule::CodexUnknownNestedKey,
            LintRule::CodexProjectDocMaxBytes,
            LintRule::CodexProjectDocFallbackNames,
            LintRule::CodexApprovalPolicy,
            LintRule::CodexSandboxMode,
            LintRule::CodexReasoningEffort,
            LintRule::CodexModelVerbosity,
            LintRule::CodexPersonality,
            LintRule::CodexCliCredentialsStore,
            LintRule::CodexMcpCredentialsStore,
            LintRule::CodexReasoningSummary,
            LintRule::CodexApprovalsReviewer,
            LintRule::CodexServiceTier,
            LintRule::CodexModelType,
            LintRule::CodexModelProviderType,
            LintRule::CodexContextWindow,
            LintRule::CodexAutoCompactLimit,
            LintRule::CodexHistoryType,
            LintRule::CodexTuiType,
            LintRule::CodexFileOpenerType,
            LintRule::CodexSkillsType,
            LintRule::CodexProfileType,
            LintRule::CodexShellEnvironmentInherit,
            LintRule::CodexWorkspaceWriteMode,
            LintRule::CodexMcpServerTransport,
            LintRule::CodexInlineBearerToken,
            LintRule::CodexMultiAgentThreadLimit,
            LintRule::CodexAppApprovalMode,
        ] {
            assert!(has(&diag, rule), "missing {:?}", rule);
        }
    }

    #[test]
    #[serial_test::serial]
    fn full_access_needs_explicit_acknowledgement() {
        assert!(has(
            &with_config("sandbox_mode = 'danger-full-access'"),
            LintRule::CodexFullAccessAcknowledgment
        ));
    }

    #[test]
    #[serial_test::serial]
    fn mcp_secret_uses_shared_secret_heuristics() {
        assert!(has(
            &with_config(
                "[mcp_servers.server]\ncommand = 'x'\nenv = { API_KEY = 'sk-abcdefghijklmnopqrstuvwxyz' }"
            ),
            LintRule::CodexHardcodedSecret
        ));
    }

    #[test]
    #[serial_test::serial]
    fn approval_policy_table_rejects_unknown_fields() {
        assert!(has(
            &with_config("[approval_policy]\ntypo = true"),
            LintRule::CodexApprovalPolicyField
        ));
    }

    #[test]
    #[serial_test::serial]
    fn remaining_unknown_and_suppressed_rules_are_covered() {
        let diag = with_config(
            "unknown = true\n[features]\ntypo = true\n[permissions.network]\ntypo = true\n[windows]\nsandbox = 'bad'\n",
        );
        for rule in [
            LintRule::CodexTopLevelKey,
            LintRule::CodexFeatureKey,
            LintRule::CodexNetworkPermissionField,
            LintRule::CodexWindowsSandbox,
        ] {
            assert!(has(&diag, rule), "missing {:?}", rule);
        }
    }
}
