use crate::context::ManifestError;
use crate::diagnostic::DiagnosticMetadata;
use regex::Regex;
use std::ops::Range;
use std::path::{Component, Path};
use std::sync::LazyLock;
use url::{Host, Url};

/// Convert a manifest loader failure's structured parse location into the
/// renderer-independent metadata used by every manifest-owning validator.
pub(crate) fn manifest_error_metadata(error: &ManifestError) -> DiagnosticMetadata {
    match error.location() {
        Some(location) => match location.column() {
            Some(column) => DiagnosticMetadata::at_point(location.line(), column),
            None => DiagnosticMetadata::at_line(location.line()),
        },
        None => DiagnosticMetadata::default(),
    }
}

/// Shared regex: matches characters outside [a-z0-9-] in names.
/// Used by skill_content name validation and agents name validation.
pub(crate) static RE_NAME_INVALID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^a-z0-9-]").unwrap());

/// Evidence-integrity prohibition required by A013 and exempted by Q002.
/// Keeping the accepted verbs here prevents the two rule contracts from
/// drifting into contradictory diagnostics.
pub(crate) static NEVER_INVENT_PROHIBITION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:never|do\s+not|don't)\s+(?:invent|fabricate|guess)\b").unwrap()
});

/// Shared recognition vocabulary for concrete retry bounds and stop controls.
///
/// Q005 and A029 deliberately have different applicability and operativity
/// gates, but a phrase must never count as a bound for one while being absent
/// from the other's recognition vocabulary. Keep every numeric count anchored
/// to a control noun so unrelated numbers do not satisfy either rule.
static BOUND_OR_FALLBACK_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    const NUMBER: &str = r"(?:\d+(?:\.\d+)?|one|two|three|four|five|six|seven|eight|nine|ten)";
    const COUNT_UNIT: &str =
        r"(?:attempts?|retries|tries|times|iterations?|tool[ -]?calls?|steps?)";
    const TIME_UNIT: &str = r"(?:milliseconds?|seconds?|minutes?|hours?|ms|secs?|mins?|hrs?)";
    [
        format!(r"(?i)\b(?:at\s+most|no\s+more\s+than|up\s+to|within|after|for|(?:a\s+)?maximum(?:\s+of)?|max)\s+{NUMBER}\s+{COUNT_UNIT}\b"),
        format!(r"(?i)\b(?:limit|cap)\s+(?:of\s+)?{NUMBER}\s+{COUNT_UNIT}\b"),
        format!(r"(?i)\b{NUMBER}\s+{COUNT_UNIT}\s+(?:maximum|max)\b"),
        format!(r"(?i)\b(?:or|after|within|for)\s+{NUMBER}\s+{COUNT_UNIT}\b"),
        format!(r"(?i)\b(?:stop|abort|give\s+up|halt)\b.{{0,80}}\b(?:after|within|for)\s+{NUMBER}\s+(?:{COUNT_UNIT}|{TIME_UNIT})\b"),
        format!(r"(?i)\b(?:timeout|time(?:\s|-)?limit|deadline|time(?:\s|-)?budget|token(?:\s|-)?budget|cost(?:\s|-)?budget|budget)\s*(?:of|:|is)?\s*{NUMBER}\s*(?:{TIME_UNIT}|tokens?|%|usd|dollars?)\b"),
        format!(r"(?i)\b(?:within|for\s+no\s+more\s+than|at\s+most)\s+{NUMBER}\s*{TIME_UNIT}\b"),
        r"(?i)\b(?:token|cost)\s+budget\s*(?:of|:|is)?\s*(?:\$?\d[\d,.]*|\d+[kKmM]?)\b".to_string(),
        r"(?i)\b(?:at\s+most|no\s+more\s+than|up\s+to)\s+(?:\$?\d[\d,.]*|\d+[kKmM]?)\s+(?:tokens?|dollars?|usd)\b".to_string(),
        r"(?i)\b(?:deadline\s*(?:of|:|is)?|by)\s+(?:\d{4}-\d{2}-\d{2}|\d{1,2}:\d{2}\s*(?:am|pm)?|end\s+of\s+(?:day|week)|(?:monday|tuesday|wednesday|thursday|friday|saturday|sunday))\b".to_string(),
        r"(?i)\b(?:if|when|after|on|upon)\b.{0,100}\b(?:fail(?:ed|s|ure)?|no\s+progress|cannot\s+make\s+progress|unable\s+to\s+(?:make\s+progress|continue))\b.{0,120}\b(?:stop|abort|return|report|escalat(?:e|ion)|ask\s+(?:for\s+)?help|handoff|surface|give\s+up|fall\s+back)\b".to_string(),
        r"(?i)\b(?:stop\s+and\s+report|report\s+and\s+stop|escalat(?:e|ion)|ask\s+(?:for\s+)?help|handoff|return)\b.{0,100}\b(?:if|when|after|on|upon)\b.{0,100}\b(?:fail(?:ed|s|ure)?|no\s+progress|cannot\s+make\s+progress|unable\s+to\s+(?:make\s+progress|continue))\b".to_string(),
        r"(?i)\botherwise\s*,?\s*(?:stop|abort|return|report|escalate|surface|give\s+up|fall\s+back)\b".to_string(),
    ]
    .into_iter()
    .map(|pattern| Regex::new(&pattern).expect("shared bound-control regex is valid"))
    .collect()
});

/// Whether `text` contains a concrete bound or failure fallback recognized by
/// both Q005 and A029. Callers remain responsible for their own scope and
/// operativity rules.
pub(crate) fn has_bound_or_fallback(text: &str) -> bool {
    BOUND_OR_FALLBACK_PATTERNS
        .iter()
        .any(|pattern| pattern.is_match(text))
}

/// Strip Markdown emphasis marker runs for A029/Q005 operativity and label
/// gates only.
///
/// Removes runs of `*`, `**`, `***`, `_`, or `__` (and longer same-marker runs)
/// at token boundaries so bold/italic wrappers do not block directive-verb or
/// `Important:`/`Note:`/`Warning:` matching. Mid-word markers such as `a*b` or
/// `snake_case` are left untouched. Diagnostics must keep using the original
/// sentence for evidence and coordinates.
pub(crate) fn normalize_emphasis_for_gates(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut result = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        let marker = chars[index];
        if marker != '*' && marker != '_' {
            result.push(marker);
            index += 1;
            continue;
        }

        let start = index;
        index += 1;
        while index < chars.len() && chars[index] == marker {
            index += 1;
        }

        let before_is_word = start
            .checked_sub(1)
            .is_some_and(|previous| chars[previous].is_alphanumeric());
        let after_is_word = chars.get(index).is_some_and(|next| next.is_alphanumeric());
        if before_is_word && after_is_word {
            result.extend(std::iter::repeat_n(marker, index - start));
        }
    }
    result
}

/// Split prose into sentence ranges without treating the decimal point in a
/// numeric value (for example `1.5 hours`) as a sentence boundary.
pub(crate) fn sentence_ranges(text: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0;
    for (index, character) in text.char_indices() {
        if !matches!(character, '.' | '!' | '?' | ';') {
            continue;
        }
        let is_decimal_point = character == '.'
            && text[..index]
                .chars()
                .next_back()
                .is_some_and(|previous| previous.is_ascii_digit())
            && text[index + character.len_utf8()..]
                .chars()
                .next()
                .is_some_and(|next| next.is_ascii_digit());
        if is_decimal_point {
            continue;
        }
        let end = index + character.len_utf8();
        ranges.push(start..end);
        start = end;
    }
    if start < text.len() {
        ranges.push(start..text.len());
    }
    ranges
}

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

/// Established dotfile and dot-directory names that remain filesystem references even when they
/// overlap the lexical shape of a bare extension marker.
///
/// This is intentionally a dotfile policy, not an extension allowlist: the
/// extension classifier below accepts any conservative lexical extension form.
const WELL_KNOWN_DOT_ENTRIES: &[&str] = &[
    ".env",
    ".git",
    ".gitignore",
    ".claude",
    ".claude-plugin",
    ".github",
    ".vscode",
    ".codex",
    ".cursor",
    ".venv",
    ".husky",
    ".idea",
    ".devcontainer",
    ".cursorrules",
    ".mcp.json",
    ".editorconfig",
    ".dockerignore",
    ".npmrc",
    ".nvmrc",
    ".prettierrc",
    ".eslintrc",
    ".babelrc",
    ".stylelintrc",
    ".tool-versions",
];

/// Whether `token` is conservative bare-extension prose rather than a path.
///
/// The token must be a single leading dot followed by one to twelve lowercase
/// ASCII alphanumeric characters. That covers conventional source and format
/// extensions (including `.c`, `.cpp`, `.html`, `.properties`, and `.tsx`) without treating
/// punctuation-bearing, uppercase, or long dot-prefixed names as extensions.
/// Known dotfiles take precedence in [`classify_inline_code_path`].
fn is_bare_extension_marker(token: &str) -> bool {
    let Some(extension) = token.strip_prefix('.') else {
        return false;
    };
    is_extension_component(extension)
}

/// Whether one dotted suffix component has the conservative extension shape.
///
/// This remains deliberately lexical: it is shared by bare extension notation
/// and filename-shaped paths so numeric version components do not become paths.
fn is_extension_component(component: &str) -> bool {
    (1..=12).contains(&component.len())
        && component
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase())
        && component
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
}

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
    if token.contains(['*', '?']) {
        return InlineCodePathKind::ExtensionOrGlob;
    }
    if token.starts_with("./") || token.contains('/') {
        return InlineCodePathKind::ConcreteRelativePath;
    }
    if token.starts_with('.') && token.len() > 1 {
        if WELL_KNOWN_DOT_ENTRIES.contains(&token) {
            return InlineCodePathKind::Dotfile;
        }
        if is_bare_extension_marker(token) {
            return InlineCodePathKind::ExtensionOrGlob;
        }
        return InlineCodePathKind::Dotfile;
    }
    if token
        .rsplit_once('.')
        .is_some_and(|(_, extension)| is_extension_component(extension))
    {
        return InlineCodePathKind::ConcreteRelativePath;
    }
    InlineCodePathKind::NonPath
}

/// Remove one Markdown fragment and one `::` symbol suffix before a filesystem
/// probe. Diagnostics retain the original token as evidence.
pub(crate) fn normalize_inline_code_path_probe(token: &str) -> &str {
    let without_fragment = token.split_once('#').map_or(token, |(path, _)| path);
    without_fragment
        .split_once("::")
        .map_or(without_fragment, |(path, _)| path)
}

/// Whether a filesystem probe is absolute, traverses a parent directory, or
/// resolves to a symlink. Both I003 and D005 reject these references.
pub(crate) fn is_unsafe_inline_code_path_probe(path: &Path) -> bool {
    path.is_absolute()
        || path
            .components()
            .any(|component| component == Component::ParentDir)
        || path.is_symlink()
}

/// Model aliases accepted by Claude Code `/model` plus skill/agent `inherit`.
/// Full Anthropic model IDs (`claude-…`) are also accepted.
/// Shared by skill frontmatter (S063) and agent frontmatter (A014).
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
/// Canonical vocabulary for S063 (skills) and A014 (agents).
pub(crate) fn is_valid_model_value(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() {
        return false;
    }
    if MODEL_ALIASES.contains(&value) {
        return true;
    }
    // Full Anthropic model IDs and version pins (e.g. claude-sonnet-4-5,
    // claude-opus-4-6). A bare family or trailing separator is not an ID.
    value
        .strip_prefix("claude-")
        .is_some_and(|rest| rest.contains('-') && !rest.ends_with('-'))
}

/// Built-in Claude Code tool names (PascalCase). Shared by S040
/// (skill `allowed-tools`) and A019/A020 (agent `tools`/`disallowedTools`).
pub(crate) const KNOWN_TOOLS: &[&str] = &[
    "AskUserQuestion",
    "Bash",
    "EndConversation",
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
    "PowerShell",
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
        // RFC 6761 §6.3: `localhost` and every name under `.localhost` resolve
        // to loopback. Compare case-insensitively; require a label boundary so
        // `localhost.example.com` stays remote.
        Host::Domain(host) => {
            let host = host.to_ascii_lowercase();
            host == "localhost" || host.ends_with(".localhost")
        }
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

/// Whether `value` uses `scheme` and has a parsed non-local host.
pub(crate) fn is_nonlocal_url_with_scheme(value: &str, scheme: &str) -> bool {
    Url::parse(value).is_ok_and(|url| {
        url.scheme() == scheme && url.host().is_some_and(|host| !is_local_host(host))
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

    /// Shared S063/A014 vocabulary table: both rules call this one function.
    #[test]
    fn s063_a014_shared_vocabulary_table() {
        let cases: &[(&str, bool)] = &[
            ("fable", true),
            ("opusplan", true),
            ("best", true),
            ("claude-fable-5", true),
            ("haiku[1m]", false),
            ("inherit[1m]", false),
            ("sonet", false),
            ("", false),
        ];
        for &(value, expected) in cases {
            assert_eq!(
                is_valid_model_value(value),
                expected,
                "shared model vocabulary mismatch for {value:?}"
            );
        }
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
        assert!(is_known_tool_name("EndConversation"));
        assert!(is_known_tool_name("PowerShell"));
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
        assert!(!is_known_tool_name("ExitPlanMode"));
        assert!(!is_known_tool_name(""));
    }
}

#[cfg(test)]
mod emphasis_gate_normalization_tests {
    use super::normalize_emphasis_for_gates;

    #[test]
    fn strips_boundary_marker_runs_and_preserves_mid_word_markers() {
        assert_eq!(
            normalize_emphasis_for_gates("**Stop after 3 attempts and report the blocker.**"),
            "Stop after 3 attempts and report the blocker."
        );
        assert_eq!(
            normalize_emphasis_for_gates("- **Stop after 3 attempts and report the blocker.**"),
            "- Stop after 3 attempts and report the blocker."
        );
        assert_eq!(
            normalize_emphasis_for_gates("__Give up after 10 minutes and escalate.__"),
            "Give up after 10 minutes and escalate."
        );
        assert_eq!(
            normalize_emphasis_for_gates("**Important**: keep retrying until the build passes."),
            "Important: keep retrying until the build passes."
        );
        assert_eq!(
            normalize_emphasis_for_gates("__Note__: retry until success."),
            "Note: retry until success."
        );
        assert_eq!(
            normalize_emphasis_for_gates("***Warning***: keep trying until it succeeds."),
            "Warning: keep trying until it succeeds."
        );
        assert_eq!(
            normalize_emphasis_for_gates("Please **stop after 3 attempts** and report."),
            "Please stop after 3 attempts and report."
        );
        assert_eq!(normalize_emphasis_for_gates("a*b"), "a*b");
        assert_eq!(normalize_emphasis_for_gates("snake_case"), "snake_case");
        assert_eq!(normalize_emphasis_for_gates("file*name"), "file*name");
    }

    #[test]
    fn normalization_is_idempotent() {
        let samples = [
            "**Important**: keep retrying until the build passes.",
            "__Give up after 10 minutes and escalate.__",
            "Please **stop after 3 attempts** and report.",
            "a*b and snake_case stay put.",
        ];
        for sample in samples {
            let once = normalize_emphasis_for_gates(sample);
            let twice = normalize_emphasis_for_gates(&once);
            assert_eq!(once, twice, "{sample}");
        }
    }
}

#[cfg(test)]
mod url_tests {
    use super::{is_nonlocal_url_with_scheme, is_valid_http_url};

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
    fn identifies_only_remote_urls_with_the_requested_scheme_as_nonlocal() {
        for value in [
            "http://example.com",
            "http://user:password@b\u{fc}cher.example:8080",
            "http://[2001:db8::1]",
        ] {
            assert!(
                is_nonlocal_url_with_scheme(value, "http"),
                "expected {value} to be non-local"
            );
        }
        for value in [
            "https://example.com",
            "http://localhost",
            "http://LocalHost",
            "http://foo.localhost:3000/mcp",
            "http://a.b.localhost",
            "http://127.1.2.3",
            "http://0.0.0.0",
            "http://[::1]",
            "http://[::]",
            "http://[::ffff:127.0.0.1]",
            "http://",
        ] {
            assert!(
                !is_nonlocal_url_with_scheme(value, "http"),
                "expected {value} to be local or invalid"
            );
        }
        for value in [
            "http://localhost.example.com",
            "http://notlocalhost.example",
            "http://example.localhost.evil.com",
        ] {
            assert!(
                is_nonlocal_url_with_scheme(value, "http"),
                "expected {value} to stay non-local (label boundary)"
            );
        }
        assert!(is_nonlocal_url_with_scheme("ws://example.com", "ws"));
        assert!(!is_nonlocal_url_with_scheme("ws://localhost", "ws"));
        assert!(!is_nonlocal_url_with_scheme("ws://a.b.localhost", "ws"));
        assert!(!is_nonlocal_url_with_scheme("wss://example.com", "ws"));
    }
}

#[cfg(test)]
mod inline_code_path_tests {
    use super::{
        InlineCodePathKind, classify_inline_code_path, is_unsafe_inline_code_path_probe,
        normalize_inline_code_path_probe,
    };
    use std::path::Path;

    #[test]
    fn classifies_concrete_relative_paths() {
        for token in [
            "missing.md",
            "missing.ts",
            "Node.js",
            "api.example.com",
            "docs/missing.md",
            "docs/missing.ts",
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
    fn classifies_well_known_dot_entries_separately_from_bare_extensions() {
        for token in [
            ".env",
            ".gitignore",
            ".claude",
            ".claude-plugin",
            ".github",
            ".vscode",
            ".codex",
            ".cursor",
            ".venv",
            ".husky",
            ".idea",
            ".devcontainer",
            ".cursorrules",
            ".mcp.json",
        ] {
            assert_eq!(
                classify_inline_code_path(token),
                InlineCodePathKind::Dotfile,
                "expected {token} to be a dotfile"
            );
        }
        for token in [
            ".c",
            ".cpp",
            ".css",
            ".go",
            ".html",
            ".java",
            ".js",
            ".json",
            ".md",
            ".py",
            ".rs",
            ".sh",
            ".toml",
            ".ts",
            ".tsx",
            ".yaml",
            ".yml",
            ".properties",
            "*.py",
        ] {
            assert_eq!(
                classify_inline_code_path(token),
                InlineCodePathKind::ExtensionOrGlob,
                "expected {token} to be extension/glob notation"
            );
        }
    }

    #[test]
    fn applies_the_twelve_character_extension_boundary() {
        assert_eq!(
            classify_inline_code_path(".abcdefghijkl"),
            InlineCodePathKind::ExtensionOrGlob
        );
        for token in [".UPPER", ".abcdefghijklm", ".tool-versions", ".config_file"] {
            assert_eq!(
                classify_inline_code_path(token),
                InlineCodePathKind::Dotfile,
                "expected {token} to remain a dotfile"
            );
        }
    }

    #[test]
    fn rejects_numeric_final_components_as_non_paths() {
        for token in ["3.12", "1.2.3", "v20.11.1"] {
            assert_eq!(
                classify_inline_code_path(token),
                InlineCodePathKind::NonPath,
                "expected {token} not to be a path"
            );
        }
    }

    #[test]
    fn normalizes_fragments_and_symbol_suffixes_before_probing() {
        assert_eq!(
            normalize_inline_code_path_probe("docs/README.md#usage"),
            "docs/README.md"
        );
        assert_eq!(
            normalize_inline_code_path_probe("src/main.rs::main"),
            "src/main.rs"
        );
        assert_eq!(
            normalize_inline_code_path_probe("src/main.rs::main#usage"),
            "src/main.rs"
        );
        assert!(is_unsafe_inline_code_path_probe(Path::new("/tmp/file.md")));
        assert!(is_unsafe_inline_code_path_probe(Path::new(
            "docs/../file.md"
        )));
        assert!(!is_unsafe_inline_code_path_probe(Path::new("docs/file.md")));
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
