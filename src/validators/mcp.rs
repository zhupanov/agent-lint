use crate::config::ExcludeSet;
use crate::context::{LintContext, ManifestState};
use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata, SourceSpan};
use crate::platforms::ValidationTargets;
use crate::rules::LintRule;
use crate::traversal;
use crate::validators::common::is_nonlocal_url_with_scheme;
use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
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
            let location = if error.column() == 0 {
                eof_location(&content)
            } else {
                SourceSpan::point(error.line().max(1), error.column())
            };
            diag.report_at_with(
                LintRule::McpJsonInvalid,
                &display,
                &format!("{display} is not valid JSON: {error}"),
                DiagnosticMetadata::default()
                    .with_location(location)
                    .with_evidence("JSON syntax")
                    .with_suggestion("fix the JSON syntax"),
            );
            return;
        }
    };
    let raw_keys = scan_mcp_keys(&content);
    let raw_tokens = RawMcpTokens::parse(&content);
    diag.with_subject_path(&display, |diag| {
        validate_document(
            &display,
            &value,
            adapter,
            Some((&content, &raw_keys, &raw_tokens)),
            diag,
        );
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
    raw_document: Option<(&str, &RawMcpKeys, &RawMcpTokens)>,
    diag: &mut DiagnosticCollector,
) {
    if let Some((source, raw_keys, raw_tokens)) = raw_document {
        for duplicate in raw_keys.server_maps.iter().skip(1) {
            report_structure(
                diag,
                &format!("{display}: duplicate top-level mcpServers key"),
                Some((source, duplicate.key_range.clone())),
                None,
            );
        }
        if let Some(first) = raw_keys.server_maps.first()
            && !first.value_is_object
        {
            report_structure(
                diag,
                &format!("{display}: mcpServers must be an object"),
                Some((source, first.value_range.clone())),
                None,
            );
        }
        for map in &raw_keys.server_maps {
            for entry in &raw_keys.server_entries[map.entries.clone()] {
                if !entry.value_is_object {
                    report_structure(
                        diag,
                        &format!("{display}: mcpServers.{} must be an object", entry.name),
                        Some((source, entry.value_range.clone())),
                        None,
                    );
                }
            }
        }
        for duplicate in &raw_tokens.duplicates {
            let (line, column) = position_at_offset(source, duplicate.first_key.start);
            diag.report_with(
                LintRule::McpDuplicateServer,
                &format!(
                    "{display}: mcpServers contains duplicate server name '{}'",
                    duplicate.name
                ),
                metadata(
                    source,
                    Some(&duplicate.duplicate_key),
                    &format!("first defined at line {line}, column {column}"),
                    "remove or rename this duplicate server key",
                ),
            );
        }
    }

    let servers = value.get("mcpServers");
    if servers.is_none() && !adapter.requires_server_map() {
        return;
    }
    let Some(servers) = servers.and_then(Value::as_object) else {
        let has_duplicate_map =
            raw_document.is_some_and(|(_, raw_keys, _)| raw_keys.server_maps.len() > 1);
        let first_invalid_map = raw_document.is_some_and(|(_, raw_keys, _)| {
            raw_keys
                .server_maps
                .first()
                .is_some_and(|map| !map.value_is_object)
        });
        if !has_duplicate_map && !first_invalid_map {
            let location = raw_document.and_then(|(source, raw_keys, _)| {
                raw_keys
                    .server_maps
                    .last()
                    .map(|map| (source, map.value_range.clone()))
            });
            report_structure(
                diag,
                &format!("{display}: mcpServers must be an object"),
                location,
                servers
                    .is_none()
                    .then_some("add a top-level mcpServers object"),
            );
        }
        return;
    };

    for (name, config) in servers {
        let label = format!("{display}: mcpServers.{name}");
        let token = raw_document.and_then(|(_, _, tokens)| tokens.servers.get(name));
        let source = raw_document.map(|(source, _, _)| source);
        let server_key = token.map(|token| &token.key);
        if adapter.allows_claude_only_rules() && RESERVED_SERVER_NAMES.contains(&name.as_str()) {
            report(
                diag,
                LintRule::McpServerReserved,
                &format!("{label} uses reserved server name '{name}'"),
                source,
                server_key,
                name,
                "rename this server to a non-reserved name",
            );
        }
        let Some(config) = config.as_object() else {
            if raw_document.is_none() {
                diag.report(
                    LintRule::McpStructureInvalid,
                    &format!("{label} must be an object"),
                );
            }
            continue;
        };
        if config.is_empty() {
            report(
                diag,
                LintRule::McpServerEmpty,
                &format!("{label} must not be an empty object"),
                source,
                server_key,
                "server configuration",
                "add the required server fields",
            );
            continue;
        }

        if adapter.allows_claude_transport_rules() {
            validate_claude_transport(&label, config, source, token, server_key, diag);
        } else {
            validate_cursor_selector(&label, config, source, token, server_key, diag);
        }
        if let Some(args) = config.get("args")
            && !args
                .as_array()
                .is_some_and(|items| items.iter().all(Value::is_string))
        {
            report(
                diag,
                LintRule::McpArgsInvalid,
                &format!("{label}.args must be an array of strings"),
                source,
                invalid_arg_token(token, args)
                    .or_else(|| field_value(token, "args"))
                    .or(server_key),
                "args",
                "use only string argv entries",
            );
        }
        if adapter.allows_claude_only_rules()
            && config
                .get("alwaysLoad")
                .is_some_and(|value| !value.is_boolean())
        {
            report(
                diag,
                LintRule::McpAlwaysLoadInvalid,
                &format!("{label}.alwaysLoad must be a boolean"),
                source,
                field_value(token, "alwaysLoad").or(server_key),
                "alwaysLoad",
                "use a JSON boolean",
            );
        }
        if let Some(env_key) = literal_secret_key(config.get("env")) {
            report(
                diag,
                LintRule::McpEnvSecretLiteral,
                &format!("{label}.env contains a literal secret-like value"),
                source,
                token
                    .and_then(|token| token.env_key(env_key))
                    .or(server_key),
                env_key,
                &format!("replace the literal value with ${{{env_key}}}"),
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
            report(
                diag,
                LintRule::McpCommandDangerous,
                &format!("{label} contains a dangerous command pattern"),
                source,
                server_key,
                "dangerous command pattern",
                "remove the dangerous command pattern",
            );
        }
    }
}

fn report_structure(
    diag: &mut DiagnosticCollector,
    message: &str,
    location: Option<(&str, std::ops::Range<usize>)>,
    suggestion: Option<&str>,
) {
    let mut metadata = DiagnosticMetadata::default();
    if let Some((source, range)) = location {
        if let Some(span) = SourceSpan::from_byte_range(source, range) {
            metadata = metadata.with_location(span);
        }
    }
    if let Some(suggestion) = suggestion {
        metadata = metadata.with_suggestion(suggestion);
    }
    diag.report_with(LintRule::McpStructureInvalid, message, metadata);
}

fn validate_claude_transport(
    label: &str,
    config: &serde_json::Map<String, Value>,
    source: Option<&str>,
    token: Option<&ServerTokens>,
    server_key: Option<&Range<usize>>,
    diag: &mut DiagnosticCollector,
) {
    let transport = McpTransport::parse(config.get("type"));
    let Some(transport) = transport else {
        report(
            diag,
            LintRule::McpTypeInvalid,
            &format!("{label}.type must be {}", McpTransport::ACCEPTED_TYPES),
            source,
            field_value(token, "type").or(server_key),
            "type",
            "set type to a supported MCP transport",
        );
        return;
    };
    if transport == McpTransport::Stdio && !has_nonempty_string(config.get("command")) {
        report(
            diag,
            LintRule::McpStdioCommandMissing,
            &format!("{label}: stdio server requires a non-empty command"),
            source,
            server_key,
            "command",
            "add a non-empty command for this stdio server",
        );
    }
    if transport.is_remote() {
        match config.get("url").and_then(Value::as_str) {
            Some(url) if transport.accepts_url(url) => validate_url_security(
                label,
                url,
                source,
                field_value(token, "url").or(server_key),
                diag,
            ),
            _ => report(
                diag,
                LintRule::McpHttpUrlMissing,
                &format!(
                    "{label}: {} server requires a valid non-empty URL using {}",
                    transport.name(),
                    transport.url_scheme_description(),
                ),
                source,
                field_value(token, "url").or(server_key),
                "url",
                &format!(
                    "use a valid URL with {}",
                    transport.url_scheme_description()
                ),
            ),
        }
    }
    if transport == McpTransport::Sse {
        report(
            diag,
            LintRule::McpSseDeprecated,
            &format!("{label}: SSE transport is deprecated; use Streamable HTTP"),
            source,
            field_value(token, "type").or(server_key),
            "type",
            "replace sse with streamable-http",
        );
    }
}

fn validate_cursor_selector(
    label: &str,
    config: &serde_json::Map<String, Value>,
    source: Option<&str>,
    token: Option<&ServerTokens>,
    server_key: Option<&Range<usize>>,
    diag: &mut DiagnosticCollector,
) {
    match (config.contains_key("command"), config.contains_key("url")) {
        (false, false) | (true, true) => diag.report(
            LintRule::McpStructureInvalid,
            &format!("{label} must define exactly one of command or url"),
        ),
        (false, true) => {
            if let Some(url) = config.get("url").and_then(Value::as_str) {
                validate_url_security(
                    label,
                    url,
                    source,
                    field_value(token, "url").or(server_key),
                    diag,
                );
            }
        }
        (true, false) => {}
    }
}

fn validate_url_security(
    label: &str,
    url: &str,
    source: Option<&str>,
    location: Option<&Range<usize>>,
    diag: &mut DiagnosticCollector,
) {
    for (insecure_scheme, secure_scheme) in [("http", "https"), ("ws", "wss")] {
        if is_nonlocal_url_with_scheme(url, insecure_scheme) {
            report(
                diag,
                LintRule::McpUrlNotHttps,
                &format!("{label}.url uses non-local {insecure_scheme}://; use {secure_scheme}://"),
                source,
                location,
                "url",
                &format!("use a {secure_scheme}:// URL for this non-local server"),
            );
        }
    }
}

fn has_nonempty_string(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
}

fn literal_secret_key(env: Option<&Value>) -> Option<&str> {
    env.and_then(Value::as_object).and_then(|env| {
        env.iter().find_map(|(key, value)| {
            let is_literal = value.as_str().is_some_and(|value| {
                !value.trim().is_empty() && !value.starts_with('$') && !value.starts_with("{{")
            });
            (RE_SECRET_ENV_KEY.is_match(key) && is_literal).then_some(key.as_str())
        })
    })
}

fn report(
    diag: &mut DiagnosticCollector,
    rule: LintRule,
    message: &str,
    source: Option<&str>,
    location: Option<&Range<usize>>,
    evidence: &str,
    suggestion: &str,
) {
    let metadata = source.map_or_else(DiagnosticMetadata::default, |source| {
        metadata(source, location, evidence, suggestion)
    });
    diag.report_with(rule, message, metadata);
}

fn metadata(
    source: &str,
    location: Option<&Range<usize>>,
    evidence: &str,
    suggestion: &str,
) -> DiagnosticMetadata {
    let metadata = location
        .and_then(|range| SourceSpan::from_byte_range(source, range.clone()))
        .map_or_else(DiagnosticMetadata::default, |span| {
            DiagnosticMetadata::default().with_location(span)
        });
    metadata.with_evidence(evidence).with_suggestion(suggestion)
}

fn field_value<'a>(token: Option<&'a ServerTokens>, field: &str) -> Option<&'a Range<usize>> {
    token.and_then(|token| token.fields.get(field).map(|field| &field.value))
}

fn invalid_arg_token<'a>(
    token: Option<&'a ServerTokens>,
    args: &Value,
) -> Option<&'a Range<usize>> {
    let index = args
        .as_array()?
        .iter()
        .position(|value| !value.is_string())?;
    token
        .and_then(|token| token.fields.get("args"))
        .and_then(|field| field.elements.get(index))
}

fn eof_location(content: &str) -> SourceSpan {
    let (line, column) = position_at_offset(content, content.len());
    SourceSpan::point(line, column)
}

fn position_at_offset(content: &str, offset: usize) -> (usize, usize) {
    let prefix = &content[..offset];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit_once('\n')
        .map_or(prefix.chars().count() + 1, |(_, tail)| {
            tail.chars().count() + 1
        });
    (line, column)
}

#[derive(Debug, Default)]
struct RawMcpTokens {
    servers: BTreeMap<String, ServerTokens>,
    duplicates: Vec<DuplicateServer>,
}

#[derive(Debug)]
struct DuplicateServer {
    name: String,
    first_key: Range<usize>,
    duplicate_key: Range<usize>,
}

#[derive(Debug)]
struct ServerTokens {
    key: Range<usize>,
    fields: BTreeMap<String, FieldTokens>,
}

impl ServerTokens {
    fn env_key(&self, name: &str) -> Option<&Range<usize>> {
        self.fields
            .get("env")
            .and_then(|field| field.env.get(name))
            .map(|token| &token.key)
    }
}

#[derive(Debug)]
struct FieldTokens {
    value: Range<usize>,
    elements: Vec<Range<usize>>,
    env: BTreeMap<String, EnvToken>,
}

#[derive(Debug)]
struct EnvToken {
    key: Range<usize>,
}

impl RawMcpTokens {
    fn parse(content: &str) -> Self {
        let mut scanner = TokenScanner::new(content.as_bytes());
        scanner.scan_root();
        scanner.tokens
    }
}

struct TokenScanner<'a> {
    input: &'a [u8],
    pos: usize,
    tokens: RawMcpTokens,
}

impl<'a> TokenScanner<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self {
            input,
            pos: 0,
            tokens: RawMcpTokens::default(),
        }
    }

    fn scan_root(&mut self) {
        self.skip_ws();
        if self.input.get(self.pos) == Some(&b'{') {
            self.scan_root_object();
        } else {
            self.scan_value();
        }
    }

    fn scan_root_object(&mut self) {
        self.pos += 1;
        loop {
            self.skip_ws();
            if self.consume(b'}') {
                return;
            }
            let (key, _) = self.scan_string();
            self.skip_ws();
            self.consume(b':');
            self.skip_ws();
            if key == "mcpServers" {
                self.tokens.servers.clear();
                self.tokens.duplicates.clear();
                if self.input.get(self.pos) == Some(&b'{') {
                    self.scan_servers_object();
                } else {
                    self.scan_value();
                }
            } else {
                self.scan_value();
            }
            self.skip_ws();
            self.consume(b',');
        }
    }

    fn scan_servers_object(&mut self) {
        self.pos += 1;
        let mut first_keys: HashMap<String, Range<usize>> = HashMap::new();
        loop {
            self.skip_ws();
            if self.consume(b'}') {
                return;
            }
            let (name, key) = self.scan_string();
            self.skip_ws();
            self.consume(b':');
            self.skip_ws();
            let fields = if self.input.get(self.pos) == Some(&b'{') {
                self.scan_server_object()
            } else {
                self.scan_value();
                BTreeMap::new()
            };
            if let Some(first_key) = first_keys.get(&name) {
                self.tokens.duplicates.push(DuplicateServer {
                    name: name.clone(),
                    first_key: first_key.clone(),
                    duplicate_key: key.clone(),
                });
            } else {
                first_keys.insert(name.clone(), key.clone());
            }
            self.tokens
                .servers
                .insert(name, ServerTokens { key, fields });
            self.skip_ws();
            self.consume(b',');
        }
    }

    fn scan_server_object(&mut self) -> BTreeMap<String, FieldTokens> {
        self.pos += 1;
        let mut fields = BTreeMap::new();
        loop {
            self.skip_ws();
            if self.consume(b'}') {
                return fields;
            }
            let (name, _) = self.scan_string();
            self.skip_ws();
            self.consume(b':');
            self.skip_ws();
            let start = self.pos;
            let (elements, env) = match name.as_str() {
                "args" if self.input.get(self.pos) == Some(&b'[') => {
                    (self.scan_array_elements(), BTreeMap::new())
                }
                "env" if self.input.get(self.pos) == Some(&b'{') => {
                    (Vec::new(), self.scan_env_object())
                }
                _ => {
                    self.scan_value();
                    (Vec::new(), BTreeMap::new())
                }
            };
            fields.insert(
                name,
                FieldTokens {
                    value: start..self.pos,
                    elements,
                    env,
                },
            );
            self.skip_ws();
            self.consume(b',');
        }
    }

    fn scan_env_object(&mut self) -> BTreeMap<String, EnvToken> {
        self.pos += 1;
        let mut entries = BTreeMap::new();
        loop {
            self.skip_ws();
            if self.consume(b'}') {
                return entries;
            }
            let (name, key) = self.scan_string();
            self.skip_ws();
            self.consume(b':');
            self.scan_value();
            entries.insert(name, EnvToken { key });
            self.skip_ws();
            self.consume(b',');
        }
    }

    fn scan_array_elements(&mut self) -> Vec<Range<usize>> {
        self.pos += 1;
        let mut elements = Vec::new();
        loop {
            self.skip_ws();
            if self.consume(b']') {
                return elements;
            }
            let start = self.pos;
            self.scan_value();
            elements.push(start..self.pos);
            self.skip_ws();
            self.consume(b',');
        }
    }

    fn scan_value(&mut self) {
        self.skip_ws();
        match self.input.get(self.pos) {
            Some(b'{') => self.scan_object(),
            Some(b'[') => self.scan_array(),
            Some(b'"') => {
                self.scan_string();
            }
            _ => self.scan_scalar(),
        }
    }

    fn scan_object(&mut self) {
        self.pos += 1;
        loop {
            self.skip_ws();
            if self.consume(b'}') {
                return;
            }
            self.scan_string();
            self.skip_ws();
            self.consume(b':');
            self.scan_value();
            self.skip_ws();
            self.consume(b',');
        }
    }

    fn scan_array(&mut self) {
        self.pos += 1;
        loop {
            self.skip_ws();
            if self.consume(b']') {
                return;
            }
            self.scan_value();
            self.skip_ws();
            self.consume(b',');
        }
    }

    fn scan_string(&mut self) -> (String, Range<usize>) {
        let start = self.pos;
        self.pos += 1;
        while self.pos < self.input.len() {
            match self.input[self.pos] {
                b'\\' => self.pos += 2,
                b'"' => {
                    self.pos += 1;
                    break;
                }
                _ => self.pos += 1,
            }
        }
        let range = start..self.pos;
        (
            serde_json::from_slice(&self.input[range.clone()]).expect("validated JSON string"),
            range,
        )
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
    fn consume(&mut self, byte: u8) -> bool {
        if self.input.get(self.pos) == Some(&byte) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
}

/// Serde intentionally keeps only the last duplicate key. This scanner runs
/// after parsing succeeds so P023/P027 retain raw-key identity and spans.
struct RawMcpKeys {
    server_maps: Vec<RawServerMap>,
    server_entries: Vec<RawServerEntry>,
}

struct RawServerMap {
    key_range: std::ops::Range<usize>,
    value_range: std::ops::Range<usize>,
    value_is_object: bool,
    entries: std::ops::Range<usize>,
}

struct RawServerEntry {
    name: String,
    value_range: std::ops::Range<usize>,
    value_is_object: bool,
}

fn scan_mcp_keys(content: &str) -> RawMcpKeys {
    let mut scanner = JsonScanner::new(content.as_bytes());
    scanner.scan_value(ScanObject::TopLevel);
    RawMcpKeys {
        server_maps: scanner.server_maps,
        server_entries: scanner.server_entries,
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
    server_maps: Vec<RawServerMap>,
    server_entries: Vec<RawServerEntry>,
}

impl<'a> JsonScanner<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self {
            input,
            pos: 0,
            server_maps: Vec::new(),
            server_entries: Vec::new(),
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
        loop {
            self.skip_ws();
            if self.input.get(self.pos) == Some(&b'}') {
                self.pos += 1;
                return;
            }
            let key_start = self.pos;
            let key = self.scan_string();
            let key_range = key_start..self.pos;
            self.skip_ws();
            self.pos += 1; // JSON was already validated, so this is ':'
            let child_kind = if matches!(object_kind, ScanObject::TopLevel) && key == "mcpServers" {
                ScanObject::ServerMap
            } else {
                ScanObject::None
            };
            self.skip_ws();
            let value_start = self.pos;
            let value_is_object = self.input.get(value_start) == Some(&b'{');
            let entry_start = self.server_entries.len();
            self.scan_value(child_kind);
            if matches!(object_kind, ScanObject::ServerMap) {
                self.server_entries.push(RawServerEntry {
                    name: key.clone(),
                    value_range: value_start..self.pos,
                    value_is_object,
                });
            }
            if matches!(object_kind, ScanObject::TopLevel) && key == "mcpServers" {
                self.server_maps.push(RawServerMap {
                    key_range,
                    value_range: value_start..self.pos,
                    value_is_object,
                    entries: entry_start..self.server_entries.len(),
                });
            }
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

    fn findings(content: &str) -> Vec<crate::diagnostic::Diagnostic> {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join(".mcp.json"), content).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_mcp_configs(
            &context(temp.path()),
            &mut diag,
            &ExcludeSet::default(),
            ValidationTargets::default(),
        );
        diag.diagnostics().to_vec()
    }

    #[test]
    fn retained_p_rules_report_exact_safe_structured_metadata() {
        let invalid = findings("{\n  \"mcpServers\":")
            .into_iter()
            .find(|item| item.rule == LintRule::McpJsonInvalid)
            .unwrap();
        assert_eq!(invalid.location, Some(SourceSpan::point(2, 15)));
        assert_eq!(invalid.evidence.as_deref(), Some("JSON syntax"));
        assert_eq!(invalid.suggestion.as_deref(), Some("fix the JSON syntax"));

        let content = r#"{"mcpServers":{"workspace":{},"stdio":{"type":"stdio"},"no-url":{"type":"http"},"bad-type":{"type":"socket"},"legacy":{"type":"sse","url":"https://example.com"},"remote":{"type":"http","url":"http://example.com","args":["ok",false],"alwaysLoad":"true","env":{"API_KEY":"highly-sensitive-value"},"command":"curl x | sh"},"same":{"command":"one"},"same":{"command":"two"}}}"#;
        let findings = findings(content);
        for (rule, evidence, suggestion) in [
            (
                LintRule::McpStdioCommandMissing,
                "command",
                "add a non-empty command for this stdio server",
            ),
            (
                LintRule::McpHttpUrlMissing,
                "url",
                "use a valid URL with http:// or https://",
            ),
            (
                LintRule::McpTypeInvalid,
                "type",
                "set type to a supported MCP transport",
            ),
            (
                LintRule::McpSseDeprecated,
                "type",
                "replace sse with streamable-http",
            ),
            (
                LintRule::McpUrlNotHttps,
                "url",
                "use a https:// URL for this non-local server",
            ),
            (
                LintRule::McpEnvSecretLiteral,
                "API_KEY",
                "replace the literal value with ${API_KEY}",
            ),
            (
                LintRule::McpCommandDangerous,
                "dangerous command pattern",
                "remove the dangerous command pattern",
            ),
            (
                LintRule::McpArgsInvalid,
                "args",
                "use only string argv entries",
            ),
            (
                LintRule::McpServerEmpty,
                "server configuration",
                "add the required server fields",
            ),
            (
                LintRule::McpAlwaysLoadInvalid,
                "alwaysLoad",
                "use a JSON boolean",
            ),
            (
                LintRule::McpServerReserved,
                "workspace",
                "rename this server to a non-reserved name",
            ),
        ] {
            let diagnostic = findings.iter().find(|item| item.rule == rule).unwrap();
            assert!(diagnostic.location.is_some(), "{rule:?}");
            assert_eq!(diagnostic.evidence.as_deref(), Some(evidence), "{rule:?}");
            assert_eq!(
                diagnostic.suggestion.as_deref(),
                Some(suggestion),
                "{rule:?}"
            );
        }
        let duplicate = findings
            .iter()
            .find(|item| item.rule == LintRule::McpDuplicateServer)
            .unwrap();
        assert!(duplicate.location.is_some());
        assert_eq!(
            duplicate.evidence.as_deref(),
            Some("first defined at line 1, column 321")
        );
        assert_eq!(
            duplicate.suggestion.as_deref(),
            Some("remove or rename this duplicate server key")
        );
        let secret = findings
            .iter()
            .find(|item| item.rule == LintRule::McpEnvSecretLiteral)
            .unwrap();
        assert!(!secret.message.contains("highly-sensitive-value"));
        assert!(
            !secret
                .evidence
                .as_deref()
                .unwrap()
                .contains("highly-sensitive-value")
        );
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
    fn inline_plugin_map_is_optional_but_present_invalid_shapes_are_p027() {
        let temp = tempfile::tempdir().unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_mcp_configs(
            &LintContext {
                base_path: temp.path().to_path_buf(),
                mode: LintMode::Plugin,
                plugin_json: ManifestState::Parsed(serde_json::json!({"name": "example"})),
                marketplace_json: ManifestState::Missing,
                hooks_json: ManifestState::Missing,
                settings_json: ManifestState::Missing,
                settings_local_json: ManifestState::Missing,
            },
            &mut diag,
            &ExcludeSet::default(),
            ValidationTargets::default(),
        );
        assert!(diag.diagnostics().is_empty());

        let mut invalid = DiagnosticCollector::new_all_enabled();
        validate_mcp_configs(
            &LintContext {
                base_path: temp.path().to_path_buf(),
                mode: LintMode::Plugin,
                plugin_json: ManifestState::Parsed(serde_json::json!({"mcpServers": []})),
                marketplace_json: ManifestState::Missing,
                hooks_json: ManifestState::Missing,
                settings_json: ManifestState::Missing,
                settings_local_json: ManifestState::Missing,
            },
            &mut invalid,
            &ExcludeSet::default(),
            ValidationTargets::default(),
        );
        assert_eq!(
            invalid
                .diagnostics()
                .iter()
                .map(|diagnostic| diagnostic.rule)
                .collect::<Vec<_>>(),
            vec![LintRule::McpStructureInvalid]
        );
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
            1
        );
        assert_eq!(
            rules
                .iter()
                .filter(|rule| **rule == LintRule::McpServerEmpty)
                .count(),
            1
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
    fn structural_shapes_have_one_canonical_diagnostic() {
        for content in [
            "{}",
            "[]",
            r#"{"mcpServers":null}"#,
            r#"{"mcpServers":"servers"}"#,
            r#"{"mcpServers":[]}"#,
        ] {
            assert_eq!(
                reported_rules(content, ".mcp.json"),
                vec![LintRule::McpStructureInvalid],
                "{content}"
            );
        }
        assert!(reported_rules(r#"{"mcpServers":{}}"#, ".mcp.json").is_empty());

        for value in [r#"null"#, r#"[]"#, r#""server""#] {
            let content = format!(r#"{{"mcpServers":{{"bad":{value}}}}}"#);
            assert_eq!(
                reported_rules(&content, ".mcp.json"),
                vec![LintRule::McpStructureInvalid],
                "{content}"
            );
        }
        assert_eq!(
            reported_rules(r#"{"mcpServers":{"empty":{}}}"#, ".mcp.json"),
            vec![LintRule::McpServerEmpty]
        );
    }

    #[test]
    fn standalone_missing_map_has_suggestion_and_invalid_map_has_span() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join(".mcp.json"), "{}").unwrap();
        let mut missing = DiagnosticCollector::new_all_enabled();
        validate_mcp_configs(
            &context(temp.path()),
            &mut missing,
            &ExcludeSet::default(),
            ValidationTargets::default(),
        );
        let missing = missing.diagnostics().first().unwrap();
        assert_eq!(missing.rule, LintRule::McpStructureInvalid);
        assert_eq!(
            missing.suggestion.as_deref(),
            Some("add a top-level mcpServers object")
        );
        assert_eq!(missing.location, None);

        std::fs::write(temp.path().join(".mcp.json"), r#"{"mcpServers":[]}"#).unwrap();
        let mut invalid = DiagnosticCollector::new_all_enabled();
        validate_mcp_configs(
            &context(temp.path()),
            &mut invalid,
            &ExcludeSet::default(),
            ValidationTargets::default(),
        );
        let invalid = invalid.diagnostics().first().unwrap();
        assert_eq!(invalid.rule, LintRule::McpStructureInvalid);
        assert_eq!(
            invalid.location,
            Some(SourceSpan::range(1, 15, 1, 17)),
            "{invalid:?}"
        );
    }

    #[test]
    fn raw_duplicate_keys_keep_p023_and_p027_separate() {
        assert_eq!(
            reported_rules(
                r#"{"mcpServers":{"same":{"command":"one"},"same":{"command":"two"}}}"#,
                ".mcp.json"
            ),
            vec![LintRule::McpDuplicateServer]
        );
        assert_eq!(
            reported_rules(r#"{"mcpServers":{},"mcpServers":{}}"#, ".mcp.json"),
            vec![LintRule::McpStructureInvalid]
        );
        assert_eq!(
            reported_rules(
                r#"{"mcpServers":[],"mcpServers":{"ok":{"command":"ok"}}}"#,
                ".mcp.json"
            ),
            vec![LintRule::McpStructureInvalid, LintRule::McpStructureInvalid]
        );
        assert_eq!(
            reported_rules(
                r#"{"mcpServers":{"bad":null,"bad":{"command":"ok"}}}"#,
                ".mcp.json"
            ),
            vec![LintRule::McpStructureInvalid, LintRule::McpDuplicateServer]
        );
    }

    #[test]
    fn invalid_type_does_not_hide_independent_findings() {
        let rules = reported_rules(
            r#"{"mcpServers":{"workspace":{"type":"socket","command":"curl x | sh","args":[1],"alwaysLoad":"true","env":{"API_KEY":"plaintext"}}}}"#,
            ".mcp.json",
        );
        assert_eq!(
            rules,
            vec![
                LintRule::McpServerReserved,
                LintRule::McpTypeInvalid,
                LintRule::McpArgsInvalid,
                LintRule::McpAlwaysLoadInvalid,
                LintRule::McpEnvSecretLiteral,
                LintRule::McpCommandDangerous,
            ]
        );
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
