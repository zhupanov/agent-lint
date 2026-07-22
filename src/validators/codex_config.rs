//! Validation for an optional project `.codex/config.toml`.

use crate::config::ExcludeSet;
use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata, SourceSpan};
use crate::rules::LintRule;
use crate::sensitive::{
    contains_codex_mcp_token_signature, is_safe_env_placeholder, is_sensitive_key,
};
use crate::validators::codex_constants::*;
use toml::Value;
use toml::value::Table;
use toml_edit::{ImDocument, Item, TableLike};

const CONFIG_PATH: &str = ".codex/config.toml";
pub(crate) const CODEX_DEFAULT_PROJECT_DOC_MAX_BYTES: usize = 32_768;

/// Repository-local inputs that control Codex project-document selection.
/// `None` from [`project_document_settings`] means an existing config could
/// not supply valid inputs, so dependent document rules must not cascade.
#[derive(Debug, Clone)]
pub(crate) struct ProjectDocumentSettings {
    pub max_bytes: usize,
    pub fallback_filenames: Vec<String>,
    pub config: Option<Value>,
}

pub(crate) fn project_document_settings(exclude: &ExcludeSet) -> Option<ProjectDocumentSettings> {
    if exclude.is_excluded(CONFIG_PATH) {
        return Some(ProjectDocumentSettings {
            max_bytes: CODEX_DEFAULT_PROJECT_DOC_MAX_BYTES,
            fallback_filenames: Vec::new(),
            config: None,
        });
    }
    let bytes = match std::fs::read(CONFIG_PATH) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Some(ProjectDocumentSettings {
                max_bytes: CODEX_DEFAULT_PROJECT_DOC_MAX_BYTES,
                fallback_filenames: Vec::new(),
                config: None,
            });
        }
        Err(_) => return None,
    };
    let content = String::from_utf8(bytes).ok()?;
    let value: Value = content.parse().ok()?;
    let Some(root) = value.as_table() else {
        return Some(ProjectDocumentSettings {
            max_bytes: CODEX_DEFAULT_PROJECT_DOC_MAX_BYTES,
            fallback_filenames: Vec::new(),
            config: Some(value),
        });
    };
    let max_bytes = match root.get("project_doc_max_bytes") {
        None => CODEX_DEFAULT_PROJECT_DOC_MAX_BYTES,
        Some(value) => value
            .as_integer()
            .filter(|value| *value >= 0)
            .and_then(|value| usize::try_from(value).ok())?,
    };
    let fallback_filenames = match root.get("project_doc_fallback_filenames") {
        None => Vec::new(),
        Some(value) => value
            .as_array()?
            .iter()
            .map(|value| value.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()?,
    };
    Some(ProjectDocumentSettings {
        max_bytes,
        fallback_filenames,
        config: Some(value),
    })
}

struct SourceMap<'a> {
    content: &'a str,
    // `ImDocument` is toml_edit's source-preserving document. `DocumentMut`
    // deliberately drops spans in 0.22, so it cannot satisfy diagnostics.
    document: ImDocument<&'a str>,
}

impl<'a> SourceMap<'a> {
    fn new(content: &'a str) -> Result<Self, toml_edit::TomlError> {
        Ok(Self {
            content,
            document: ImDocument::parse(content)?,
        })
    }

    fn item(&self, path: &[&str]) -> Option<&Item> {
        let (last, parents) = path.split_last()?;
        let mut table: &dyn TableLike = self.document.as_table();
        for key in parents {
            table = table.get(key)?.as_table_like()?;
        }
        table.get(last)
    }

    fn value_span(&self, path: &[&str]) -> Option<SourceSpan> {
        self.item(path)
            .and_then(|item| {
                item.as_value()
                    .and_then(toml_edit::Value::span)
                    .or_else(|| item.span())
            })
            .and_then(|span| SourceSpan::from_byte_range(self.content, span))
    }

    fn key_span(&self, path: &[&str]) -> Option<SourceSpan> {
        let (last, parents) = path.split_last()?;
        let mut table: &dyn TableLike = self.document.as_table();
        for key in parents {
            table = table.get(key)?.as_table_like()?;
        }
        table
            .get_key_value(last)
            .and_then(|(key, _)| key.span())
            .and_then(|span| SourceSpan::from_byte_range(self.content, span))
    }
}

#[allow(clippy::too_many_arguments)] // keeps each validator's rule, source path, and safe metadata explicit at the call site.
fn report_config(
    diag: &mut DiagnosticCollector,
    source: &SourceMap<'_>,
    rule: LintRule,
    message: &str,
    path: &[&str],
    key_only: bool,
    evidence: Option<&str>,
    suggestion: &str,
) {
    let span = if key_only {
        source.key_span(path)
    } else {
        source.value_span(path)
    };
    let fallback_evidence = match path {
        ["mcp_servers", _] | ["apps", _] => None,
        _ => path.last().copied(),
    };
    let evidence = evidence.or(span.and(fallback_evidence));
    let mut metadata = DiagnosticMetadata::default().with_suggestion(suggestion);
    if let Some(span) = span {
        metadata = metadata.with_location(span);
    }
    if let Some(evidence) = evidence {
        metadata = metadata.with_evidence(evidence);
    }
    diag.report_at_with(rule, CONFIG_PATH, message, metadata);
}

/// Validate project-local Codex TOML. The configuration is optional.
pub fn validate_config(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    if exclude.is_excluded(CONFIG_PATH) {
        return;
    }
    let bytes = match std::fs::read(CONFIG_PATH) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(_) => {
            diag.report_at_with(
                LintRule::CodexTomlInvalid,
                CONFIG_PATH,
                &format!("{CONFIG_PATH} could not be read as UTF-8 configuration"),
                DiagnosticMetadata::default()
                    .with_suggestion("save .codex/config.toml as readable UTF-8 text"),
            );
            return;
        }
    };
    let content = match String::from_utf8(bytes) {
        Ok(content) => content,
        Err(_) => {
            diag.report_at_with(
                LintRule::CodexTomlInvalid,
                CONFIG_PATH,
                &format!("{CONFIG_PATH} could not be read as UTF-8 configuration"),
                DiagnosticMetadata::default()
                    .with_suggestion("save .codex/config.toml as readable UTF-8 text"),
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
                &format!("{CONFIG_PATH} is not valid TOML"),
                metadata.with_suggestion("correct the TOML syntax at the reported location"),
            );
            return;
        }
    };
    let source = match SourceMap::new(&content) {
        Ok(source) => source,
        Err(_) => return,
    };
    let Some(root) = value.as_table() else { return };
    let root_scope = ValueScope::root(root);
    validate_unknown_keys(
        diag,
        &source,
        CONFIG_PATH,
        root,
        TOP_LEVEL_KEYS,
        LintRule::CodexTopLevelKey,
        &[],
    );
    validate_project_docs(diag, root, &source);
    validate_scalar_enums(diag, &root_scope, &source);
    validate_types(diag, &root_scope, &source);
    validate_nested(diag, root, &source);
    validate_profiles(diag, root, &source);
}

/// Identifies a table whose scalar/typed values are validated with the shared
/// root Codex per-key rules, plus how each finding is labelled and located.
/// Codex applies identical per-key contracts at the document root and inside
/// every `[profiles.<name>]` table (issue #388), so both flow through one
/// `ValueScope` rather than a duplicated per-profile validator.
struct ValueScope<'a> {
    table: &'a Table,
    /// Message prefix before ": '<key>' …": `CONFIG_PATH` at the document root,
    /// `"CONFIG_PATH [profiles.<name>]"` inside a profile.
    label: &'a str,
    /// Structural path from the document root to `table`; empty at the root.
    prefix: &'a [&'a str],
}

impl<'a> ValueScope<'a> {
    fn root(table: &'a Table) -> Self {
        Self {
            table,
            label: CONFIG_PATH,
            prefix: &[],
        }
    }

    /// True only for the document root, where `profile` (CX032) is validated; a
    /// profile cannot itself select a profile (#388 decision 1).
    fn is_root(&self) -> bool {
        self.prefix.is_empty()
    }

    /// Document-root path to `key` within this scope, used for span resolution.
    fn path(&self, key: &'a str) -> Vec<&'a str> {
        let mut path = self.prefix.to_vec();
        path.push(key);
        path
    }
}

fn validate_project_docs(diag: &mut DiagnosticCollector, root: &Table, source: &SourceMap<'_>) {
    if let Some(value) = root.get("project_doc_max_bytes")
        && (!value.is_integer() || value.as_integer().is_none_or(|n| n < 0))
    {
        report_config(
            diag,
            source,
            LintRule::CodexProjectDocMaxBytes,
            &format!("{CONFIG_PATH}: 'project_doc_max_bytes' must be a nonnegative integer"),
            &["project_doc_max_bytes"],
            false,
            None,
            "use a nonnegative integer",
        );
    }
    if let Some(value) = root.get("project_doc_fallback_filenames") {
        let valid = value
            .as_array()
            .is_some_and(|items| items.iter().all(Value::is_str));
        if !valid {
            report_config(
                diag,
                source,
                LintRule::CodexProjectDocFallbackNames,
                &format!(
                    "{CONFIG_PATH}: 'project_doc_fallback_filenames' must be an array of strings"
                ),
                &["project_doc_fallback_filenames"],
                false,
                None,
                "use an array of strings",
            );
        }
    }
}

fn validate_scalar_enums(
    diag: &mut DiagnosticCollector,
    scope: &ValueScope<'_>,
    source: &SourceMap<'_>,
) {
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
        if let Some(value) = scope.table.get(key)
            && !(key == "approval_policy" && value.is_table())
            && !value.as_str().is_some_and(|value| allowed.contains(&value))
        {
            report_config(
                diag,
                source,
                rule,
                &format!(
                    "{}: '{key}' must be one of: {}",
                    scope.label,
                    allowed.join(", ")
                ),
                &scope.path(key),
                false,
                value.as_str(),
                "select one of the supported values",
            );
        }
    }
    if let Some(value) = scope.table.get("model_reasoning_effort")
        && value.as_str().is_none_or(str::is_empty)
    {
        report_config(
            diag,
            source,
            LintRule::CodexReasoningEffort,
            &format!(
                "{}: 'model_reasoning_effort' must be a non-empty string",
                scope.label
            ),
            &scope.path("model_reasoning_effort"),
            false,
            value.as_str(),
            "use a non-empty string",
        );
    }
    if let Some(value) = scope.table.get("service_tier")
        && !value.is_str()
    {
        report_config(
            diag,
            source,
            LintRule::CodexServiceTier,
            &format!("{}: 'service_tier' must be a string", scope.label),
            &scope.path("service_tier"),
            false,
            value.as_str(),
            "use a string",
        );
    }
}

fn validate_types(diag: &mut DiagnosticCollector, scope: &ValueScope<'_>, source: &SourceMap<'_>) {
    for (key, rule) in [
        ("model", LintRule::CodexModelType),
        ("model_provider", LintRule::CodexModelProviderType),
        ("file_opener", LintRule::CodexFileOpenerType),
        ("profile", LintRule::CodexProfileType),
    ] {
        // CX032 stays root-only: a profile cannot itself select a profile
        // (#388 decision 1). Every other string type applies in both scopes.
        if key == "profile" && !scope.is_root() {
            continue;
        }
        if let Some(value) = scope.table.get(key)
            && !value.is_str()
        {
            report_config(
                diag,
                source,
                rule,
                &format!("{}: '{key}' must be a string", scope.label),
                &scope.path(key),
                false,
                value.as_str(),
                "use a string",
            );
        }
    }
    for (key, rule) in [
        ("history", LintRule::CodexHistoryType),
        ("tui", LintRule::CodexTuiType),
        ("skills", LintRule::CodexSkillsType),
    ] {
        if let Some(value) = scope.table.get(key)
            && !value.is_table()
        {
            report_config(
                diag,
                source,
                rule,
                &format!("{}: '{key}' must be a TOML table", scope.label),
                &scope.path(key),
                false,
                None,
                "use a TOML table",
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
        if let Some(value) = scope.table.get(key)
            && (!value.is_integer() || value.as_integer().is_none_or(|number| number <= 0))
        {
            report_config(
                diag,
                source,
                rule,
                &format!("{}: '{key}' must be a positive integer", scope.label),
                &scope.path(key),
                false,
                value.as_str(),
                "use a positive integer",
            );
        }
    }
}

fn validate_nested(diag: &mut DiagnosticCollector, root: &Table, source: &SourceMap<'_>) {
    validate_container_types(diag, root, source);
    if let Some(table) = root.get("features").and_then(Value::as_table) {
        validate_unknown_keys(
            diag,
            source,
            &format!("{CONFIG_PATH} [features]"),
            table,
            FEATURE_KEYS,
            LintRule::CodexFeatureKey,
            &["features"],
        );
    }
    if let Some(table) = root.get("tui").and_then(Value::as_table) {
        validate_unknown_keys(
            diag,
            source,
            &format!("{CONFIG_PATH} [tui]"),
            table,
            TUI_KEYS,
            LintRule::CodexUnknownNestedKey,
            &["tui"],
        );
    }
    if let Some(table) = root
        .get("shell_environment_policy")
        .and_then(Value::as_table)
    {
        validate_unknown_keys(
            diag,
            source,
            &format!("{CONFIG_PATH} [shell_environment_policy]"),
            table,
            SHELL_POLICY_KEYS,
            LintRule::CodexUnknownNestedKey,
            &["shell_environment_policy"],
        );
        if let Some(value) = table.get("inherit")
            && !value
                .as_str()
                .is_some_and(|value| SHELL_INHERIT_VALUES.contains(&value))
        {
            report_config(
                diag,
                source,
                LintRule::CodexShellEnvironmentInherit,
                &format!(
                    "{CONFIG_PATH}: shell_environment_policy.inherit must be one of: {}",
                    SHELL_INHERIT_VALUES.join(", ")
                ),
                &["shell_environment_policy", "inherit"],
                false,
                value.as_str(),
                "select one of the supported values",
            );
        }
    }
    if let Some(table) = root
        .get("sandbox_workspace_write")
        .and_then(Value::as_table)
    {
        validate_unknown_keys(
            diag,
            source,
            &format!("{CONFIG_PATH} [sandbox_workspace_write]"),
            table,
            SANDBOX_WORKSPACE_WRITE_KEYS,
            LintRule::CodexUnknownNestedKey,
            &["sandbox_workspace_write"],
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
                    source,
                    LintRule::CodexWorkspaceWrite,
                    &format!("{CONFIG_PATH}: sandbox_workspace_write.{key} has an invalid type"),
                    &["sandbox_workspace_write", key],
                    false,
                    None,
                    "use the required field type",
                );
            }
        }
    }
    validate_mcp_servers(diag, root.get("mcp_servers"), source);
    validate_apps(diag, root.get("apps"), source);
    validate_approval_policy(diag, root.get("approval_policy"), source);
    validate_agent_thread_limit(diag, root, source);
    validate_suppressed_permissions(diag, root.get("permissions"), source);
    validate_suppressed_windows(diag, root.get("windows"), source);
}

fn validate_container_types(diag: &mut DiagnosticCollector, root: &Table, source: &SourceMap<'_>) {
    for key in [
        "agents",
        "apps",
        "features",
        "permissions",
        "profiles",
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
                source,
                LintRule::CodexConfigContainerType,
                &format!("{CONFIG_PATH}: '{key}' must be a TOML table"),
                &[key],
                false,
                None,
                "use a TOML table",
            );
        }
    }
    if let Some(permissions) = root.get("permissions").and_then(Value::as_table)
        && let Some(value) = permissions.get("network")
        && !value.is_table()
    {
        report_config(
            diag,
            source,
            LintRule::CodexConfigContainerType,
            &format!("{CONFIG_PATH}: 'permissions.network' must be a TOML table"),
            &["permissions", "network"],
            false,
            None,
            "use a TOML table",
        );
    }
}

/// Validate `[profiles.<name>]` tables. Codex applies the same per-key value
/// contracts inside a profile as at the document root, so each profile table
/// reuses the shared [`validate_scalar_enums`]/[`validate_types`] with a
/// profile-labelled [`ValueScope`] (issue #388, decision 1). A non-table
/// `profiles` is reported by [`validate_container_types`]; a non-table
/// `profiles.<name>` entry is reported here — both through CX062 and both
/// skipping child validation, matching the container no-cascade policy
/// (#388 decision 5). Structured profile sub-sections (`mcp_servers`,
/// `sandbox_workspace_write`, …) are deliberately not recursed into
/// (#388 decision 4), and `profiles` are never cross-referenced against a
/// top-level `profile` selector (#388 decision 6).
///
/// Decision-8 recheck, codex-cli 0.144.6, 2026-07-21: codex parses and
/// validates inline `[profiles.<name>]` values under the
/// `profiles.<name>.<key>` error path even when the profile is unselected,
/// confirming the false-negative gap this closes. Its profile struct is a
/// subset of the root schema, so some root keys (e.g. `file_opener`,
/// `history`, `skills`, both credential stores) are accepted-and-ignored
/// inside a profile; per decision 3 those remain defects — an unsupported key
/// or a wrong value — and are still reported with the root rule identity.
fn validate_profiles(diag: &mut DiagnosticCollector, root: &Table, source: &SourceMap<'_>) {
    let Some(profiles) = root.get("profiles").and_then(Value::as_table) else {
        return;
    };
    for (name, value) in profiles {
        let Some(profile) = value.as_table() else {
            report_config(
                diag,
                source,
                LintRule::CodexConfigContainerType,
                &format!("{CONFIG_PATH}: 'profiles.{name}' must be a TOML table"),
                &["profiles", name],
                false,
                None,
                "use a TOML table",
            );
            continue;
        };
        let label = format!("{CONFIG_PATH} [profiles.{name}]");
        let prefix = ["profiles", name.as_str()];
        let scope = ValueScope {
            table: profile,
            label: &label,
            prefix: &prefix,
        };
        validate_scalar_enums(diag, &scope, source);
        validate_types(diag, &scope, source);
    }
}

fn validate_mcp_servers(
    diag: &mut DiagnosticCollector,
    value: Option<&Value>,
    source: &SourceMap<'_>,
) {
    let Some(servers) = value.and_then(Value::as_table) else {
        return;
    };
    for (name, value) in servers {
        let label = format!("{CONFIG_PATH}: mcp_servers.{name}");
        let Some(server) = value.as_table() else {
            report_config(
                diag,
                source,
                LintRule::CodexMcpServerTransport,
                &format!("{label} must be an object with 'command' or 'url'"),
                &["mcp_servers", name],
                false,
                None,
                "define a server table with exactly one transport",
            );
            continue;
        };
        validate_unknown_keys(
            diag,
            source,
            &label,
            server,
            MCP_SERVER_KEYS,
            LintRule::CodexUnknownNestedKey,
            &["mcp_servers", name],
        );
        let command = server.get("command").and_then(Value::as_str);
        let url = server.get("url").and_then(Value::as_str);
        let valid_command = command.is_some_and(|value| !value.is_empty());
        let valid_url = url.is_some_and(|value| !value.is_empty());
        if valid_command == valid_url {
            report_config(
                diag,
                source,
                LintRule::CodexMcpServerTransport,
                &format!("{label} must define exactly one non-empty string 'command' or 'url'"),
                &[
                    "mcp_servers",
                    name,
                    if server.contains_key("url") {
                        "url"
                    } else {
                        "command"
                    },
                ],
                false,
                None,
                "define exactly one non-empty transport field",
            );
        } else if valid_command {
            validate_mcp_transport_fields(diag, &label, server, source, name, McpTransport::Stdio);
        } else {
            validate_mcp_transport_fields(diag, &label, server, source, name, McpTransport::Http);
        }
        if server.contains_key("bearer_token") {
            report_config(
                diag,
                source,
                LintRule::CodexInlineBearerToken,
                &format!("{label}.bearer_token is forbidden; use bearer_token_env_var"),
                &["mcp_servers", name, "bearer_token"],
                true,
                Some("bearer_token"),
                "replace bearer_token with bearer_token_env_var",
            );
        }
    }
    validate_mcp_secret_literals(diag, source.content);
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
    source: &SourceMap<'_>,
    server_name: &str,
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
                source,
                LintRule::CodexMcpServerTransport,
                &format!("{label}.{key} is invalid for this transport"),
                &["mcp_servers", server_name, key],
                false,
                None,
                "use only fields valid for this transport",
            );
        }
    }
}

fn is_string_table(value: &Value) -> bool {
    value
        .as_table()
        .is_some_and(|table| table.values().all(Value::is_str))
}

fn validate_apps(diag: &mut DiagnosticCollector, value: Option<&Value>, source: &SourceMap<'_>) {
    let Some(apps) = value.and_then(Value::as_table) else {
        return;
    };
    for (name, value) in apps {
        let label = format!("{CONFIG_PATH}: apps.{name}");
        let Some(app) = value.as_table() else {
            report_config(
                diag,
                source,
                LintRule::CodexUnknownNestedKey,
                &format!("{label} must be a table"),
                &["apps", name],
                false,
                None,
                "use a TOML table",
            );
            continue;
        };
        validate_unknown_keys(
            diag,
            source,
            &label,
            app,
            if name == "_default" {
                APP_DEFAULT_KEYS
            } else {
                APP_KEYS
            },
            LintRule::CodexUnknownNestedKey,
            &["apps", name],
        );
        if let Some(mode) = app.get("default_tools_approval_mode")
            && !mode
                .as_str()
                .is_some_and(|mode| APP_APPROVAL_MODES.contains(&mode))
        {
            report_config(
                diag,
                source,
                LintRule::CodexAppApprovalMode,
                &format!(
                    "{label}.default_tools_approval_mode must be one of: {}",
                    APP_APPROVAL_MODES.join(", ")
                ),
                &["apps", name, "default_tools_approval_mode"],
                false,
                mode.as_str(),
                "select one of the supported values",
            );
        }
    }
}

fn validate_approval_policy(
    diag: &mut DiagnosticCollector,
    value: Option<&Value>,
    source: &SourceMap<'_>,
) {
    let Some(table) = value.and_then(Value::as_table) else {
        return;
    };
    if table.len() != 1 || !table.contains_key("granular") {
        report_config(
            diag,
            source,
            LintRule::CodexApprovalPolicyShape,
            &format!("{CONFIG_PATH}: [approval_policy] must contain exactly a 'granular' table"),
            &["approval_policy"],
            false,
            None,
            "use only the granular approval policy table",
        );
        return;
    }
    let Some(granular) = table.get("granular").and_then(Value::as_table) else {
        report_config(
            diag,
            source,
            LintRule::CodexApprovalPolicyShape,
            &format!("{CONFIG_PATH}: approval_policy.granular must be a TOML table"),
            &["approval_policy", "granular"],
            false,
            None,
            "use a TOML table",
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
        source,
        &format!("{CONFIG_PATH} [approval_policy.granular]"),
        granular,
        REQUIRED_KEYS,
        LintRule::CodexApprovalPolicyField,
        &["approval_policy", "granular"],
    );
    for key in REQUIRED_KEYS {
        if !granular.get(*key).is_some_and(Value::is_bool) {
            report_config(
                diag,
                source,
                LintRule::CodexApprovalPolicyShape,
                &format!("{CONFIG_PATH}: approval_policy.granular.{key} must be a boolean"),
                &["approval_policy", "granular", key],
                false,
                None,
                "use a boolean",
            );
        }
    }
}

fn validate_agent_thread_limit(
    diag: &mut DiagnosticCollector,
    root: &Table,
    source: &SourceMap<'_>,
) {
    let Some(agents) = root.get("agents").and_then(Value::as_table) else {
        return;
    };
    if let Some(value) = agents.get("max_threads")
        && (!value.is_integer() || value.as_integer().is_none_or(|number| number < 1))
    {
        report_config(
            diag,
            source,
            LintRule::CodexAgentThreads,
            &format!("{CONFIG_PATH}: agents.max_threads must be an integer greater than zero"),
            &["agents", "max_threads"],
            false,
            value.as_str(),
            "use an integer greater than zero",
        );
    }
}

fn validate_unknown_keys(
    diag: &mut DiagnosticCollector,
    source: &SourceMap<'_>,
    label: &str,
    table: &Table,
    allowed: &[&str],
    rule: LintRule,
    parent: &[&str],
) {
    for key in table.keys().filter(|key| !allowed.contains(&key.as_str())) {
        let mut path = parent.to_vec();
        path.push(key);
        report_config(
            diag,
            source,
            rule,
            &format!("{label}: unknown key '{key}'"),
            &path,
            true,
            Some(key),
            "remove or correct this unsupported key",
        );
    }
}

fn validate_suppressed_permissions(
    diag: &mut DiagnosticCollector,
    value: Option<&Value>,
    source: &SourceMap<'_>,
) {
    let Some(network) = value
        .and_then(Value::as_table)
        .and_then(|table| table.get("network"))
        .and_then(Value::as_table)
    else {
        return;
    };
    validate_unknown_keys(
        diag,
        source,
        &format!("{CONFIG_PATH} [permissions.network]"),
        network,
        NETWORK_PERMISSION_KEYS,
        LintRule::CodexNetworkPermissionField,
        &["permissions", "network"],
    );
}

fn validate_suppressed_windows(
    diag: &mut DiagnosticCollector,
    value: Option<&Value>,
    source: &SourceMap<'_>,
) {
    if let Some(value) = value
        .and_then(Value::as_table)
        .and_then(|table| table.get("sandbox"))
        && !value
            .as_str()
            .is_some_and(|value| WINDOWS_SANDBOX_MODES.contains(&value))
    {
        report_config(
            diag,
            source,
            LintRule::CodexWindowsSandbox,
            &format!(
                "{CONFIG_PATH}: windows.sandbox must be one of: {}",
                WINDOWS_SANDBOX_MODES.join(", ")
            ),
            &["windows", "sandbox"],
            false,
            value.as_str(),
            "select one of the supported values",
        );
    }
}

const ENV_NAME_FIELDS: &[&str] = &["env_vars", "bearer_token_env_var", "env_http_headers"];
const SENSITIVE_HTTP_HEADERS: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
];

/// CX013 owns only literal credential facts in known Codex MCP locations. It
/// intentionally parses the original TOML again so locations and ordering come
/// from source, rather than from a semantic table that may reorder keys.
fn validate_mcp_secret_literals(diag: &mut DiagnosticCollector, content: &str) {
    let Ok(document) = ImDocument::parse(content) else {
        return; // `toml::Value` already owns the parse diagnostic.
    };
    let Some(servers) = document
        .as_table()
        .get("mcp_servers")
        .and_then(Item::as_table_like)
    else {
        return;
    };
    for (_, server) in servers.iter() {
        let Some(server) = server.as_table_like() else {
            continue;
        };
        validate_server_secret_literals(diag, content, server);
    }
}

#[derive(Clone, Copy)]
enum SecretMapKind {
    Environment,
    HttpHeaders,
}

fn validate_server_secret_literals(
    diag: &mut DiagnosticCollector,
    content: &str,
    server: &dyn TableLike,
) {
    for (field, value) in server.iter() {
        match field {
            "env" => validate_typed_secret_map(diag, content, value, SecretMapKind::Environment),
            "http_headers" => {
                validate_typed_secret_map(diag, content, value, SecretMapKind::HttpHeaders)
            }
            "bearer_token" => {} // CX028 exclusively owns this field.
            _ if ENV_NAME_FIELDS.contains(&field) => {}
            _ if item_contains_codex_token_signature(value) => report_cx013(
                diag,
                content,
                server,
                field,
                "move the credential to an environment variable instead of storing a literal",
            ),
            _ => {}
        }
    }
}

fn validate_typed_secret_map(
    diag: &mut DiagnosticCollector,
    content: &str,
    value: &Item,
    kind: SecretMapKind,
) {
    let Some(values) = value.as_table_like() else {
        return;
    };
    for (key, value) in values.iter() {
        let Some(value) = value.as_str() else {
            continue;
        };
        let literal = !value.trim().is_empty() && !is_safe_env_placeholder(value, true);
        let sensitive_key = match kind {
            SecretMapKind::Environment => is_sensitive_key(key),
            SecretMapKind::HttpHeaders => {
                SENSITIVE_HTTP_HEADERS
                    .iter()
                    .any(|name| key.eq_ignore_ascii_case(name))
                    || is_sensitive_key(key)
            }
        };
        if !(contains_codex_mcp_token_signature(value) || (sensitive_key && literal)) {
            continue;
        }
        let suggestion = match kind {
            SecretMapKind::Environment => {
                "move the variable name to env_vars instead of storing a credential literal"
            }
            SecretMapKind::HttpHeaders => {
                "move the variable name to env_http_headers or bearer_token_env_var instead of storing a credential literal"
            }
        };
        report_cx013(diag, content, values, key, suggestion);
    }
}

fn item_contains_codex_token_signature(item: &Item) -> bool {
    if item
        .as_str()
        .is_some_and(contains_codex_mcp_token_signature)
    {
        return true;
    }
    item.as_array().is_some_and(|items| {
        items
            .iter()
            .filter_map(toml_edit::Value::as_str)
            .any(contains_codex_mcp_token_signature)
    })
}

fn report_cx013(
    diag: &mut DiagnosticCollector,
    content: &str,
    table: &dyn TableLike,
    key: &str,
    suggestion: &str,
) {
    let location = table
        .get_key_value(key)
        .and_then(|(key, _)| key.span())
        .and_then(|range| SourceSpan::from_byte_range(content, range));
    let mut metadata = DiagnosticMetadata::default()
        .with_evidence(key)
        .with_suggestion(suggestion);
    if let Some(location) = location {
        metadata = metadata.with_location(location);
    }
    diag.report_at_with(
        LintRule::CodexHardcodedSecret,
        CONFIG_PATH,
        ".codex/config.toml contains a literal MCP credential",
        metadata,
    );
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

    fn with_default_config(content: &str) -> DiagnosticCollector {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir(".codex").unwrap();
        std::fs::write(CONFIG_PATH, content).unwrap();
        let mut diag = DiagnosticCollector::new();
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
    fn cx001_never_echoes_parser_or_secret_text() {
        let secret = "sk-abcdefghijklmnopqrstuvwxyz";
        let diag = with_config(&format!("command = \"{secret}"));
        let finding = diag.diagnostics().first().unwrap();
        assert_eq!(finding.message, ".codex/config.toml is not valid TOML");
        assert_eq!(
            finding.suggestion.as_deref(),
            Some("correct the TOML syntax at the reported location")
        );
        assert!(!finding.message.contains(secret));
        assert!(finding.evidence.is_none());
        assert!(finding.location.is_some());
    }

    #[test]
    #[serial_test::serial]
    fn loader_failures_are_default_errors_with_source_metadata() {
        let diagnostics = with_default_config(
            "model = 7\nmodel_reasoning_summary = 'nope'\nhistory = false\ntui = []\nfile_opener = 1\napprovals_reviewer = 'nope'\nskills = false\nprofile = 2\n",
        );
        let expected = [
            LintRule::CodexReasoningSummary,
            LintRule::CodexApprovalsReviewer,
            LintRule::CodexModelType,
            LintRule::CodexFileOpenerType,
            LintRule::CodexProfileType,
            LintRule::CodexHistoryType,
            LintRule::CodexTuiType,
            LintRule::CodexSkillsType,
        ];
        assert_eq!(rules(&diagnostics), expected);
        for finding in diagnostics.diagnostics() {
            assert_eq!(finding.severity, crate::diagnostic::Severity::Error);
            assert_eq!(
                finding.subject_path.as_deref(),
                Some(std::path::Path::new(CONFIG_PATH))
            );
            assert!(finding.location.is_some(), "{finding:?}");
            assert!(finding.evidence.is_some(), "{finding:?}");
            assert!(finding.suggestion.is_some(), "{finding:?}");
        }
    }

    #[test]
    #[serial_test::serial]
    fn source_metadata_uses_structural_spans_and_never_bearer_values() {
        let secret = "sk-abcdefghijklmnopqrstuvwxyz";
        let diag = with_config(&format!(
            "title = 'é'\r\nmodel = 7\r\n[mcp_servers.server]\r\ncommand = 'run'\r\nbearer_token = '{secret}'\r\n[features]\r\nunknown_flag = true\r\n"
        ));
        let model = diag
            .diagnostics()
            .iter()
            .find(|d| d.rule == LintRule::CodexModelType)
            .unwrap();
        assert_eq!(model.location.unwrap().start().line_number(), 2);
        let bearer = diag
            .diagnostics()
            .iter()
            .find(|d| d.rule == LintRule::CodexInlineBearerToken)
            .unwrap();
        assert_eq!(bearer.evidence.as_deref(), Some("bearer_token"));
        assert_eq!(
            bearer.suggestion.as_deref(),
            Some("replace bearer_token with bearer_token_env_var")
        );
        for finding in diag.diagnostics() {
            assert!(!finding.message.contains(secret));
            assert!(
                !finding
                    .evidence
                    .as_deref()
                    .is_some_and(|value| value.contains(secret))
            );
            assert!(
                !finding
                    .suggestion
                    .as_deref()
                    .is_some_and(|value| value.contains(secret))
            );
        }
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
    fn cx013_walks_only_typed_mcp_credential_locations_without_leaking_values() {
        let secret = "sk-abcdefghijklmnopqrstuvwxyz";
        let github = "ghp_abcdefghijklmnopqrstuvwxyz1234567890";
        let slack = "xoxb-1abcdefghij";
        let config = format!(
            "[mcp_servers.sk-abcdefghijklmnopqrstuv]\ncommand = 'server'\nenv = {{ SECRET = 'one', TOKEN = '${{TOKEN}}', PASSWORD = '${{PASSWORD:-}}', PASSWD = '${{PASSWD:-default}}', PRIVATE_KEY = 'five', ACCESS_KEY = 'six', API_KEY = 'seven', CLIENT_SECRET = 'eight', TOKENIZER_MODEL = 'clean', ORDINARY = '{secret}' }}\n[mcp_servers.http]\nurl = 'https://example.com/mcp'\nhttp_headers = {{ Authorization = 'Bearer header-literal-value', Proxy-Authorization = '${{PROXY}}', Cookie = '', Set-Cookie = '${{COOKIE:-default}}', X-API-Key = 'custom', Accept = '{slack}' }}\nbearer_token = '{secret}'\nbearer_token_env_var = 'GITHUB_TOKEN'\nenv_http_headers = {{ Authorization = 'GITHUB_TOKEN' }}\noauth_resource = '{github}'\n"
        );
        let diagnostics = with_config(&config);
        let secrets: Vec<_> = diagnostics
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.rule == LintRule::CodexHardcodedSecret)
            .collect();
        let evidence: Vec<_> = secrets
            .iter()
            .map(|diagnostic| diagnostic.evidence.as_deref())
            .collect();
        assert_eq!(
            evidence,
            [
                Some("SECRET"),
                Some("PASSWD"),
                Some("PRIVATE_KEY"),
                Some("ACCESS_KEY"),
                Some("API_KEY"),
                Some("CLIENT_SECRET"),
                Some("ORDINARY"),
                Some("Authorization"),
                Some("Set-Cookie"),
                Some("X-API-Key"),
                Some("Accept"),
                Some("oauth_resource"),
            ]
        );
        for diagnostic in &secrets {
            assert_eq!(
                diagnostic.subject_path.as_deref(),
                Some(std::path::Path::new(CONFIG_PATH))
            );
            assert_eq!(diagnostic.severity, crate::diagnostic::Severity::Error);
            assert!(diagnostic.location.is_some(), "{diagnostic:?}");
            assert!(diagnostic.message.contains("literal MCP credential"));
            for forbidden in [
                secret,
                github,
                slack,
                "sk-abcdefghijklmnopqrstuv",
                "header-literal-value",
            ] {
                assert!(
                    !diagnostic.message.contains(forbidden)
                        && !diagnostic
                            .evidence
                            .as_deref()
                            .is_some_and(|value| value.contains(forbidden))
                        && !diagnostic
                            .suggestion
                            .as_deref()
                            .is_some_and(|value| value.contains(forbidden)),
                    "leaked {forbidden} in {diagnostic:?}"
                );
            }
        }
        assert_eq!(
            rules(&diagnostics)
                .iter()
                .filter(|rule| **rule == LintRule::CodexInlineBearerToken)
                .count(),
            1
        );
        assert!(
            secrets
                .iter()
                .all(|diagnostic| diagnostic.evidence.as_deref() != Some("bearer_token"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn cx013_placeholder_boundaries_and_empty_values_are_clean() {
        let clean = "[mcp_servers.stdio]\ncommand = 'token = literal-but-not-an-explicit-signature'\nenv = { SECRET = '$NAME', TOKEN = '${NAME}', PASSWORD = '${NAME:-}', PASSWD = '', TOKENIZER_MODEL = 'value' }\nenv_vars = ['sk-abcdefghijklmnopqrstuvwxyz']\n[mcp_servers.http]\nurl = 'https://example.com/mcp'\nhttp_headers = { Authorization = '$NAME', Cookie = '${NAME}', Set-Cookie = '${NAME:-}', X-API-Key = '' }\nbearer_token_env_var = 'sk-abcdefghijklmnopqrstuvwxyz'\nenv_http_headers = { Authorization = 'sk-abcdefghijklmnopqrstuvwxyz' }\n";
        assert!(
            with_config(clean)
                .diagnostics()
                .iter()
                .all(|diagnostic| diagnostic.rule != LintRule::CodexHardcodedSecret)
        );
        for literal in ["prefix$NAME", "${NAME}suffix", "${NAME:-default}"] {
            let config = format!(
                "[mcp_servers.server]\ncommand = 'server'\nenv = {{ TOKEN = '{literal}' }}\n"
            );
            assert_eq!(
                rules(&with_config(&config)),
                vec![LintRule::CodexHardcodedSecret],
                "{literal}"
            );
        }
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
            "profiles = 'bad'",
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

    /// Every scalar-enum, string/typed, and integer key group fires inside a
    /// profile with the identical root rule identity and a
    /// `[profiles.<name>]`-prefixed message on the `.codex/config.toml` subject
    /// (issue #388, decisions 1 and 2).
    #[test]
    #[serial_test::serial]
    fn profile_values_reuse_root_rule_identities_and_labels() {
        for (body, rule) in [
            ("approval_policy = 'yolo'", LintRule::CodexApprovalPolicy),
            ("sandbox_mode = 'yolo'", LintRule::CodexSandboxMode),
            ("model_verbosity = 'extreme'", LintRule::CodexModelVerbosity),
            (
                "cli_auth_credentials_store = 'plaintext'",
                LintRule::CodexCliCredentialsStore,
            ),
            ("model = 5", LintRule::CodexModelType),
            ("model_provider = 1", LintRule::CodexModelProviderType),
            ("file_opener = 2", LintRule::CodexFileOpenerType),
            ("history = 5", LintRule::CodexHistoryType),
            ("tui = 5", LintRule::CodexTuiType),
            ("skills = 5", LintRule::CodexSkillsType),
            ("model_context_window = 0", LintRule::CodexContextWindow),
        ] {
            let diag = with_config(&format!("[profiles.risky]\n{body}\n"));
            assert_eq!(rules(&diag), vec![rule], "{body}");
            let finding = diag
                .diagnostics()
                .iter()
                .find(|diagnostic| diagnostic.rule == rule)
                .unwrap();
            assert!(
                finding
                    .message
                    .starts_with(".codex/config.toml [profiles.risky]: "),
                "{body}: {}",
                finding.message
            );
            assert_eq!(
                finding.subject_path.as_deref(),
                Some(std::path::Path::new(CONFIG_PATH)),
                "{body}"
            );
            assert!(finding.location.is_some(), "{body}");
        }
    }

    /// A profile-scoped finding keeps its root default severity: CX005 stays an
    /// error, the advisory CX023 stays a warning, and both carry the profile
    /// label so per-file overrides on `.codex/config.toml` still match.
    #[test]
    #[serial_test::serial]
    fn profile_findings_keep_root_default_severity() {
        let diag = with_default_config(
            "[profiles.risky]\napproval_policy = 'yolo'\nmodel_context_window = 0\n",
        );
        let approval = diag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.rule == LintRule::CodexApprovalPolicy)
            .unwrap();
        assert_eq!(approval.severity, crate::diagnostic::Severity::Error);
        assert_eq!(
            approval.message,
            ".codex/config.toml [profiles.risky]: 'approval_policy' must be one of: untrusted, on-request, on-failure, never"
        );
        let window = diag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.rule == LintRule::CodexContextWindow)
            .unwrap();
        assert_eq!(window.severity, crate::diagnostic::Severity::Warning);
        assert!(
            window
                .message
                .starts_with(".codex/config.toml [profiles.risky]: 'model_context_window'")
        );
    }

    /// Hard negatives: post-#324 accepted values, multiple profiles where only
    /// one is invalid, an empty profile table, keys the linter does not
    /// validate, and a profile-scoped `profile` selector all stay silent
    /// (issue #388, decisions 1 and 3).
    #[test]
    #[serial_test::serial]
    fn valid_and_unvalidated_profiles_stay_silent() {
        let valid = "[profiles.work]\napproval_policy = 'on-failure'\nsandbox_mode = 'danger-full-access'\nmodel_reasoning_effort = 'ultra'\nmodel = 'gpt-5'\n[profiles.empty]\n";
        assert_eq!(rules(&with_config(valid)), Vec::<LintRule>::new());

        let one_bad = "[profiles.good]\napproval_policy = 'never'\n[profiles.bad]\napproval_policy = 'yolo'\n";
        let diag = with_config(one_bad);
        assert_eq!(rules(&diag), vec![LintRule::CodexApprovalPolicy]);
        assert!(
            diag.diagnostics()[0]
                .message
                .contains("[profiles.bad]: 'approval_policy'")
        );

        // Keys outside the validated set and the excluded `profile` selector are
        // never flagged: no unknown-profile-key check exists (decision 3), and a
        // profile cannot select a profile (decision 1).
        for body in ["chatgpt_base_url = 'x'", "profile = 5", "unknown_key = 42"] {
            assert_eq!(
                rules(&with_config(&format!("[profiles.p]\n{body}\n"))),
                Vec::<LintRule>::new(),
                "{body}"
            );
        }
    }

    /// A non-table `profiles` and a non-table `profiles.<name>` entry each
    /// report exactly one CX062 with no cascaded child diagnostics
    /// (issue #388, decision 5).
    #[test]
    #[serial_test::serial]
    fn malformed_profiles_report_cx062_without_cascade() {
        for content in [
            "profiles = 5",
            "profiles = []",
            "[profiles]\ndirect = 'scalar'",
            // A scalar entry that mimics a valid value is still just a bad
            // container: one CX062, never a cascaded value check.
            "[profiles]\nrisky = 'danger-full-access'",
        ] {
            assert_eq!(
                rules(&with_config(content)),
                vec![LintRule::CodexConfigContainerType],
                "{content}"
            );
        }
        // A valid sibling profile keeps validating after a malformed entry.
        assert_eq!(
            rules(&with_config(
                "[profiles]\nbroken = 1\n[profiles.ok]\napproval_policy = 'yolo'\n"
            )),
            vec![
                LintRule::CodexConfigContainerType,
                LintRule::CodexApprovalPolicy,
            ]
        );
    }

    /// Root and profile findings for one rule appear in a deterministic order:
    /// root first (validated ahead of profiles), then profiles in sorted-table
    /// order — `alpha` before `beta` even though the file declares `beta`
    /// first — and the rendered output is byte-identical across independent
    /// runs (issue #388, decision 7).
    #[test]
    #[serial_test::serial]
    fn profile_and_root_findings_are_deterministic() {
        let messages = |diag: &DiagnosticCollector| -> Vec<String> {
            diag.diagnostics()
                .iter()
                .map(|diagnostic| diagnostic.message.clone())
                .collect()
        };
        let expected = vec![
            ".codex/config.toml: 'approval_policy' must be one of: untrusted, on-request, on-failure, never".to_string(),
            ".codex/config.toml [profiles.alpha]: 'approval_policy' must be one of: untrusted, on-request, on-failure, never".to_string(),
            ".codex/config.toml [profiles.beta]: 'approval_policy' must be one of: untrusted, on-request, on-failure, never".to_string(),
        ];
        // `beta` precedes `alpha` in the source, yet sorted-table iteration
        // still emits alpha before beta with root ahead of both, identically on
        // every independent parse+validate run.
        let content = "approval_policy = 'yolo'\n[profiles.beta]\napproval_policy = 'yolo'\n[profiles.alpha]\napproval_policy = 'yolo'\n";
        assert_eq!(messages(&with_config(content)), expected);
        assert_eq!(messages(&with_config(content)), expected);
    }
}
