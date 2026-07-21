use crate::config::ExcludeSet;
use crate::context::{LintContext, ManifestState};
use crate::diagnostic::DiagnosticCollector;
use crate::platforms::ValidationTargets;
use crate::rules::LintRule;
use crate::traversal;
use crate::validators::common::is_nonlocal_url_with_scheme;
use regex::Regex;
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

const RESERVED_SERVER_NAMES: &[&str] = &["workspace"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum McpTransport {
    Stdio,
    Http,
    StreamableHttp,
    Sse,
    WebSocket,
}

impl McpTransport {
    const ACCEPTED_TYPES: &str = "'stdio', 'http', 'streamable-http', 'sse', or 'ws'";

    fn parse(value: Option<&Value>) -> Option<Self> {
        match value {
            None => Some(Self::Stdio),
            Some(Value::String(kind)) => match kind.as_str() {
                "stdio" => Some(Self::Stdio),
                "http" => Some(Self::Http),
                "streamable-http" => Some(Self::StreamableHttp),
                "sse" => Some(Self::Sse),
                "ws" => Some(Self::WebSocket),
                _ => None,
            },
            Some(_) => None,
        }
    }

    fn is_remote(self) -> bool {
        !matches!(self, Self::Stdio)
    }

    fn name(self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::Http => "http",
            Self::StreamableHttp => "streamable-http",
            Self::Sse => "sse",
            Self::WebSocket => "ws",
        }
    }

    fn accepts_url(self, value: &str) -> bool {
        let Ok(url) = url::Url::parse(value) else {
            return false;
        };
        let scheme_is_appropriate = match self {
            Self::Http | Self::StreamableHttp | Self::Sse => {
                matches!(url.scheme(), "http" | "https")
            }
            Self::WebSocket => matches!(url.scheme(), "ws" | "wss"),
            Self::Stdio => return true,
        };
        scheme_is_appropriate && url.host().is_some()
    }

    fn url_scheme_description(self) -> &'static str {
        match self {
            Self::Http | Self::StreamableHttp | Self::Sse => "http:// or https://",
            Self::WebSocket => "ws:// or wss://",
            Self::Stdio => unreachable!("stdio does not accept a remote URL"),
        }
    }
}

static RE_SECRET_ENV_KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:api[_-]?key|secret|token|password)").expect("valid secret-key regex")
});
static RE_DANGEROUS_COMMAND: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:\b(?:curl|wget)\b[^\n|]*\|\s*(?:ba)?sh\b|\bsudo\s+rm\b|\brm\s+-[a-z]*r[a-z]*f[a-z]*\s+/(?:\s|$))")
        .expect("valid dangerous-command regex")
});

#[derive(Clone, Copy)]
enum McpAdapter {
    ClaudeStandalone,
    ClaudeInlinePlugin,
    Cursor,
}

impl McpAdapter {
    fn requires_server_map(self) -> bool {
        !matches!(self, Self::ClaudeInlinePlugin)
    }

    fn allows_claude_transport_rules(self) -> bool {
        matches!(self, Self::ClaudeStandalone | Self::ClaudeInlinePlugin)
    }

    fn allows_claude_only_rules(self) -> bool {
        self.allows_claude_transport_rules()
    }
}

/// Validate MCP configuration through explicit adapters for the supported
/// Claude and Cursor repository surfaces. Codex TOML remains CX-owned.
pub fn validate_mcp_configs(
    ctx: &LintContext,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    targets: ValidationTargets,
) {
    for path in claude_mcp_config_paths(ctx) {
        validate_json_document(
            &path,
            McpAdapter::ClaudeStandalone,
            &ctx.base_path,
            diag,
            exclude,
        );
    }

    if targets.cursor {
        validate_json_document(
            &ctx.base_path.join(".cursor/mcp.json"),
            McpAdapter::Cursor,
            &ctx.base_path,
            diag,
            exclude,
        );
    }

    if let ManifestState::Parsed(value) = &ctx.plugin_json {
        let display = ".claude-plugin/plugin.json";
        if !exclude.is_excluded(display) {
            diag.with_subject_path(display, |diag| {
                validate_document(display, value, McpAdapter::ClaudeInlinePlugin, None, diag);
            });
        }
    }
}

fn validate_json_document(
    path: &Path,
    adapter: McpAdapter,
    base_path: &Path,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
) {
    if !path.is_file() {
        return;
    }
    let display = display_path(base_path, path);
    if exclude.is_excluded(&display) {
        return;
    }
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) => {
            diag.report_at(
                LintRule::McpJsonInvalid,
                &display,
                &format!("{display} cannot be read: {error}"),
            );
            return;
        }
    };
    let value: Value = match serde_json::from_str(&content) {
        Ok(value) => value,
        Err(error) => {
            diag.report_at(
                LintRule::McpJsonInvalid,
                &display,
                &format!("{display} is not valid JSON: {error}"),
            );
            return;
        }
    };
    let duplicates = duplicate_mcp_keys(&content);
    diag.with_subject_path(&display, |diag| {
        validate_document(&display, &value, adapter, Some(duplicates), diag);
    });
}

fn claude_mcp_config_paths(ctx: &LintContext) -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    for entry in traversal::recursive_files(&ctx.base_path, &ctx.base_path, None).entries {
        if entry.path.file_name().and_then(|name| name.to_str()) == Some(".mcp.json") {
            paths.push(entry.path);
        }
    }
    paths.sort();
    paths
}

fn display_path(base: &Path, path: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn validate_document(
    display: &str,
    value: &Value,
    adapter: McpAdapter,
    duplicates: Option<DuplicateMcpKeys>,
    diag: &mut DiagnosticCollector,
) {
    if let Some(duplicates) = duplicates {
        for () in duplicates.top_level_server_maps {
            diag.report(
                LintRule::McpStructureInvalid,
                &format!("{display}: duplicate top-level mcpServers key"),
            );
        }
        for name in duplicates.server_names {
            diag.report(
                LintRule::McpDuplicateServer,
                &format!("{display}: mcpServers contains duplicate server name '{name}'"),
            );
        }
    }

    let servers = value.get("mcpServers");
    if servers.is_none() && !adapter.requires_server_map() {
        return;
    }
    let Some(servers) = servers.and_then(Value::as_object) else {
        diag.report(
            LintRule::McpStructureInvalid,
            &format!("{display}: mcpServers must be an object"),
        );
        return;
    };

    for (name, config) in servers {
        let label = format!("{display}: mcpServers.{name}");
        if adapter.allows_claude_only_rules() && RESERVED_SERVER_NAMES.contains(&name.as_str()) {
            diag.report(
                LintRule::McpServerReserved,
                &format!("{label} uses reserved server name '{name}'"),
            );
        }
        let Some(config) = config.as_object() else {
            diag.report(
                LintRule::McpStructureInvalid,
                &format!("{label} must be an object"),
            );
            continue;
        };
        if config.is_empty() {
            if matches!(adapter, McpAdapter::Cursor) {
                validate_cursor_selector(&label, config, diag);
            } else {
                diag.report(
                    LintRule::McpServerEmpty,
                    &format!("{label} must not be an empty object"),
                );
            }
            continue;
        }

        if adapter.allows_claude_transport_rules() {
            validate_claude_transport(&label, config, diag);
        } else {
            validate_cursor_selector(&label, config, diag);
        }
        if let Some(args) = config.get("args")
            && !args
                .as_array()
                .is_some_and(|items| items.iter().all(Value::is_string))
        {
            diag.report(
                LintRule::McpArgsInvalid,
                &format!("{label}.args must be an array of strings"),
            );
        }
        if adapter.allows_claude_only_rules()
            && config
                .get("alwaysLoad")
                .is_some_and(|value| !value.is_boolean())
        {
            diag.report(
                LintRule::McpAlwaysLoadInvalid,
                &format!("{label}.alwaysLoad must be a boolean"),
            );
        }
        if has_literal_secret(config.get("env")) {
            diag.report(
                LintRule::McpEnvSecretLiteral,
                &format!("{label}.env contains a literal secret-like value"),
            );
        }
        let command = std::iter::once(config.get("command").and_then(Value::as_str))
            .flatten()
            .chain(
                config
                    .get("args")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str),
            )
            .collect::<Vec<_>>()
            .join(" ");
        if RE_DANGEROUS_COMMAND.is_match(&command) {
            diag.report(
                LintRule::McpCommandDangerous,
                &format!("{label} contains a dangerous command pattern"),
            );
        }
    }
}

fn validate_claude_transport(
    label: &str,
    config: &serde_json::Map<String, Value>,
    diag: &mut DiagnosticCollector,
) {
    let transport = McpTransport::parse(config.get("type"));
    let Some(transport) = transport else {
        diag.report(
            LintRule::McpTypeInvalid,
            &format!("{label}.type must be {}", McpTransport::ACCEPTED_TYPES),
        );
        return;
    };
    if transport == McpTransport::Stdio && !has_nonempty_string(config.get("command")) {
        diag.report(
            LintRule::McpStdioCommandMissing,
            &format!("{label}: stdio server requires a non-empty command"),
        );
    }
    if transport.is_remote() {
        match config.get("url").and_then(Value::as_str) {
            Some(url) if transport.accepts_url(url) => validate_url_security(label, url, diag),
            _ => diag.report(
                LintRule::McpHttpUrlMissing,
                &format!(
                    "{label}: {} server requires a valid non-empty URL using {}",
                    transport.name(),
                    transport.url_scheme_description(),
                ),
            ),
        }
    }
    if transport == McpTransport::Sse {
        diag.report(
            LintRule::McpSseDeprecated,
            &format!("{label}: SSE transport is deprecated; use Streamable HTTP"),
        );
    }
}

fn validate_cursor_selector(
    label: &str,
    config: &serde_json::Map<String, Value>,
    diag: &mut DiagnosticCollector,
) {
    match (config.contains_key("command"), config.contains_key("url")) {
        (false, false) | (true, true) => diag.report(
            LintRule::McpStructureInvalid,
            &format!("{label} must define exactly one of command or url"),
        ),
        (false, true) => {
            if let Some(url) = config.get("url").and_then(Value::as_str) {
                validate_url_security(label, url, diag);
            }
        }
        (true, false) => {}
    }
}

fn validate_url_security(label: &str, url: &str, diag: &mut DiagnosticCollector) {
    for (insecure_scheme, secure_scheme) in [("http", "https"), ("ws", "wss")] {
        if is_nonlocal_url_with_scheme(url, insecure_scheme) {
            diag.report(
                LintRule::McpUrlNotHttps,
                &format!("{label}.url uses non-local {insecure_scheme}://; use {secure_scheme}://"),
            );
        }
    }
}

fn has_nonempty_string(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
}

fn has_literal_secret(env: Option<&Value>) -> bool {
    env.and_then(Value::as_object).is_some_and(|env| {
        env.iter().any(|(key, value)| {
            RE_SECRET_ENV_KEY.is_match(key)
                && value.as_str().is_some_and(|value| {
                    !value.trim().is_empty() && !value.starts_with('$') && !value.starts_with("{{")
                })
        })
    })
}

/// Serde intentionally keeps the last duplicate key. This lightweight scanner
/// runs only after JSON parsing succeeds, preserving raw duplicate keys.
struct DuplicateMcpKeys {
    top_level_server_maps: Vec<()>,
    server_names: Vec<String>,
}

fn duplicate_mcp_keys(content: &str) -> DuplicateMcpKeys {
    let mut scanner = JsonScanner::new(content.as_bytes());
    scanner.scan_value(ScanObject::TopLevel);
    DuplicateMcpKeys {
        top_level_server_maps: scanner.top_level_server_maps,
        server_names: scanner.server_names,
    }
}

#[derive(Clone, Copy)]
enum ScanObject {
    None,
    TopLevel,
    ServerMap,
}

struct JsonScanner<'a> {
    input: &'a [u8],
    pos: usize,
    top_level_server_maps: Vec<()>,
    server_names: Vec<String>,
}

impl<'a> JsonScanner<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self {
            input,
            pos: 0,
            top_level_server_maps: Vec::new(),
            server_names: Vec::new(),
        }
    }

    fn scan_value(&mut self, object_kind: ScanObject) {
        self.skip_ws();
        match self.input.get(self.pos) {
            Some(b'{') => self.scan_object(object_kind),
            Some(b'[') => self.scan_array(),
            Some(b'\"') => {
                self.scan_string();
            }
            _ => self.scan_scalar(),
        }
    }

    fn scan_object(&mut self, object_kind: ScanObject) {
        self.pos += 1;
        let mut names = HashSet::new();
        loop {
            self.skip_ws();
            if self.input.get(self.pos) == Some(&b'}') {
                self.pos += 1;
                return;
            }
            let key = self.scan_string();
            self.skip_ws();
            self.pos += 1; // JSON was already validated, so this is ':'
            if !names.insert(key.clone()) {
                match object_kind {
                    ScanObject::TopLevel if key == "mcpServers" => {
                        self.top_level_server_maps.push(())
                    }
                    ScanObject::ServerMap => self.server_names.push(key.clone()),
                    ScanObject::None | ScanObject::TopLevel => {}
                }
            }
            let child_kind = if matches!(object_kind, ScanObject::TopLevel) && key == "mcpServers" {
                ScanObject::ServerMap
            } else {
                ScanObject::None
            };
            self.scan_value(child_kind);
            self.skip_ws();
            if self.input.get(self.pos) == Some(&b',') {
                self.pos += 1;
            }
        }
    }

    fn scan_array(&mut self) {
        self.pos += 1;
        loop {
            self.skip_ws();
            if self.input.get(self.pos) == Some(&b']') {
                self.pos += 1;
                return;
            }
            self.scan_value(ScanObject::None);
            self.skip_ws();
            if self.input.get(self.pos) == Some(&b',') {
                self.pos += 1;
            }
        }
    }

    fn scan_string(&mut self) -> String {
        let start = self.pos;
        self.pos += 1;
        while self.pos < self.input.len() {
            match self.input[self.pos] {
                b'\\' => self.pos += 2,
                b'\"' => {
                    self.pos += 1;
                    break;
                }
                _ => self.pos += 1,
            }
        }
        serde_json::from_slice(&self.input[start..self.pos]).expect("validated JSON string")
    }

    fn scan_scalar(&mut self) {
        while self.pos < self.input.len()
            && !matches!(
                self.input[self.pos],
                b',' | b']' | b'}' | b' ' | b'\n' | b'\r' | b'\t'
            )
        {
            self.pos += 1;
        }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.input.len() && self.input[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{LintMode, ManifestState};

    fn context(base_path: &Path) -> LintContext {
        LintContext {
            base_path: base_path.to_path_buf(),
            mode: LintMode::Basic,
            plugin_json: ManifestState::Missing,
            marketplace_json: ManifestState::Missing,
            hooks_json: ManifestState::Missing,
            settings_json: ManifestState::Missing,
            settings_local_json: ManifestState::Missing,
        }
    }

    fn diagnostics(content: &str, path: &str) -> Vec<String> {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, content).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_mcp_configs(
            &context(temp.path()),
            &mut diag,
            &ExcludeSet::default(),
            ValidationTargets::default(),
        );
        diag.errors()
    }

    fn reported_rules(content: &str, path: &str) -> Vec<LintRule> {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, content).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_mcp_configs(
            &context(temp.path()),
            &mut diag,
            &ExcludeSet::default(),
            ValidationTargets::default(),
        );
        diag.diagnostics()
            .iter()
            .map(|diagnostic| diagnostic.rule)
            .collect()
    }

    #[test]
    fn validates_root_and_plugin_mcp_surfaces() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join(".mcp.json"),
            r#"{"mcpServers":{"root":{"command":"ok"}}}"#,
        )
        .unwrap();
        std::fs::create_dir_all(temp.path().join("plugins/example")).unwrap();
        std::fs::write(
            temp.path().join("plugins/example/.mcp.json"),
            r#"{"mcpServers":{"plugin":{"type":"http","url":"https://example.com"}}}"#,
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_mcp_configs(
            &context(temp.path()),
            &mut diag,
            &ExcludeSet::default(),
            ValidationTargets::default(),
        );
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn does_not_treat_claude_settings_as_an_mcp_surface() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join(".claude")).unwrap();
        std::fs::write(temp.path().join(".claude/settings.json"), "{").unwrap();
        std::fs::write(temp.path().join(".claude/settings.local.json"), "{").unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_mcp_configs(
            &context(temp.path()),
            &mut diag,
            &ExcludeSet::default(),
            ValidationTargets::default(),
        );
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn validates_inline_plugin_servers_from_the_parsed_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let ctx = LintContext {
            base_path: temp.path().to_path_buf(),
            mode: LintMode::Plugin,
            plugin_json: ManifestState::Parsed(serde_json::json!({
                "mcpServers": {
                    "missing-command": {"type": "stdio"},
                    "secret": {"command": "ok", "env": {"API_KEY": "plaintext"}}
                }
            })),
            marketplace_json: ManifestState::Missing,
            hooks_json: ManifestState::Missing,
            settings_json: ManifestState::Missing,
            settings_local_json: ManifestState::Missing,
        };
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_mcp_configs(
            &ctx,
            &mut diag,
            &ExcludeSet::default(),
            ValidationTargets::default(),
        );
        assert!(diag.diagnostics().iter().any(|diagnostic| {
            diagnostic.rule == LintRule::McpStdioCommandMissing
                && diagnostic.subject_path.as_deref()
                    == Some(Path::new(".claude-plugin/plugin.json"))
        }));
        assert!(diag.diagnostics().iter().any(|diagnostic| {
            diagnostic.rule == LintRule::McpEnvSecretLiteral
                && diagnostic.subject_path.as_deref()
                    == Some(Path::new(".claude-plugin/plugin.json"))
        }));
    }

    #[test]
    fn cursor_uses_selector_presence_without_claude_transport_defaults() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(".cursor/mcp.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"mcpServers":{"remote":{"url":"https://example.com/mcp"},"stdio":{"command":"server"},"bad":{"url":"http://example.com/mcp","args":[1],"env":{"TOKEN":"plaintext"}},"neither":{},"both":{"command":"server","url":"https://example.com/mcp"}}}"#,
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_mcp_configs(
            &context(temp.path()),
            &mut diag,
            &ExcludeSet::default(),
            ValidationTargets {
                cursor: true,
                ..ValidationTargets::default()
            },
        );
        let rules: Vec<_> = diag.diagnostics().iter().map(|item| item.rule).collect();
        assert!(rules.contains(&LintRule::McpUrlNotHttps));
        assert!(rules.contains(&LintRule::McpArgsInvalid));
        assert!(rules.contains(&LintRule::McpEnvSecretLiteral));
        assert_eq!(
            rules
                .iter()
                .filter(|rule| **rule == LintRule::McpStructureInvalid)
                .count(),
            2
        );
        for claude_only in [
            LintRule::McpStdioCommandMissing,
            LintRule::McpHttpUrlMissing,
            LintRule::McpTypeInvalid,
            LintRule::McpSseDeprecated,
            LintRule::McpAlwaysLoadInvalid,
            LintRule::McpServerReserved,
        ] {
            assert!(!rules.contains(&claude_only), "{claude_only:?}");
        }
    }

    #[test]
    fn invalid_json_is_reported() {
        assert!(diagnostics("{", ".mcp.json")[0].contains("not valid JSON"));
    }

    #[test]
    fn validates_transport_requirements_and_deprecation() {
        let errors = diagnostics(
            r#"{"mcpServers":{"stdio":{"args":[]},"http":{"type":"http"},"sse":{"type":"sse","url":"https://x"},"bad":{"type":"socket"}}}"#,
            ".mcp.json",
        );
        assert!(
            errors
                .iter()
                .any(|error| error.contains("requires a non-empty command"))
        );
        assert!(
            errors
                .iter()
                .any(|error| error.contains("requires a valid non-empty URL"))
        );
        assert!(
            errors
                .iter()
                .any(|error| error.contains("SSE transport is deprecated"))
        );
        assert!(errors.iter().any(|error| {
            error.contains("must be 'stdio', 'http', 'streamable-http', 'sse', or 'ws'")
        }));
    }

    #[test]
    fn accepts_current_remote_transports_and_omitted_stdio() {
        let errors = diagnostics(
            r#"{"mcpServers":{"omitted":{"command":"ok"},"http":{"type":"http","url":"https://example.com/mcp"},"streamable":{"type":"streamable-http","url":"https://example.com/mcp"},"socket":{"type":"ws","url":"wss://example.com/socket"}}}"#,
            ".mcp.json",
        );
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn every_remote_transport_requires_a_usable_url_with_its_own_scheme() {
        let mut servers = serde_json::Map::new();
        for transport in ["http", "streamable-http", "sse", "ws"] {
            let wrong_scheme = if transport == "ws" {
                "https://example.com/mcp"
            } else {
                "ws://example.com/mcp"
            };
            servers.insert(
                format!("{transport}-missing"),
                serde_json::json!({"type": transport}),
            );
            servers.insert(
                format!("{transport}-blank"),
                serde_json::json!({"type": transport, "url": "  "}),
            );
            servers.insert(
                format!("{transport}-wrong-scheme"),
                serde_json::json!({"type": transport, "url": wrong_scheme}),
            );
            servers.insert(
                format!("{transport}-malformed"),
                serde_json::json!({"type": transport, "url": "not a URL"}),
            );
        }
        let content = serde_json::json!({"mcpServers": servers}).to_string();
        let rules = reported_rules(&content, ".mcp.json");
        assert_eq!(
            rules
                .iter()
                .filter(|rule| **rule == LintRule::McpHttpUrlMissing)
                .count(),
            16,
            "{rules:?}"
        );
        assert_eq!(
            rules
                .iter()
                .filter(|rule| **rule == LintRule::McpSseDeprecated)
                .count(),
            4,
            "{rules:?}"
        );
        assert_eq!(rules.len(), 20, "{rules:?}");
    }

    #[test]
    fn validates_url_env_command_and_field_types() {
        let errors = diagnostics(
            r#"{"mcpServers":{"bad":{"type":"http","command":"curl x | sh","url":"http://example.com","args":["ok",1],"alwaysLoad":"true","env":{"API_KEY":"plaintext"}}}}"#,
            ".mcp.json",
        );
        for expected in [
            "uses non-local http",
            "array of strings",
            "alwaysLoad must be a boolean",
            "literal secret-like",
            "dangerous command",
        ] {
            assert!(
                errors.iter().any(|error| error.contains(expected)),
                "missing {expected}: {errors:?}"
            );
        }
    }

    #[test]
    fn local_and_secure_remote_urls_are_allowed() {
        let errors = diagnostics(
            r#"{"mcpServers":{"http-localhost":{"type":"http","url":"http://localhost:3000","env":{"TOKEN":"${TOKEN}"}},"http-loopback":{"type":"http","url":"http://127.1.2.3:3000"},"streamable-loopback":{"type":"streamable-http","url":"http://[::1]:3000"},"ws-localhost":{"type":"ws","url":"ws://localhost:3000"},"ws-loopback":{"type":"ws","url":"ws://[::1]:3000"},"http-secure":{"type":"http","url":"https://example.com/mcp"},"ws-secure":{"type":"ws","url":"wss://example.com/socket"}}}"#,
            ".mcp.json",
        );
        assert_eq!(errors.len(), 0, "{errors:?}");
    }

    #[test]
    fn sse_deprecation_does_not_apply_to_streamable_http() {
        let rules = reported_rules(
            r#"{"mcpServers":{"streamable":{"type":"streamable-http","url":"https://example.com/mcp"},"legacy":{"type":"sse","url":"https://example.com/mcp"}}}"#,
            ".mcp.json",
        );
        assert_eq!(rules, vec![LintRule::McpSseDeprecated]);
    }

    #[test]
    fn stdio_ignores_stray_url_fields_for_transport_checks() {
        let rules = reported_rules(
            r#"{"mcpServers":{"stdio":{"type":"stdio","command":"ok","url":"ws://example.com/socket"},"omitted":{"command":"ok","url":"not a URL"}}}"#,
            ".mcp.json",
        );
        assert!(rules.is_empty(), "{rules:?}");
    }

    #[test]
    fn duplicate_empty_and_reserved_servers_are_reported() {
        let errors = diagnostics(
            r#"{"mcpServers":{"workspace":{},"same":{"command":"one"},"same":{"command":"two"}}}"#,
            ".mcp.json",
        );
        for expected in [
            "reserved server name",
            "empty object",
            "duplicate server name",
        ] {
            assert!(
                errors.iter().any(|error| error.contains(expected)),
                "missing {expected}: {errors:?}"
            );
        }
    }
}
