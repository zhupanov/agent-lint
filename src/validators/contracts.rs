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
use crate::script_paths::{ScriptKind, script_kind};
use crate::script_paths::{ScriptReference, ScriptReferenceBase, extract_script_token_references};
use crate::traversal;
use crate::validators::common::{
    classify_inline_code_path, is_unsafe_inline_code_path_probe, normalize_inline_code_path_probe,
};
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

static SKILL_INVOKE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\b(?:re-)?[Ii]nvoke\b\s+(?:the\s+)?(?:\*\*[^*\n]{1,40}\*\*\s+)?`/[-\w]+`(?:\s+skill\b)?",
    )
    .unwrap()
});
static FLAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|\s)--([A-Za-z0-9][A-Za-z0-9_-]*)\b").unwrap());
static AWK_FIELD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\$[0-9]+").unwrap());
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
pub fn validate_contracts(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    include_public: bool,
) {
    validate_skill_contracts(diag, exclude, include_public);
    validate_reference_consecutive_bash(diag, exclude, include_public);
    validate_script_contracts(diag, exclude, include_public);
    let import_graph = InstructionImportGraph::build(&diag.config().instruction_files, exclude);
    validate_claude_import_budget(&import_graph, diag);
    validate_prompt_source_budgets(diag);
    validate_inline_paths(diag, exclude);
    validate_import_graph(&import_graph, diag);
    validate_markdown_links(diag, exclude);
    super::npm_scripts::validate_npm_scripts(diag, exclude);
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

fn one_level_files(root: &str, filename: &str) -> Vec<PathBuf> {
    crate::traversal::shallow_directories(Path::new(root), Path::new("."), None)
        .entries
        .into_iter()
        .map(|entry| entry.path.join(filename))
        .filter(|path| path.is_file())
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

fn validate_flag_signature(
    skill: &Path,
    line: usize,
    command: &str,
    diag: &mut DiagnosticCollector,
) {
    if !command.contains("--") || has_reasoned_marker(command, "lint-skill-md-flag-signature: ok") {
        return;
    }
    for (script, arguments) in script_invocations(skill, command) {
        let Ok(source) = fs::read_to_string(&script) else {
            continue;
        };
        let source = executable_script_source(&script, &source);
        if forwards_all_args(&script, &source) {
            continue;
        }
        for flag in invocation_flags(&arguments) {
            if !script_declares_flag(&script, &source, &flag) {
                diag.report_at_with(
                    LintRule::SkillFlagMismatch,
                    skill,
                    &format!(
                        "{}:{line}: invocation uses --{flag}, but {} does not accept it",
                        skill.display(),
                        script.display()
                    ),
                    DiagnosticMetadata::default()
                        .with_location(SourceSpan::line(line))
                        .with_suggestion(
                            "remove the unsupported flag or add it to the shipped script's parser",
                        ),
                );
            }
        }
    }
}

/// Each command token gets the shared script-reference parser. A candidate's
/// arguments end at the next control operator, so adjacent command clauses
/// cannot donate flags to one another.
fn script_invocations(skill: &Path, command: &str) -> Vec<(PathBuf, Vec<String>)> {
    let tokens = shell_lex(command);
    let mut invocations = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        if is_control_operator(token) {
            continue;
        }
        let span_end = tokens[index + 1..]
            .iter()
            .position(|next| is_control_operator(next))
            .map_or(tokens.len(), |offset| index + 1 + offset);
        for reference in extract_script_token_references(token, 0) {
            if let Some(script) = resolve_signature_script(skill, &reference) {
                invocations.push((script, tokens[index + 1..span_end].to_vec()));
            }
        }
    }
    invocations
}

fn is_control_operator(token: &str) -> bool {
    matches!(token, "|" | "|&" | "||" | "&&" | ";" | "&")
}

fn resolve_signature_script(skill: &Path, reference: &ScriptReference) -> Option<PathBuf> {
    let candidates: Vec<PathBuf> = match reference.base {
        ScriptReferenceBase::RepositoryRoot | ScriptReferenceBase::Absolute => {
            vec![reference.path.clone()]
        }
        ScriptReferenceBase::Relative => skill
            .parent()
            .map(|parent| parent.join(&reference.path))
            .into_iter()
            .chain(std::iter::once(reference.path.clone()))
            .collect(),
    };
    candidates.into_iter().find(|candidate| candidate.is_file())
}

fn invocation_flags(arguments: &[String]) -> Vec<String> {
    let mut flags = Vec::new();
    for argument in arguments {
        if argument == "--" {
            break;
        }
        for capture in FLAG.captures_iter(argument) {
            flags.push(capture[1].to_string());
        }
    }
    flags
}

fn script_declares_flag(script: &Path, source: &str, flag: &str) -> bool {
    let escaped = regex::escape(flag);
    match script.extension().and_then(|value| value.to_str()) {
        Some("sh") => [
            format!(r#"(?:^|[\s|])["']?--{escaped}["']?(?:[|)=*])"#),
            format!(
                r#"(?:(?:==|!=|=)\s*["']--{escaped}["']|["']--{escaped}["']\s*(?:\]|==|!=|=))"#
            ),
        ]
        .iter()
        .any(|raw| Regex::new(raw).is_ok_and(|pattern| pattern.is_match(source))),
        Some("py") => [
            format!(r#"add_argument\s*\([^)]*["']--{escaped}["']"#),
            format!(r#"(?:click\.)?option\s*\([^)]*["']--{escaped}["']"#),
            format!(r#"typer\.Option\s*\([^)]*["']--{escaped}["']"#),
            format!(r#"["']--{escaped}["']\s+in\s+sys\.argv"#),
        ]
        .iter()
        .any(|raw| Regex::new(raw).is_ok_and(|pattern| pattern.is_match(source))),
        _ => false,
    }
}

fn forwards_all_args(script: &Path, source: &str) -> bool {
    match script.extension().and_then(|value| value.to_str()) {
        Some("sh") => {
            source.contains("\"$@\"") || source.contains("${@}") || forwards_collected_args(source)
        }
        Some("py") => ["sys.argv[1:]", "parse_known_args", "argparse.REMAINDER"]
            .iter()
            .any(|marker| source.contains(marker)),
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

/// Remove comments before checking declarations or forwarding behavior. This
/// intentionally stays lexical: S059 needs only the same quote/escape boundary
/// as command parsing, not a shell or Python executor.
fn executable_script_source(script: &Path, source: &str) -> String {
    match script.extension().and_then(|value| value.to_str()) {
        Some("sh") => strip_shell_comments(source),
        Some("py") => strip_python_comments(source),
        _ => source.to_string(),
    }
}

fn strip_shell_comments(source: &str) -> String {
    let mut result = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut comment = false;
    for character in source.chars() {
        if comment {
            if character == '\n' {
                result.push(character);
                comment = false;
            }
        } else if escaped {
            result.push(character);
            escaped = false;
        } else if character == '\\' && quote != Some('\'') {
            result.push(character);
            escaped = true;
        } else if let Some(active) = quote {
            result.push(character);
            if character == active {
                quote = None;
            }
        } else if matches!(character, '\'' | '"') {
            result.push(character);
            quote = Some(character);
        } else if character == '#' {
            comment = true;
        } else {
            result.push(character);
        }
    }
    result
}

fn strip_python_comments(source: &str) -> String {
    let mut result = String::new();
    let mut quote: Option<(char, bool)> = None;
    let mut escaped = false;
    let mut characters = source.chars().peekable();
    while let Some(character) = characters.next() {
        if let Some((active, triple)) = quote {
            result.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if triple && character == active {
                if characters.peek() == Some(&active) {
                    let mut probe = characters.clone();
                    probe.next();
                    if probe.peek() == Some(&active) {
                        result.push(characters.next().unwrap_or_default());
                        result.push(characters.next().unwrap_or_default());
                        quote = None;
                    }
                }
            } else if !triple && character == active {
                quote = None;
            }
        } else if character == '#' {
            while characters.next().is_some_and(|next| next != '\n') {}
            result.push('\n');
        } else if matches!(character, '\'' | '"') {
            let mut probe = characters.clone();
            let triple = probe.next() == Some(character) && probe.next() == Some(character);
            result.push(character);
            if triple {
                result.push(characters.next().unwrap_or_default());
                result.push(characters.next().unwrap_or_default());
            }
            quote = Some((character, triple));
        } else {
            result.push(character);
        }
    }
    result
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

/// Repository-relative `references/*.md` paths (never `SKILL.md`) that S021
/// scans, in deterministic order. Shared with the autofix so the validator and
/// the fixer resolve the same reference files under the same roots and
/// exclusions (`.claude/skills` in both modes, `skills` when `include_public`).
pub(crate) fn reference_bash_markdown_paths(
    include_public: bool,
    exclude: &ExcludeSet,
) -> Vec<PathBuf> {
    let mut roots = vec![PathBuf::from(".claude/skills")];
    if include_public {
        roots.push(PathBuf::from("skills"));
    }
    let mut paths = Vec::new();
    for root in roots {
        if !root.is_dir() {
            continue;
        }
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
    }
    paths.sort();
    paths
}

fn validate_reference_consecutive_bash(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    include_public: bool,
) {
    for path in reference_bash_markdown_paths(include_public, exclude) {
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

/// A source-positioned `@path` directive. `target == None` means that the
/// directive is intentionally external (home, absolute, or root-escaping).
#[derive(Debug, Clone)]
struct ImportDirective {
    target: Option<PathBuf>,
    span: SourceSpan,
}

#[derive(Debug)]
struct ImportNode {
    content: String,
    directives: Vec<ImportDirective>,
}

/// Shared, repository-local instruction import graph used by D004 and L001--L004.
/// It deliberately owns parsing, safe resolution, and exclusion boundaries so
/// the two rule families cannot drift.
#[derive(Debug)]
struct InstructionImportGraph {
    roots: Vec<PathBuf>,
    nodes: BTreeMap<PathBuf, ImportNode>,
    opaque_targets: BTreeSet<PathBuf>,
}

impl InstructionImportGraph {
    fn build(instruction_files: &[String], exclude: &ExcludeSet) -> Self {
        let mut roots = BTreeSet::new();
        for raw in instruction_files {
            let Some(path) = normalize_repo_relative(Path::new(raw)) else {
                continue;
            };
            if !exclude.is_excluded(&path.to_string_lossy()) && safe_read_repo_file(&path).is_some()
            {
                roots.insert(path);
            }
        }
        let roots: Vec<_> = roots.into_iter().collect();
        let mut graph = Self {
            roots: roots.clone(),
            nodes: BTreeMap::new(),
            opaque_targets: BTreeSet::new(),
        };
        let mut pending = VecDeque::new();
        for root in roots {
            pending.push_back(root);
        }
        while let Some(source) = pending.pop_front() {
            if graph.nodes.contains_key(&source) {
                continue;
            }
            let Some(content) = safe_read_repo_file(&source) else {
                continue;
            };
            let directives = extract_import_directives(&source, &content);
            let node = ImportNode {
                content,
                directives,
            };
            for directive in &node.directives {
                let Some(target) = &directive.target else {
                    continue;
                };
                // An excluded target is an opaque existing boundary. Do not
                // parse or measure it, but retain its lexical identity for L001.
                if exclude.is_excluded(&target.to_string_lossy()) {
                    graph.opaque_targets.insert(target.clone());
                    continue;
                }
                if safe_read_repo_file(target).is_some() && !graph.nodes.contains_key(target) {
                    pending.push_back(target.clone());
                }
            }
            graph.nodes.insert(source, node);
        }
        graph
    }

    fn node(&self, path: &Path) -> Option<&ImportNode> {
        self.nodes.get(path)
    }

    fn reachable_from(&self, root: &Path) -> BTreeSet<PathBuf> {
        let mut result = BTreeSet::new();
        let mut pending = VecDeque::from([root.to_path_buf()]);
        while let Some(source) = pending.pop_front() {
            if !result.insert(source.clone()) {
                continue;
            }
            let Some(node) = self.node(&source) else {
                continue;
            };
            for directive in &node.directives {
                if let Some(target) = &directive.target {
                    if self.nodes.contains_key(target) {
                        pending.push_back(target.clone());
                    }
                }
            }
        }
        result
    }

    fn first_path_from(&self, root: &Path, target: &Path) -> Option<Vec<PathBuf>> {
        let mut pending = VecDeque::from([(root.to_path_buf(), vec![root.to_path_buf()])]);
        let mut seen = BTreeSet::new();
        while let Some((source, chain)) = pending.pop_front() {
            if !seen.insert(source.clone()) {
                continue;
            }
            if source == target {
                return Some(chain);
            }
            for next in graph_targets(self, &source) {
                let mut next_chain = chain.clone();
                next_chain.push(next.clone());
                pending.push_back((next.clone(), next_chain));
            }
        }
        None
    }
}

/// Return readable regular repository files without following a final or
/// ancestor symlink. The lexical resolver supplies a root-relative path, so
/// this does not disclose or traverse an outside target.
fn safe_read_repo_file(path: &Path) -> Option<String> {
    if path.as_os_str().is_empty() {
        return None;
    }
    let mut current = PathBuf::new();
    for component in path.components() {
        let Component::Normal(part) = component else {
            return None;
        };
        current.push(part);
        let metadata = fs::symlink_metadata(&current).ok()?;
        if metadata.file_type().is_symlink() {
            return None;
        }
    }
    let metadata = fs::symlink_metadata(path).ok()?;
    metadata
        .is_file()
        .then(|| fs::read_to_string(path).ok())
        .flatten()
}

fn target_exists_as_regular_file(path: &Path) -> bool {
    safe_read_repo_file(path).is_some()
}

fn resolve_instruction_import(source: &Path, raw: &str) -> Option<PathBuf> {
    if raw.starts_with("~/") || Path::new(raw).is_absolute() {
        return None;
    }
    normalize_repo_relative(&source.parent().unwrap_or_else(|| Path::new(".")).join(raw))
}

/// Extract repository import tokens from live Markdown prose. MarkdownDocument
/// removes frontmatter, fences, indented code, links, blockquotes, inline code,
/// and balanced quoted examples while preserving Unicode columns.
fn extract_import_directives(source: &Path, content: &str) -> Vec<ImportDirective> {
    let document = MarkdownDocument::parse(content);
    let mut directives = Vec::new();
    let example_scopes = crate::live_instructions::example_scopes_for(&document);
    for (line, is_example) in document.body_prose().iter().zip(example_scopes) {
        if is_example {
            continue;
        }
        let chars: Vec<char> = line.text.chars().collect();
        let mut index = 0;
        while index < chars.len() {
            if chars[index] != '@'
                || !import_token_boundary(chars.get(index.wrapping_sub(1)).copied())
                || document.links().iter().any(|link| {
                    (link.line..=link.end_line).contains(&line.line)
                        && (line.line != link.line || index + 1 >= link.start_column)
                        && (line.line != link.end_line || index < link.end_column)
                })
            {
                index += 1;
                continue;
            }
            let start = index;
            index += 1;
            let token_start = index;
            while index < chars.len() && !chars[index].is_whitespace() {
                index += 1;
            }
            let mut token_end = index;
            while token_end > token_start
                && matches!(
                    chars[token_end - 1],
                    ')' | ']' | '}' | '>' | ',' | '.' | ';' | ':' | '!' | '?'
                )
            {
                token_end -= 1;
            }
            if token_end == token_start {
                continue;
            }
            let raw: String = chars[token_start..token_end].iter().collect();
            if raw.contains('@')
                || raw.starts_with('/')
                || looks_like_package_scope(&line.text, start, &raw)
            {
                continue;
            }
            directives.push(ImportDirective {
                target: resolve_instruction_import(source, &raw),
                span: SourceSpan::range(line.line, start + 1, line.line, token_end + 1),
            });
        }
    }
    directives
}

fn import_token_boundary(previous: Option<char>) -> bool {
    previous.is_none_or(|ch| ch.is_whitespace() || matches!(ch, '(' | '[' | '{' | '<' | '"' | '\''))
}

fn looks_like_package_scope(line: &str, at: usize, raw: &str) -> bool {
    if !raw.contains('/') {
        return false;
    }
    let prefix: String = line.chars().take(at).collect();
    let prior = prefix.to_ascii_lowercase();
    prior.ends_with("install ") || prior.ends_with("package ") || prior.ends_with("dependency ")
}

fn import_metadata(
    directive: &ImportDirective,
    evidence: impl AsRef<str>,
    suggestion: &str,
) -> DiagnosticMetadata {
    DiagnosticMetadata::default()
        .with_location(directive.span)
        .with_evidence(evidence)
        .with_suggestion(suggestion)
}

fn validate_claude_import_budget(graph: &InstructionImportGraph, diag: &mut DiagnosticCollector) {
    let per_file = diag.config().claude_import_max_lines;
    let total_cap = diag.config().claude_import_total_max_lines;
    let path_budgets = diag.config().claude_import_path_budgets.clone();
    if per_file.is_none() && total_cap.is_none() && path_budgets.is_empty() {
        return;
    }
    let root = Path::new("CLAUDE.md");
    if !graph.roots.iter().any(|item| item == root) {
        return;
    }
    let closure = graph.reachable_from(root);
    let mut total = 0;
    for path in &closure {
        let node = &graph.nodes[path];
        let count = crate::prompt_budget::source_metrics(&node.content).lines;
        total += count;
        if path == root {
            continue;
        }
        let normalized = crate::config::normalize_path(&path.to_string_lossy());
        let effective_cap = path_budgets.get(&normalized).copied().or(per_file);
        if effective_cap.is_some_and(|cap| count > cap) {
            let chain = graph.first_path_from(root, path).unwrap_or_default();
            let importing = chain.get(chain.len().saturating_sub(2));
            let directive = importing
                .and_then(|source| graph.node(source))
                .and_then(|node| {
                    node.directives
                        .iter()
                        .find(|item| item.target.as_deref() == Some(path.as_path()))
                });
            if let Some(directive) = directive {
                diag.report_at_with(
                    LintRule::ClaudeImportLarge,
                    path,
                    &format!(
                        "{}: imported prompt source has {count} lines (effective maximum {})",
                        path.display(),
                        effective_cap.unwrap_or_default()
                    ),
                    import_metadata(
                        directive,
                        path.to_string_lossy(),
                        "Split this imported source or reduce its live instruction lines.",
                    )
                    .with_related_subjects(chain),
                );
            }
        }
    }
    if total_cap.is_some_and(|cap| total > cap) {
        diag.report_at_with(
            LintRule::ClaudeImportLarge,
            root,
            &format!("CLAUDE.md repository-local import closure has {total} lines across {} files (configured maximum {})", closure.len(), total_cap.unwrap_or_default()),
            DiagnosticMetadata::default()
                .with_evidence(format!("CLAUDE.md closure: {total} lines"))
                .with_suggestion("Split imported instruction sources or reduce their live instruction lines.")
                .with_related_subjects(closure),
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
                let probe = normalize_inline_code_path_probe(token);
                let candidate = Path::new(probe);
                if is_unsafe_inline_code_path_probe(candidate) || !candidate.exists() {
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

fn format_import_chain(chain: &[PathBuf]) -> String {
    chain
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(" → ")
}

/// L001--L004: report facts from the shared graph without changing the
/// graph's inclusion policy. Per-file policy is resolved by report_at_with.
fn validate_import_graph(graph: &InstructionImportGraph, diag: &mut DiagnosticCollector) {
    for (source, node) in &graph.nodes {
        let mut missing = BTreeSet::new();
        let mut direct = BTreeSet::new();
        for directive in &node.directives {
            let Some(target) = &directive.target else {
                continue;
            };
            if !graph.opaque_targets.contains(target)
                && !target_exists_as_regular_file(target)
                && missing.insert(target.clone())
            {
                diag.report_at_with(
                    LintRule::ImportPathMissing,
                    source,
                    &format!(
                        "{}: @import target is missing or unreadable: {}",
                        source.display(),
                        target.display()
                    ),
                    import_metadata(
                        directive,
                        target.to_string_lossy(),
                        "Create the repository-local target or correct this import path.",
                    ),
                );
            }
            if !direct.insert(target.clone()) {
                diag.report_at_with(
                    LintRule::DuplicateImport,
                    source,
                    &format!(
                        "{}: duplicate @import of {}",
                        source.display(),
                        target.display()
                    ),
                    import_metadata(
                        directive,
                        target.to_string_lossy(),
                        "Keep one normalized direct import for this target.",
                    ),
                );
            }
        }
    }
    for root in &graph.roots {
        let mut cycles = BTreeSet::new();
        let mut stack = vec![root.clone()];
        collect_cycles(graph, root, &mut stack, &mut cycles);
        for cycle in cycles {
            let source = cycle.last().expect("cycles are nonempty");
            let target = &cycle[0];
            let directive = graph
                .node(source)
                .and_then(|node| {
                    node.directives
                        .iter()
                        .find(|item| item.target.as_deref() == Some(target.as_path()))
                })
                .expect("cycle edge is present in graph");
            let mut chain = cycle.clone();
            chain.push(target.clone());
            diag.report_at_with(
                LintRule::CircularImport,
                source,
                &format!(
                    "{}: circular @import chain: {}",
                    source.display(),
                    format_import_chain(&chain)
                ),
                import_metadata(
                    directive,
                    format_import_chain(&chain),
                    "Break this repository-local import cycle.",
                ),
            );
        }
        if let Some(chain) = shortest_overdepth_prefix(graph, root) {
            let source = &chain[IMPORT_MAX_DEPTH];
            let target = &chain[IMPORT_MAX_DEPTH + 1];
            let directive = graph
                .node(source)
                .and_then(|node| {
                    node.directives
                        .iter()
                        .find(|item| item.target.as_deref() == Some(target.as_path()))
                })
                .expect("over-depth edge is present in graph");
            diag.report_at_with(
                LintRule::ImportDepthExceeded,
                source,
                &format!(
                    "{}: @import chain depth exceeds {IMPORT_MAX_DEPTH} hops: {}",
                    source.display(),
                    format_import_chain(&chain)
                ),
                import_metadata(
                    directive,
                    format_import_chain(&chain),
                    "Reduce this repository-local import chain to at most five hops.",
                ),
            );
        }
    }
}

fn graph_targets<'a>(
    graph: &'a InstructionImportGraph,
    source: &Path,
) -> impl Iterator<Item = &'a PathBuf> {
    graph
        .node(source)
        .into_iter()
        .flat_map(|node| node.directives.iter())
        .filter_map(|directive| directive.target.as_ref())
        .filter(|target| graph.nodes.contains_key(*target))
}

fn collect_cycles(
    graph: &InstructionImportGraph,
    source: &Path,
    stack: &mut Vec<PathBuf>,
    cycles: &mut BTreeSet<Vec<PathBuf>>,
) {
    for target in graph_targets(graph, source) {
        if let Some(index) = stack.iter().position(|item| item == target) {
            cycles.insert(canonical_cycle(&stack[index..]));
        } else {
            stack.push(target.clone());
            collect_cycles(graph, target, stack, cycles);
            stack.pop();
        }
    }
}

fn canonical_cycle(cycle: &[PathBuf]) -> Vec<PathBuf> {
    (0..cycle.len())
        .map(|start| {
            cycle[start..]
                .iter()
                .chain(&cycle[..start])
                .cloned()
                .collect::<Vec<_>>()
        })
        .min()
        .unwrap_or_default()
}

fn shortest_overdepth_prefix(graph: &InstructionImportGraph, root: &Path) -> Option<Vec<PathBuf>> {
    fn visit(
        graph: &InstructionImportGraph,
        source: &Path,
        path: &mut Vec<PathBuf>,
        best: &mut Option<Vec<PathBuf>>,
    ) {
        if path.len() == IMPORT_MAX_DEPTH + 2 {
            if best
                .as_ref()
                .is_none_or(|current| path.as_slice() < current.as_slice())
            {
                *best = Some(path.clone());
            }
            return;
        }
        for target in graph_targets(graph, source) {
            if !path.contains(target) {
                path.push(target.clone());
                visit(graph, target, path, best);
                path.pop();
            }
        }
    }
    let mut best = None;
    let mut path = vec![root.to_path_buf()];
    visit(graph, root, &mut path, &mut best);
    best
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
                let Some(resolved) = resolve_repo_reference(path, target) else {
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

#[derive(Clone, Copy, Default)]
struct ScriptScope {
    portability: bool,
    ignore_exclude: bool,
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
            let kind = script_kind(&path);
            if !path.is_symlink()
                && kind.is_some()
                && (path.components().any(|part| part.as_os_str() == "scripts")
                    || kind == Some(ScriptKind::Awk))
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
        let kind = script_kind(&path).unwrap_or(ScriptKind::Other);
        if kind == ScriptKind::Shell && g008_shell_script(&path) {
            validate_gh_inline(&path, &content, diag);
        }
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

/// The GitHub CLI body options this rule knows how to replace safely.
///
/// Reviewed against https://cli.github.com/manual/gh_help_reference on
/// 2026-07-21. Update this table (and its table-driven test) when that
/// reference changes; only entries with a documented file/stdin equivalent
/// belong here.
#[derive(Clone, Copy)]
struct GhBodyOption {
    command: &'static [&'static str],
    inline_long: &'static str,
    inline_short: &'static str,
    file_long: &'static str,
    file_short: &'static str,
}

const GH_BODY_OPTIONS: &[GhBodyOption] = &[
    GhBodyOption {
        command: &["issue", "create"],
        inline_long: "--body",
        inline_short: "-b",
        file_long: "--body-file",
        file_short: "-F",
    },
    GhBodyOption {
        command: &["issue", "edit"],
        inline_long: "--body",
        inline_short: "-b",
        file_long: "--body-file",
        file_short: "-F",
    },
    GhBodyOption {
        command: &["issue", "comment"],
        inline_long: "--body",
        inline_short: "-b",
        file_long: "--body-file",
        file_short: "-F",
    },
    GhBodyOption {
        command: &["pr", "create"],
        inline_long: "--body",
        inline_short: "-b",
        file_long: "--body-file",
        file_short: "-F",
    },
    GhBodyOption {
        command: &["pr", "edit"],
        inline_long: "--body",
        inline_short: "-b",
        file_long: "--body-file",
        file_short: "-F",
    },
    GhBodyOption {
        command: &["pr", "comment"],
        inline_long: "--body",
        inline_short: "-b",
        file_long: "--body-file",
        file_short: "-F",
    },
    GhBodyOption {
        command: &["pr", "review"],
        inline_long: "--body",
        inline_short: "-b",
        file_long: "--body-file",
        file_short: "-F",
    },
    GhBodyOption {
        command: &["discussion", "create"],
        inline_long: "--body",
        inline_short: "-b",
        file_long: "--body-file",
        file_short: "-F",
    },
    GhBodyOption {
        command: &["discussion", "edit"],
        inline_long: "--body",
        inline_short: "-b",
        file_long: "--body-file",
        file_short: "-F",
    },
    GhBodyOption {
        command: &["discussion", "comment"],
        inline_long: "--body",
        inline_short: "-b",
        file_long: "--body-file",
        file_short: "-F",
    },
    GhBodyOption {
        command: &["release", "create"],
        inline_long: "--notes",
        inline_short: "-n",
        file_long: "--notes-file",
        file_short: "-F",
    },
];

#[derive(Debug, Clone)]
struct ShellWord {
    value: String,
    range: std::ops::Range<usize>,
    dynamic: bool,
    multiline: bool,
}

/// Parse enough shell grammar to identify command words and option arguments
/// without treating comments or quoted data as executable commands. This is a
/// deliberately small lexer, not a shell evaluator: aliases and variable
/// command names remain outside G008's contract.
fn shell_commands(source: &str, offset: usize, commands: &mut Vec<Vec<ShellWord>>) {
    let mut command = Vec::new();
    let mut index = 0;
    while index < source.len() {
        let ch = source[index..].chars().next().unwrap();
        if ch.is_whitespace() {
            if ch == '\n' && !command.is_empty() {
                commands.push(std::mem::take(&mut command));
            }
            index += ch.len_utf8();
            continue;
        }
        if matches!(ch, ';' | '|' | '&' | '(' | ')') {
            if !command.is_empty() {
                commands.push(std::mem::take(&mut command));
            }
            index += ch.len_utf8();
            continue;
        }
        if ch == '#' {
            while index < source.len() {
                let comment_ch = source[index..].chars().next().unwrap();
                index += comment_ch.len_utf8();
                if comment_ch == '\n' {
                    break;
                }
            }
            if !command.is_empty() {
                commands.push(std::mem::take(&mut command));
            }
            continue;
        }
        if ch == '\\' && source[index + 1..].starts_with('\n') {
            index += 2;
            continue;
        }

        let start = index;
        let mut value = String::new();
        let mut dynamic = false;
        let mut multiline = false;
        while index < source.len() {
            let word_ch = source[index..].chars().next().unwrap();
            if word_ch.is_whitespace() || matches!(word_ch, ';' | '|' | '&' | '(' | ')') {
                break;
            }
            if word_ch == '\\' {
                index += 1;
                if index < source.len() {
                    let escaped = source[index..].chars().next().unwrap();
                    index += escaped.len_utf8();
                    if escaped == '\n' {
                        value.push(' ');
                    } else {
                        value.push(escaped);
                    }
                }
                continue;
            }
            if word_ch == '\'' || word_ch == '"' {
                let quote = word_ch;
                let ansi_c_quote = quote == '\'' && value.ends_with('$');
                index += 1;
                while index < source.len() {
                    let quoted = source[index..].chars().next().unwrap();
                    index += quoted.len_utf8();
                    if quoted == quote {
                        break;
                    }
                    if quoted == '\n' {
                        multiline = true;
                    }
                    if ansi_c_quote && quoted == '\\' && source[index..].starts_with('n') {
                        multiline = true;
                    }
                    if quote == '"' && matches!(quoted, '$' | '`') {
                        dynamic = true;
                    }
                    if quote == '"' && quoted == '$' && source[index..].starts_with('(') {
                        let inner_start = index + 1;
                        if let Some(close) = matching_command_substitution(source, inner_start) {
                            shell_commands(
                                &source[inner_start..close],
                                offset + inner_start,
                                commands,
                            );
                            value.push_str("$(…)");
                            index = close + 1;
                            continue;
                        }
                    }
                    if quote == '"' && quoted == '\\' && index < source.len() {
                        let escaped = source[index..].chars().next().unwrap();
                        index += escaped.len_utf8();
                        if escaped == '\n' {
                            value.push(' ');
                        } else {
                            value.push(escaped);
                        }
                    } else {
                        value.push(quoted);
                    }
                }
                continue;
            }
            if word_ch == '$' || word_ch == '`' {
                if word_ch == '$' && source[index + 1..].starts_with('\'') {
                    value.push('$');
                    index += 1;
                    continue;
                }
                dynamic = true;
                if word_ch == '$' && source[index + 1..].starts_with('(') {
                    let inner_start = index + 2;
                    if let Some(close) = matching_command_substitution(source, inner_start) {
                        shell_commands(&source[inner_start..close], offset + inner_start, commands);
                        value.push_str("$(…)");
                        index = close + 1;
                        continue;
                    }
                }
            }
            if word_ch == '\n' {
                multiline = true;
            }
            value.push(word_ch);
            index += word_ch.len_utf8();
        }
        if start != index {
            command.push(ShellWord {
                value,
                range: offset + start..offset + index,
                dynamic,
                multiline,
            });
        } else {
            index += source[index..].chars().next().unwrap().len_utf8();
        }
    }
    if !command.is_empty() {
        commands.push(command);
    }
}

fn matching_command_substitution(source: &str, mut index: usize) -> Option<usize> {
    let mut depth = 1;
    while index < source.len() {
        let ch = source[index..].chars().next()?;
        if ch == '\\' {
            index += 1;
            if index < source.len() {
                index += source[index..].chars().next()?.len_utf8();
            }
            continue;
        }
        if ch == '\'' || ch == '"' {
            let quote = ch;
            index += 1;
            while index < source.len() {
                let quoted = source[index..].chars().next()?;
                index += quoted.len_utf8();
                if quoted == quote {
                    break;
                }
            }
            continue;
        }
        if ch == '(' {
            depth += 1;
        } else if ch == ')' {
            depth -= 1;
            if depth == 0 {
                return Some(index);
            }
        }
        index += ch.len_utf8();
    }
    None
}

fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && name
                .chars()
                .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
    })
}

fn gh_command_start(words: &[ShellWord]) -> Option<usize> {
    let mut index = 0;
    while words
        .get(index)
        .is_some_and(|word| is_assignment(&word.value))
    {
        index += 1;
    }
    while words.get(index).is_some_and(|word| {
        matches!(
            word.value.as_str(),
            "if" | "then" | "do" | "while" | "until" | "!"
        )
    }) {
        index += 1;
    }
    if words.get(index).is_some_and(|word| word.value == "command") {
        index += 1;
    } else if words.get(index).is_some_and(|word| word.value == "env") {
        index += 1;
        while words
            .get(index)
            .is_some_and(|word| word.value.starts_with('-') || is_assignment(&word.value))
        {
            index += 1;
        }
    }
    let executable = &words.get(index)?.value;
    (executable == "gh"
        || (executable.starts_with('/') && executable.rsplit('/').next() == Some("gh")))
    .then_some(index + 1)
}

fn option_value<'a>(words: &'a [ShellWord], index: usize, option: &str) -> Option<&'a ShellWord> {
    let word = words.get(index)?;
    if word.value == option {
        return words.get(index + 1);
    }
    word.value
        .strip_prefix(option)
        .and_then(|value| value.strip_prefix('='))
        .map(|_| word)
}

fn command_has_option(words: &[ShellWord], option: &str) -> bool {
    words
        .iter()
        .any(|word| word.value == option || word.value.starts_with(&format!("{option}=")))
}

fn validate_gh_inline(path: &Path, content: &str, diag: &mut DiagnosticCollector) {
    let source = strip_heredoc_bodies(content);
    let mut commands = Vec::new();
    shell_commands(&source, 0, &mut commands);
    for words in commands {
        let Some(start) = gh_command_start(&words) else {
            continue;
        };
        for specification in GH_BODY_OPTIONS {
            if words[start..]
                .iter()
                .take(specification.command.len())
                .map(|word| word.value.as_str())
                .eq(specification.command.iter().copied())
                && !command_has_option(&words, specification.file_long)
                && !command_has_option(&words, specification.file_short)
            {
                for (index, word) in words
                    .iter()
                    .enumerate()
                    .skip(start + specification.command.len())
                {
                    let option = if word.value == specification.inline_long
                        || word
                            .value
                            .starts_with(&format!("{}=", specification.inline_long))
                    {
                        specification.inline_long
                    } else if word.value == specification.inline_short
                        || word
                            .value
                            .starts_with(&format!("{}=", specification.inline_short))
                    {
                        specification.inline_short
                    } else {
                        continue;
                    };
                    let Some(payload) = option_value(&words, index, option) else {
                        continue;
                    };
                    if !(payload.dynamic || payload.multiline) {
                        continue;
                    }
                    if gh_inline_waived(content, word.range.start) {
                        continue;
                    }
                    let location = SourceSpan::from_byte_range(content, word.range.clone())
                        .unwrap_or_else(|| SourceSpan::line(1));
                    let line = location.start().line_number();
                    diag.report_at_with(
                        LintRule::GhInlineBody,
                        path,
                        &format!(
                            "{}:{line}: inline gh {option} payload; use {}",
                            path.display(),
                            specification.file_long
                        ),
                        DiagnosticMetadata::default()
                            .with_location(location)
                            .with_redacted_evidence()
                            .with_suggestion(format!(
                                "use {} with a file path or '-' for stdin",
                                specification.file_long
                            )),
                    );
                }
            }
        }
    }
}

fn g008_shell_script(path: &Path) -> bool {
    let path = path.to_string_lossy();
    path.ends_with(".sh") || path.ends_with(".inc.bash")
}

fn strip_heredoc_bodies(source: &str) -> String {
    let mut result = String::with_capacity(source.len());
    let mut heredoc: Option<String> = None;
    for line in source.split_inclusive('\n') {
        if let Some(delimiter) = &heredoc {
            if line.trim() == delimiter {
                heredoc = None;
                result.push_str(line);
            } else {
                for ch in line.chars() {
                    if ch == '\n' {
                        result.push('\n');
                    } else {
                        result.extend((0..ch.len_utf8()).map(|_| ' '));
                    }
                }
            }
            continue;
        }
        result.push_str(line);
        heredoc = (!line.trim_start().starts_with('#'))
            .then(|| heredoc_state(line, false).map(|state| state.delimiter))
            .flatten();
    }
    result
}

fn gh_inline_waived(source: &str, offset: usize) -> bool {
    let line_start = source[..offset].rfind('\n').map_or(0, |index| index + 1);
    let line_end = source[offset..]
        .find('\n')
        .map_or(source.len(), |index| offset + index);
    has_reasoned_marker(&source[line_start..line_end], "lint-gh-body-inline: ok")
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
        let import_graph =
            InstructionImportGraph::build(&diag.config().instruction_files, &ExcludeSet::default());
        validate_claude_import_budget(&import_graph, &mut diag);
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
    fn d005_normalizes_probes_and_preserves_its_documented_suppression_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir("docs").unwrap();
        fs::write("docs/present.md", "present\n").unwrap();
        fs::write("outside.md", "present\n").unwrap();
        fs::write(
            "AGENTS.md",
            "See `docs/present.md#usage` and `docs/present.md::entry`.\n\
             See `docs/../outside.md` and `/absolute-missing.md`.\n\
             See `docs/suppressed.md`. <!-- lint-doc-pointer-paths: ok documented exception -->\n",
        )
        .unwrap();
        let config = LintConfig {
            inline_path_prefixes: vec!["docs/".into(), "/".into()],
            ..LintConfig::default()
        };
        let mut diag = all_enabled_with(config);

        validate_inline_paths(&mut diag, &ExcludeSet::default());

        let evidence: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::InlinePathMissing)
            .filter_map(|item| item.evidence.as_deref())
            .collect();
        assert_eq!(evidence, vec!["docs/../outside.md", "/absolute-missing.md"]);
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

        let import_graph =
            InstructionImportGraph::build(&diag.config().instruction_files, &ExcludeSet::default());
        validate_claude_import_budget(&import_graph, &mut diag);

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
        let agent_budget = diag
            .diagnostics()
            .iter()
            .find(|item| {
                item.rule == LintRule::ClaudeImportLarge
                    && item.subject_path.as_deref() == Some(Path::new("AGENTS.md"))
            })
            .expect("AGENTS.md budget diagnostic");
        assert!(agent_budget.location.is_some());
        assert_eq!(agent_budget.evidence.as_deref(), Some("AGENTS.md"));
        assert!(agent_budget.suggestion.is_some());
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
    #[serial_test::serial]
    fn flag_parity_scopes_each_script_and_preserves_structured_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all("skills/demo/scripts").unwrap();
        fs::write(
            "skills/demo/scripts/first.sh",
            "case \"$1\" in --first) ;; esac\n",
        )
        .unwrap();
        fs::write(
            "skills/demo/scripts/second.sh",
            "case \"$1\" in --second) ;; esac\n",
        )
        .unwrap();
        fs::write("skills/demo/scripts/forward.sh", "exec other \"$@\"\n").unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_flag_signature(
            Path::new("skills/demo/SKILL.md"),
            17,
            "scripts/first.sh --first && scripts/second.sh --missing",
            &mut diag,
        );
        let findings: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::SkillFlagMismatch)
            .collect();
        assert_eq!(findings.len(), 1);
        assert!(
            findings[0]
                .message
                .contains("skills/demo/scripts/second.sh")
        );
        assert_eq!(findings[0].location, Some(SourceSpan::line(17)));
        assert_eq!(
            findings[0].suggestion.as_deref(),
            Some("remove the unsupported flag or add it to the shipped script's parser")
        );

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_flag_signature(
            Path::new("skills/demo/SKILL.md"),
            1,
            "scripts/first.sh --wrong && scripts/second.sh --missing",
            &mut diag,
        );
        let messages: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::SkillFlagMismatch)
            .map(|item| item.message.as_str())
            .collect();
        assert_eq!(messages.len(), 2);
        assert!(messages[0].contains("--wrong") && messages[0].contains("first.sh"));
        assert!(messages[1].contains("--missing") && messages[1].contains("second.sh"));

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_flag_signature(
            Path::new("skills/demo/SKILL.md"),
            1,
            "scripts/forward.sh --delegated && scripts/second.sh --missing",
            &mut diag,
        );
        assert_eq!(
            diag.diagnostics()
                .iter()
                .filter(|item| item.rule == LintRule::SkillFlagMismatch)
                .count(),
            1
        );
    }

    #[test]
    #[serial_test::serial]
    fn flag_parity_recognizes_executable_forms_and_ignores_comments() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all("skills/demo/scripts").unwrap();
        fs::write(
            "skills/demo/scripts/forms.sh",
            "case \"$1\" in --out=*) ;; esac\nif [ \"$1\" = \"--json\" ]; then :; fi\nif [ \"--reverse\" = \"$1\" ]; then :; fi\n",
        )
        .unwrap();
        fs::write(
            "skills/demo/scripts/forms.py",
            "parser.add_argument(\n    \"--verbose\",\n    action=\"store_true\",\n)\nif \"--json\" in sys.argv:\n    pass\n",
        )
        .unwrap();
        fs::write(
            "skills/demo/scripts/comment-forward.sh",
            "# Do not forward \"$@\"; this command accepts only --known.\ncase \"$1\" in --known) ;; esac # \"$@\"\n",
        )
        .unwrap();
        fs::write(
            "skills/demo/scripts/comment-declare.py",
            "# parser.add_argument(\"--phantom\") is obsolete documentation.\nparser.add_argument(\"--known\") # sys.argv[1:]\n",
        )
        .unwrap();

        let mut accepted = DiagnosticCollector::new_all_enabled();
        for command in [
            "scripts/forms.sh --out=result.txt --json --reverse",
            "scripts/forms.py --verbose --json",
        ] {
            validate_flag_signature(Path::new("skills/demo/SKILL.md"), 1, command, &mut accepted);
        }
        assert!(accepted.diagnostics().is_empty());

        let mut comments = DiagnosticCollector::new_all_enabled();
        validate_flag_signature(
            Path::new("skills/demo/SKILL.md"),
            1,
            "scripts/comment-forward.sh --missing && scripts/comment-declare.py --phantom",
            &mut comments,
        );
        assert_eq!(
            comments
                .diagnostics()
                .iter()
                .filter(|item| item.rule == LintRule::SkillFlagMismatch)
                .count(),
            2
        );
        assert_eq!(
            strip_shell_comments("echo \"# remains quoted\n# and remains quoted\"\n# removed\n"),
            "echo \"# remains quoted\n# and remains quoted\"\n\n"
        );
    }

    #[test]
    #[serial_test::serial]
    fn flag_parity_resolves_documented_roots_and_skill_relative_scripts() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all("skills/demo/scripts").unwrap();
        fs::create_dir_all("scripts").unwrap();
        fs::write(
            "skills/demo/scripts/run.sh",
            "case \"$1\" in --known) ;; esac\n",
        )
        .unwrap();
        fs::write(
            "skills/demo/scripts/other.sh",
            "case \"$1\" in --real) ;; esac\n",
        )
        .unwrap();
        fs::write("scripts/other.sh", "case \"$1\" in --root) ;; esac\n").unwrap();

        let skill = Path::new("skills/demo/SKILL.md");
        let mut diag = DiagnosticCollector::new_all_enabled();
        for command in [
            "\"${CLAUDE_PLUGIN_ROOT}\"/skills/demo/scripts/run.sh --quoted",
            "${CLAUDE_PROJECT_DIR}/skills/demo/scripts/run.sh --project",
            "scripts/run.sh --bare",
        ] {
            validate_flag_signature(skill, 1, command, &mut diag);
        }
        validate_flag_signature(skill, 1, "./scripts/other.sh --real", &mut diag);
        assert_eq!(
            diag.diagnostics()
                .iter()
                .filter(|item| item.rule == LintRule::SkillFlagMismatch)
                .count(),
            3
        );

        let mut waiver = DiagnosticCollector::new_all_enabled();
        validate_flag_signature(
            skill,
            1,
            "scripts/run.sh --missing # lint-skill-md-flag-signature: ok reviewed",
            &mut waiver,
        );
        assert!(waiver.diagnostics().is_empty());
        validate_flag_signature(
            skill,
            1,
            "scripts/run.sh --missing # lint-skill-md-flag-signature: ok",
            &mut waiver,
        );
        assert_eq!(waiver.diagnostics().len(), 1);
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
            "gh pr create --body \"$payload\"\ndeclare -A values\nempty=()\nprintf '%s' \"${empty[@]}\"\nout=${text//x/$replacement}\ncat <<EOF\ncopy=${text//x/$replacement}\nEOF\ncat <<'SAFE'\nliteral=${text//x/$replacement}\nSAFE\nawk -v re='—' '$0 ~ re'\n",
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
    fn gh_inline_body_matches_the_documented_command_matrix() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all("scripts").unwrap();
        let content = GH_BODY_OPTIONS
            .iter()
            .flat_map(|specification| {
                [
                    format!(
                        "gh {} {} \"$PAYLOAD\"",
                        specification.command.join(" "),
                        specification.inline_short
                    ),
                    format!(
                        "gh {} {} \"$PAYLOAD\"",
                        specification.command.join(" "),
                        specification.inline_long
                    ),
                    format!(
                        "gh {} {}=\"$PAYLOAD\"",
                        specification.command.join(" "),
                        specification.inline_long
                    ),
                    format!(
                        "gh {} {}=\"$PAYLOAD\"",
                        specification.command.join(" "),
                        specification.inline_short
                    ),
                ]
            })
            .collect::<Vec<_>>()
            .join("\n");
        fs::write("scripts/matrix.sh", content).unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_script_contracts(&mut diag, &ExcludeSet::default(), true);
        let findings: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::GhInlineBody)
            .collect();
        assert_eq!(findings.len(), GH_BODY_OPTIONS.len() * 4);
        assert_eq!(
            findings[0].location,
            Some(SourceSpan::range(1, 17, 1, 19)),
            "the first -b option has an exact structured source span"
        );
        for (findings, specification) in findings.chunks(4).zip(GH_BODY_OPTIONS) {
            for finding in findings {
                assert!(finding.message.contains(specification.file_long));
                assert_eq!(
                    finding.subject_path.as_deref(),
                    Some(Path::new("scripts/matrix.sh"))
                );
                assert!(finding.location.is_some());
                assert!(finding.evidence.is_some());
                let expected_suggestion = format!(
                    "use {} with a file path or '-' for stdin",
                    specification.file_long
                );
                assert_eq!(
                    finding.suggestion.as_deref(),
                    Some(expected_suggestion.as_str())
                );
            }
        }
    }

    #[test]
    #[serial_test::serial]
    fn gh_inline_body_uses_shell_lexing_and_excludes_non_prose_commands() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all("scripts").unwrap();
        fs::write(
            "scripts/cases.sh",
            r#"# gh pr create --body "$BODY"
printf '%s\n' 'gh pr create --body "$BODY"'
weigh --body 5
sleigh --notes x
gh secret set TOKEN --body "$TOKEN"
gh variable set NAME --body "$VALUE"
gh project item-create 1 --body "$BODY"
gh pr create --body 'static body'
gh pr create --body $'static body'
gh release create v1 --notes="static notes"
gh pr create --body "$BODY" --body-file body.md
gh release create v1 -n "$NOTES" -F notes.md
gh pr create --body "$BODY" # lint-gh-body-inline: ok reviewed constant boundary
gh pr create \
  -b "$BODY"; gh issue comment -b "$BODY"
command gh pr create -b "$BODY"
env -i gh issue create --body="$BODY"
/usr/local/bin/gh release create v1 --notes "$NOTES"
echo "$(gh discussion comment 1 -b "$BODY")"
gh issue create --body "first
second"
cat <<'EOF'
gh pr create --body "$BODY"
EOF
"#,
        )
        .unwrap();
        fs::write(
            "scripts/example.py",
            "value = 'gh pr create --body \"$BODY\"'\n",
        )
        .unwrap();
        fs::write("scripts/example.bash", "gh pr create --body \"$BODY\"\n").unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_script_contracts(&mut diag, &ExcludeSet::default(), true);
        let findings: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::GhInlineBody)
            .collect();
        assert_eq!(findings.len(), 7);
        assert!(
            findings
                .iter()
                .all(|item| item.subject_path.as_deref() == Some(Path::new("scripts/cases.sh")))
        );
        for line in [15, 16, 17, 18, 19, 20] {
            assert!(
                findings
                    .iter()
                    .any(|item| item.message.contains(&format!(":{line}:"))),
                "missing diagnostic at line {line}"
            );
        }
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
        let graph =
            InstructionImportGraph::build(&diag.config().instruction_files, &ExcludeSet::default());
        validate_import_graph(&graph, &mut diag);
        let l001: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::ImportPathMissing)
            .collect();
        assert_eq!(l001.len(), 1, "dedup missing target per (source, target)");
        assert!(l001[0].message.contains("missing or unreadable"));
        assert_eq!(
            l001[0].subject_path.as_deref(),
            Some(Path::new("CLAUDE.md"))
        );
        assert_eq!(
            l001[0]
                .location
                .expect("directive span")
                .start()
                .line_number(),
            1
        );
        assert_eq!(l001[0].evidence.as_deref(), Some("docs/missing.md"));
        assert!(l001[0].suggestion.is_some());
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
        fs::write("docs/a.md", "@../CLAUDE.md\n").unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        let graph =
            InstructionImportGraph::build(&diag.config().instruction_files, &ExcludeSet::default());
        validate_import_graph(&graph, &mut diag);
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
        fs::write("docs/a.md", "@b.md\n").unwrap();
        fs::write("docs/b.md", "@c.md\n").unwrap();
        fs::write("docs/c.md", "@d.md\n").unwrap();
        fs::write("docs/d.md", "@e.md\n").unwrap();
        fs::write("docs/e.md", "@f.md\n").unwrap();
        fs::write("docs/f.md", "end\n").unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        let graph =
            InstructionImportGraph::build(&diag.config().instruction_files, &ExcludeSet::default());
        validate_import_graph(&graph, &mut diag);
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
        fs::write("docs/a.md", "@b.md\n").unwrap();
        fs::write("docs/b.md", "@c.md\n").unwrap();
        fs::write("docs/c.md", "@d.md\n").unwrap();
        fs::write("docs/d.md", "@e.md\n").unwrap();
        fs::write("docs/e.md", "end\n").unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        let graph =
            InstructionImportGraph::build(&diag.config().instruction_files, &ExcludeSet::default());
        validate_import_graph(&graph, &mut diag);
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
        let graph =
            InstructionImportGraph::build(&diag.config().instruction_files, &ExcludeSet::default());
        validate_import_graph(&graph, &mut diag);
        let dup: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::DuplicateImport)
            .collect();
        assert_eq!(dup.len(), 1);
        assert!(dup[0].message.contains("duplicate"));
    }

    #[test]
    #[serial_test::serial]
    fn import_graph_is_source_relative_and_ignores_non_live_markdown_contexts() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all("docs").unwrap();
        fs::write("child.txt", "root shadow\n").unwrap();
        fs::write(
            "CLAUDE.md",
            "@docs/a.md\nExample: @missing.txt\n`@missing.txt` [link](@missing.txt)\n> @missing.txt\n    @missing.txt\n```text\n@missing.txt\n```\n",
        )
        .unwrap();
        fs::write("docs/a.md", "@child.txt\n").unwrap();
        fs::write("docs/child.txt", "nested source\n").unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        let graph =
            InstructionImportGraph::build(&diag.config().instruction_files, &ExcludeSet::default());
        validate_import_graph(&graph, &mut diag);
        assert!(graph.nodes.contains_key(Path::new("docs/child.txt")));
        assert!(!graph.nodes.contains_key(Path::new("child.txt")));
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::ImportPathMissing)
        );
    }

    #[test]
    #[serial_test::serial]
    fn import_graph_keeps_excluded_targets_opaque_and_reports_depth_with_cycles() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all("docs/excluded").unwrap();
        fs::write("CLAUDE.md", "@docs/a.txt\n@docs/excluded/hidden.txt\n").unwrap();
        for (name, next) in [
            ("a.txt", "b.txt"),
            ("b.txt", "c.txt"),
            ("c.txt", "d.txt"),
            ("d.txt", "e.txt"),
            ("e.txt", "f.txt"),
        ] {
            fs::write(format!("docs/{name}"), format!("@{next}\n")).unwrap();
        }
        fs::write("docs/f.txt", "@a.txt\n").unwrap();
        let exclude = ExcludeSet::new(&["docs/excluded/**".to_string()]).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        let graph = InstructionImportGraph::build(&diag.config().instruction_files, &exclude);
        validate_import_graph(&graph, &mut diag);
        assert!(
            !graph
                .nodes
                .contains_key(Path::new("docs/excluded/hidden.txt"))
        );
        assert!(
            diag.diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::CircularImport)
        );
        assert!(
            diag.diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::ImportDepthExceeded)
        );
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::ImportPathMissing)
        );
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
}
