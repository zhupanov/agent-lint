//! Prompt, reference, and shipped-script contracts shared by public and private skills.

use crate::config::{ExcludeSet, PromptMetricCaps, PromptSourceBudget};
use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata, SourceSpan};
use crate::fence::{CodeFenceTracker, LineClass, consecutive_bash_pairs};
use crate::frontmatter;
use crate::markdown::MarkdownDocument;
use crate::prompt_budget::{
    INLINE_CODE, MARKDOWN_LINK, normalize_repo_relative, resolve_repo_reference,
};
use crate::rules::LintRule;
use crate::traversal;
use crate::validators::common::{NEVER_INVENT_PROHIBITION, classify_inline_code_path};
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

static READ_INTENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:read|open)\s+(?:the|each|every|all|any|its|their|this|that)\b[^.\n]{0,60}\b(?:file|files|bundle|bundles|path|paths|diff|diffs|body|bodies|artifact|artifacts|markdown|log|logs)\b|\buse\s+(?:the\s+)?Read\b").unwrap()
});
static OUTPUT_ONLY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bstrict\s+JSONL?\b|\b(?:emit|output|return|respond\s+with|reply\s+with)\s+(?:strict\s+|valid\s+)?JSONL?\s+only\b|\bonly\s+(?:emit|output|return)\s+(?:strict\s+|valid\s+)?JSONL?\b|\boutput\s+must\s+be\s+(?:strict\s+|valid\s+)?JSONL?\b").unwrap()
});
static CANNOT_READ: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bunreadable\b|\b(?:cannot|can't|could\s+not|unable\s+to)\s+(?:read|open)\b|\bRead\s+fails\b|\bfail[ -]+closed\b").unwrap()
});
static SKILL_INVOKE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\b(?:re-)?[Ii]nvoke\b\s+(?:the\s+)?(?:\*\*[^*\n]{1,40}\*\*\s+)?`/[-\w]+`(?:\s+skill\b)?",
    )
    .unwrap()
});
static FLAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|\s)--([A-Za-z0-9][A-Za-z0-9_-]*)\b").unwrap());
static AWK_FIELD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\$[0-9]+").unwrap());
static IMPORT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|\s)@([A-Za-z0-9._/-]+\.md)\b").unwrap());
static HEREDOC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<<-?\s*(?:'([A-Za-z_][A-Za-z0-9_]*)'|\"([A-Za-z_][A-Za-z0-9_]*)\"|([A-Za-z_][A-Za-z0-9_]*))"#,
    )
    .unwrap()
});
static BASH_REPLACEMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\$\{[A-Za-z_][A-Za-z0-9_]*//[^/]*/(?:\$[A-Za-z_][A-Za-z0-9_]*|\$\{[A-Za-z_])")
        .unwrap()
});
static ARRAY_ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|[\s;|&({])([A-Za-z_][A-Za-z0-9_]*)\+?=\(([^)]*)\)").unwrap()
});
static ARRAY_LENGTH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\$\{#([A-Za-z_][A-Za-z0-9_]*)\[@\]\}").unwrap());
static IF_THEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|[\s;])if\s+.*(?:^|[\s;])then(?:[\s;]|$)").unwrap());
static FI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|[\s;])fi(?:[\s;]|$)").unwrap());
static EXIT_OR_RETURN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|;)\s*(?:exit|return)(?:\s|;|$)").unwrap());
static POSITIVE_ARRAY_COMPARISON: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:-gt|-ne|>|!=)\s*0(?:\s|\]|\)|;|$)").unwrap());
static EMPTY_ARRAY_COMPARISON: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:(?:-eq|-le|==)\s*0|(?:^|\s)=\s*0)(?:\s|\]|\)|;|$)").unwrap());
static AWK_COMMAND: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:[A-Za-z_][A-Za-z0-9_]*=[^\s]+\s+)*(?:command\s+)?awk(?:\s|$)").unwrap()
});
static AWK_VALUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"-v\s+[A-Za-z_][A-Za-z0-9_]*\s*=\s*(?:'([^']*)'|\"([^\"]*)\"|([^\s'\"\\]+))"#)
        .unwrap()
});
static AWK_REGEX_CONTEXT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|[^A-Za-z0-9_])(?:match|gsub|sub|split)\s*\(|\s!?~\s").unwrap()
});
static BASH32_PATTERNS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        (
            r"(?:^|[\s;|&({])declare\s+(?:-[A-Za-z]+\s+)*-[A-Za-z]*A[A-Za-z]*(?:[\s;|&)]|$)",
            "declare -A associative arrays",
        ),
        (
            r"(?:^|[\s;|&({])typeset\s+(?:-[A-Za-z]+\s+)*-[A-Za-z]*A[A-Za-z]*(?:[\s;|&)]|$)",
            "typeset -A associative arrays",
        ),
        (
            r"(?:^|[\s;|&({])(?:mapfile|readarray)(?:[\s;|&)]|$)",
            "mapfile/readarray",
        ),
        (
            r"\$\{[!A-Za-z_@*][A-Za-z0-9_]*(?:\^\^?|,,?)",
            "parameter case conversion",
        ),
        (
            r"(?:^|[\s;|&({])declare\s+(?:-[A-Za-z]+\s+)*-[A-Za-z]*n[A-Za-z]*(?:[\s;|&)]|$)",
            "declare -n nameref",
        ),
        (
            r"(?:^|[\s;|&({])local\s+(?:-[A-Za-z]+\s+)*-[A-Za-z]*n[A-Za-z]*(?:[\s;|&)]|$)",
            "local -n nameref",
        ),
        (r"&>>", "&>> append-all redirection"),
        (
            r"(?:^|[\s;|&({])coproc(?:\s+[A-Za-z_][A-Za-z0-9_]*)?\s*\{",
            "coproc",
        ),
        (
            r"\$\{[!A-Za-z_@*][A-Za-z0-9_]*\[\s*-[0-9]",
            "negative array index",
        ),
        (
            r"\{(?:-?[0-9]+|[A-Za-z])\.\.(?:-?[0-9]+|[A-Za-z])\.\.-?[0-9]",
            "stepped brace expansion",
        ),
        (
            r"(?:^|[\s;|&(])(?:if|elif)\s+(?:!\s+)?command\s+(?:grep|egrep|fgrep|rg|ripgrep)(?:[\s;|&)]|$)",
            "if/elif command grep-family condition",
        ),
    ]
    .into_iter()
    .map(|(pattern, label)| (Regex::new(pattern).unwrap(), label))
    .collect()
});
static FORWARDED_ARRAY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?m)^[^#\n]*(?:exec\s+)?[^\n]*"\$\{([A-Za-z_][A-Za-z0-9_]*)\[@\]\}""#).unwrap()
});
static NPM_RUN: LazyLock<Regex> = LazyLock::new(|| {
    // The name class allows `:` because npm scripts are commonly
    // colon-namespaced (e.g. `build:css`, `test:integration`); rejecting `:`
    // would truncate `npm run build:css` to `build` and false-positive L006.
    Regex::new(r"\bnpm\s+run(?:-script)?\s+([A-Za-z0-9][A-Za-z0-9_:-]*)").unwrap()
});

pub fn validate_contracts(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    include_public: bool,
) {
    validate_agent_contracts(diag, exclude, include_public);
    validate_skill_contracts(diag, exclude, include_public);
    validate_reference_consecutive_bash(diag, exclude, include_public);
    validate_script_contracts(diag, exclude, include_public);
    validate_claude_import_budget(diag, exclude);
    validate_prompt_source_budgets(diag);
    validate_inline_paths(diag, exclude);
    validate_import_graph(diag, exclude);
    validate_markdown_links(diag, exclude);
    validate_npm_scripts(diag, exclude);
}

fn scoped_skill_files(include_public: bool) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if include_public {
        paths.extend(one_level_files("skills", "SKILL.md"));
    }
    paths.extend(one_level_files(".claude/skills", "SKILL.md"));
    paths.sort();
    paths
}

fn scoped_agent_files(include_public: bool) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if include_public {
        paths.extend(direct_markdown_files("agents"));
    }
    paths.extend(direct_markdown_files(".claude/agents"));
    paths.sort();
    paths
}

fn one_level_files(root: &str, filename: &str) -> Vec<PathBuf> {
    crate::traversal::shallow_directories(Path::new(root), Path::new("."), None)
        .entries
        .into_iter()
        .map(|entry| entry.path.join(filename))
        .filter(|path| path.is_file())
        .collect()
}

fn direct_markdown_files(root: &str) -> Vec<PathBuf> {
    crate::traversal::shallow_files(Path::new(root), Path::new("."), None)
        .entries
        .into_iter()
        .map(|entry| entry.path)
        .filter(|path| {
            path.is_file() && path.extension().and_then(|value| value.to_str()) == Some("md")
        })
        .collect()
}

fn read_text(path: &Path, exclude: &ExcludeSet) -> Option<String> {
    let display = path.to_string_lossy();
    if path.is_symlink() || exclude.is_excluded(&display) {
        return None;
    }
    fs::read_to_string(path).ok()
}

fn frontmatter_tools(content: &str, key: &str) -> Option<Vec<String>> {
    let lines = frontmatter::extract_frontmatter(content)?;
    let prefix = format!("{key}:");
    for (index, line) in lines.iter().enumerate() {
        let Some(value) = line.strip_prefix(&prefix) else {
            continue;
        };
        let value = value.split(" #").next().unwrap_or(value).trim();
        if !value.is_empty() {
            let value = value.trim_matches(|ch| matches!(ch, '[' | ']' | '\'' | '"'));
            return Some(
                value
                    .split(',')
                    .map(|token| token.trim().trim_matches(['\'', '"']).to_string())
                    .filter(|token| !token.is_empty())
                    .collect(),
            );
        }
        let mut tools = Vec::new();
        for child in &lines[index + 1..] {
            if !child.starts_with([' ', '\t']) {
                break;
            }
            if let Some(item) = child.trim().strip_prefix("- ") {
                tools.push(item.trim_matches(['\'', '"']).to_string());
            }
        }
        return Some(tools);
    }
    None
}

fn frontmatter_explicit_tools(content: &str) -> Option<Vec<String>> {
    let lines = frontmatter::extract_frontmatter(content)?;
    for (index, line) in lines.iter().enumerate() {
        let Some(value) = line.strip_prefix("tools:") else {
            continue;
        };
        let value = value.split(" #").next().unwrap_or(value).trim();
        if !value.is_empty() {
            if !value.starts_with('[') || !value.ends_with(']') {
                return None;
            }
            return Some(
                value[1..value.len() - 1]
                    .split(',')
                    .map(|token| token.trim().trim_matches(['\'', '"']).to_string())
                    .filter(|token| !token.is_empty())
                    .collect(),
            );
        }
        let mut tools = Vec::new();
        for child in &lines[index + 1..] {
            if !child.starts_with([' ', '\t']) {
                break;
            }
            if let Some(item) = child.trim().strip_prefix("- ") {
                tools.push(item.trim_matches(['\'', '"']).to_string());
            }
        }
        return Some(tools);
    }
    None
}

fn validate_agent_contracts(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    include_public: bool,
) {
    for path in scoped_agent_files(include_public) {
        let Some(content) = read_text(&path, exclude) else {
            continue;
        };
        let document = MarkdownDocument::parse(&content);
        let body = document.body();
        let read_line = first_matching_line(body, &READ_INTENT);
        if let (Some(tools), Some(line)) = (frontmatter_explicit_tools(&content), read_line) {
            let suppressed = has_reasoned_marker(&content, "lint-agent-tool-contract: ok");
            if !tools.iter().any(|tool| tool == "Read") && !suppressed {
                diag.report_at(
                    LintRule::AgentReadMismatch,
                    &path,
                    &format!(
                        "{}:{}: explicit tools omit Read but the prompt instructs reading evidence",
                        path.display(),
                        body_line_number(&content, line)
                    ),
                );
            }
        }
        if let (Some(read), Some(output)) = (read_line, first_matching_line(body, &OUTPUT_ONLY)) {
            if (!CANNOT_READ.is_match(body) || !NEVER_INVENT_PROHIBITION.is_match(body))
                && !has_reasoned_marker(&content, "lint-agent-output-mandate: ok")
            {
                diag.report_at(
                    LintRule::AgentOutputUnsafe,
                    &path,
                    &format!(
                        "{}:{}: machine-only output that reads evidence must define an unreadable-evidence outcome and prohibit invented evidence (read instruction at body line {})",
                        path.display(),
                        body_line_number(&content, output),
                        read + 1
                    ),
                );
            }
        }
    }
}

fn first_matching_line(text: &str, pattern: &Regex) -> Option<usize> {
    text.lines().position(|line| pattern.is_match(line))
}

fn body_line_number(content: &str, body_offset: usize) -> usize {
    let body = frontmatter::extract_body(content);
    let prefix_len = content.len().saturating_sub(body.len());
    content[..prefix_len].lines().count() + body_offset + 1
}

fn has_reasoned_marker(content: &str, marker: &str) -> bool {
    content.lines().any(|line| {
        line.find(marker).is_some_and(|index| {
            let remainder = &line[index + marker.len()..];
            remainder.chars().next().is_some_and(char::is_whitespace)
                && !remainder.trim_matches([' ', '-', '>']).is_empty()
        })
    })
}

fn validate_skill_contracts(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    include_public: bool,
) {
    for path in scoped_skill_files(include_public) {
        let Some(content) = read_text(&path, exclude) else {
            continue;
        };
        let document = MarkdownDocument::parse(&content);
        let body = document.body();
        if frontmatter_tools(&content, "allowed-tools")
            .is_some_and(|tools| tools.iter().any(|tool| tool == "Skill"))
        {
            let has_clear_step =
                body.contains("Invoke the Skill tool") || body.contains("via the Skill tool");
            if !has_clear_step {
                diag.report_at(
                    LintRule::SkillInvokeMissing,
                    &path,
                    &format!(
                        "{}: allowed-tools includes Skill but the body has no explicit Skill tool invocation step",
                        path.display()
                    ),
                );
            }
            for (number, line) in lines_outside_fences_with_numbers(body) {
                if SKILL_INVOKE.is_match(line) && !line.contains("via the Skill tool") {
                    diag.report_at(
                        LintRule::SkillInvokeMissing,
                        &path,
                        &format!(
                            "{}:{}: ambiguous skill invocation; identify the Skill tool on the same line",
                            path.display(),
                            body_line_number(&content, number - 1)
                        ),
                    );
                }
            }
        }

        for fence in document.fences() {
            if !is_shell_language(&fence.info) {
                continue;
            }
            for (line_number, command) in logical_commands(&fence.body) {
                validate_flag_signature(&path, line_number, &command, diag);
                validate_awk_fields(&path, line_number, &command, diag);
                validate_grep_probe(&path, line_number, &command, diag);
            }
        }
        validate_skill_closure(&path, diag);
    }
}

fn lines_outside_fences_with_numbers(text: &str) -> Vec<(usize, &str)> {
    let mut tracker = CodeFenceTracker::new();
    text.lines()
        .enumerate()
        .filter_map(|(index, line)| {
            (tracker.process_line(line) == LineClass::Outside).then_some((index + 1, line))
        })
        .collect()
}

fn is_shell_language(language: &str) -> bool {
    matches!(
        language
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "bash" | "sh" | "shell"
    )
}

fn logical_commands(lines: &[(usize, String)]) -> Vec<(usize, String)> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut start = 0;
    for (number, line) in lines {
        if current.is_empty() {
            start = *number;
        } else {
            current.push(' ');
        }
        current.push_str(line.trim_end_matches([' ', '\t', '\\']));
        if !line.trim_end().ends_with('\\') {
            result.push((start, std::mem::take(&mut current)));
        }
    }
    if !current.is_empty() {
        result.push((start, current));
    }
    result
}

fn script_path_from_command(command: &str) -> Option<PathBuf> {
    for raw in command.split_whitespace() {
        let token = raw.trim_matches(|ch: char| "'\"();\\".contains(ch));
        if !token.contains("/scripts/") || !(token.ends_with(".sh") || token.ends_with(".py")) {
            continue;
        }
        for prefix in ["${CLAUDE_PLUGIN_ROOT}/", "$CLAUDE_PLUGIN_ROOT/", "$PWD/"] {
            if let Some(relative) = token.strip_prefix(prefix) {
                return Some(PathBuf::from(relative));
            }
        }
        if Path::new(token).is_absolute() {
            return Some(PathBuf::from(token));
        }
        return Some(PathBuf::from(token.trim_start_matches("./")));
    }
    None
}

fn validate_flag_signature(
    skill: &Path,
    line: usize,
    command: &str,
    diag: &mut DiagnosticCollector,
) {
    if !command.contains("--") || has_reasoned_marker(command, "lint-skill-md-flag-signature: ok") {
        return;
    }
    let Some(script) = script_path_from_command(command) else {
        return;
    };
    let Ok(source) = fs::read_to_string(&script) else {
        return;
    };
    if [
        "\"$@\"",
        "${@}",
        "sys.argv[1:]",
        "parse_known_args",
        "argparse.REMAINDER",
    ]
    .iter()
    .any(|marker| source.contains(marker))
        || forwards_collected_args(&source)
    {
        return;
    }
    for capture in FLAG.captures_iter(command) {
        let flag = &capture[1];
        if !script_declares_flag(&script, &source, flag) {
            diag.report_at(
                LintRule::SkillFlagMismatch,
                skill,
                &format!(
                    "{}:{line}: invocation uses --{flag}, but {} does not accept it",
                    skill.display(),
                    script.display()
                ),
            );
        }
    }
}

fn script_declares_flag(script: &Path, source: &str, flag: &str) -> bool {
    let escaped = regex::escape(flag);
    match script.extension().and_then(|value| value.to_str()) {
        Some("sh") => Regex::new(&format!(r#"(?:^|[\s|])["']?--{escaped}["']?(?:[|)])"#))
            .is_ok_and(|pattern| pattern.is_match(source)),
        Some("py") => [
            format!(r#"add_argument\s*\([^\n)]*["']--{escaped}["']"#),
            format!(r#"(?:click\.)?option\s*\([^)]*["']--{escaped}["']"#),
            format!(r#"typer\.Option\s*\([^)]*["']--{escaped}["']"#),
        ]
        .iter()
        .any(|raw| Regex::new(raw).is_ok_and(|pattern| pattern.is_match(source))),
        _ => false,
    }
}

fn forwards_collected_args(source: &str) -> bool {
    FORWARDED_ARRAY.captures_iter(source).any(|capture| {
        let name = &capture[1];
        source.contains(&format!(r#"{name}+=("$1""#))
            || source.contains(&format!(r#"{name}+=("${{1}}""#))
    })
}

fn shell_lex(command: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut chars = command.chars().peekable();
    while let Some(ch) = chars.next() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(active) = quote {
            if ch == active {
                quote = None;
            } else {
                current.push(ch);
            }
            continue;
        }
        if ch == '\'' || ch == '"' {
            quote = Some(ch);
        } else if ch.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else if "|;&".contains(ch) {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            let mut operator = ch.to_string();
            if chars
                .peek()
                .is_some_and(|next| (*next == ch && ch != ';') || (ch == '|' && *next == '&'))
            {
                operator.push(chars.next().unwrap_or_default());
            }
            tokens.push(operator);
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn awk_programs(command: &str) -> Vec<String> {
    let tokens = shell_lex(command);
    let mut programs = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index].trim_matches(|ch: char| "();".contains(ch));
        if token != "awk" && !token.ends_with("/awk") {
            index += 1;
            continue;
        }
        index += 1;
        let mut source_from_file = false;
        while index < tokens.len() && !matches!(tokens[index].as_str(), "|" | ";" | "&") {
            let token = &tokens[index];
            if matches!(token.as_str(), "-F" | "-v") {
                index += 2;
            } else if token.starts_with("-F") || token.starts_with("-v") {
                index += 1;
            } else if token == "-f" {
                source_from_file = true;
                index += 2;
            } else if token.starts_with("-f") {
                source_from_file = true;
                index += 1;
            } else if token.starts_with('-') || source_from_file {
                index += 1;
            } else {
                programs.push(token.clone());
                index += 1;
                break;
            }
        }
    }
    programs
}

fn validate_awk_fields(skill: &Path, line: usize, command: &str, diag: &mut DiagnosticCollector) {
    if has_reasoned_marker(command, "lint-skill-awk-field-ref: ok") {
        return;
    }
    if awk_programs(command)
        .iter()
        .any(|program| AWK_FIELD.is_match(program))
    {
        diag.report_at(
            LintRule::AwkFieldRef,
            skill,
            &format!(
                "{}:{line}: bare awk positional field in a skill shell fence; move parsing into a shipped script",
                skill.display()
            ),
        );
    }
}

fn validate_grep_probe(skill: &Path, line: usize, command: &str, diag: &mut DiagnosticCollector) {
    if command.trim_start().starts_with('#')
        || has_reasoned_marker(command, "lint-bare-grep-probe: ok")
    {
        return;
    }
    let words = shell_lex(command);
    for (index, word) in words.iter().enumerate() {
        if !matches!(word.as_str(), "grep" | "egrep" | "fgrep" | "rg" | "ripgrep") {
            continue;
        }
        let prefix = &words[..index];
        let end = words[index + 1..]
            .iter()
            .position(|value| matches!(value.as_str(), "|" | "|&" | "||" | "&&" | ";" | "&"))
            .map_or(words.len(), |offset| index + 1 + offset);
        let args = &words[index + 1..end];
        let pipe_fed = prefix
            .last()
            .is_some_and(|value| value == "|" || value == "|&");
        if args.iter().any(|value| {
            Path::new(value)
                .components()
                .any(|part| part == Component::ParentDir)
        }) {
            diag.report_at(
                LintRule::UnsafeGrepProbe,
                skill,
                &format!(
                    "{}:{line}: grep-family path ascends through a parent directory",
                    skill.display()
                ),
            );
            continue;
        }
        let clause_prefix = prefix
            .iter()
            .rev()
            .take_while(|value| !matches!(value.as_str(), "|" | "|&" | "||" | "&&" | ";" | "&"));
        let conditional = clause_prefix
            .clone()
            .any(|value| value == "if" || value == "elif");
        let wrapped = clause_prefix.clone().any(|value| value == "command");
        let bare_grep = word == "grep" && !wrapped && (index == 0 || conditional);
        let dev_null = command.contains("< /dev/null") || command.contains("</dev/null");
        let has_path = grep_has_explicit_path(args);
        if bare_grep {
            diag.report_at(
                LintRule::UnsafeGrepProbe,
                skill,
                &format!(
                    "{}:{line}: bare top-level grep in a shell fence; wrap it or use command grep",
                    skill.display()
                ),
            );
        } else if !pipe_fed && !dev_null && !has_path {
            diag.report_at(
                LintRule::UnsafeGrepProbe,
                skill,
                &format!(
                    "{}:{line}: grep-family probe has no explicit path and may block on stdin",
                    skill.display()
                ),
            );
        }
    }
}

fn grep_has_explicit_path(args: &[String]) -> bool {
    let value_options = [
        "-e",
        "--regexp",
        "-f",
        "--file",
        "-g",
        "--glob",
        "--iglob",
        "-t",
        "--type",
        "-A",
        "-B",
        "-C",
        "-m",
        "--max-count",
        "--max-depth",
        "--include",
        "--exclude",
    ];
    let mut skip = false;
    let mut explicit_pattern = false;
    let mut count = 0;
    for arg in args {
        if skip {
            skip = false;
            continue;
        }
        if matches!(arg.as_str(), "-e" | "--regexp" | "-f" | "--file") {
            explicit_pattern = true;
            skip = true;
        } else if arg.starts_with("-e")
            || arg.starts_with("--regexp=")
            || arg.starts_with("-f")
            || arg.starts_with("--file=")
        {
            explicit_pattern = true;
        } else if value_options.contains(&arg.as_str()) {
            skip = true;
        } else if !arg.starts_with('-') && !matches!(arg.as_str(), "|" | "||" | "&&" | ";") {
            count += 1;
        }
    }
    count >= if explicit_pattern { 1 } else { 2 }
}

fn validate_reference_consecutive_bash(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    include_public: bool,
) {
    let mut roots = vec![PathBuf::from(".claude/skills")];
    if include_public {
        roots.push(PathBuf::from("skills"));
    }
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        let mut paths = Vec::new();
        for entry in traversal::recursive_files(&root, Path::new("."), Some(exclude)).entries {
            let path = entry.path;
            if path.file_name().and_then(|name| name.to_str()) == Some("SKILL.md")
                || path.extension().and_then(|value| value.to_str()) != Some("md")
                || !path
                    .components()
                    .any(|part| part.as_os_str() == "references")
            {
                continue;
            }
            paths.push(path);
        }
        paths.sort();
        for path in paths {
            let Some(content) = read_text(&path, exclude) else {
                continue;
            };
            for (first, second) in consecutive_bash_pairs(&content) {
                diag.report_at(
                    LintRule::ConsecutiveBash,
                    &path,
                    &format!(
                        "{}:{first}: consecutive bash tool-call fences at lines {first} and {second}; combine them or add a reason-bearing lint-consecutive-bash waiver",
                        path.display()
                    ),
                );
            }
        }
    }
}

fn validate_skill_closure(skill: &Path, diag: &mut DiagnosticCollector) {
    let Some(max_lines) = diag.config().skill_closure_max_lines else {
        return;
    };
    let budget = PromptSourceBudget {
        name: skill.display().to_string(),
        roots: vec![skill.display().to_string()],
        conditional_sources: Vec::new(),
        root_caps: PromptMetricCaps::default(),
        closure_caps: PromptMetricCaps {
            lines: Some(max_lines),
            ..PromptMetricCaps::default()
        },
        conditional_caps: PromptMetricCaps::default(),
    };
    let Ok(measurement) = crate::prompt_budget::measure_budget(&budget) else {
        return;
    };
    if measurement.closure.lines > max_lines {
        diag.report_at(
            LintRule::SkillClosureLarge,
            skill,
            &format!(
                "{}: always-loaded prompt closure is {} lines across {} files (configured maximum {max_lines})",
                skill.display(),
                measurement.closure.lines,
                measurement.closure_files.len()
            ),
        );
    }
}

fn validate_prompt_source_budgets(diag: &mut DiagnosticCollector) {
    let budgets = diag.config().prompt_source_budgets.clone();
    for budget in budgets {
        let measurement = match crate::prompt_budget::measure_budget(&budget) {
            Ok(measurement) => measurement,
            Err(message) => {
                diag.report(
                    LintRule::SkillClosureLarge,
                    &format!("prompt-source group '{}': {message}", budget.name),
                );
                continue;
            }
        };
        for row in crate::prompt_budget::report_rows(&budget, &measurement) {
            if row.cap.is_some_and(|cap| row.measured_value > cap) {
                diag.report(
                    LintRule::SkillClosureLarge,
                    &format!(
                        "prompt-source group '{}': {} {} {} is {} (configured maximum {})",
                        row.group,
                        row.source_set,
                        row.scope,
                        row.metric,
                        row.measured_value,
                        row.cap.unwrap_or_default()
                    ),
                );
            }
        }
    }
}

fn validate_claude_import_budget(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let per_file = diag.config().claude_import_max_lines;
    let total_cap = diag.config().claude_import_total_max_lines;
    let path_budgets = diag.config().claude_import_path_budgets.clone();
    if per_file.is_none() && total_cap.is_none() && path_budgets.is_empty()
        || exclude.is_excluded("CLAUDE.md")
    {
        return;
    }
    let mut seen = BTreeSet::new();
    let mut pending = vec![PathBuf::from("CLAUDE.md")];
    let mut total = 0;
    while let Some(path) = pending.pop() {
        let Some(path) = normalize_repo_relative(&path) else {
            continue;
        };
        if seen.contains(&path) || !path.is_file() || path.is_symlink() {
            continue;
        }
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        seen.insert(path.clone());
        let count = crate::prompt_budget::source_metrics(&content).lines;
        total += count;
        if path != Path::new("CLAUDE.md") {
            let normalized = crate::config::normalize_path(&path.to_string_lossy());
            let effective_cap = path_budgets.get(&normalized).copied().or(per_file);
            if effective_cap.is_some_and(|cap| count > cap) {
                diag.report_at(
                    LintRule::ClaudeImportLarge,
                    &path,
                    &format!(
                        "{}: imported prompt source has {count} lines (effective maximum {})",
                        path.display(),
                        effective_cap.unwrap_or_default()
                    ),
                );
            }
        }
        for line in crate::fence::lines_outside_fences(&content) {
            for capture in IMPORT.captures_iter(line) {
                if let Some(candidate) = resolve_repo_reference(&path, &capture[1]) {
                    pending.push(candidate);
                }
            }
        }
    }
    if total_cap.is_some_and(|cap| total > cap) {
        diag.report_at(
            LintRule::ClaudeImportLarge,
            "CLAUDE.md",
            &format!(
                "CLAUDE.md import closure has {total} lines across {} files (configured maximum {})",
                seen.len(),
                total_cap.unwrap_or_default()
            ),
        );
    }
}

fn validate_inline_paths(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let files = diag.config().instruction_files.clone();
    let prefixes = diag.config().inline_path_prefixes.clone();
    for relpath in files {
        if exclude.is_excluded(&relpath) {
            continue;
        }
        let path = Path::new(&relpath);
        let Some(content) = read_text(path, exclude) else {
            continue;
        };
        for (number, line) in lines_outside_fences_with_numbers(&content) {
            if has_reasoned_marker(line, "lint-doc-pointer-paths: ok") {
                continue;
            }
            for capture in INLINE_CODE.captures_iter(line) {
                let token_match = capture
                    .get(1)
                    .expect("INLINE_CODE always has a token capture");
                let token = token_match.as_str();
                if !classify_inline_code_path(token).is_repository_path()
                    || !prefixes.iter().any(|prefix| token.starts_with(prefix))
                {
                    continue;
                }
                let probe = token
                    .split("::")
                    .next()
                    .unwrap_or(token)
                    .split('#')
                    .next()
                    .unwrap_or(token);
                let candidate = Path::new(probe);
                if candidate.is_absolute()
                    || candidate
                        .components()
                        .any(|part| part == Component::ParentDir)
                    || candidate.is_symlink()
                    || !candidate.exists()
                {
                    let start_column = line[..token_match.start()].chars().count() + 1;
                    let end_column = start_column + token.chars().count();
                    let metadata = DiagnosticMetadata::default()
                        .with_location(SourceSpan::range(number, start_column, number, end_column))
                        .with_evidence(token);
                    diag.report_at_with(
                        LintRule::InlinePathMissing,
                        &relpath,
                        &format!("{relpath}:{number}: dead or escaping inline path `{token}`"),
                        metadata,
                    );
                }
            }
        }
    }
}

/// Maximum number of `@import` hops Claude Code resolves before giving up.
const IMPORT_MAX_DEPTH: usize = 5;

/// Resolve an `@import` target to a normalized repository-relative path
/// without requiring it to exist. Returns `None` if the path is absolute,
/// escapes the repository root, or cannot be normalized. Mirrors
/// [`resolve_repo_reference`] minus the existence check so callers can
/// distinguish "unresolvable" from "missing on disk".
fn resolve_import_target(source: &Path, raw: &str) -> Option<PathBuf> {
    let direct = PathBuf::from(raw.trim_start_matches("./"));
    if !direct.is_absolute() && !direct.components().any(|part| part == Component::ParentDir) {
        return normalize_repo_relative(&direct);
    }
    let joined = source.parent().unwrap_or_else(|| Path::new(".")).join(raw);
    normalize_repo_relative(&joined)
}

fn format_import_chain(chain: &[PathBuf], target: &Path) -> String {
    let mut parts: Vec<String> = chain.iter().map(|p| p.display().to_string()).collect();
    parts.push(target.display().to_string());
    parts.join(" → ")
}

/// L001--L004: `@import` graph integrity for each configured instruction file.
///
/// Walks the `@import` tree once per root instruction file, reporting missing
/// targets (L001), circular chains (L002), chains deeper than
/// [`IMPORT_MAX_DEPTH`] hops (L003), and duplicate imports of the same file
/// within a single source (L004, after normalizing leading `./`).
fn validate_import_graph(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    for relpath in diag.config().instruction_files.clone() {
        if exclude.is_excluded(&relpath) {
            continue;
        }
        let root = PathBuf::from(&relpath);
        let Some(root) = normalize_repo_relative(&root) else {
            continue;
        };
        if !root.is_file() || root.is_symlink() {
            continue;
        }
        let mut visited: HashSet<PathBuf> = HashSet::new();
        visited.insert(root.clone());
        let mut stack: Vec<PathBuf> = vec![root.clone()];
        walk_imports(&root, &mut stack, &mut visited, diag);
    }
}

fn walk_imports(
    source: &Path,
    stack: &mut Vec<PathBuf>,
    visited: &mut HashSet<PathBuf>,
    diag: &mut DiagnosticCollector,
) {
    let Ok(content) = fs::read_to_string(source) else {
        return;
    };
    let mut reported_missing: HashSet<PathBuf> = HashSet::new();
    let mut seen_targets: HashSet<PathBuf> = HashSet::new();
    for (number, line) in lines_outside_fences_with_numbers(&content) {
        for capture in IMPORT.captures_iter(line) {
            let raw = &capture[1];
            let resolved = resolve_import_target(source, raw);
            let Some(target) = resolved else {
                let key = PathBuf::from(raw);
                if reported_missing.insert(key.clone()) {
                    diag.report_at(
                        LintRule::ImportPathMissing,
                        source,
                        &format!(
                            "{}:{number}: @import target does not resolve in the repository: {raw}",
                            source.display()
                        ),
                    );
                }
                continue;
            };
            if !target.is_file() {
                if reported_missing.insert(target.clone()) {
                    diag.report_at(
                        LintRule::ImportPathMissing,
                        source,
                        &format!(
                            "{}:{number}: @import target does not exist: {}",
                            source.display(),
                            target.display()
                        ),
                    );
                }
                continue;
            }
            if !seen_targets.insert(target.clone()) {
                diag.report_at(
                    LintRule::DuplicateImport,
                    source,
                    &format!(
                        "{}:{number}: duplicate @import of {}",
                        source.display(),
                        target.display()
                    ),
                );
                continue;
            }
            if stack.len() > IMPORT_MAX_DEPTH {
                diag.report_at(
                    LintRule::ImportDepthExceeded,
                    source,
                    &format!(
                        "{}:{number}: @import chain depth exceeds {IMPORT_MAX_DEPTH} hops: {}",
                        source.display(),
                        format_import_chain(stack, &target)
                    ),
                );
                continue;
            }
            if let Some(index) = stack.iter().position(|p| p == &target) {
                diag.report_at(
                    LintRule::CircularImport,
                    source,
                    &format!(
                        "{}:{number}: circular @import chain: {}",
                        source.display(),
                        format_import_chain(&stack[index..], &target)
                    ),
                );
                continue;
            }
            if visited.insert(target.clone()) {
                stack.push(target.clone());
                walk_imports(&target, stack, visited, diag);
                stack.pop();
            }
        }
    }
}

fn is_external_link(target: &str) -> bool {
    target.contains("://") || target.starts_with("mailto:") || target.starts_with("//")
}

/// L005: broken relative markdown link `[text](path.md)` in any configured
/// instruction file. External URLs, pure anchors, and links inside code fences
/// are skipped; the shared [`MARKDOWN_LINK`] regex already restricts captures
/// to `.md` targets and strips `#anchor` suffixes.
fn validate_markdown_links(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    for relpath in diag.config().instruction_files.clone() {
        if exclude.is_excluded(&relpath) {
            continue;
        }
        let path = Path::new(&relpath);
        let Some(content) = read_text(path, exclude) else {
            continue;
        };
        for (number, line) in lines_outside_fences_with_numbers(&content) {
            for capture in MARKDOWN_LINK.captures_iter(line) {
                let target = &capture[1];
                if is_external_link(target) {
                    continue;
                }
                let Some(resolved) = resolve_import_target(path, target) else {
                    diag.report_at(
                        LintRule::BrokenMarkdownLink,
                        &relpath,
                        &format!("{relpath}:{number}: broken markdown link target: {target}"),
                    );
                    continue;
                };
                if !resolved.is_file() {
                    diag.report_at(
                        LintRule::BrokenMarkdownLink,
                        &relpath,
                        &format!("{relpath}:{number}: broken markdown link target: {target}"),
                    );
                }
            }
        }
    }
}

/// L006: `npm run <script>` referenced from a configured instruction file
/// must exist in `package.json`'s `scripts` map. Silently skipped when there
/// is no `package.json` or it defines no `scripts` object.
fn validate_npm_scripts(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let Ok(pkg_text) = fs::read_to_string("package.json") else {
        return;
    };
    let Ok(pkg) = serde_json::from_str::<serde_json::Value>(&pkg_text) else {
        return;
    };
    let Some(scripts) = pkg.get("scripts").and_then(|value| value.as_object()) else {
        return;
    };
    for relpath in diag.config().instruction_files.clone() {
        if exclude.is_excluded(&relpath) {
            continue;
        }
        let path = Path::new(&relpath);
        let Some(content) = read_text(path, exclude) else {
            continue;
        };
        for (number, line) in lines_outside_fences_with_numbers(&content) {
            for capture in NPM_RUN.captures_iter(line) {
                let script = &capture[1];
                if !scripts.contains_key(script) {
                    diag.report_at(
                        LintRule::NpmScriptMissing,
                        &relpath,
                        &format!(
                            "{relpath}:{number}: npm run {script} is not defined in package.json scripts"
                        ),
                    );
                }
            }
        }
    }
}

#[derive(Clone, Copy, Default)]
struct ScriptScope {
    portability: bool,
    ignore_exclude: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScriptKind {
    Shell,
    Awk,
    Other,
}

fn script_kind(path: &Path) -> ScriptKind {
    let value = path.to_string_lossy();
    if value.ends_with(".sh") || value.ends_with(".inc.bash") {
        ScriptKind::Shell
    } else if value.ends_with(".awk") {
        ScriptKind::Awk
    } else {
        ScriptKind::Other
    }
}

fn conventionally_scoped_scripts(include_public: bool) -> Vec<PathBuf> {
    let mut roots = vec![PathBuf::from(".claude/skills")];
    if include_public {
        roots.extend([PathBuf::from("scripts"), PathBuf::from("skills")]);
    }
    let mut paths = BTreeSet::new();
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        for entry in traversal::recursive_files(&root, Path::new("."), None).entries {
            let path = entry.path;
            if !path.is_symlink()
                && (matches!(script_kind(&path), ScriptKind::Shell | ScriptKind::Awk)
                    || path.extension().and_then(|value| value.to_str()) == Some("py"))
                && (path.components().any(|part| part.as_os_str() == "scripts")
                    || script_kind(&path) == ScriptKind::Awk)
            {
                paths.insert(path);
            }
        }
    }
    paths.into_iter().collect()
}

fn script_scopes(
    diag: &DiagnosticCollector,
    include_public: bool,
) -> BTreeMap<PathBuf, ScriptScope> {
    let has_inventory = diag.config().script_inventory.is_some();
    let mut paths = BTreeMap::new();
    for path in conventionally_scoped_scripts(include_public) {
        paths.insert(
            path,
            ScriptScope {
                portability: !has_inventory,
                ignore_exclude: false,
            },
        );
    }
    if let Some(inventory) = &diag.config().script_inventory {
        for value in inventory {
            paths
                .entry(PathBuf::from(value))
                .and_modify(|scope| {
                    scope.portability = true;
                    scope.ignore_exclude = true;
                })
                .or_insert(ScriptScope {
                    portability: true,
                    ignore_exclude: true,
                });
        }
    }
    paths
}

#[derive(Default)]
struct BashArrayState {
    empty: HashSet<String>,
    known_nonempty: HashSet<String>,
    guards: Vec<ArrayGuard>,
    depth: usize,
}

struct ArrayGuard {
    depth: usize,
    positive: HashSet<String>,
    empty_exit: HashSet<String>,
    exits: bool,
}

impl BashArrayState {
    fn scan_line(
        &mut self,
        path: &Path,
        line_number: usize,
        line: &str,
        suppress_diagnostic: bool,
        diag: &mut DiagnosticCollector,
    ) {
        for capture in ARRAY_ASSIGNMENT.captures_iter(line) {
            let name = capture[1].to_string();
            let values = capture[2].trim();
            if values.is_empty() {
                self.empty.insert(name.clone());
                self.known_nonempty.remove(&name);
            } else {
                self.empty.remove(&name);
                self.known_nonempty.insert(name.clone());
            }
            self.guards.iter_mut().for_each(|guard| {
                guard.positive.remove(&name);
                guard.empty_exit.remove(&name);
            });
        }

        let names: HashSet<_> = ARRAY_LENGTH
            .captures_iter(line)
            .map(|capture| capture[1].to_string())
            .collect();
        let opens = IF_THEN.is_match(line);
        if opens {
            self.depth += 1;
            let positive = if is_positive_array_guard(line) {
                names.clone()
            } else {
                HashSet::new()
            };
            let empty_exit = if is_empty_array_guard(line) {
                names.clone()
            } else {
                HashSet::new()
            };
            self.guards.push(ArrayGuard {
                depth: self.depth,
                positive,
                empty_exit,
                exits: false,
            });
        }

        for name in self.empty.clone() {
            if self.known_nonempty.contains(&name)
                || names.contains(&name)
                || self
                    .guards
                    .iter()
                    .any(|guard| guard.depth <= self.depth && guard.positive.contains(&name))
            {
                continue;
            }
            for suffix in ["@", "*"] {
                let expansion = format!("${{{name}[{suffix}]}}");
                if !suppress_diagnostic
                    && line.contains(&expansion)
                    && !safe_conditional_array_expansion(line, &name, suffix)
                {
                    diag.report_at(
                        LintRule::Bash32Incompatible,
                        path,
                        &format!(
                            "{}:{line_number}: Bash 3.2 incompatible unguarded empty-array expansion {expansion}",
                            path.display()
                        ),
                    );
                }
            }
        }

        if EXIT_OR_RETURN.is_match(line) {
            for guard in &mut self.guards {
                if guard.depth == self.depth && !guard.empty_exit.is_empty() {
                    guard.exits = true;
                }
            }
        }

        if FI.is_match(line) && self.depth > 0 {
            let depth = self.depth;
            let completed: Vec<_> = self
                .guards
                .iter()
                .enumerate()
                .filter(|(_, guard)| guard.depth == depth)
                .map(|(index, _)| index)
                .collect();
            for index in completed.into_iter().rev() {
                let guard = self.guards.remove(index);
                if guard.exits {
                    for name in guard.empty_exit {
                        self.empty.remove(&name);
                        self.known_nonempty.insert(name);
                    }
                }
            }
            self.depth -= 1;
        }
    }
}

fn is_positive_array_guard(line: &str) -> bool {
    POSITIVE_ARRAY_COMPARISON.is_match(line)
}

fn is_empty_array_guard(line: &str) -> bool {
    EMPTY_ARRAY_COMPARISON.is_match(line)
}

fn safe_conditional_array_expansion(line: &str, name: &str, suffix: &str) -> bool {
    line.contains(&format!("${{{name}[{suffix}]+\"${{{name}[{suffix}]}}\"}}"))
}

struct HeredocState {
    delimiter: String,
    awk: bool,
    quoted: bool,
}

fn heredoc_state(line: &str, awk: bool) -> Option<HeredocState> {
    let captures = HEREDOC.captures(line)?;
    let quoted = captures.get(1).is_some() || captures.get(2).is_some();
    let delimiter = captures
        .get(1)
        .or_else(|| captures.get(2))
        .or_else(|| captures.get(3))?
        .as_str()
        .to_string();
    Some(HeredocState {
        delimiter,
        awk,
        quoted,
    })
}

fn closes_heredoc(line: &str, delimiter: &str) -> bool {
    line.trim() == delimiter
}

fn validate_script_contracts(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    include_public: bool,
) {
    for (path, scope) in script_scopes(diag, include_public) {
        let content = if scope.ignore_exclude {
            (!path.is_symlink())
                .then(|| fs::read_to_string(&path).ok())
                .flatten()
        } else {
            read_text(&path, exclude)
        };
        let Some(content) = content else {
            continue;
        };
        let kind = script_kind(&path);
        let mut heredoc: Option<HeredocState> = None;
        let mut continuation = String::new();
        let mut awk_single_body = false;
        let mut previous = "";
        let mut arrays = BashArrayState::default();
        for (index, raw) in content.lines().enumerate() {
            let continued = raw.trim_end().ends_with('\\');
            if continued {
                if !continuation.is_empty() {
                    continuation.push(' ');
                }
                continuation.push_str(raw.trim_end().trim_end_matches('\\'));
                continue;
            }
            let line = if continuation.is_empty() {
                raw.to_string()
            } else {
                continuation.push(' ');
                continuation.push_str(raw);
                std::mem::take(&mut continuation)
            };
            let line_number = index + 1;

            if let Some(active) = &heredoc {
                if closes_heredoc(&line, &active.delimiter) {
                    heredoc = None;
                    previous = raw;
                    continue;
                }
                if scope.portability && active.awk {
                    validate_awk_body(&path, line_number, &line, diag);
                } else if scope.portability && !active.quoted && kind == ScriptKind::Shell {
                    validate_bash_replacement(&path, line_number, &line, previous, diag);
                }
                previous = raw;
                continue;
            }

            if awk_single_body {
                if scope.portability {
                    validate_awk_body(&path, line_number, &line, diag);
                }
                if line.matches('\'').count() % 2 == 1 {
                    awk_single_body = false;
                }
                previous = raw;
                continue;
            }

            if line.trim_start().starts_with('#') {
                previous = raw;
                continue;
            }

            if kind != ScriptKind::Awk {
                validate_gh_inline(&path, line_number, &line, diag);
            }
            if scope.portability {
                match kind {
                    ScriptKind::Shell => {
                        validate_bash_replacement(&path, line_number, &line, previous, diag);
                        let bash32_suppressed = has_reasoned_marker(&line, "lint-bash32: ok");
                        arrays.scan_line(&path, line_number, &line, bash32_suppressed, diag);
                        validate_bash32(&path, line_number, &line, diag);
                        validate_awk_command(&path, line_number, &line, diag);
                    }
                    ScriptKind::Awk => validate_awk_body(&path, line_number, &line, diag),
                    ScriptKind::Other => {}
                }
            }

            let is_awk_command = kind == ScriptKind::Shell && AWK_COMMAND.is_match(&line);
            heredoc = heredoc_state(&line, is_awk_command);
            if scope.portability
                && is_awk_command
                && heredoc.is_none()
                && line.matches('\'').count() % 2 == 1
            {
                awk_single_body = true;
            }
            previous = raw;
        }
        if !continuation.is_empty() && scope.portability {
            let line_number = content.lines().count();
            match kind {
                ScriptKind::Shell => {
                    validate_bash_replacement(&path, line_number, &continuation, previous, diag);
                    let bash32_suppressed = has_reasoned_marker(&continuation, "lint-bash32: ok");
                    arrays.scan_line(&path, line_number, &continuation, bash32_suppressed, diag);
                    validate_bash32(&path, line_number, &continuation, diag);
                    validate_awk_command(&path, line_number, &continuation, diag);
                    if awk_single_body {
                        validate_awk_body(&path, line_number, &continuation, diag);
                    }
                }
                ScriptKind::Awk => validate_awk_body(&path, line_number, &continuation, diag),
                ScriptKind::Other => {}
            }
        }
    }
}

fn validate_bash_replacement(
    path: &Path,
    line_number: usize,
    line: &str,
    previous: &str,
    diag: &mut DiagnosticCollector,
) {
    if BASH_REPLACEMENT.is_match(line)
        && !has_reasoned_marker(line, "lint-renderer-safe: ok")
        && !has_reasoned_marker(previous, "lint-renderer-safe: ok")
    {
        diag.report_at(
            LintRule::BashReplacementUnsafe,
            path,
            &format!(
                "{}:{line_number}: unsafe Bash global substitution with a variable replacement",
                path.display()
            ),
        );
    }
}

fn validate_gh_inline(path: &Path, line_number: usize, line: &str, diag: &mut DiagnosticCollector) {
    if has_reasoned_marker(line, "lint-gh-body-inline: ok")
        || !(line.contains("gh ") || line.contains("/gh "))
    {
        return;
    }
    for (option, replacement) in [("--body", "--body-file"), ("--notes", "--notes-file")] {
        if line
            .split_whitespace()
            .any(|word| word == option || word.starts_with(&format!("{option}=")))
            && !line.contains(replacement)
        {
            diag.report_at(
                LintRule::GhInlineBody,
                path,
                &format!(
                    "{}:{line_number}: inline gh {option} payload; use {replacement}",
                    path.display()
                ),
            );
        }
    }
}

fn validate_bash32(path: &Path, line_number: usize, line: &str, diag: &mut DiagnosticCollector) {
    if has_reasoned_marker(line, "lint-bash32: ok") {
        return;
    }
    for (pattern, label) in BASH32_PATTERNS.iter() {
        if pattern.is_match(line) {
            diag.report_at(
                LintRule::Bash32Incompatible,
                path,
                &format!(
                    "{}:{line_number}: Bash 3.2 incompatible {label}",
                    path.display()
                ),
            );
        }
    }
}

fn validate_awk_command(
    path: &Path,
    line_number: usize,
    line: &str,
    diag: &mut DiagnosticCollector,
) {
    if line.is_ascii()
        || has_reasoned_marker(line, "lint-awk-multibyte-regex: ok")
        || !AWK_COMMAND.is_match(line)
    {
        return;
    }
    if AWK_VALUE.captures_iter(line).any(|capture| {
        capture
            .get(1)
            .or_else(|| capture.get(2))
            .or_else(|| capture.get(3))
            .is_some_and(|value| !value.as_str().is_ascii())
    }) {
        diag.report_at(
            LintRule::AwkRegexNonascii,
            path,
            &format!(
                "{}:{line_number}: non-ASCII awk -v value may be used as an implementation-dependent regex",
                path.display()
            ),
        );
    }
    validate_awk_body(path, line_number, line, diag);
}

fn validate_awk_body(path: &Path, line_number: usize, line: &str, diag: &mut DiagnosticCollector) {
    if line.is_ascii()
        || line.trim_start().starts_with('#')
        || has_reasoned_marker(line, "lint-awk-multibyte-regex: ok")
        || !AWK_REGEX_CONTEXT.is_match(line)
    {
        return;
    }
    diag.report_at(
        LintRule::AwkRegexNonascii,
        path,
        &format!(
            "{}:{line_number}: non-ASCII text in an awk regex context is not portable",
            path.display()
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LintConfig;

    fn all_enabled_with(mut config: LintConfig) -> DiagnosticCollector {
        for rule in crate::rules::ALL_RULES {
            config.error.insert(*rule);
        }
        DiagnosticCollector::with_config_silent(config)
    }

    #[test]
    #[serial_test::serial]
    fn agent_contracts_are_distinct_and_fail_closed() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all(".claude/agents").unwrap();
        fs::write(
            ".claude/agents/judge.md",
            "---\nname: judge\ndescription: Evidence judge prompt\ntools: [Bash]\n---\nRead every evidence file. Output strict JSONL only.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agent_contracts(&mut diag, &ExcludeSet::default(), false);
        assert!(
            diag.diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::AgentReadMismatch)
        );
        assert!(
            diag.diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::AgentOutputUnsafe)
        );
    }

    #[test]
    #[serial_test::serial]
    fn agent_output_contract_accepts_explicit_unreadable_and_do_not_invent_language() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all(".claude/agents").unwrap();
        fs::write(
            ".claude/agents/judge.md",
            "---\nname: judge\ndescription: Evidence judge prompt\ntools: [Read]\n---\nRead every evidence file. For an unreadable file, return NEEDS_DEEP and do not invent evidence. Emit strict JSONL only.\n",
        )
        .unwrap();
        fs::write(
            ".claude/agents/scalar.md",
            "---\nname: scalar\ndescription: Scalar tools are not an explicit list\ntools: Bash\n---\nRead every evidence file.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agent_contracts(&mut diag, &ExcludeSet::default(), false);
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::AgentOutputUnsafe)
        );
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::AgentReadMismatch)
        );
    }

    #[test]
    #[serial_test::serial]
    fn skill_invocation_rule_ignores_non_skill_invoke_prose() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all(".claude/skills/demo").unwrap();
        fs::write(
            ".claude/skills/demo/SKILL.md",
            "---\nname: demo\ndescription: Use when validating invocation language\nallowed-tools: Skill, Bash\n---\nInvoke the Skill tool for child skills. On dry-run, invoke `python/cli.py check` synchronously.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_contracts(&mut diag, &ExcludeSet::default(), false);
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::SkillInvokeMissing)
        );
    }

    #[test]
    #[serial_test::serial]
    fn skill_shell_contracts_find_each_problem_without_shell_positional_false_positive() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all(".claude/skills/demo/scripts").unwrap();
        fs::write(
            ".claude/skills/demo/scripts/run.sh",
            "#!/bin/sh\ncase \"$1\" in --known) ;; esac\n",
        )
        .unwrap();
        fs::write(
            ".claude/skills/demo/SKILL.md",
            "---\nname: demo\ndescription: Use when validating shell prompt contracts\nallowed-tools: Skill, Bash\n---\nInvoke `/other`.\n```bash\n$PWD/.claude/skills/demo/scripts/run.sh --missing\necho $1\nawk -F ',' '{print $1}' input\ncommand rg needle\n```\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_contracts(&mut diag, &ExcludeSet::default(), false);
        for rule in [
            LintRule::SkillInvokeMissing,
            LintRule::SkillFlagMismatch,
            LintRule::AwkFieldRef,
            LintRule::UnsafeGrepProbe,
        ] {
            assert!(
                diag.diagnostics().iter().any(|item| item.rule == rule),
                "missing {rule:?}"
            );
        }
        assert_eq!(
            diag.diagnostics()
                .iter()
                .filter(|item| item.rule == LintRule::AwkFieldRef)
                .count(),
            1
        );
    }

    #[test]
    #[serial_test::serial]
    fn configured_closures_and_inline_paths_fire() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all("skills/demo").unwrap();
        fs::create_dir_all("docs").unwrap();
        fs::create_dir_all("skills/shared").unwrap();
        fs::write(
            "skills/demo/SKILL.md",
            "Read `../shared/large.md` completely.\n",
        )
        .unwrap();
        fs::write("skills/shared/large.md", "line\nline\nline\n").unwrap();
        fs::write("docs/large.md", "line\nline\nline\n").unwrap();
        fs::write("CLAUDE.md", "@docs/large.md\n").unwrap();
        fs::write("AGENTS.md", "See `docs/missing.md`.\n").unwrap();
        let config = LintConfig {
            skill_closure_max_lines: Some(2),
            claude_import_max_lines: Some(2),
            claude_import_total_max_lines: Some(3),
            ..LintConfig::default()
        };
        let mut diag = all_enabled_with(config);
        validate_skill_closure(Path::new("skills/demo/SKILL.md"), &mut diag);
        validate_claude_import_budget(&mut diag, &ExcludeSet::default());
        validate_inline_paths(&mut diag, &ExcludeSet::default());
        for rule in [
            LintRule::SkillClosureLarge,
            LintRule::ClaudeImportLarge,
            LintRule::InlinePathMissing,
        ] {
            assert!(
                diag.diagnostics().iter().any(|item| item.rule == rule),
                "missing {rule:?}"
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn d005_uses_shared_classification_within_its_prefix_scope() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::write(
            "AGENTS.md",
            "Python files may match `docs/*.py`; see `docs/missing.md`.\n",
        )
        .unwrap();
        let mut diag = all_enabled_with(LintConfig::default());

        validate_inline_paths(&mut diag, &ExcludeSet::default());

        let findings: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::InlinePathMissing)
            .collect();
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].subject_path.as_deref(),
            Some(Path::new("AGENTS.md"))
        );
        assert_eq!(findings[0].evidence.as_deref(), Some("docs/missing.md"));
        assert!(findings[0].location.is_some());
    }

    #[test]
    #[serial_test::serial]
    fn path_specific_import_caps_override_the_global_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::write(
            "CLAUDE.md",
            "@./AGENTS.md\n@KARPATHY_CLAUDE.md\n@BASH_AUTHORING.md\n",
        )
        .unwrap();
        fs::write("AGENTS.md", "a\na\na\n").unwrap();
        fs::write("KARPATHY_CLAUDE.md", "k\nk\nk\nk\n").unwrap();
        fs::write("BASH_AUTHORING.md", "b\nb\nb\nb\nb\n").unwrap();
        let config = LintConfig {
            claude_import_max_lines: Some(1),
            claude_import_path_budgets: BTreeMap::from([
                ("AGENTS.md".into(), 2),
                ("BASH_AUTHORING.md".into(), 5),
                ("KARPATHY_CLAUDE.md".into(), 3),
            ]),
            ..LintConfig::default()
        };
        let mut diag = all_enabled_with(config);

        validate_claude_import_budget(&mut diag, &ExcludeSet::default());

        let findings: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::ClaudeImportLarge)
            .map(|item| item.message.as_str())
            .collect();
        assert_eq!(findings.len(), 2);
        assert!(findings.iter().any(|message| message.contains("AGENTS.md")
            && message.contains("3 lines")
            && message.contains("maximum 2")));
        assert!(findings.iter().any(|message| {
            message.contains("KARPATHY_CLAUDE.md")
                && message.contains("4 lines")
                && message.contains("maximum 3")
        }));
        assert!(
            !findings
                .iter()
                .any(|message| message.contains("BASH_AUTHORING"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn named_prompt_source_groups_measure_conditional_sources_separately() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all("skills/review").unwrap();
        fs::write(
            "skills/review/SKILL.md",
            "Read `always.md` completely.\nroot\n",
        )
        .unwrap();
        fs::write("skills/review/always.md", "always\nalways\n").unwrap();
        fs::write("skills/review/branch.md", "branch\nbranch\nbranch\n").unwrap();
        let config = LintConfig {
            prompt_source_budgets: vec![PromptSourceBudget {
                name: "review".into(),
                roots: vec!["skills/review/SKILL.md".into()],
                conditional_sources: vec!["skills/review/branch.md".into()],
                root_caps: PromptMetricCaps {
                    lines: Some(2),
                    ..PromptMetricCaps::default()
                },
                closure_caps: PromptMetricCaps {
                    lines: Some(3),
                    ..PromptMetricCaps::default()
                },
                conditional_caps: PromptMetricCaps {
                    lines: Some(2),
                    ..PromptMetricCaps::default()
                },
            }],
            ..LintConfig::default()
        };
        let mut diag = all_enabled_with(config);

        validate_prompt_source_budgets(&mut diag);

        let findings: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::SkillClosureLarge)
            .map(|item| item.message.as_str())
            .collect();
        assert_eq!(findings.len(), 2);
        assert!(
            findings
                .iter()
                .any(|message| message.contains("always closure lines is 4"))
        );
        assert!(
            findings
                .iter()
                .any(|message| message.contains("conditional closure lines is 3"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn flag_parity_understands_python_and_skips_forwarders() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all("skills/demo/scripts").unwrap();
        fs::write(
            "skills/demo/scripts/python.py",
            "parser.add_argument(\"--known-option\")\n",
        )
        .unwrap();
        fs::write(
            "skills/demo/scripts/forward.sh",
            "#!/bin/sh\nexec other \"$@\"\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_flag_signature(
            Path::new("skills/demo/SKILL.md"),
            1,
            "${CLAUDE_PLUGIN_ROOT}/skills/demo/scripts/python.py --known-option",
            &mut diag,
        );
        validate_flag_signature(
            Path::new("skills/demo/SKILL.md"),
            2,
            "${CLAUDE_PLUGIN_ROOT}/skills/demo/scripts/forward.sh --delegated",
            &mut diag,
        );
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::SkillFlagMismatch)
        );
    }

    #[test]
    #[serial_test::serial]
    fn flag_parity_does_not_treat_help_text_as_a_declaration() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all("skills/demo/scripts").unwrap();
        fs::write(
            "skills/demo/scripts/run.sh",
            "#!/bin/sh\n# Usage mentions --undocumented but the parser does not accept it.\ncase \"$1\" in --known) ;; esac\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_flag_signature(
            Path::new("skills/demo/SKILL.md"),
            1,
            "${CLAUDE_PLUGIN_ROOT}/skills/demo/scripts/run.sh --undocumented",
            &mut diag,
        );
        assert!(
            diag.diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::SkillFlagMismatch)
        );
    }

    #[test]
    #[serial_test::serial]
    fn flag_parity_accepts_quoted_alternate_case_arms() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all("skills/demo/scripts").unwrap();
        fs::write(
            "skills/demo/scripts/run.sh",
            "#!/bin/sh\ncase \"$1\" in \"--primary\"|\"--alias\") shift ;; esac\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_flag_signature(
            Path::new("skills/demo/SKILL.md"),
            1,
            "${CLAUDE_PLUGIN_ROOT}/skills/demo/scripts/run.sh --alias",
            &mut diag,
        );
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::SkillFlagMismatch)
        );
    }

    #[test]
    fn grep_probe_accepts_explicit_pattern_option_and_attached_pipe() {
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_grep_probe(
            Path::new("skills/demo/SKILL.md"),
            1,
            "command grep -e needle file.txt",
            &mut diag,
        );
        validate_grep_probe(
            Path::new("skills/demo/SKILL.md"),
            2,
            "printf data|rg needle",
            &mut diag,
        );
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::UnsafeGrepProbe)
        );
    }

    #[test]
    #[serial_test::serial]
    fn reference_bash_scope_excludes_script_documentation() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all("skills/demo/scripts").unwrap();
        fs::create_dir_all("skills/demo/references").unwrap();
        let adjacent = "```bash\necho one\n```\n```bash\necho two\n```\n";
        fs::write("skills/demo/scripts/usage.md", adjacent).unwrap();
        fs::write("skills/demo/references/workflow.md", adjacent).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_reference_consecutive_bash(&mut diag, &ExcludeSet::default(), true);
        let findings: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::ConsecutiveBash)
            .collect();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("references/workflow.md"));
    }

    #[test]
    #[serial_test::serial]
    fn reference_bash_diagnostics_are_deterministic_across_files() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let adjacent = "```bash\necho one\n```\n```bash\necho two\n```\n";
        for name in ["zeta", "alpha"] {
            fs::create_dir_all(format!("skills/{name}/references")).unwrap();
            fs::write(format!("skills/{name}/references/workflow.md"), adjacent).unwrap();
        }
        let mut diag = DiagnosticCollector::new_all_enabled();

        validate_reference_consecutive_bash(&mut diag, &ExcludeSet::default(), true);

        let findings: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::ConsecutiveBash)
            .map(|item| item.message.as_str())
            .collect();
        assert_eq!(findings.len(), 2);
        assert!(findings[0].contains("skills/alpha/"));
        assert!(findings[1].contains("skills/zeta/"));
    }

    #[test]
    fn awk_parser_distinguishes_programs_from_option_values() {
        assert_eq!(awk_programs("echo $1"), Vec::<String>::new());
        assert_eq!(
            awk_programs("awk -F ',' '{print $1}' input"),
            ["{print $1}"]
        );
        assert_eq!(
            awk_programs("awk -v value='$1' 'BEGIN { print value }'"),
            ["BEGIN { print value }"]
        );
    }

    #[test]
    #[serial_test::serial]
    fn shipped_script_safety_rules_fire() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all("scripts").unwrap();
        fs::write(
            "scripts/bad.sh",
            "gh pr create --body payload\ndeclare -A values\nempty=()\nprintf '%s' \"${empty[@]}\"\nout=${text//x/$replacement}\ncat <<EOF\ncopy=${text//x/$replacement}\nEOF\ncat <<'SAFE'\nliteral=${text//x/$replacement}\nSAFE\nawk -v re='—' '$0 ~ re'\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_script_contracts(&mut diag, &ExcludeSet::default(), true);
        for rule in [
            LintRule::GhInlineBody,
            LintRule::BashReplacementUnsafe,
            LintRule::Bash32Incompatible,
            LintRule::AwkRegexNonascii,
        ] {
            assert!(
                diag.diagnostics().iter().any(|item| item.rule == rule),
                "missing {rule:?}"
            );
        }
        assert_eq!(
            diag.diagnostics()
                .iter()
                .filter(|item| item.rule == LintRule::BashReplacementUnsafe)
                .count(),
            2
        );
    }

    #[test]
    #[serial_test::serial]
    fn bash_replacement_matches_renderer_harness_cases() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir("scripts").unwrap();
        fs::write(
            "scripts/unsafe.sh",
            "out=\"${out//TOKEN/$rep}\"\nout=\"${out//TOKEN/${rep}}\"\nout=\"${out//TOKEN/$arr[0]}\"\nout=\"${out//TOKEN/$rep}\" # lint-renderer-safe: okay is not a waiver\n",
        )
        .unwrap();
        fs::write(
            "scripts/safe.sh",
            "before=\"${body%%TOKEN*}\"\nafter=\"${body##*TOKEN}\"\nout=\"${out//$'\\n'/$'\\n    '}\"\nout=\"${out//TOKEN/$rep}\" # lint-renderer-safe: ok trusted constant\n# lint-renderer-safe: ok reviewed fixture\nout=\"${out//TOKEN/$rep}\"\ncat <<'FIXTURE'\nout=\"${out//TOKEN/$rep}\"\nFIXTURE\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();

        validate_script_contracts(&mut diag, &ExcludeSet::default(), true);

        let findings: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::BashReplacementUnsafe)
            .map(|item| item.message.as_str())
            .collect();
        assert_eq!(findings.len(), 4);
        for line in 1..=4 {
            assert!(
                findings
                    .iter()
                    .any(|message| message.contains(&format!("scripts/unsafe.sh:{line}:")))
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn bash32_accepts_exit_guarded_arrays_and_parameterized_parent_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all("scripts").unwrap();
        fs::write(
            "scripts/portable.sh",
            "#!/usr/bin/env bash\nROOT=\"${ROOT:-$(cd \"$DIR/../../..\" && pwd)}\"\nfiles=()\nif [ ${#files[@]} -eq 0 ]; then\n  exit 0\nfi\nprintf '%s\\n' \"${files[@]}\"\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_script_contracts(&mut diag, &ExcludeSet::default(), true);
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::Bash32Incompatible)
        );
    }

    #[test]
    #[serial_test::serial]
    fn bash32_ports_forbidden_and_negative_larch_fixtures() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all("scripts").unwrap();
        fs::write(
            "scripts/contracts.inc.bash",
            r#"# shellcheck shell=bash
# declare -A documentation is ignored
declare -A seen=()
typeset -x -A legacy=()
mapfile -t rows < input
readarray -t more < input
printf '%s' "${NAME^^}" "${NAME^}" "${NAME,,}" "${NAME,}"
declare -n ref=target
local -x -n inner=target
cmd &>>log
coproc WORKER { cat; }
arr=(a b); printf '%s' "${arr[-1]}"
printf '%s' {1..10..2}
if command grep -q needle file; then :; fi
elif command rg -q needle .; then :; fi
if ( command grep -q needle file ) 2>/dev/null; then :; fi
safe="${MYVAR//[^A-Za-z0-9_-]/_}"
declare -A reviewed=() # lint-bash32: ok intentional compatibility shim
declare -A no_reason=() # lint-bash32: ok
"#,
        )
        .unwrap();
        let mut diag = DiagnosticCollector::with_config_silent(LintConfig::default());
        validate_script_contracts(&mut diag, &ExcludeSet::default(), true);
        let findings: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::Bash32Incompatible)
            .collect();
        for label in [
            "declare -A associative arrays",
            "typeset -A associative arrays",
            "mapfile/readarray",
            "parameter case conversion",
            "declare -n nameref",
            "local -n nameref",
            "&>> append-all redirection",
            "coproc",
            "negative array index",
            "stepped brace expansion",
            "if/elif command grep-family condition",
        ] {
            assert!(
                findings.iter().any(|item| item.message.contains(label)),
                "missing fixture for {label}"
            );
        }
        assert!(findings.iter().any(|item| item.message.contains(":19:")));
        assert!(!findings.iter().any(|item| item.message.contains(":18:")));
        assert!(!findings.iter().any(|item| item.message.contains(":16:")));
    }

    #[test]
    #[serial_test::serial]
    fn bash32_empty_array_analysis_tracks_guards_assignments_and_safe_expansion() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all("scripts").unwrap();
        fs::write(
            "scripts/arrays.sh",
            r#"items=()
printf '%s\n' "${items[@]}"
if [ "${#items[@]}" -gt 0 ]; then
  printf '%s\n' "${items[@]}"
fi
printf '%s\n' "${items[*]}"
items=(one)
printf '%s\n' "${items[@]}"
items=()
printf '%s\n' ${items[@]+"${items[@]}"}
if [ "${#items[@]}" -eq 0 ]; then
  exit 0
fi
printf '%s\n' "${items[@]}"
other=()
if [ "${#other[@]}" -gt 0 ]; then
  printf '%s\n' "${other[@]}"
fi
printf '%s\n' "${other[@]}"
suppressed=() # lint-bash32: ok state still must be tracked
printf '%s\n' "${suppressed[@]}"
reverse=()
if [ "${#reverse[@]}" != 0 ]; then
  exit 0
fi
printf '%s\n' "${reverse[@]}"
"#,
        )
        .unwrap();
        let mut diag = DiagnosticCollector::with_config_silent(LintConfig::default());
        validate_script_contracts(&mut diag, &ExcludeSet::default(), true);
        let lines: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::Bash32Incompatible)
            .map(|item| item.message.clone())
            .collect();
        assert_eq!(lines.len(), 5, "unexpected findings: {lines:?}");
        for line in [2, 6, 19, 21, 26] {
            assert!(
                lines
                    .iter()
                    .any(|message| message.contains(&format!(":{line}:"))),
                "missing line {line}: {lines:?}"
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn awk_ports_continuation_heredoc_multiline_and_display_fixtures() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all("scripts").unwrap();
        fs::write(
            "scripts/awk.sh",
            r#"# awk -v label='テスト' 'BEGIN { print label }'
awk -v label='テスト' 'BEGIN { print label }'
awk 'BEGIN { printf "テスト\n" }'
awk -v label = \
  'テスト' 'BEGIN { print label }'
cat <<'DOC'
awk -v ignored='テスト' 'BEGIN { print ignored }'
DOC
awk -f - <<'AWK'
BEGIN { if (match($0, "—")) print "hit" }
AWK
awk 'BEGIN {
  if ($0 !~ "—") print
  gsub("—", "-", $0)
  sub("—", "-", $0)
  split($0, parts, "—")
}' | cat
awk -v reviewed='テスト' 'BEGIN { print }' # lint-awk-multibyte-regex: ok display only
awk -v no_reason='テスト' 'BEGIN { print }' # lint-awk-multibyte-regex: ok
"#,
        )
        .unwrap();
        let mut diag = DiagnosticCollector::with_config_silent(LintConfig::default());
        validate_script_contracts(&mut diag, &ExcludeSet::default(), true);
        let lines: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::AwkRegexNonascii)
            .map(|item| item.message.clone())
            .collect();
        for line in [2, 5, 10, 13, 14, 15, 16, 19] {
            assert!(
                lines
                    .iter()
                    .any(|message| message.contains(&format!(":{line}:"))),
                "missing line {line}: {lines:?}"
            );
        }
        for line in [1, 3, 7, 18] {
            assert!(
                !lines
                    .iter()
                    .any(|message| message.contains(&format!(":{line}:"))),
                "unexpected line {line}: {lines:?}"
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn explicit_inventory_scans_untracked_excluded_files_and_limits_scope() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all("portable").unwrap();
        fs::write(
            "portable/inventory.txt",
            "portable/in-scope.sh\nportable/helper.inc.bash\nportable/rules.awk\n",
        )
        .unwrap();
        fs::write(
            "portable/in-scope.sh",
            "declare -A bad=()\nout=${text//x/$replacement}\n",
        )
        .unwrap();
        fs::write("portable/helper.inc.bash", "mapfile -t rows < input\n").unwrap();
        fs::write("portable/rules.awk", "BEGIN { match($0, \"—\") }\n").unwrap();
        fs::write("portable/out-of-scope.sh", "declare -A hidden=()\n").unwrap();
        fs::write(
            "agent-lint.toml",
            "[lint]\nerror = [\"G010\", \"G011\"]\nscript-inventory = \"portable/inventory.txt\"\nexclude = [\"portable/**\"]\n",
        )
        .unwrap();
        let config = LintConfig::load(".").unwrap();
        let exclude = config.build_exclude_set();
        let mut diag = DiagnosticCollector::with_config_silent(config);
        validate_script_contracts(&mut diag, &exclude, false);

        for rule in [
            LintRule::BashReplacementUnsafe,
            LintRule::Bash32Incompatible,
            LintRule::AwkRegexNonascii,
        ] {
            assert!(
                diag.diagnostics().iter().any(|item| item.rule == rule),
                "missing {rule:?}"
            );
        }
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|item| item.message.contains("out-of-scope"))
        );

        let mut all_config = LintConfig::load(".").unwrap();
        all_config
            .suppress
            .extend([LintRule::Bash32Incompatible, LintRule::AwkRegexNonascii]);
        all_config.apply_cli_mode(crate::config::CliMode::All);
        let all_exclude = all_config.build_exclude_set();
        let mut all_diag = DiagnosticCollector::with_config_silent(all_config);
        validate_script_contracts(&mut all_diag, &all_exclude, false);
        assert!(
            all_diag
                .diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::Bash32Incompatible)
        );
        assert!(
            all_diag
                .diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::AwkRegexNonascii)
        );
    }

    #[test]
    fn robust_fences_keep_embedded_short_delimiters_inside() {
        let fences = crate::fence::markdown_fences("````bash\necho hi\n```\necho still\n````\n");
        assert_eq!(fences.len(), 1);
        assert_eq!(fences[0].body.len(), 3);
    }

    // ── L001: import-path-missing ───────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn l001_flags_missing_import_target_once_per_pair() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::write("CLAUDE.md", "@docs/missing.md\n@./docs/missing.md\n").unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_import_graph(&mut diag, &ExcludeSet::default());
        let l001: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::ImportPathMissing)
            .collect();
        assert_eq!(l001.len(), 1, "dedup missing target per (source, target)");
        assert!(l001[0].message.contains("does not exist"));
    }

    // ── L002: circular-import ───────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn l002_reports_circular_import_chain() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all("docs").unwrap();
        fs::write("CLAUDE.md", "@docs/a.md\n").unwrap();
        fs::write("docs/a.md", "@CLAUDE.md\n").unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_import_graph(&mut diag, &ExcludeSet::default());
        let cycle: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::CircularImport)
            .collect();
        assert_eq!(cycle.len(), 1);
        assert!(cycle[0].message.contains("circular"));
        assert!(cycle[0].message.contains("CLAUDE.md"));
        assert!(cycle[0].message.contains("docs/a.md"));
    }

    // ── L003: import-depth-exceeded ─────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn l003_flags_chain_deeper_than_five_hops() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all("docs").unwrap();
        fs::write("CLAUDE.md", "@docs/a.md\n").unwrap();
        fs::write("docs/a.md", "@docs/b.md\n").unwrap();
        fs::write("docs/b.md", "@docs/c.md\n").unwrap();
        fs::write("docs/c.md", "@docs/d.md\n").unwrap();
        fs::write("docs/d.md", "@docs/e.md\n").unwrap();
        fs::write("docs/e.md", "@docs/f.md\n").unwrap();
        fs::write("docs/f.md", "end\n").unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_import_graph(&mut diag, &ExcludeSet::default());
        let depth: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::ImportDepthExceeded)
            .collect();
        assert_eq!(depth.len(), 1);
        assert!(depth[0].message.contains("depth exceeds 5"));
    }

    #[test]
    #[serial_test::serial]
    fn l003_allows_chain_of_five_hops() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all("docs").unwrap();
        // CLAUDE → a → b → c → d → e: exactly 5 hops, no violation.
        fs::write("CLAUDE.md", "@docs/a.md\n").unwrap();
        fs::write("docs/a.md", "@docs/b.md\n").unwrap();
        fs::write("docs/b.md", "@docs/c.md\n").unwrap();
        fs::write("docs/c.md", "@docs/d.md\n").unwrap();
        fs::write("docs/d.md", "@docs/e.md\n").unwrap();
        fs::write("docs/e.md", "end\n").unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_import_graph(&mut diag, &ExcludeSet::default());
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::ImportDepthExceeded)
        );
    }

    // ── L004: duplicate-import ──────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn l004_flags_duplicate_import_with_dot_slash_normalization() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all("docs").unwrap();
        fs::write("docs/a.md", "shared\n").unwrap();
        fs::write("CLAUDE.md", "@docs/a.md\n@./docs/a.md\n").unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_import_graph(&mut diag, &ExcludeSet::default());
        let dup: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::DuplicateImport)
            .collect();
        assert_eq!(dup.len(), 1);
        assert!(dup[0].message.contains("duplicate"));
    }

    // ── L005: broken-markdown-link ──────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn l005_flags_broken_relative_markdown_link() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::write("CLAUDE.md", "See [details](docs/missing.md) for more.\n").unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_markdown_links(&mut diag, &ExcludeSet::default());
        let broken: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::BrokenMarkdownLink)
            .collect();
        assert_eq!(broken.len(), 1);
        assert!(broken[0].message.contains("docs/missing.md"));
    }

    #[test]
    #[serial_test::serial]
    fn l005_skips_external_anchor_fenced_and_image_links() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::write(
            "CLAUDE.md",
            "External [ex](https://example.com/page.md) ok.\n\
             Anchor [an](#section) ok.\n\
             Image ![pic](assets/missing.png) ok.\n\
             ```text\n\
             [in fence](docs/missing.md)\n\
             ```\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_markdown_links(&mut diag, &ExcludeSet::default());
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::BrokenMarkdownLink),
            "external, anchor, image, and fenced links must not trigger L005"
        );
    }

    // ── L006: npm-script-missing ────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn l006_flags_npm_run_script_missing_from_package_json() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::write(
            "package.json",
            "{\"name\":\"demo\",\"scripts\":{\"test\":\"echo hi\"}}",
        )
        .unwrap();
        fs::write("CLAUDE.md", "Run `npm run build` to compile.\n").unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_npm_scripts(&mut diag, &ExcludeSet::default());
        let missing: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::NpmScriptMissing)
            .collect();
        assert_eq!(missing.len(), 1);
        assert!(missing[0].message.contains("build"));
    }

    #[test]
    #[serial_test::serial]
    fn l006_accepts_colon_namespaced_script_names() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::write(
            "package.json",
            "{\"name\":\"demo\",\"scripts\":{\"build:css\":\"postcss\"}}",
        )
        .unwrap();
        fs::write("CLAUDE.md", "Run `npm run build:css` to compile styles.\n").unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_npm_scripts(&mut diag, &ExcludeSet::default());
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::NpmScriptMissing),
            "colon-namespaced npm scripts must be matched in full, not truncated at the colon"
        );
    }

    #[test]
    #[serial_test::serial]
    fn l006_silent_when_no_package_json_or_no_scripts() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::write("CLAUDE.md", "Run `npm run build` to compile.\n").unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_npm_scripts(&mut diag, &ExcludeSet::default());
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::NpmScriptMissing),
            "L006 must be silent when package.json is absent"
        );

        // package.json present but with no scripts map: still silent.
        fs::write("package.json", "{\"name\":\"demo\"}").unwrap();
        let mut diag2 = DiagnosticCollector::new_all_enabled();
        validate_npm_scripts(&mut diag2, &ExcludeSet::default());
        assert!(
            !diag2
                .diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::NpmScriptMissing),
            "L006 must be silent when package.json has no scripts object"
        );
    }
}
