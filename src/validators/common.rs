use regex::Regex;
use std::sync::LazyLock;
use url::{Host, Url};

/// Shared regex: matches characters outside [a-z0-9-] in names.
/// Used by skill_content name validation and agents name validation.
pub(crate) static RE_NAME_INVALID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^a-z0-9-]").unwrap());

/// Shared regex: matches TODO/FIXME/HACK/XXX markers (case-insensitive).
/// Used by hygiene TODO scanning and docs TODO scanning.
/// Note: docs.rs previously defined this as `RE_TODO` with the same pattern.
pub(crate) static RE_TODO_MARKER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(TODO|FIXME|HACK|XXX)\b").unwrap());

/// Shared enum: accepted values for a `shell` field.
/// Used by skill_content frontmatter validation (S026) and hook schema
/// validation (H022), which must stay in agreement.
pub(crate) const VALID_SHELLS: &[&str] = &["bash", "powershell"];

/// Lexical category for one whitespace-free inline-code token.
///
/// I003 and D005 intentionally share this classifier. Their validation scopes
/// differ (D005 additionally requires a configured repository-path prefix),
/// but equivalent tokens must not drift between path and non-path treatment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InlineCodePathKind {
    ConcreteRelativePath,
    Dotfile,
    ExtensionOrGlob,
    UrlVariableOrPlaceholder,
    NonPath,
}

impl InlineCodePathKind {
    pub(crate) fn is_repository_path(self) -> bool {
        matches!(self, Self::ConcreteRelativePath | Self::Dotfile)
    }
}

/// Bare extension markers that are common in prose and are not dotfile paths.
///
/// Other single-segment dot-prefixed tokens are classified as dotfiles and
/// resolved against the filesystem. This deliberately makes `.env`,
/// `.gitignore`, `.cursorrules`, and `.mcp.json` existence-sensitive while
/// keeping language/file-format markers such as `.py` as hard negatives.
const BARE_EXTENSION_MARKERS: &[&str] = &[".py", ".md", ".json", ".toml", ".yaml", ".yml", ".rs"];

pub(crate) fn classify_inline_code_path(token: &str) -> InlineCodePathKind {
    if token.is_empty() || token.contains(char::is_whitespace) {
        return InlineCodePathKind::NonPath;
    }
    if token.contains("://")
        || token.starts_with("mailto:")
        || token.starts_with("//")
        || token.contains(['$', '{', '}', '<', '>'])
    {
        return InlineCodePathKind::UrlVariableOrPlaceholder;
    }
    if token.contains(['*', '?']) || BARE_EXTENSION_MARKERS.contains(&token) {
        return InlineCodePathKind::ExtensionOrGlob;
    }
    if token.starts_with("./") || token.contains('/') {
        return InlineCodePathKind::ConcreteRelativePath;
    }
    if token.starts_with('.') && token.len() > 1 {
        return InlineCodePathKind::Dotfile;
    }
    if token.rsplit_once('.').is_some() {
        return InlineCodePathKind::ConcreteRelativePath;
    }
    InlineCodePathKind::NonPath
}

/// Model aliases accepted by Claude Code `/model` plus skill-only `inherit`.
/// Full Anthropic model IDs (`claude-…`) are also accepted.
/// Shared by skill frontmatter (S063) and future agent frontmatter validation.
const MODEL_ALIASES: &[&str] = &[
    "inherit",
    "default",
    "best",
    "sonnet",
    "opus",
    "haiku",
    "fable",
    "sonnet[1m]",
    "opus[1m]",
    "opusplan",
    "opusplan[1m]",
    "fable[1m]",
];

/// Return true if `value` is a recognized Claude Code model alias or ID.
pub(crate) fn is_valid_model_value(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() {
        return false;
    }
    if MODEL_ALIASES.contains(&value) {
        return true;
    }
    // Full Anthropic model IDs and version pins (e.g. claude-sonnet-4-5, claude-opus-4-6).
    value.starts_with("claude-")
}

/// Built-in Claude Code tool names (PascalCase). Shared by S040
/// (skill `allowed-tools`) and A019/A020 (agent `tools`/`disallowedTools`).
pub(crate) const KNOWN_TOOLS: &[&str] = &[
    "AskUserQuestion",
    "Bash",
    "Read",
    "Edit",
    "Write",
    "Grep",
    "Glob",
    "Agent",
    "Task",
    "WebFetch",
    "WebSearch",
    "Skill",
    "NotebookEdit",
    "LSP",
    "TaskCreate",
    "TaskUpdate",
    "TaskList",
    "TaskGet",
    "TaskStop",
    "TaskOutput",
];

/// Whether a single tool entry is recognized by Claude Code.
///
/// Accepts:
/// - Built-in tool names (see [`KNOWN_TOOLS`]), case-sensitive PascalCase.
/// - Tool-restricted forms like `Bash(git *)` — the `(...)` suffix is ignored.
/// - MCP tools written as `mcp__<server>__<tool>` (server and tool both required).
///
/// Returns `false` for empty input so callers can skip blank entries.
pub(crate) fn is_known_tool_name(tool: &str) -> bool {
    let tool = tool.trim();
    if tool.is_empty() {
        return false;
    }
    // Strip a trailing argument-restriction suffix, e.g. "Bash(git *)" -> "Bash".
    let base_name = match tool.find('(') {
        Some(paren) => tool[..paren].trim(),
        None => tool,
    };
    if base_name.is_empty() {
        return false;
    }
    // MCP tools: mcp__<server>__<tool> — both parts must be non-empty.
    if let Some(rest) = base_name.strip_prefix("mcp__") {
        return !rest.is_empty()
            && !rest.starts_with("__")
            && !rest.ends_with("__")
            && rest.contains("__");
    }
    KNOWN_TOOLS.contains(&base_name)
}

/// Whether `value` is an absolute HTTP(S) URL with a parsed host.
pub(crate) fn is_valid_http_url(value: &str) -> bool {
    Url::parse(value)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host().is_some())
}

fn is_local_host(host: Host<&str>) -> bool {
    match host {
        Host::Domain(host) => host == "localhost",
        Host::Ipv4(address) => address.is_loopback() || address.is_unspecified(),
        Host::Ipv6(address) => {
            address.is_loopback()
                || address.is_unspecified()
                || address
                    .to_ipv4_mapped()
                    .is_some_and(|address| address.is_loopback() || address.is_unspecified())
        }
    }
}

/// Whether `value` is an HTTP URL whose parsed host is not local.
pub(crate) fn is_nonlocal_http_url(value: &str) -> bool {
    Url::parse(value).is_ok_and(|url| {
        url.scheme() == "http" && url.host().is_some_and(|host| !is_local_host(host))
    })
}

#[cfg(test)]
mod model_tests {
    use super::is_valid_model_value;

    #[test]
    fn accepts_aliases_and_full_ids() {
        assert!(is_valid_model_value("sonnet"));
        assert!(is_valid_model_value("opus[1m]"));
        assert!(is_valid_model_value("inherit"));
        assert!(is_valid_model_value("claude-sonnet-4-5"));
        assert!(is_valid_model_value("claude-opus-4-6"));
    }

    #[test]
    fn rejects_typos_and_empty() {
        assert!(!is_valid_model_value("sonet"));
        assert!(!is_valid_model_value(""));
        assert!(!is_valid_model_value("gpt-4"));
    }
}

#[cfg(test)]
mod tool_tests {
    use super::is_known_tool_name;

    #[test]
    fn accepts_builtin_tools() {
        assert!(is_known_tool_name("Bash"));
        assert!(is_known_tool_name("Read"));
        assert!(is_known_tool_name("Agent"));
    }

    #[test]
    fn accepts_restricted_form() {
        assert!(is_known_tool_name("Bash(git *)"));
        assert!(is_known_tool_name("Grep(pat)"));
    }

    #[test]
    fn accepts_mcp_tools() {
        assert!(is_known_tool_name("mcp__server__tool"));
        assert!(is_known_tool_name("mcp__my_server__my_tool"));
    }

    #[test]
    fn rejects_mcp_malformed() {
        assert!(!is_known_tool_name("mcp__"));
        assert!(!is_known_tool_name("mcp__tool"));
        assert!(!is_known_tool_name("mcp__server__"));
    }

    #[test]
    fn rejects_unknown_tools() {
        assert!(!is_known_tool_name("UnknownTool"));
        assert!(!is_known_tool_name(""));
    }
}

#[cfg(test)]
mod url_tests {
    use super::{is_nonlocal_http_url, is_valid_http_url};

    #[test]
    fn accepts_valid_http_urls_across_host_forms() {
        for value in [
            "https://example.com:8443/path?query=value#fragment",
            "http://user:password@example.com",
            "https://b\u{fc}cher.example",
            "http://127.0.0.1",
            "https://[::1]:3000",
        ] {
            assert!(is_valid_http_url(value), "expected {value} to be valid");
        }
    }

    #[test]
    fn rejects_urls_without_a_valid_http_host() {
        for value in [
            "ftp://example.com",
            "example.com",
            "https://",
            "https://example.com:invalid",
            "https://example .com",
        ] {
            assert!(!is_valid_http_url(value), "expected {value} to be invalid");
        }
    }

    #[test]
    fn identifies_only_remote_http_urls_as_nonlocal() {
        for value in [
            "http://example.com",
            "http://user:password@b\u{fc}cher.example:8080",
            "http://[2001:db8::1]",
        ] {
            assert!(
                is_nonlocal_http_url(value),
                "expected {value} to be non-local"
            );
        }
        for value in [
            "https://example.com",
            "http://localhost",
            "http://127.1.2.3",
            "http://0.0.0.0",
            "http://[::1]",
            "http://[::]",
            "http://[::ffff:127.0.0.1]",
            "http://",
        ] {
            assert!(
                !is_nonlocal_http_url(value),
                "expected {value} to be local or invalid"
            );
        }
    }
}

#[cfg(test)]
mod inline_code_path_tests {
    use super::{InlineCodePathKind, classify_inline_code_path};

    #[test]
    fn classifies_concrete_relative_paths() {
        for token in [
            "missing.md",
            "docs/missing.md",
            "./missing",
            "nested/path/missing.json",
        ] {
            assert_eq!(
                classify_inline_code_path(token),
                InlineCodePathKind::ConcreteRelativePath,
                "expected {token} to be a concrete path"
            );
        }
    }

    #[test]
    fn classifies_dotfiles_separately_from_bare_extensions() {
        for token in [".env", ".gitignore", ".cursorrules", ".mcp.json"] {
            assert_eq!(
                classify_inline_code_path(token),
                InlineCodePathKind::Dotfile,
                "expected {token} to be a dotfile"
            );
        }
        for token in [
            ".py", "*.py", ".md", ".json", ".toml", ".yaml", ".yml", ".rs",
        ] {
            assert_eq!(
                classify_inline_code_path(token),
                InlineCodePathKind::ExtensionOrGlob,
                "expected {token} to be extension/glob notation"
            );
        }
    }

    #[test]
    fn preserves_url_variable_placeholder_and_whitespace_exclusions() {
        for token in [
            "https://example.com/file.md",
            "$FILE",
            "${ROOT}/file.md",
            "<path/to/file.md>",
        ] {
            assert_eq!(
                classify_inline_code_path(token),
                InlineCodePathKind::UrlVariableOrPlaceholder,
                "expected {token} to be excluded syntax"
            );
        }
        for token in ["", "two words.md", "README"] {
            assert_eq!(
                classify_inline_code_path(token),
                InlineCodePathKind::NonPath,
                "expected {token:?} to be a non-path"
            );
        }
    }
}
