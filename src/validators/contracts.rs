//! Prompt, reference, and shipped-script contracts shared by public and private skills.

use crate::config::ExcludeSet;
use crate::diagnostic::DiagnosticCollector;
use crate::fence::{CodeFenceTracker, LineClass, consecutive_bash_pairs, markdown_fences};
use crate::frontmatter;
use crate::rules::LintRule;
use regex::Regex;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;
use walkdir::WalkDir;

static READ_INTENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:read|open)\s+(?:the|each|every|all|any|its|their|this|that)\b[^.\n]{0,60}\b(?:file|files|bundle|bundles|path|paths|diff|diffs|body|bodies|artifact|artifacts|markdown|log|logs)\b|\buse\s+(?:the\s+)?Read\b").unwrap()
});
static OUTPUT_ONLY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bstrict\s+JSONL?\b|\b(?:emit|output|return|respond\s+with|reply\s+with)\s+(?:strict\s+|valid\s+)?JSONL?\s+only\b|\bonly\s+(?:emit|output|return)\s+(?:strict\s+|valid\s+)?JSONL?\b|\boutput\s+must\s+be\s+(?:strict\s+|valid\s+)?JSONL?\b").unwrap()
});
static CANNOT_READ: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bunreadable\b|\b(?:cannot|can't|could\s+not|unable\s+to)\s+(?:read|open)\b|\bRead\s+fails\b|\bfail[ -]+closed\b").unwrap()
});
static NEVER_INVENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:never|do\s+not|don't)\s+(?:invent|fabricate|guess)\b").unwrap()
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
static INLINE_CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`([^`\n]+)`").unwrap());
static MARKDOWN_LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[[^\]]*\]\(([^)\s]+\.md)(?:#[^)]*)?\)").unwrap());
static PLAIN_MD_PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?:^|[\s`'(])((?:skills|\.claude/skills|docs|agents|scripts)/[A-Za-z0-9._/-]+\.md)\b",
    )
    .unwrap()
});
static ALWAYS_LOAD_DIRECTIVE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:\b(?:read|load|open)\b.*\b(?:before|first|completely|always|entire|required|must)\b|\b(?:before|first|always|required|must)\b.*\b(?:read|load|open)\b|^\s*@)").unwrap()
});
static QUOTED_HEREDOC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<<-?\s*(?:'([A-Za-z_][A-Za-z0-9_]*)'|\"([A-Za-z_][A-Za-z0-9_]*)\")"#).unwrap()
});
static BASH_REPLACEMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\$\{[A-Za-z_][A-Za-z0-9_]*//[^/]*/(?:\$[A-Za-z_][A-Za-z0-9_]*|\$\{[A-Za-z_])")
        .unwrap()
});
static EMPTY_ARRAY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|[\s;|&({])([A-Za-z_][A-Za-z0-9_]*)=\(\s*\)").unwrap());
static ARRAY_LENGTH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\$\{#([A-Za-z_][A-Za-z0-9_]*)\[@\]\}").unwrap());
static IF_THEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|[\s;])if\s+.*(?:^|[\s;])then(?:[\s;]|$)").unwrap());
static FI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|[\s;])fi(?:[\s;]|$)").unwrap());
static FORWARDED_ARRAY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?m)^[^#\n]*(?:exec\s+)?[^\n]*"\$\{([A-Za-z_][A-Za-z0-9_]*)\[@\]\}""#).unwrap()
});
static NPM_RUN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bnpm\s+run(?:-script)?\s+([A-Za-z0-9][A-Za-z0-9_-]*)").unwrap());

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
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path().join(filename))
        .filter(|path| path.is_file())
        .collect()
}

fn direct_markdown_files(root: &str) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
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
        let body = frontmatter::extract_body(&content);
        let read_line = first_matching_line(body, &READ_INTENT);
        if let (Some(tools), Some(line)) = (frontmatter_explicit_tools(&content), read_line) {
            let suppressed = has_reasoned_marker(&content, "lint-agent-tool-contract: ok");
            if !tools.iter().any(|tool| tool == "Read") && !suppressed {
                diag.report(
                    LintRule::AgentReadMismatch,
                    &format!(
                        "{}:{}: explicit tools omit Read but the prompt instructs reading evidence",
                        path.display(),
                        body_line_number(&content, line)
                    ),
                );
            }
        }
        if let (Some(read), Some(output)) = (read_line, first_matching_line(body, &OUTPUT_ONLY)) {
            if (!CANNOT_READ.is_match(body) || !NEVER_INVENT.is_match(body))
                && !has_reasoned_marker(&content, "lint-agent-output-mandate: ok")
            {
                diag.report(
                    LintRule::AgentOutputUnsafe,
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
            !line[index + marker.len()..]
                .trim_matches([' ', '-', '>'])
                .is_empty()
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
        let body = frontmatter::extract_body(&content);
        if frontmatter_tools(&content, "allowed-tools")
            .is_some_and(|tools| tools.iter().any(|tool| tool == "Skill"))
        {
            let has_clear_step =
                body.contains("Invoke the Skill tool") || body.contains("via the Skill tool");
            if !has_clear_step {
                diag.report(
                    LintRule::SkillInvokeMissing,
                    &format!(
                        "{}: allowed-tools includes Skill but the body has no explicit Skill tool invocation step",
                        path.display()
                    ),
                );
            }
            for (number, line) in lines_outside_fences_with_numbers(body) {
                if SKILL_INVOKE.is_match(line) && !line.contains("via the Skill tool") {
                    diag.report(
                        LintRule::SkillInvokeMissing,
                        &format!(
                            "{}:{}: ambiguous skill invocation; identify the Skill tool on the same line",
                            path.display(),
                            body_line_number(&content, number - 1)
                        ),
                    );
                }
            }
        }

        for fence in markdown_fences(&content) {
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
            diag.report(
                LintRule::SkillFlagMismatch,
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
        diag.report(
            LintRule::AwkFieldRef,
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
            diag.report(
                LintRule::UnsafeGrepProbe,
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
            diag.report(
                LintRule::UnsafeGrepProbe,
                &format!(
                    "{}:{line}: bare top-level grep in a shell fence; wrap it or use command grep",
                    skill.display()
                ),
            );
        } else if !pipe_fed && !dev_null && !has_path {
            diag.report(
                LintRule::UnsafeGrepProbe,
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
        for entry in WalkDir::new(&root).into_iter().flatten() {
            let path = entry.path();
            if !path.is_file()
                || path.file_name().and_then(|name| name.to_str()) == Some("SKILL.md")
                || path.extension().and_then(|value| value.to_str()) != Some("md")
                || !path
                    .components()
                    .any(|part| part.as_os_str() == "references")
            {
                continue;
            }
            let Some(content) = read_text(path, exclude) else {
                continue;
            };
            if let Some((first, second)) = consecutive_bash_pairs(&content).first() {
                diag.report(
                    LintRule::ConsecutiveBash,
                    &format!(
                        "{}: consecutive bash code blocks (lines {first} and {second}) could be combined into one",
                        path.display()
                    ),
                );
            }
        }
    }
}

fn markdown_references(source_path: &Path, content: &str) -> Vec<PathBuf> {
    let mut refs = BTreeSet::new();
    for line in crate::fence::lines_outside_fences(content) {
        if !ALWAYS_LOAD_DIRECTIVE.is_match(line) {
            continue;
        }
        for capture in INLINE_CODE.captures_iter(line) {
            add_markdown_reference(source_path, &capture[1], &mut refs);
        }
        for capture in MARKDOWN_LINK.captures_iter(line) {
            add_markdown_reference(source_path, &capture[1], &mut refs);
        }
        for capture in PLAIN_MD_PATH.captures_iter(line) {
            add_markdown_reference(source_path, &capture[1], &mut refs);
        }
    }
    refs.into_iter().collect()
}

fn add_markdown_reference(source: &Path, raw: &str, refs: &mut BTreeSet<PathBuf>) {
    let raw = raw.split(['#', ':']).next().unwrap_or(raw);
    if !raw.ends_with(".md") || raw.contains(['$', '{', '}', '<', '>', '*']) {
        return;
    }
    if let Some(candidate) = resolve_repo_reference(source, raw) {
        if candidate.is_file() && !candidate.is_symlink() {
            refs.insert(candidate);
        }
    }
}

fn validate_skill_closure(skill: &Path, diag: &mut DiagnosticCollector) {
    let Some(max_lines) = diag.config().skill_closure_max_lines else {
        return;
    };
    let mut seen = BTreeSet::new();
    let mut pending = vec![skill.to_path_buf()];
    let mut total = 0;
    while let Some(path) = pending.pop() {
        let Some(normalized) = normalize_repo_relative(&path) else {
            continue;
        };
        if !seen.insert(normalized.clone()) {
            continue;
        }
        let Ok(content) = fs::read_to_string(&normalized) else {
            continue;
        };
        total += content.lines().count();
        pending.extend(markdown_references(&normalized, &content));
    }
    if total > max_lines {
        diag.report(
            LintRule::SkillClosureLarge,
            &format!(
                "{}: always-loaded prompt closure is {total} lines across {} files (configured maximum {max_lines})",
                skill.display(),
                seen.len()
            ),
        );
    }
}

fn normalize_repo_relative(path: &Path) -> Option<PathBuf> {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !result.pop() {
                    return None;
                }
            }
            Component::Normal(value) => result.push(value),
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(result)
}

fn resolve_repo_reference(source: &Path, raw: &str) -> Option<PathBuf> {
    let direct = PathBuf::from(raw.trim_start_matches("./"));
    if !direct.is_absolute()
        && !direct.components().any(|part| part == Component::ParentDir)
        && direct.is_file()
    {
        return normalize_repo_relative(&direct);
    }
    normalize_repo_relative(&source.parent().unwrap_or_else(|| Path::new(".")).join(raw))
}

fn validate_claude_import_budget(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let per_file = diag.config().claude_import_max_lines;
    let total_cap = diag.config().claude_import_total_max_lines;
    if per_file.is_none() && total_cap.is_none() || exclude.is_excluded("CLAUDE.md") {
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
        let count = content.lines().count();
        total += count;
        if path != Path::new("CLAUDE.md") && per_file.is_some_and(|cap| count > cap) {
            diag.report(
                LintRule::ClaudeImportLarge,
                &format!(
                    "{}: imported prompt source has {count} lines (configured maximum {})",
                    path.display(),
                    per_file.unwrap_or_default()
                ),
            );
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
        diag.report(
            LintRule::ClaudeImportLarge,
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
                let token = &capture[1];
                if token.contains([' ', '$', '{', '}', '<', '>', '*', '?'])
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
                    diag.report(
                        LintRule::InlinePathMissing,
                        &format!("{relpath}:{number}: dead or escaping inline path `{token}`"),
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
                    diag.report(
                        LintRule::ImportPathMissing,
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
                    diag.report(
                        LintRule::ImportPathMissing,
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
                diag.report(
                    LintRule::DuplicateImport,
                    &format!(
                        "{}:{number}: duplicate @import of {}",
                        source.display(),
                        target.display()
                    ),
                );
                continue;
            }
            if stack.len() > IMPORT_MAX_DEPTH {
                diag.report(
                    LintRule::ImportDepthExceeded,
                    &format!(
                        "{}:{number}: @import chain depth exceeds {IMPORT_MAX_DEPTH} hops: {}",
                        source.display(),
                        format_import_chain(stack, &target)
                    ),
                );
                continue;
            }
            if let Some(index) = stack.iter().position(|p| p == &target) {
                diag.report(
                    LintRule::CircularImport,
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
                    diag.report(
                        LintRule::BrokenMarkdownLink,
                        &format!("{relpath}:{number}: broken markdown link target: {target}"),
                    );
                    continue;
                };
                if !resolved.is_file() {
                    diag.report(
                        LintRule::BrokenMarkdownLink,
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
                    diag.report(
                        LintRule::NpmScriptMissing,
                        &format!(
                            "{relpath}:{number}: npm run {script} is not defined in package.json scripts"
                        ),
                    );
                }
            }
        }
    }
}

fn scoped_scripts(include_public: bool) -> Vec<PathBuf> {
    let mut roots = vec![PathBuf::from(".claude/skills")];
    if include_public {
        roots.extend([PathBuf::from("scripts"), PathBuf::from("skills")]);
    }
    let mut paths = BTreeSet::new();
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        for entry in WalkDir::new(root).into_iter().flatten() {
            let path = entry.path();
            let extension = path.extension().and_then(|value| value.to_str());
            if path.is_file()
                && !path.is_symlink()
                && matches!(extension, Some("sh" | "py" | "awk"))
                && (path.components().any(|part| part.as_os_str() == "scripts")
                    || extension == Some("awk"))
            {
                paths.insert(path.to_path_buf());
            }
        }
    }
    paths.into_iter().collect()
}

fn validate_script_contracts(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    include_public: bool,
) {
    for path in scoped_scripts(include_public) {
        let Some(content) = read_text(&path, exclude) else {
            continue;
        };
        let mut heredoc: Option<String> = None;
        let mut previous = "";
        let mut empty_arrays = HashSet::new();
        let mut array_guard_depth: HashMap<String, usize> = HashMap::new();
        let mut zero_exit_guards: HashMap<String, (usize, bool)> = HashMap::new();
        let mut proven_nonempty = HashSet::new();
        let mut shell_depth = 0;
        for (index, line) in content.lines().enumerate() {
            if let Some(delimiter) = &heredoc {
                if line.trim() == delimiter {
                    heredoc = None;
                }
                previous = line;
                continue;
            }
            if let Some(capture) = QUOTED_HEREDOC.captures(line) {
                heredoc = capture
                    .get(1)
                    .or_else(|| capture.get(2))
                    .map(|value| value.as_str().to_string());
            }
            if !line.trim_start().starts_with('#') {
                for capture in EMPTY_ARRAY.captures_iter(line) {
                    empty_arrays.insert(capture[1].to_string());
                }
                let opens_guard = IF_THEN.is_match(line);
                for capture in ARRAY_LENGTH.captures_iter(line) {
                    if opens_guard {
                        array_guard_depth.insert(capture[1].to_string(), shell_depth + 1);
                        if line.contains("-eq 0") || line.contains("== 0") {
                            zero_exit_guards
                                .insert(capture[1].to_string(), (shell_depth + 1, false));
                        }
                    }
                }
                if matches!(line.split_whitespace().next(), Some("exit" | "return")) {
                    for (depth, exits) in zero_exit_guards.values_mut() {
                        if shell_depth >= *depth {
                            *exits = true;
                        }
                    }
                }
                for name in &empty_arrays {
                    let expands = line.contains(&format!("${{{name}[@]}}"))
                        || line.contains(&format!("${{{name}[*]}}"));
                    let guarded = array_guard_depth
                        .get(name)
                        .is_some_and(|guard_depth| shell_depth >= *guard_depth);
                    if expands
                        && !guarded
                        && !proven_nonempty.contains(name)
                        && !ARRAY_LENGTH.is_match(line)
                    {
                        diag.report(
                            LintRule::Bash32Incompatible,
                            &format!(
                                "{}:{}: Bash 3.2 incompatible unguarded empty-array expansion for {name}",
                                path.display(),
                                index + 1
                            ),
                        );
                    }
                }
                validate_gh_inline(&path, index + 1, line, diag);
                if BASH_REPLACEMENT.is_match(line)
                    && !has_reasoned_marker(line, "lint-renderer-safe: ok")
                    && !has_reasoned_marker(previous, "lint-renderer-safe: ok")
                {
                    diag.report(
                        LintRule::BashReplacementUnsafe,
                        &format!(
                            "{}:{}: unsafe Bash global substitution with a variable replacement",
                            path.display(),
                            index + 1
                        ),
                    );
                }
                validate_bash32(&path, index + 1, line, diag);
                validate_awk_nonascii(&path, index + 1, line, diag);
                if opens_guard {
                    shell_depth += 1;
                }
                if FI.is_match(line) {
                    let completed: Vec<_> = zero_exit_guards
                        .iter()
                        .filter(|(_, (depth, _))| *depth == shell_depth)
                        .map(|(name, (_, exits))| (name.clone(), *exits))
                        .collect();
                    for (name, exits) in completed {
                        zero_exit_guards.remove(&name);
                        if exits {
                            proven_nonempty.insert(name);
                        }
                    }
                    shell_depth = shell_depth.saturating_sub(1);
                }
            }
            previous = line;
        }
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
            diag.report(
                LintRule::GhInlineBody,
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
    let patterns = [
        (
            r"\b(?:declare|typeset)\s+-[A-Za-z]*A\b",
            "associative arrays",
        ),
        (r"\b(?:mapfile|readarray)\b", "mapfile/readarray"),
        (
            r"\$\{[A-Za-z_][A-Za-z0-9_]*(?:\^\^?|,,?)",
            "parameter case conversion",
        ),
        (r"\b(?:declare|local)\s+-[A-Za-z]*n\b", "namerefs"),
        (r"&>>", "append-all redirection"),
        (r"\bcoproc\b", "coproc"),
        (
            r"\$\{[A-Za-z_][A-Za-z0-9_]*\[\s*-[0-9]",
            "negative array indexes",
        ),
        (
            r"\{[A-Za-z0-9-]+\.\.[A-Za-z0-9-]+\.\.-?[0-9]+\}",
            "stepped brace expansion",
        ),
        (
            r"\b(?:if|elif)\s+!?\s*command\s+(?:grep|egrep|fgrep|rg|ripgrep)\b",
            "grep-family command conditions",
        ),
    ];
    for (pattern, label) in patterns {
        if Regex::new(pattern).unwrap().is_match(line) {
            diag.report(
                LintRule::Bash32Incompatible,
                &format!(
                    "{}:{line_number}: Bash 3.2 incompatible {label}",
                    path.display()
                ),
            );
        }
    }
}

fn validate_awk_nonascii(
    path: &Path,
    line_number: usize,
    line: &str,
    diag: &mut DiagnosticCollector,
) {
    if line.is_ascii() || has_reasoned_marker(line, "lint-awk-multibyte-regex: ok") {
        return;
    }
    let dynamic_value = line.contains("awk") && line.contains("-v ");
    let regex_body = path.extension().and_then(|value| value.to_str()) == Some("awk")
        || line.contains("match(")
        || line.contains("sub(")
        || line.contains("gsub(")
        || line.contains("split(")
        || line.contains(" ~ ")
        || line.contains(" !~ ");
    if dynamic_value || regex_body {
        diag.report(
            LintRule::AwkRegexNonascii,
            &format!(
                "{}:{line_number}: non-ASCII text in a dynamic awk regex is not portable",
                path.display()
            ),
        );
    }
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
    fn robust_fences_keep_embedded_short_delimiters_inside() {
        let fences = markdown_fences("````bash\necho hi\n```\necho still\n````\n");
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
