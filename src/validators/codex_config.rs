//! Validation for an optional project `.codex/config.toml`.

use crate::config::ExcludeSet;
use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata, SourceSpan};
use crate::rules::LintRule;
use crate::sensitive::contains_possible_secret;
use crate::validators::codex_constants::*;
use toml::Value;
use toml::value::Table;

const CONFIG_PATH: &str = ".codex/config.toml";

fn report_config(diag: &mut DiagnosticCollector, rule: LintRule, message: &str) {
    diag.report_at(rule, CONFIG_PATH, message);
}

/// Validate project-local Codex TOML. The configuration is optional.
pub fn validate_config(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    if exclude.is_excluded(CONFIG_PATH) {
        return;
    }
    let bytes = match std::fs::read(CONFIG_PATH) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            diag.report_at(
                LintRule::CodexTomlInvalid,
                CONFIG_PATH,
                &format!("{CONFIG_PATH} could not be read: {error}"),
            );
            return;
        }
    };
    let content = match String::from_utf8(bytes) {
        Ok(content) => content,
        Err(error) => {
            diag.report_at(
                LintRule::CodexTomlInvalid,
                CONFIG_PATH,
                &format!("{CONFIG_PATH} is not valid UTF-8: {error}"),
            );
            return;
        }
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
        && (!value.is_integer() || value.as_integer().is_none_or(|n| n < 0))
    {
        report_config(
            diag,
            LintRule::CodexProjectDocMaxBytes,
            &format!("{CONFIG_PATH}: 'project_doc_max_bytes' must be a nonnegative integer"),
        );
    }
    if let Some(value) = root.get("project_doc_fallback_filenames") {
        let valid = value
            .as_array()
            .is_some_and(|items| items.iter().all(Value::is_str));
        if !valid {
            report_config(
                diag,
                LintRule::CodexProjectDocFallbackNames,
                &format!(
                    "{CONFIG_PATH}: 'project_doc_fallback_filenames' must be an array of strings"
                ),
            );
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
    ] {
        if let Some(value) = root.get(key)
            && !(key == "approval_policy" && value.is_table())
            && !value.as_str().is_some_and(|value| allowed.contains(&value))
        {
            report_config(
                diag,
                rule,
                &format!(
                    "{CONFIG_PATH}: '{key}' must be one of: {}",
                    allowed.join(", ")
                ),
            );
        }
    }
    if let Some(value) = root.get("model_reasoning_effort")
        && value.as_str().is_none_or(str::is_empty)
    {
        report_config(
            diag,
            LintRule::CodexReasoningEffort,
            &format!("{CONFIG_PATH}: 'model_reasoning_effort' must be a non-empty string"),
        );
    }
    if let Some(value) = root.get("service_tier")
        && !value.is_str()
    {
        report_config(
            diag,
            LintRule::CodexServiceTier,
            &format!("{CONFIG_PATH}: 'service_tier' must be a string"),
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
            report_config(
                diag,
                rule,
                &format!("{CONFIG_PATH}: '{key}' must be a string"),
            );
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
            report_config(
                diag,
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
            report_config(
                diag,
                rule,
                &format!("{CONFIG_PATH}: '{key}' must be a positive integer"),
            );
        }
    }
}

fn validate_nested(diag: &mut DiagnosticCollector, root: &Table) {
    validate_container_types(diag, root);
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
            report_config(
                diag,
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
    {
        validate_unknown_keys(
            diag,
            &format!("{CONFIG_PATH} [sandbox_workspace_write]"),
            table,
            SANDBOX_WORKSPACE_WRITE_KEYS,
            LintRule::CodexUnknownNestedKey,
        );
        for (key, valid) in [
            (
                "writable_roots",
                table.get("writable_roots").is_none_or(|value| {
                    value
                        .as_array()
                        .is_some_and(|values| values.iter().all(Value::is_str))
                }),
            ),
            (
                "network_access",
                table.get("network_access").is_none_or(Value::is_bool),
            ),
            (
                "exclude_tmpdir_env_var",
                table
                    .get("exclude_tmpdir_env_var")
                    .is_none_or(Value::is_bool),
            ),
            (
                "exclude_slash_tmp",
                table.get("exclude_slash_tmp").is_none_or(Value::is_bool),
            ),
        ] {
            if !valid {
                report_config(
                    diag,
                    LintRule::CodexWorkspaceWrite,
                    &format!("{CONFIG_PATH}: sandbox_workspace_write.{key} has an invalid type"),
                );
            }
        }
    }
    validate_mcp_servers(diag, root.get("mcp_servers"));
    validate_apps(diag, root.get("apps"));
    validate_approval_policy(diag, root.get("approval_policy"));
    validate_agent_thread_limit(diag, root);
    validate_suppressed_permissions(diag, root.get("permissions"));
    validate_suppressed_windows(diag, root.get("windows"));
}

fn validate_container_types(diag: &mut DiagnosticCollector, root: &Table) {
    for key in [
        "agents",
        "apps",
        "features",
        "permissions",
        "windows",
        "shell_environment_policy",
        "sandbox_workspace_write",
        "mcp_servers",
    ] {
        if let Some(value) = root.get(key)
            && !value.is_table()
        {
            report_config(
                diag,
                LintRule::CodexConfigContainerType,
                &format!("{CONFIG_PATH}: '{key}' must be a TOML table"),
            );
        }
    }
    if let Some(permissions) = root.get("permissions").and_then(Value::as_table)
        && let Some(value) = permissions.get("network")
        && !value.is_table()
    {
        report_config(
            diag,
            LintRule::CodexConfigContainerType,
            &format!("{CONFIG_PATH}: 'permissions.network' must be a TOML table"),
        );
    }
}

fn validate_mcp_servers(diag: &mut DiagnosticCollector, value: Option<&Value>) {
    let Some(servers) = value.and_then(Value::as_table) else {
        return;
    };
    for (name, value) in servers {
        let label = format!("{CONFIG_PATH}: mcp_servers.{name}");
        let Some(server) = value.as_table() else {
            report_config(
                diag,
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
        let command = server.get("command").and_then(Value::as_str);
        let url = server.get("url").and_then(Value::as_str);
        let valid_command = command.is_some_and(|value| !value.is_empty());
        let valid_url = url.is_some_and(|value| !value.is_empty());
        if valid_command == valid_url {
            report_config(
                diag,
                LintRule::CodexMcpServerTransport,
                &format!("{label} must define exactly one non-empty string 'command' or 'url'"),
            );
        } else if valid_command {
            validate_mcp_transport_fields(diag, &label, server, McpTransport::Stdio);
        } else {
            validate_mcp_transport_fields(diag, &label, server, McpTransport::Http);
        }
        if server.contains_key("bearer_token") {
            report_config(
                diag,
                LintRule::CodexInlineBearerToken,
                &format!("{label}.bearer_token is forbidden; use bearer_token_env_var"),
            );
        }
        if table_contains_secret(server) {
            report_config(
                diag,
                LintRule::CodexHardcodedSecret,
                &format!("{label} contains a potential hardcoded secret"),
            );
        }
    }
}

#[derive(Clone, Copy)]
enum McpTransport {
    Stdio,
    Http,
}

fn validate_mcp_transport_fields(
    diag: &mut DiagnosticCollector,
    label: &str,
    server: &Table,
    transport: McpTransport,
) {
    for (key, value) in server {
        let invalid = match transport {
            McpTransport::Stdio => match key.as_str() {
                "args" | "env_vars" => !value
                    .as_array()
                    .is_some_and(|values| values.iter().all(Value::is_str)),
                "env" => !is_string_table(value),
                "cwd" => !value.is_str(),
                "url"
                | "bearer_token_env_var"
                | "http_headers"
                | "env_http_headers"
                | "oauth_resource" => true,
                _ => false,
            },
            McpTransport::Http => match key.as_str() {
                "bearer_token_env_var" | "oauth_resource" => !value.is_str(),
                "http_headers" | "env_http_headers" => !is_string_table(value),
                "command" | "args" | "env" | "env_vars" | "cwd" => true,
                _ => false,
            },
        };
        if invalid {
            report_config(
                diag,
                LintRule::CodexMcpServerTransport,
                &format!("{label}.{key} is invalid for this transport"),
            );
        }
    }
}

fn is_string_table(value: &Value) -> bool {
    value
        .as_table()
        .is_some_and(|table| table.values().all(Value::is_str))
}

fn validate_apps(diag: &mut DiagnosticCollector, value: Option<&Value>) {
    let Some(apps) = value.and_then(Value::as_table) else {
        return;
    };
    for (name, value) in apps {
        let label = format!("{CONFIG_PATH}: apps.{name}");
        let Some(app) = value.as_table() else {
            report_config(
                diag,
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
            report_config(
                diag,
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
    if table.len() != 1 || !table.contains_key("granular") {
        report_config(
            diag,
            LintRule::CodexApprovalPolicyShape,
            &format!("{CONFIG_PATH}: [approval_policy] must contain exactly a 'granular' table"),
        );
        return;
    }
    let Some(granular) = table.get("granular").and_then(Value::as_table) else {
        report_config(
            diag,
            LintRule::CodexApprovalPolicyShape,
            &format!("{CONFIG_PATH}: approval_policy.granular must be a TOML table"),
        );
        return;
    };
    const REQUIRED_KEYS: &[&str] = &[
        "sandbox_approval",
        "rules",
        "mcp_elicitations",
        "request_permissions",
        "skill_approval",
    ];
    validate_unknown_keys(
        diag,
        &format!("{CONFIG_PATH} [approval_policy.granular]"),
        granular,
        REQUIRED_KEYS,
        LintRule::CodexApprovalPolicyField,
    );
    for key in REQUIRED_KEYS {
        if !granular.get(*key).is_some_and(Value::is_bool) {
            report_config(
                diag,
                LintRule::CodexApprovalPolicyShape,
                &format!("{CONFIG_PATH}: approval_policy.granular.{key} must be a boolean"),
            );
        }
    }
}

fn validate_agent_thread_limit(diag: &mut DiagnosticCollector, root: &Table) {
    let Some(agents) = root.get("agents").and_then(Value::as_table) else {
        return;
    };
    if let Some(value) = agents.get("max_threads")
        && (!value.is_integer() || value.as_integer().is_none_or(|number| number < 1))
    {
        report_config(
            diag,
            LintRule::CodexAgentThreads,
            &format!("{CONFIG_PATH}: agents.max_threads must be an integer greater than zero"),
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
        report_config(diag, rule, &format!("{label}: unknown key '{key}'"));
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
        report_config(
            diag,
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

    fn with_config_bytes(content: &[u8]) -> DiagnosticCollector {
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

    fn rules(diag: &DiagnosticCollector) -> Vec<LintRule> {
        diag.diagnostics()
            .iter()
            .map(|diagnostic| diagnostic.rule)
            .collect()
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
        assert_eq!(
            rules(&with_config_bytes(b"\xff")),
            vec![LintRule::CodexTomlInvalid]
        );
    }

    #[test]
    #[serial_test::serial]
    fn schema_contracts_cover_boundaries_and_shapes() {
        for (content, expected) in [
            (
                "project_doc_max_bytes = -1",
                vec![LintRule::CodexProjectDocMaxBytes],
            ),
            ("project_doc_max_bytes = 0", vec![]),
            ("project_doc_max_bytes = 1000000", vec![]),
            (
                "project_doc_max_bytes = 1.5",
                vec![LintRule::CodexProjectDocMaxBytes],
            ),
            (
                "project_doc_fallback_filenames = ['a/b', '', 'a/b']",
                vec![],
            ),
            (
                "project_doc_fallback_filenames = [1]",
                vec![LintRule::CodexProjectDocFallbackNames],
            ),
            ("approval_policy = 'on-failure'", vec![]),
            ("model_reasoning_effort = 'future-value'", vec![]),
            (
                "model_reasoning_effort = ''",
                vec![LintRule::CodexReasoningEffort],
            ),
            ("service_tier = ''", vec![]),
            ("service_tier = true", vec![LintRule::CodexServiceTier]),
            ("sandbox_mode = 'danger-full-access'", vec![]),
        ] {
            assert_eq!(rules(&with_config(content)), expected, "{content}");
        }
    }

    #[test]
    #[serial_test::serial]
    fn granular_approval_shape_and_unknown_fields_have_distinct_owners() {
        let valid = "[approval_policy.granular]\nsandbox_approval = true\nrules = true\nmcp_elicitations = true\nrequest_permissions = true\nskill_approval = true\n";
        assert_eq!(rules(&with_config(valid)), Vec::<LintRule>::new());
        for content in [
            "[approval_policy]\nsandbox_approval = true",
            "[approval_policy]\ngranular = true",
            "[approval_policy.granular]\nsandbox_approval = true\nrules = true\nmcp_elicitations = true\nrequest_permissions = true",
            "[approval_policy.granular]\nsandbox_approval = true\nrules = true\nmcp_elicitations = true\nrequest_permissions = true\nskill_approval = 'true'",
        ] {
            assert!(
                has(&with_config(content), LintRule::CodexApprovalPolicyShape),
                "{content}"
            );
        }
        let unknown = format!("{valid}typo = true");
        assert_eq!(
            rules(&with_config(&unknown)),
            vec![LintRule::CodexApprovalPolicyField]
        );
    }

    #[test]
    #[serial_test::serial]
    fn mcp_transports_and_workspace_write_validate_types_without_cascades() {
        let clean = "[mcp_servers.stdio]\ncommand = 'server'\nargs = ['--ok']\nenv = { KEY = 'value' }\nenv_vars = ['TOKEN']\ncwd = '.'\n[mcp_servers.http]\nurl = 'https://example.com/mcp'\nbearer_token_env_var = 'TOKEN'\nhttp_headers = { Accept = 'application/json' }\nenv_http_headers = { Authorization = 'TOKEN' }\noauth_resource = 'resource'\n[sandbox_workspace_write]\nwritable_roots = ['.']\nnetwork_access = true\nexclude_tmpdir_env_var = false\nexclude_slash_tmp = false\n";
        assert_eq!(rules(&with_config(clean)), Vec::<LintRule>::new());
        let broken = "[mcp_servers.server]\ncommand = 'server'\nargs = [1]\nurl = 'https://example.com'\n[mcp_servers.other]\ncommand = 'one'\nurl = 'two'\n[sandbox_workspace_write]\nwritable_roots = [1]\nnetwork_access = 'yes'\nmode = 'all'\n";
        assert_eq!(
            rules(&with_config(broken)),
            vec![
                LintRule::CodexUnknownNestedKey,
                LintRule::CodexWorkspaceWrite,
                LintRule::CodexWorkspaceWrite,
                LintRule::CodexMcpServerTransport,
                LintRule::CodexMcpServerTransport,
            ]
        );
        for content in [
            "[mcp_servers.server]\ncommand = 1",
            "[mcp_servers.server]\ncommand = ''",
            "[mcp_servers.server]\ncommand = 'one'\nurl = 'https://example.com'",
            "mcp_servers = { server = 'bad' }",
        ] {
            assert_eq!(
                rules(&with_config(content)),
                vec![LintRule::CodexMcpServerTransport],
                "{content}"
            );
        }
        let invalid_stdio = "[mcp_servers.server]\ncommand = 'server'\nargs = 'bad'\nenv = { KEY = 1 }\nenv_vars = [1]\ncwd = 1\nbearer_token_env_var = 'TOKEN'\nhttp_headers = { Accept = 'application/json' }\nenv_http_headers = { Authorization = 'TOKEN' }\noauth_resource = 'resource'";
        assert_eq!(
            rules(&with_config(invalid_stdio)),
            vec![LintRule::CodexMcpServerTransport; 8]
        );
        let invalid_http = "[mcp_servers.server]\nurl = 'https://example.com'\nbearer_token_env_var = 1\noauth_resource = 1\nhttp_headers = { Accept = 1 }\nenv_http_headers = 'bad'\ncommand = 1\nargs = ['bad']\nenv = { KEY = 'value' }\nenv_vars = ['TOKEN']\ncwd = '.'";
        assert_eq!(
            rules(&with_config(invalid_http)),
            vec![LintRule::CodexMcpServerTransport; 9]
        );
        assert_eq!(
            rules(&with_config(
                "[mcp_servers.server]\ncommand = 'server'\nenv = { API_KEY = 'sk-abcdefghijklmnopqrstuvwxyz' }"
            )),
            vec![LintRule::CodexHardcodedSecret]
        );
    }

    #[test]
    #[serial_test::serial]
    fn agent_threads_and_feature_allowlist_follow_codex_0_144_6() {
        // Recorded from `codex features list` at implementation start. Keep
        // compatibility aliases in FEATURE_KEYS even when this release labels
        // them removed, but do not let the current official names drift out.
        const CODEX_0_144_6_FEATURES: &str = "apply_patch_freeform apply_patch_streaming_events apps apps_mcp_path_override artifact auth_elicitation browser_use browser_use_external browser_use_full_cdp_access chronicle code_mode code_mode_host code_mode_only codex_git_commit collaboration_modes computer_use concurrent_reasoning_summaries current_time_reminder default_mode_request_user_input deferred_executor elevated_windows_sandbox enable_fanout enable_mcp_apps enable_request_compression exec_permission_approvals experimental_windows_sandbox external_migration fast_mode goals guardian_approval hooks image_detail_original image_generation in_app_browser item_ids js_repl js_repl_tools_only local_thread_store_compression memories mentions_v2 multi_agent multi_agent_mode multi_agent_v2 network_proxy non_prefixed_mcp_tool_names personality plugin_hooks plugin_sharing plugins prevent_idle_sleep realtime_conversation remote_compaction_v2 remote_control remote_models remote_plugin request_permissions_tool request_rule resize_all_images respect_system_proxy responses_websockets responses_websockets_v2 rollout_budget runtime_metrics search_tool secret_auth_storage shell_snapshot shell_tool shell_zsh_fork skill_env_var_dependency_prompt skill_mcp_dependency_install sqlite standalone_web_search steer terminal_resize_reflow terminal_visualization_instructions token_budget tool_call_mcp_elicitation tool_search tool_search_always_defer_mcp_tools tool_suggest tui_app_server unavailable_dummy_tools undo unified_exec unified_exec_zsh_fork use_agent_identity use_legacy_landlock use_linux_sandbox_bwrap web_search_cached web_search_request workspace_dependencies workspace_owner_usage_nudge";
        for key in CODEX_0_144_6_FEATURES.split_whitespace() {
            assert!(
                FEATURE_KEYS.contains(&key),
                "missing Codex 0.144.6 feature: {key}"
            );
        }
        for content in [
            "[agents]\nmax_threads = 1",
            "[agents]\nmax_threads = 2\n[features]\nmulti_agent_v2 = true",
            "[agents]\nmax_threads = 2\n[features.multi_agent_v2]\nenabled = true",
            "[features]\nartifact = true",
        ] {
            assert_eq!(
                rules(&with_config(content)),
                Vec::<LintRule>::new(),
                "{content}"
            );
        }
        for content in [
            "[agents]\nmax_threads = 0",
            "[agents]\nmax_threads = -1",
            "[agents]\nmax_threads = 'two'",
        ] {
            assert_eq!(
                rules(&with_config(content)),
                vec![LintRule::CodexAgentThreads]
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn malformed_containers_report_cx062_once_and_skip_children() {
        for content in [
            "agents = 'bad'",
            "apps = 'bad'",
            "features = 'bad'",
            "permissions = 'bad'",
            "windows = 'bad'",
            "shell_environment_policy = 'bad'",
            "sandbox_workspace_write = 'bad'",
            "mcp_servers = 'bad'",
            "[permissions]\nnetwork = 'bad'",
        ] {
            assert_eq!(
                rules(&with_config(content)),
                vec![LintRule::CodexConfigContainerType],
                "{content}"
            );
        }
        assert_eq!(
            rules(&with_config("history = 'bad'\ntui = 'bad'\nskills = 'bad'")),
            vec![
                LintRule::CodexHistoryType,
                LintRule::CodexTuiType,
                LintRule::CodexSkillsType
            ]
        );
    }

    #[test]
    #[serial_test::serial]
    fn network_unknown_field_is_a_warning_rule_and_windows_stays_strict() {
        let diag = with_config("[permissions.network]\ntypo = true\n[windows]\nsandbox = 'bad'");
        assert_eq!(
            rules(&diag),
            vec![
                LintRule::CodexNetworkPermissionField,
                LintRule::CodexWindowsSandbox
            ]
        );
    }
}
