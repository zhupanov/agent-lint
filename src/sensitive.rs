use regex::Regex;
use std::ops::Range;
use std::sync::LazyLock;

const SENSITIVE_SEGMENTS: &[&str] = &["SECRET", "TOKEN", "PASSWORD", "PASSWD"];
const SENSITIVE_SEGMENT_PAIRS: &[&[&str]] = &[
    &["PRIVATE", "KEY"],
    &["ACCESS", "KEY"],
    &["API", "KEY"],
    &["CLIENT", "SECRET"],
];

/// Header names whose values conventionally carry credentials.
///
/// P018 and CX013 share this vocabulary so equivalent MCP credential maps do
/// not drift by platform.
pub(crate) const SENSITIVE_HTTP_HEADERS: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
];

static SECRET_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        Regex::new(r"sk-[a-zA-Z0-9]{20,}").unwrap(),
        Regex::new(r"ghp_[a-zA-Z0-9]{36,}").unwrap(),
        Regex::new(r"xox[bp]-[0-9][a-zA-Z0-9\-]{8,}").unwrap(),
        Regex::new(
            r#"(?i)(api[_\-]?key|api[_\-]?secret|api[_\-]?token)\s*[=:]\s*["']?[A-Za-z0-9]{20,}"#,
        )
        .unwrap(),
        Regex::new(r#"(?i)(password|secret|token)\s*[=:]\s*["'][^"']{8,}"#).unwrap(),
    ]
});

static SENSITIVE_ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)(?:api[_\-]?(?:key|secret|token)|password|secret|token)\s*[=:]\s*["']?[^\s,"'}]+"#,
    )
    .unwrap()
});

static RE_ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| {
    // Use only horizontal whitespace so `key =\nnext-line` does not treat the
    // following line as the assignment value.
    Regex::new(r"(?i)(^|[^A-Za-z0-9_])([A-Za-z_][A-Za-z0-9_\-]*)[ \t]*[=:][ \t]*")
        .expect("valid assignment regex")
});

static RE_PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\$([A-Za-z_][A-Za-z0-9_]*)$").expect("valid $NAME placeholder regex")
});
static RE_BRACE_PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\$\{([A-Za-z_][A-Za-z0-9_]*)(?::-([^}]*))?\}$")
        .expect("valid ${NAME} placeholder regex")
});
static RE_MUSTACHE_PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\{\{([A-Za-z_][A-Za-z0-9_]*)\}\}$").expect("valid {{NAME}} placeholder regex")
});
static RE_ANGLE_PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^<([A-Za-z_][A-Za-z0-9_]*)>$").expect("valid <NAME> placeholder regex")
});
static RE_SKILL_ANGLE_PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^<[A-Za-z_][A-Za-z0-9_ -]*>$").expect("valid skill <NAME> placeholder regex")
});

struct SignaturePattern {
    regex: Regex,
    evidence: &'static str,
    /// Prefix omitted when deciding whether this signature is an intentionally
    /// repeated-character documentation placeholder on a skill surface.
    placeholder_prefixes: &'static [&'static str],
    /// When set, reject matches whose following source byte is in this class so
    /// exact-length signatures do not accept a longer near-miss prefix.
    reject_if_next: Option<fn(char) -> bool>,
}

static SIGNATURE_PATTERNS: LazyLock<Vec<SignaturePattern>> = LazyLock::new(|| {
    vec![
        SignaturePattern {
            regex: Regex::new(r"sk-[a-zA-Z0-9]{20,}").expect("valid sk- signature"),
            evidence: "openai-api-key-signature",
            placeholder_prefixes: &["sk-"],
            reject_if_next: None,
        },
        SignaturePattern {
            regex: Regex::new(r"ghp_[a-zA-Z0-9]{36}").expect("valid ghp_ signature"),
            evidence: "github-token-signature",
            placeholder_prefixes: &["ghp_"],
            reject_if_next: Some(|ch| ch.is_ascii_alphanumeric()),
        },
        SignaturePattern {
            regex: Regex::new(r"github_pat_[a-zA-Z0-9_]{20,}")
                .expect("valid github_pat_ signature"),
            evidence: "github-fine-grained-token-signature",
            placeholder_prefixes: &["github_pat_"],
            reject_if_next: None,
        },
        SignaturePattern {
            regex: Regex::new(r"xox[bp]-[0-9][a-zA-Z0-9\-]{8,}").expect("valid slack signature"),
            evidence: "slack-token-signature",
            placeholder_prefixes: &["xoxb-", "xoxp-"],
            reject_if_next: None,
        },
        SignaturePattern {
            regex: Regex::new(r"(?:AKIA|ASIA)[A-Z0-9]{16}").expect("valid aws signature"),
            evidence: "aws-access-key-signature",
            placeholder_prefixes: &["AKIA", "ASIA"],
            reject_if_next: Some(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit()),
        },
        SignaturePattern {
            regex: Regex::new(r"glpat-[a-zA-Z0-9_\-]{20,}").expect("valid glpat- signature"),
            evidence: "gitlab-token-signature",
            placeholder_prefixes: &["glpat-"],
            reject_if_next: None,
        },
        SignaturePattern {
            regex: Regex::new(r"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----")
                .expect("valid PEM signature"),
            evidence: "private-key-block",
            placeholder_prefixes: &[],
            reject_if_next: None,
        },
    ]
});

/// Conservative shared check for values that must not be echoed in output.
///
/// Intentionally separate from the I002 Markdown scanner so S032/CX consumers
/// do not inherit instruction-file assignment or placeholder semantics.
pub(crate) fn contains_possible_secret(content: &str) -> bool {
    SECRET_PATTERNS
        .iter()
        .any(|pattern| pattern.is_match(content))
}

/// Evidence is display-oriented and therefore uses a deliberately broader
/// convention than lint rules: omit any apparent sensitive-key assignment,
/// even when the value is short or is only a placeholder.
pub(crate) fn contains_sensitive_evidence(content: &str) -> bool {
    contains_possible_secret(content) || SENSITIVE_ASSIGNMENT.is_match(content)
}

/// Segmented sensitive-key matcher shared by P018 and I002.
///
/// Matches exact segments `SECRET`, `TOKEN`, `PASSWORD`, `PASSWD`, and the
/// multi-word forms `PRIVATE_KEY`, `ACCESS_KEY`, `API_KEY`, and
/// `CLIENT_SECRET` after splitting on non-alphanumeric separators. Substring
/// collisions such as `TOKENIZER_MODEL` are intentionally clean.
pub(crate) fn is_sensitive_key(key: &str) -> bool {
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

/// Return whether `value` is exactly an environment-variable reference with
/// no literal fallback. `allow_unbraced` controls support for `$NAME`; Claude
/// MCP's documented grammar accepts only the braced spelling, while Codex
/// accepts both spellings for the purpose of literal-secret classification.
pub(crate) fn is_safe_env_placeholder(value: &str, allow_unbraced: bool) -> bool {
    if allow_unbraced && RE_PLACEHOLDER.is_match(value) {
        return true;
    }
    let Some(captures) = RE_BRACE_PLACEHOLDER.captures(value) else {
        return false;
    };
    match captures.get(2) {
        None => true,
        Some(default) => default.as_str().is_empty(),
    }
}

/// Explicit token signatures used by the typed Codex MCP secret analyzer.
///
/// This deliberately does not share the broad `contains_possible_secret`
/// heuristic: CX013 owns only these signatures outside typed credential maps.
pub(crate) fn contains_codex_mcp_token_signature(value: &str) -> bool {
    static CODEX_MCP_TOKEN_SIGNATURES: LazyLock<Vec<Regex>> = LazyLock::new(|| {
        vec![
            Regex::new(r"sk-[a-zA-Z0-9]{20,}").expect("valid sk- signature"),
            Regex::new(r"ghp_[a-zA-Z0-9]{36,}").expect("valid ghp_ signature"),
            Regex::new(r"xox[bp]-[0-9][a-zA-Z0-9\-]{8,}").expect("valid slack signature"),
        ]
    });
    CODEX_MCP_TOKEN_SIGNATURES
        .iter()
        .any(|pattern| pattern.is_match(value))
}

/// First Markdown credential hit in byte order: either a sensitive assignment
/// key or a fixed signature category label. Evidence never includes credential
/// value bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MarkdownSecretFinding {
    pub evidence: String,
    /// Byte range used only for `SourceSpan` location. For assignments this is
    /// the key token; for signatures it is a zero-width point at the match
    /// start so callers never need to slice secret value bytes.
    pub location_range: Range<usize>,
}

#[derive(Clone, Copy)]
enum MarkdownSecretPolicy {
    /// I002's existing contract: placeholder-looking token signatures still
    /// count, and command substitutions are not special syntax.
    Instruction,
    /// S032's skill-document contract: common documentation placeholders are
    /// clean, including command substitutions and repeated-character tokens.
    Skill,
}

/// Scan Markdown/instruction source for the first credential fact in byte
/// order, applying only the surface-specific placeholder policy.
///
/// The assignment vocabulary, source ordering, signature families, and safe
/// evidence categories are deliberately owned here so I002 and S032 cannot
/// drift in their lexical recognition.
fn find_markdown_secret(
    content: &str,
    policy: MarkdownSecretPolicy,
) -> Option<MarkdownSecretFinding> {
    let mut best: Option<MarkdownSecretFinding> = None;
    let mut consider = |candidate: MarkdownSecretFinding| match &best {
        None => best = Some(candidate),
        Some(current) if candidate.location_range.start < current.location_range.start => {
            best = Some(candidate);
        }
        Some(current)
            if candidate.location_range.start == current.location_range.start
                && candidate.evidence < current.evidence =>
        {
            best = Some(candidate);
        }
        _ => {}
    };

    for captures in RE_ASSIGNMENT.captures_iter(content) {
        let boundary = captures.get(1).expect("assignment boundary capture");
        // Ignore separators that appear inside `$NAME`, `${NAME:-...}`, or
        // `{{NAME}}` placeholder spellings.
        if matches!(boundary.as_str(), "$" | "{") {
            continue;
        }
        let key = captures.get(2).expect("assignment key capture");
        if !is_sensitive_key(key.as_str()) {
            continue;
        }
        let full = captures.get(0).expect("full assignment match");
        let raw_value = parse_assignment_value(&content[full.end()..]);
        // `strip_assignment_quotes` removes source whitespace before deciding
        // whether one pair of outer quotes is present. Do not trim again here:
        // whitespace inside those quotes is part of the value and therefore
        // prevents a command substitution from being the entire placeholder.
        let value = strip_assignment_quotes(raw_value);
        if value.trim().is_empty() || is_secret_placeholder(value, policy) {
            continue;
        }
        consider(MarkdownSecretFinding {
            evidence: key.as_str().to_string(),
            location_range: key.start()..key.end(),
        });
    }

    for pattern in SIGNATURE_PATTERNS.iter() {
        for found in pattern.regex.find_iter(content) {
            if let Some(reject_if_next) = pattern.reject_if_next {
                if content[found.end()..]
                    .chars()
                    .next()
                    .is_some_and(reject_if_next)
                {
                    continue;
                }
            }
            if matches!(policy, MarkdownSecretPolicy::Skill)
                && pattern
                    .placeholder_prefixes
                    .iter()
                    .any(|prefix| is_repeated_signature_payload(found.as_str(), prefix))
            {
                continue;
            }
            consider(MarkdownSecretFinding {
                evidence: pattern.evidence.to_string(),
                location_range: found.start()..found.start(),
            });
            break;
        }
    }

    best
}

/// Scan AGENTS.md source under I002's intentionally strict placeholder policy.
pub(crate) fn find_instruction_secret(content: &str) -> Option<MarkdownSecretFinding> {
    find_markdown_secret(content, MarkdownSecretPolicy::Instruction)
}

/// Scan complete SKILL.md source under S032's documentation-aware policy.
pub(crate) fn find_skill_secret(content: &str) -> Option<MarkdownSecretFinding> {
    find_markdown_secret(content, MarkdownSecretPolicy::Skill)
}

fn parse_assignment_value(after_separator: &str) -> &str {
    let line_end = after_separator
        .find(['\n', '\r'])
        .unwrap_or(after_separator.len());
    &after_separator[..line_end]
}

fn strip_assignment_quotes(value: &str) -> &str {
    let trimmed = value.trim();
    if trimmed.len() >= 2 {
        let bytes = trimmed.as_bytes();
        if (bytes[0] == b'"' && bytes[trimmed.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[trimmed.len() - 1] == b'\'')
        {
            return &trimmed[1..trimmed.len() - 1];
        }
    }
    trimmed
}

fn is_secret_placeholder(value: &str, policy: MarkdownSecretPolicy) -> bool {
    if is_safe_env_placeholder(value, true)
        || RE_MUSTACHE_PLACEHOLDER.is_match(value)
        || match policy {
            MarkdownSecretPolicy::Instruction => RE_ANGLE_PLACEHOLDER.is_match(value),
            MarkdownSecretPolicy::Skill => RE_SKILL_ANGLE_PLACEHOLDER.is_match(value),
        }
    {
        return true;
    }
    matches!(policy, MarkdownSecretPolicy::Skill)
        && (is_complete_command_substitution(value) || is_complete_backtick_substitution(value))
}

/// Return whether `value` is exactly one `$()` command substitution.
///
/// This intentionally recognizes only the structural boundary needed for the
/// documentation-placeholder exemption. Quotes and backslash escapes keep a
/// parenthesis from closing the outer substitution; once the outer close is
/// found, no trailing bytes are permitted.
fn is_complete_command_substitution(value: &str) -> bool {
    #[derive(Clone, Copy)]
    enum Quote {
        Single,
        Double,
    }

    let Some(interior) = value.strip_prefix("$(") else {
        return false;
    };

    let mut depth = 1_usize;
    let mut quote = None;
    let mut escaped = false;

    for (index, byte) in interior.bytes().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }

        match quote {
            Some(Quote::Single) if byte == b'\'' => quote = None,
            Some(Quote::Single) => {}
            Some(Quote::Double) if byte == b'"' => quote = None,
            Some(Quote::Double) if byte == b'\\' => escaped = true,
            Some(Quote::Double) => {}
            None => match byte {
                b'\\' => escaped = true,
                b'\'' => quote = Some(Quote::Single),
                b'"' => quote = Some(Quote::Double),
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return index + 1 == interior.len();
                    }
                }
                _ => {}
            },
        }
    }

    false
}

/// Return whether `value` is exactly one backtick command substitution.
///
/// The chosen escape policy is deliberately small and deterministic: a
/// backslash escapes the next byte, including a backtick. The first unescaped
/// backtick closes the substitution, so any following byte makes the value a
/// literal rather than a placeholder.
fn is_complete_backtick_substitution(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.first() != Some(&b'`') {
        return false;
    }

    let mut escaped = false;
    for (index, byte) in bytes[1..].iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        if *byte == b'\\' {
            escaped = true;
        } else if *byte == b'`' {
            return index + 2 == bytes.len();
        }
    }

    false
}

fn is_repeated_signature_payload(signature: &str, prefix: &str) -> bool {
    let Some(payload) = signature.strip_prefix(prefix) else {
        return false;
    };
    let mut payload = payload.chars();
    let Some(first) = payload.next() else {
        return false;
    };
    payload.all(|ch| ch == first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_supported_secret_shapes() {
        assert!(contains_possible_secret(
            "token = 'this-is-a-sensitive-value'"
        ));
        assert!(contains_possible_secret("sk-abcdefghijklmnopqrstuvwxyz"));
        assert!(!contains_possible_secret("token = '$TOKEN'"));
        assert!(contains_sensitive_evidence("API_KEY=short-value"));
        assert!(contains_sensitive_evidence("token = '$TOKEN'"));
    }

    #[test]
    fn segmented_sensitive_keys_match_vocabulary_not_substrings() {
        for key in [
            "SECRET",
            "token",
            "Password",
            "PASSWD",
            "my_private_key",
            "access-key",
            "API_KEY",
            "client-secret",
            "MY_PASSWORD",
        ] {
            assert!(is_sensitive_key(key), "{key}");
        }
        for key in [
            "TOKENIZER_MODEL",
            "SECRETARY",
            "APIKEY",
            "MODEL_NAME",
            "HOME",
            "username",
        ] {
            assert!(!is_sensitive_key(key), "{key}");
        }
    }

    #[test]
    fn codex_mcp_placeholders_and_signature_scope_are_precise() {
        for value in ["$NAME", "${NAME}", "${NAME:-}"] {
            assert!(is_safe_env_placeholder(value, true), "{value}");
        }
        assert!(!is_safe_env_placeholder("$NAME", false));
        for value in ["${NAME:-default}", "prefix$NAME", "${NAME}suffix"] {
            assert!(!is_safe_env_placeholder(value, true), "{value}");
        }
        for value in [
            "sk-abcdefghijklmnopqrstuvwxyz",
            "ghp_abcdefghijklmnopqrstuvwxyz1234567890",
            "xoxp-1abcdefghij",
        ] {
            assert!(contains_codex_mcp_token_signature(value), "{value}");
        }
        for value in [
            "github_pat_abcdefghijklmnopqrstuvwxyz",
            "AKIAIOSFODNN7EXAMPLE",
            // Hard negative: a backslash-laden non-token must not match. The
            // Slack character class is `[a-zA-Z0-9\-]` (alphanumerics + hyphen),
            // identical to `SECRET_PATTERNS`; a widened class that admitted `\`
            // would accept this and silently diverge from the shipped pattern.
            r"xoxb-1a\b\c\d\e\f",
        ] {
            assert!(!contains_codex_mcp_token_signature(value), "{value}");
        }
    }

    #[test]
    fn instruction_scanner_finds_unquoted_password_at_key() {
        let finding = find_instruction_secret("password = hunterhunter").unwrap();
        assert_eq!(finding.evidence, "password");
        assert_eq!(
            &"password = hunterhunter"[finding.location_range.clone()],
            "password"
        );
    }

    #[test]
    fn instruction_scanner_placeholders_and_empty_values_are_clean() {
        for content in [
            "password = $TOKEN",
            "password = ${TOKEN}",
            "password = ${TOKEN:-}",
            "password = {{TOKEN}}",
            "password = <TOKEN>",
            "password =",
            "password = \"\"",
            "password = \"   \"",
            "password = ''",
            "TOKENIZER_MODEL = gpt",
            "username = hunterhunter",
        ] {
            assert!(
                find_instruction_secret(content).is_none(),
                "expected clean: {content}"
            );
        }
    }

    #[test]
    fn instruction_scanner_literal_defaults_and_quoting_are_findings() {
        for (content, evidence) in [
            ("password = ${TOKEN:-hardcoded}", "password"),
            ("password = \"$TOKEN extra\"", "password"),
            ("API_KEY: 'short'", "API_KEY"),
            ("secret=x", "secret"),
        ] {
            let finding = find_instruction_secret(content).unwrap();
            assert_eq!(finding.evidence, evidence, "{content}");
        }
    }

    #[test]
    fn instruction_scanner_signatures_use_category_labels() {
        let cases = [
            (
                "key sk-abcdefghijklmnopqrstuvwxyz",
                "openai-api-key-signature",
            ),
            (
                "token ghp_abcdefghijklmnopqrstuvwxyz1234567890",
                "github-token-signature",
            ),
            (
                "pat github_pat_abcdefghijklmnopqrstuvwxyz",
                "github-fine-grained-token-signature",
            ),
            ("slack xoxb-1abcdefghij", "slack-token-signature"),
            ("slack xoxp-1abcdefghij", "slack-token-signature"),
            ("aws AKIAIOSFODNN7EXAMPLE", "aws-access-key-signature"),
            ("aws ASIATESTKEY12EXAMPLE", "aws-access-key-signature"),
            (
                "gl glpat-abcdefghijklmnopqrstuvwxyz",
                "gitlab-token-signature",
            ),
            ("-----BEGIN RSA PRIVATE KEY-----", "private-key-block"),
            ("-----BEGIN EC PRIVATE KEY-----", "private-key-block"),
            ("-----BEGIN OPENSSH PRIVATE KEY-----", "private-key-block"),
            ("-----BEGIN PRIVATE KEY-----", "private-key-block"),
        ];
        for (content, evidence) in cases {
            let finding = find_instruction_secret(content)
                .unwrap_or_else(|| panic!("no finding for {content}"));
            assert_eq!(finding.evidence, evidence, "{content}");
            assert_eq!(
                finding.location_range.start, finding.location_range.end,
                "signature locations must be zero-width points: {content}"
            );
        }
    }

    #[test]
    fn instruction_scanner_signature_outputs_never_embed_source_values() {
        let secret = "sk-abcdefghijklmnopqrstuvwxyz";
        let content = format!("See `{secret}` in the docs.\n");
        let finding = find_instruction_secret(&content).unwrap();
        let mut text = format!(
            "evidence={} suggestion={}",
            finding.evidence,
            "replace the literal with an environment-variable or secret-store reference"
        );
        text.push_str(
            &serde_json::json!({
                "evidence": finding.evidence,
                "message": "AGENTS.md contains a potential hardcoded secret/API key",
            })
            .to_string(),
        );
        assert!(!text.contains(secret));
        assert!(!finding.evidence.contains("sk-"));
    }

    #[test]
    fn instruction_scanner_signature_near_misses_are_clean() {
        for content in [
            "sk-abcdefghijklmnopqrs",                    // 19
            "ghp_abcdefghijklmnopqrstuvwxyz123456789",   // 35
            "ghp_abcdefghijklmnopqrstuvwxyz12345678901", // 37
            "AKIAIOSFODNN7EXAMPL",                       // 15
            "glpat-abcdefghijklmnopq",                   // 19
            "-----BEGIN PUBLIC KEY-----",
            "xoxb-short",
        ] {
            assert!(
                find_instruction_secret(content).is_none(),
                "expected clean: {content}"
            );
        }
    }

    #[test]
    fn instruction_scanner_picks_first_byte_order_match() {
        let content = "password = first\nsk-abcdefghijklmnopqrstuvwxyz\n";
        let finding = find_instruction_secret(content).unwrap();
        assert_eq!(finding.evidence, "password");

        let content = "sk-abcdefghijklmnopqrstuvwxyz\npassword = second\n";
        let finding = find_instruction_secret(content).unwrap();
        assert_eq!(finding.evidence, "openai-api-key-signature");
    }

    #[test]
    fn instruction_scanner_empty_assignment_does_not_consume_following_line() {
        let content = "private_key =\n\nRun cargo test\n";
        assert!(find_instruction_secret(content).is_none());
    }

    #[test]
    fn instruction_scanner_handles_crlf_unicode_fences_and_frontmatter() {
        let crlf = "intro\r\npassword = hunterhunter\r\n";
        assert_eq!(find_instruction_secret(crlf).unwrap().evidence, "password");

        let unicode = "café\nPASSWORD: literal-ü\n";
        assert_eq!(
            find_instruction_secret(unicode).unwrap().evidence,
            "PASSWORD"
        );

        let fenced = "```\napi_key = leaked\n```\n";
        assert_eq!(find_instruction_secret(fenced).unwrap().evidence, "api_key");

        let frontmatter = "---\ntoken: committed\n---\nBody\n";
        assert_eq!(
            find_instruction_secret(frontmatter).unwrap().evidence,
            "token"
        );

        let inline = "Use `sk-abcdefghijklmnopqrstuvwxyz` in docs.\n";
        assert_eq!(
            find_instruction_secret(inline).unwrap().evidence,
            "openai-api-key-signature"
        );
    }

    #[test]
    fn skill_scanner_accepts_documented_placeholder_forms() {
        for content in [
            "password = $DB_PASSWORD",
            "token = ${GITHUB_TOKEN}",
            "secret = ${TOKEN:-}",
            "API_KEY = $(gh auth token)",
            "CLIENT_SECRET = `read_secret`",
            "access_key = {{SERVICE_TOKEN}}",
            "private_key = <your-api-key>",
            "password = <your api key>",
            "sk-xxxxxxxxxxxxxxxxxxxxxxxx",
            "ghp_000000000000000000000000000000000000",
        ] {
            assert!(
                find_skill_secret(content).is_none(),
                "expected clean skill content: {content}"
            );
        }
    }

    #[test]
    fn skill_command_substitution_placeholders_must_cover_the_entire_value() {
        for content in [
            "API_KEY = $(gh auth token)",
            "API_KEY = $(outer $(read_secret))",
            r#"API_KEY = $(printf '%s' "(nested)")"#,
            r"API_KEY = $(printf \))",
            "CLIENT_SECRET = `read_secret`",
            r"CLIENT_SECRET = `read \`escaped\``",
        ] {
            assert!(
                find_skill_secret(content).is_none(),
                "expected clean: {content}"
            );
        }

        for content in [
            "API_KEY = prefix$(gh auth token)",
            "API_KEY = $(gh auth token) literal-suffix",
            "API_KEY = $(gh auth token))",
            "API_KEY = \" $(gh auth token)\"",
            "API_KEY = \"$(gh auth token) \"",
            "API_KEY = $(outer $(read_secret)",
            "CLIENT_SECRET = prefix`read_secret`",
            "CLIENT_SECRET = `read_secret` literal-suffix",
            "CLIENT_SECRET = `first` `second`",
            r"CLIENT_SECRET = `read \`escaped` literal-suffix",
            r"CLIENT_SECRET = `read_secret\`",
        ] {
            assert!(
                find_skill_secret(content).is_some(),
                "expected finding: {content}"
            );
        }
    }

    #[test]
    fn instruction_scanner_does_not_exempt_command_substitutions() {
        for content in [
            "API_KEY = $(gh auth token)",
            "CLIENT_SECRET = `read_secret`",
        ] {
            assert!(
                find_instruction_secret(content).is_some(),
                "expected finding: {content}"
            );
        }
    }

    #[test]
    fn skill_scanner_reports_all_key_forms_literal_remainders_and_signatures() {
        for key in [
            "SECRET",
            "TOKEN",
            "PASSWORD",
            "PASSWD",
            "PRIVATE_KEY",
            "ACCESS_KEY",
            "API_KEY",
            "CLIENT_SECRET",
        ] {
            let content = format!("{key}: committed-literal");
            assert_eq!(find_skill_secret(&content).unwrap().evidence, key, "{key}");
        }
        for content in [
            "password = prefix${DB_PASSWORD}",
            "token = ${TOKEN:-committed-default}",
            "secret = `read_secret` suffix",
            "api_key = $(read_secret) suffix",
        ] {
            assert!(find_skill_secret(content).is_some(), "{content}");
        }
        for (content, evidence) in [
            ("sk-aBcDeFgHiJkLmNoPqRsT1234", "openai-api-key-signature"),
            (
                "ghp_aBcDeFgHiJkLmNoPqRsTuVwXyZ0123456789",
                "github-token-signature",
            ),
            (
                "github_pat_aBcDeFgHiJkLmNoPqRsT",
                "github-fine-grained-token-signature",
            ),
            ("xoxb-1aBcDeFgHi", "slack-token-signature"),
            ("AKIAIOSFODNN7EXAMPLE", "aws-access-key-signature"),
            ("ASIAT3STK3Y12EXAMPLE", "aws-access-key-signature"),
            ("glpat-aBcDeFgHiJkLmNoPqRsT", "gitlab-token-signature"),
            ("-----BEGIN PRIVATE KEY-----", "private-key-block"),
        ] {
            assert_eq!(
                find_skill_secret(content).unwrap().evidence,
                evidence,
                "{content}"
            );
        }
    }

    #[test]
    fn skill_scanner_covers_full_source_and_keeps_i002_strict() {
        let frontmatter = "---\ntoken: committed\n---\nBody\n";
        assert_eq!(find_skill_secret(frontmatter).unwrap().evidence, "token");
        assert_eq!(
            find_skill_secret("```bash\npassword = committed\n```\n")
                .unwrap()
                .evidence,
            "password"
        );
        assert_eq!(
            find_skill_secret("Use `sk-aBcDeFgHiJkLmNoPqRsT1234` in docs.\n")
                .unwrap()
                .evidence,
            "openai-api-key-signature"
        );
        assert!(find_instruction_secret("sk-xxxxxxxxxxxxxxxxxxxxxxxx").is_some());
        assert!(find_instruction_secret("TOKEN = $(gh auth token)").is_some());
    }

    #[test]
    fn legacy_possible_secret_helpers_retain_their_existing_contract() {
        assert!(contains_possible_secret(
            "token = 'this-is-a-sensitive-value'"
        ));
        assert!(!contains_possible_secret("token = '$TOKEN'"));
        assert!(contains_sensitive_evidence("token = '$TOKEN'"));
    }
}
