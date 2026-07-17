use crate::config::ExcludeSet;
use crate::context::LintContext;
use crate::diagnostic::DiagnosticCollector;
use crate::rules::LintRule;
use crate::traversal;
use regex::Regex;
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

const RESERVED_SERVER_NAMES: &[&str] = &["workspace"];

static RE_SECRET_ENV_KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:api[_-]?key|secret|token|password)").expect("valid secret-key regex")
});
static RE_DANGEROUS_COMMAND: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:\b(?:curl|wget)\b[^\n|]*\|\s*(?:ba)?sh\b|\bsudo\s+rm\b|\brm\s+-[a-z]*r[a-z]*f[a-z]*\s+/(?:\s|$))")
        .expect("valid dangerous-command regex")
});

/// Validate MCP configuration files found in the repository. MCP files are
/// optional; this only reports diagnostics for files that are present.
pub fn validate_mcp_configs(
    ctx: &LintContext,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
) {
    for path in mcp_config_paths(ctx) {
        let display = display_path(&ctx.base_path, &path);
        if exclude.is_excluded(&display) {
            continue;
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) => {
                diag.report(
                    LintRule::McpJsonInvalid,
                    &format!("{display} cannot be read: {error}"),
                );
                continue;
            }
        };
        let value: Value = match serde_json::from_str(&content) {
            Ok(value) => value,
            Err(error) => {
                diag.report(
                    LintRule::McpJsonInvalid,
                    &format!("{display} is not valid JSON: {error}"),
                );
                continue;
            }
        };

        for name in duplicate_mcp_server_names(&content) {
            diag.report(
                LintRule::McpDuplicateServer,
                &format!("{display}: mcpServers contains duplicate server name '{name}'"),
            );
        }
        validate_servers(&display, value.get("mcpServers"), diag);
    }
}

fn mcp_config_paths(ctx: &LintContext) -> Vec<std::path::PathBuf> {
    let mut paths = HashSet::new();
    for entry in traversal::recursive_files(&ctx.base_path, &ctx.base_path, None).entries {
        let name = entry.path.file_name().unwrap_or_default().to_string_lossy();
        if name == ".mcp.json"
            || name.ends_with(".mcp.json")
            || entry.path == ctx.base_path.join(".claude/settings.json")
            || entry.path == ctx.base_path.join(".claude/settings.local.json")
        {
            paths.insert(entry.path);
        }
    }
    let mut paths: Vec<_> = paths.into_iter().collect();
    paths.sort();
    paths
}

fn display_path(base: &Path, path: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn validate_servers(display: &str, servers: Option<&Value>, diag: &mut DiagnosticCollector) {
    let Some(servers) = servers.and_then(Value::as_object) else {
        return;
    };

    for (name, config) in servers {
        let label = format!("{display}: mcpServers.{name}");
        if RESERVED_SERVER_NAMES.contains(&name.as_str()) {
            diag.report(
                LintRule::McpServerReserved,
                &format!("{label} uses reserved server name '{name}'"),
            );
        }
        let Some(config) = config.as_object() else {
            diag.report(
                LintRule::McpServerEmpty,
                &format!("{label} must be a non-empty object"),
            );
            continue;
        };
        if config.is_empty() {
            diag.report(
                LintRule::McpServerEmpty,
                &format!("{label} must not be an empty object"),
            );
            continue;
        }

        let transport = match config.get("type") {
            None => "stdio",
            Some(Value::String(kind)) if matches!(kind.as_str(), "stdio" | "http" | "sse") => {
                kind.as_str()
            }
            Some(_) => {
                diag.report(
                    LintRule::McpTypeInvalid,
                    &format!("{label}.type must be 'stdio', 'http', or 'sse'"),
                );
                continue;
            }
        };
        match transport {
            "stdio" if !has_nonempty_string(config.get("command")) => diag.report(
                LintRule::McpStdioCommandMissing,
                &format!("{label}: stdio server requires a non-empty command"),
            ),
            "http" | "sse" if !has_nonempty_string(config.get("url")) => diag.report(
                LintRule::McpHttpUrlMissing,
                &format!("{label}: {transport} server requires a non-empty url"),
            ),
            _ => {}
        }
        if transport == "sse" {
            diag.report(
                LintRule::McpSseDeprecated,
                &format!("{label}: SSE transport is deprecated; use Streamable HTTP"),
            );
        }
        if let Some(url) = config.get("url").and_then(Value::as_str) {
            if is_nonlocal_http(url) {
                diag.report(
                    LintRule::McpUrlNotHttps,
                    &format!("{label}.url uses non-local http://; use HTTPS"),
                );
            }
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
        if config
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

fn has_nonempty_string(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
}

fn is_nonlocal_http(url: &str) -> bool {
    let Some(authority) = url.strip_prefix("http://") else {
        return false;
    };
    let host_port = authority.split('/').next().unwrap_or_default();
    let host_port = host_port.rsplit('@').next().unwrap_or_default();
    let host = if let Some(bracketed) = host_port.strip_prefix('[') {
        bracketed.split(']').next().unwrap_or_default()
    } else {
        host_port.split(':').next().unwrap_or(host_port)
    };
    !matches!(host, "localhost" | "127.0.0.1" | "::1")
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
/// runs only after JSON parsing succeeds, preserving raw duplicate server names.
fn duplicate_mcp_server_names(content: &str) -> Vec<String> {
    let mut scanner = JsonScanner::new(content.as_bytes());
    scanner.scan_value(false);
    scanner.duplicates
}

struct JsonScanner<'a> {
    input: &'a [u8],
    pos: usize,
    duplicates: Vec<String>,
}

impl<'a> JsonScanner<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self {
            input,
            pos: 0,
            duplicates: Vec::new(),
        }
    }

    fn scan_value(&mut self, duplicate_keys: bool) {
        self.skip_ws();
        match self.input.get(self.pos) {
            Some(b'{') => self.scan_object(duplicate_keys),
            Some(b'[') => self.scan_array(),
            Some(b'\"') => {
                self.scan_string();
            }
            _ => self.scan_scalar(),
        }
    }

    fn scan_object(&mut self, duplicate_keys: bool) {
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
            let is_servers = key == "mcpServers";
            if duplicate_keys && !names.insert(key.clone()) {
                self.duplicates.push(key);
            }
            self.scan_value(is_servers);
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
            self.scan_value(false);
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
        validate_mcp_configs(&context(temp.path()), &mut diag, &ExcludeSet::default());
        diag.errors()
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
        validate_mcp_configs(&context(temp.path()), &mut diag, &ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn validates_settings_and_settings_local_surfaces() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join(".claude")).unwrap();
        std::fs::write(
            temp.path().join(".claude/settings.json"),
            r#"{"mcpServers":{"a":{"command":"ok"}}}"#,
        )
        .unwrap();
        std::fs::write(
            temp.path().join(".claude/settings.local.json"),
            r#"{"mcpServers":{"b":{"command":"ok"}}}"#,
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_mcp_configs(&context(temp.path()), &mut diag, &ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
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
                .any(|error| error.contains("requires a non-empty url"))
        );
        assert!(
            errors
                .iter()
                .any(|error| error.contains("SSE transport is deprecated"))
        );
        assert!(errors.iter().any(|error| error.contains("must be 'stdio'")));
    }

    #[test]
    fn validates_url_env_command_and_field_types() {
        let errors = diagnostics(
            r#"{"mcpServers":{"bad":{"command":"curl x | sh","url":"http://example.com","args":["ok",1],"alwaysLoad":"true","env":{"API_KEY":"plaintext"}}}}"#,
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
    fn localhost_and_env_references_are_allowed() {
        let errors = diagnostics(
            r#"{"mcpServers":{"local":{"type":"http","url":"http://localhost:3000","env":{"TOKEN":"${TOKEN}"}},"ipv6":{"type":"http","url":"http://[::1]:3000"}}}"#,
            ".mcp.json",
        );
        assert_eq!(errors.len(), 0, "{errors:?}");
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
