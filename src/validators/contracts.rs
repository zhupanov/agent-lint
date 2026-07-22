//! Prompt, reference, and shipped-script contracts shared by public and private skills.

use crate::config::{ExcludeSet, PromptMetricCaps, PromptSourceBudget};
use crate::context::LintContext;
use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata, SourceSpan};
use crate::fence::consecutive_bash_pairs;
use crate::frontmatter;
use crate::markdown::MarkdownDocument;
use crate::markdown_refs::{
    MarkdownRefKind, SUGGEST_CREATE_OR_CORRECT, SUGGEST_REPLACE_SYMLINK,
    is_external_or_fragment_destination, markdown_references, percent_decode_once,
};
use crate::prompt_budget::normalize_repo_relative;
use crate::repo_path::{
    PathProbe, ResolutionBase, normalize_path_probe, normalize_separators, normalized_target_key,
    probe_contains_parent_segment, resolve_repo_path,
};
use crate::rules::LintRule;
use crate::script_paths::{ScriptKind, script_kind};
use crate::script_paths::{ScriptReference, ScriptReferenceBase, extract_script_token_references};
use crate::traversal;
use crate::validators::common::{classify_inline_code_path, tokenize_tool_field};
use crate::validators::shell;
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

static SKILL_INVOKE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:re-)?invoke\b\s+(?:the\s+)?(?:\*\*[^*\n]{1,40}\*\*\s+)?`/[-\w]+`(?:\s+skill\b)?",
    )
    .unwrap()
});
const S058_MISSING_STEP_SUGGESTION: &str = "add an operative Skill-tool invocation step";
const S058_AMBIGUOUS_INVOKE_SUGGESTION: &str = "name the Skill tool on this line";
static FLAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|\s)--([A-Za-z0-9][A-Za-z0-9_-]*)\b").unwrap());
static AWK_FIELD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\$[0-9]+").unwrap());
static HEREDOC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<<-?\s*(?:'([A-Za-z_][A-Za-z0-9_]*)'|\"([A-Za-z_][A-Za-z0-9_]*)\"|([A-Za-z_][A-Za-z0-9_]*))"#,
    )
    .unwrap()
});
static AWK_COMMAND: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:[A-Za-z_][A-Za-z0-9_]*=[^\s]+\s+)*(?:command\s+)?awk(?:\s|$)").unwrap()
});
static FORWARDED_ARRAY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?m)^[^#\n]*(?:exec\s+)?[^\n]*"\$\{([A-Za-z_][A-Za-z0-9_]*)\[@\]\}""#).unwrap()
});
pub fn validate_contracts(
    ctx: &LintContext,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    include_public: bool,
) {
    let discovery = super::skill_discovery::SkillDiscovery::from_context(ctx, exclude);
    let mut skill_files = discovery.private_skill_files;
    if include_public {
        skill_files.extend(discovery.exported_skill_files);
    }
    skill_files.sort();
    skill_files.dedup();
    validate_skill_contracts_paths(diag, exclude, skill_files);
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

#[cfg(test)]
fn scoped_skill_files(include_public: bool) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if include_public {
        paths.extend(one_level_files("skills", "SKILL.md"));
    }
    paths.extend(one_level_files(".claude/skills", "SKILL.md"));
    paths.sort();
    paths
}

#[cfg(test)]
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

/// Read S058's gate from the canonical YAML value. Invalid and non-mapping
/// frontmatter intentionally yields no value: X001/S004 own those states.
/// The shared tokenizer keeps this gate's accepted grammar aligned with S040,
/// S067, and the other tool-field consumers.
fn canonical_tool_field(content: &str, key: &str) -> Option<Vec<String>> {
    let lines = frontmatter::extract_frontmatter(content)?;
    let yaml = frontmatter::parse_yaml_strict(&lines).ok()?;
    let value = yaml.as_mapping()?.get(key)?;
    Some(tokenize_tool_field(value))
}

fn tool_base_name(tool: &str) -> &str {
    tool.split_once('(').map_or(tool, |(base, _)| base).trim()
}

fn normalize_skill_line(line: &str) -> String {
    line.chars()
        .filter(|character| !matches!(character, '*' | '_' | '`'))
        .map(|character| character.to_ascii_lowercase())
        .collect()
}

fn clause_has_clear_skill_step(clause: &str) -> bool {
    const ACTION_VERBS: &[&str] = &["invoke", "use", "call", "launch", "run", "delegate"];
    const PROHIBITIONS: &[&str] = &[
        "do not", "don't", "never", "must not", "cannot", "can't", "without",
    ];

    !PROHIBITIONS.iter().any(|phrase| clause.contains(phrase))
        && clause.match_indices("skill tool").any(|(position, _)| {
            clause[..position]
                .split(|character: char| !character.is_ascii_alphabetic())
                .any(|word| ACTION_VERBS.contains(&word))
        })
}

fn has_clear_skill_step(line: &str) -> bool {
    normalize_skill_line(line)
        .split(['.', ';', '—'])
        .any(clause_has_clear_skill_step)
}

/// Match the raw line only when the invocation verb itself is still visible in
/// `body_prose()`. That preserves a live line's backticked `/name` while
/// preventing quoted examples and HTML comments from becoming eligible again.
fn has_visible_ambiguous_skill_invoke(raw_line: &str, prose_line: &str) -> bool {
    SKILL_INVOKE.find_iter(raw_line).any(|matched| {
        let start_column = raw_line[..matched.start()].chars().count();
        raw_line.chars().nth(start_column) == prose_line.chars().nth(start_column)
    })
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

#[cfg(test)]
fn validate_skill_contracts(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    include_public: bool,
) {
    validate_skill_contracts_paths(diag, exclude, scoped_skill_files(include_public));
}

fn validate_skill_contracts_paths(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    paths: Vec<PathBuf>,
) {
    for path in paths {
        let Some(content) = read_text(&path, exclude) else {
            continue;
        };
        let document = MarkdownDocument::parse(&content);
        if canonical_tool_field(&content, "allowed-tools")
            .is_some_and(|tools| tools.iter().any(|tool| tool_base_name(tool) == "Skill"))
        {
            if !document
                .body_prose()
                .iter()
                .any(|prose_line| has_clear_skill_step(&prose_line.text))
            {
                diag.report_at_with(
                    LintRule::SkillInvokeMissing,
                    &path,
                    &format!(
                        "{}: allowed-tools includes Skill but the body has no explicit Skill tool invocation step",
                        path.display()
                    ),
                    DiagnosticMetadata::default().with_suggestion(S058_MISSING_STEP_SUGGESTION),
                );
            }
            let source_lines: Vec<_> = content.lines().collect();
            for prose_line in document.body_prose() {
                let raw_line = source_lines[prose_line.line - 1];
                if has_visible_ambiguous_skill_invoke(raw_line, &prose_line.text)
                    && !normalize_skill_line(&prose_line.text).contains("skill tool")
                {
                    diag.report_at_with(
                        LintRule::SkillInvokeMissing,
                        &path,
                        &format!(
                            "{}:{}: ambiguous skill invocation; identify the Skill tool on the same line",
                            path.display(),
                            prose_line.line
                        ),
                        DiagnosticMetadata::at_line(prose_line.line)
                            .with_suggestion(S058_AMBIGUOUS_INVOKE_SUGGESTION),
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

/// Command basenames recognized as awk-family interpreters in skill fences.
const AWK_COMMAND_NAMES: &[&str] = &["awk", "nawk", "mawk", "gawk"];
/// Command basenames recognized as grep-family probes in skill fences.
const GREP_COMMAND_NAMES: &[&str] = &["grep", "egrep", "fgrep", "rg", "ripgrep"];

/// Shell options whose following token is a value, not a path candidate.
const GREP_VALUE_OPTIONS: &[&str] = &[
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

/// Classify a shell token's command basename after removing only a recognized
/// command-position prefix (assignment+`$(`, leading `$(`, leading backtick, or
/// leading subshell parentheses), then taking the final `/` component.
fn command_basename(token: &str) -> &str {
    let command = strip_command_position_prefix(token);
    command
        .rsplit('/')
        .next()
        .filter(|part| !part.is_empty())
        .unwrap_or(command)
}

fn strip_command_position_prefix(token: &str) -> &str {
    if let Some((name, rest)) = token.split_once("=$(") {
        if is_shell_identifier(name) {
            return rest;
        }
    }
    if let Some((name, rest)) = token.split_once("=`") {
        if is_shell_identifier(name) {
            return rest;
        }
    }
    if let Some(rest) = token.strip_prefix("$(") {
        return rest;
    }
    if let Some(rest) = token.strip_prefix('`') {
        return rest;
    }
    token.trim_start_matches('(')
}

fn is_shell_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(ch) if ch.is_ascii_alphabetic() || ch == '_' => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

/// When `token` begins with a redirection operator (optionally digit-prefixed),
/// returns the remainder after that operator. An empty remainder means the
/// operator is standalone and consumes the following token as its target.
fn redirection_remainder(token: &str) -> Option<&str> {
    let bytes = token.as_bytes();
    let mut digits = 0usize;
    while digits < bytes.len() && bytes[digits].is_ascii_digit() {
        digits += 1;
    }
    let rest = &token[digits..];
    let operators: &[&str] = if digits > 0 {
        &[">>", ">|", ">&", ">", "<"]
    } else {
        &[">>", ">|", "&>", ">", "<"]
    };
    for operator in operators {
        if let Some(after) = rest.strip_prefix(operator) {
            return Some(after);
        }
    }
    None
}

fn is_input_redirection(token: &str) -> bool {
    let bytes = token.as_bytes();
    let mut digits = 0usize;
    while digits < bytes.len() && bytes[digits].is_ascii_digit() {
        digits += 1;
    }
    token[digits..].starts_with('<')
}

struct GrepArgAnalysis {
    has_explicit_path: bool,
    stdin_redirected: bool,
    path_has_parent_dir: bool,
}

fn analyze_grep_args(args: &[String]) -> GrepArgAnalysis {
    let mut skip_value = false;
    let mut explicit_pattern = false;
    let mut positional_count = 0usize;
    let mut stdin_redirected = false;
    let mut path_has_parent_dir = false;
    let mut index = 0usize;
    while index < args.len() {
        let arg = &args[index];
        if skip_value {
            skip_value = false;
            index += 1;
            continue;
        }
        if let Some(remainder) = redirection_remainder(arg) {
            if is_input_redirection(arg) {
                stdin_redirected = true;
            }
            if remainder.is_empty() {
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }
        if matches!(arg.as_str(), "-e" | "--regexp" | "-f" | "--file") {
            explicit_pattern = true;
            skip_value = true;
            index += 1;
            continue;
        }
        if arg.starts_with("-e")
            || arg.starts_with("--regexp=")
            || arg.starts_with("-f")
            || arg.starts_with("--file=")
        {
            explicit_pattern = true;
            index += 1;
            continue;
        }
        if GREP_VALUE_OPTIONS.contains(&arg.as_str()) {
            skip_value = true;
            index += 1;
            continue;
        }
        if arg.starts_with('-') || matches!(arg.as_str(), "|" | "||" | "&&" | ";") {
            index += 1;
            continue;
        }
        positional_count += 1;
        if Path::new(arg)
            .components()
            .any(|part| part == Component::ParentDir)
        {
            path_has_parent_dir = true;
        }
        index += 1;
    }
    GrepArgAnalysis {
        has_explicit_path: positional_count >= if explicit_pattern { 1 } else { 2 },
        stdin_redirected,
        path_has_parent_dir,
    }
}

fn awk_programs(command: &str) -> Vec<String> {
    let tokens = shell_lex(command);
    let mut programs = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        if !AWK_COMMAND_NAMES.contains(&command_basename(&tokens[index])) {
            index += 1;
            continue;
        }
        index += 1;
        let mut source_from_file = false;
        while index < tokens.len() && !matches!(tokens[index].as_str(), "|" | ";" | "&") {
            let token = &tokens[index];
            if matches!(token.as_str(), "-F" | "-v" | "-W") {
                index += 2;
            } else if token.starts_with("-F") || token.starts_with("-v") || token.starts_with("-W")
            {
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
        diag.report_at_with(
            LintRule::AwkFieldRef,
            skill,
            &format!(
                "{}:{line}: bare awk positional field in a skill shell fence; move parsing into a shipped script",
                skill.display()
            ),
            DiagnosticMetadata::default()
                .with_location(SourceSpan::line(line))
                .with_suggestion("move the awk parsing into a shipped script"),
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
        if !GREP_COMMAND_NAMES.contains(&command_basename(word)) {
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
        let analysis = analyze_grep_args(args);
        let metadata = |suggestion: &str| {
            DiagnosticMetadata::default()
                .with_location(SourceSpan::line(line))
                .with_suggestion(suggestion)
        };
        if analysis.path_has_parent_dir {
            diag.report_at_with(
                LintRule::UnsafeGrepProbe,
                skill,
                &format!(
                    "{}:{line}: grep-family path ascends through a parent directory",
                    skill.display()
                ),
                metadata("use a repository-contained path"),
            );
            continue;
        }
        let clause_prefix: Vec<_> = prefix
            .iter()
            .rev()
            .take_while(|value| !matches!(value.as_str(), "|" | "|&" | "||" | "&&" | ";" | "&"))
            .collect();
        let conditional = clause_prefix
            .iter()
            .any(|value| *value == "if" || *value == "elif");
        let wrapped = clause_prefix.iter().any(|value| *value == "command");
        let arg_fed = clause_prefix
            .iter()
            .any(|value| *value == "xargs" || *value == "parallel");
        // Bare-top-level arm stays limited to a literal unqualified `grep` token.
        let bare_grep = word == "grep" && !wrapped && (index == 0 || conditional);
        if bare_grep {
            diag.report_at_with(
                LintRule::UnsafeGrepProbe,
                skill,
                &format!(
                    "{}:{line}: bare top-level grep in a shell fence; wrap it or use command grep",
                    skill.display()
                ),
                metadata("prefix top-level grep with command or feed it through a pipe"),
            );
        } else if !pipe_fed && !analysis.stdin_redirected && !analysis.has_explicit_path && !arg_fed
        {
            diag.report_at_with(
                LintRule::UnsafeGrepProbe,
                skill,
                &format!(
                    "{}:{line}: grep-family probe has no explicit path and may block on stdin",
                    skill.display()
                ),
                metadata("add an explicit search path or pipe/redirect input"),
            );
        }
    }
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
        let waived_lines: BTreeSet<usize> = content
            .lines()
            .enumerate()
            .filter_map(|(index, line)| {
                has_reasoned_marker(line, "lint-doc-pointer-paths: ok").then_some(index + 1)
            })
            .collect();
        let mut seen = BTreeSet::new();
        for reference in markdown_references(&content) {
            if reference.kind != MarkdownRefKind::InlineCode {
                continue;
            }
            let line = content[..reference.byte_range.start]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count()
                + 1;
            if waived_lines.contains(&line) {
                continue;
            }
            let classified = normalize_separators(&reference.raw);
            if !classify_inline_code_path(&classified).is_repository_path()
                || !prefixes.iter().any(|prefix| {
                    classified.starts_with(prefix) || reference.raw.starts_with(prefix)
                })
            {
                continue;
            }
            let probe = normalize_path_probe(&reference.raw);
            let rejected_parent = probe_contains_parent_segment(&probe);
            let key = if rejected_parent {
                format!("unsafe:{probe}")
            } else {
                normalized_target_key(path, &reference.raw, ResolutionBase::RepositoryRoot)
                    .unwrap_or_else(|| probe.clone())
            };
            if !seen.insert(key) {
                continue;
            }
            let outcome = if rejected_parent {
                PathProbe::Rejected
            } else {
                resolve_repo_path(path, &reference.raw, ResolutionBase::RepositoryRoot)
            };
            let suggestion = match &outcome {
                PathProbe::File(_) | PathProbe::Directory(_) => continue,
                PathProbe::Missing(_) => SUGGEST_CREATE_OR_CORRECT,
                PathProbe::Rejected => SUGGEST_REPLACE_SYMLINK,
            };
            let metadata = SourceSpan::from_byte_range(&content, reference.byte_range.clone())
                .map_or_else(DiagnosticMetadata::default, |location| {
                    DiagnosticMetadata::default().with_location(location)
                })
                .with_evidence(&reference.raw)
                .with_suggestion(suggestion);
            diag.report_at_with(
                LintRule::InlinePathMissing,
                &relpath,
                &format!(
                    "{relpath}:{line}: dead or escaping inline path `{}`",
                    reference.raw
                ),
                metadata,
            );
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
    is_external_or_fragment_destination(target)
}

/// L005: broken relative markdown link `[text](path.md)` in any configured
/// instruction file. External URLs, pure anchors, image nodes, and non-`.md`
/// destinations are skipped. Destinations resolve source-relatively only.
fn validate_markdown_links(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    for relpath in diag.config().instruction_files.clone() {
        if exclude.is_excluded(&relpath) {
            continue;
        }
        let path = Path::new(&relpath);
        let Some(content) = read_text(path, exclude) else {
            continue;
        };
        let mut seen = BTreeSet::new();
        for reference in markdown_references(&content) {
            if reference.kind != MarkdownRefKind::Link {
                continue;
            }
            let raw = &reference.raw;
            let decoded = percent_decode_once(raw);
            if is_external_link(&decoded) {
                continue;
            }
            let path_only = decoded
                .split_once('#')
                .map_or(decoded.as_str(), |(path, _)| path);
            if path_only.is_empty()
                || !path_only
                    .rsplit_once('.')
                    .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("md"))
            {
                continue;
            }
            let key = normalized_target_key(path, path_only, ResolutionBase::SourceRelative)
                .unwrap_or_else(|| normalize_path_probe(path_only));
            if !seen.insert(key) {
                continue;
            }
            let outcome = resolve_repo_path(path, path_only, ResolutionBase::SourceRelative);
            let suggestion = match &outcome {
                PathProbe::File(_) => continue,
                PathProbe::Directory(_) | PathProbe::Missing(_) => SUGGEST_CREATE_OR_CORRECT,
                PathProbe::Rejected => SUGGEST_REPLACE_SYMLINK,
            };
            let line = content[..reference.byte_range.start]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count()
                + 1;
            let metadata = SourceSpan::from_byte_range(&content, reference.byte_range.clone())
                .map_or_else(DiagnosticMetadata::default, |location| {
                    DiagnosticMetadata::default().with_location(location)
                })
                .with_evidence(raw.as_str())
                .with_suggestion(suggestion);
            diag.report_at_with(
                LintRule::BrokenMarkdownLink,
                &relpath,
                &format!("{relpath}:{line}: broken markdown link target: {raw}"),
                metadata,
            );
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
/// Whether the two `set` options that gate Bash-3.2 empty-array and errexit
/// hazards are lexically in effect for a script. A sourced `.inc.bash` library
/// inherits its caller's options, so both gates are enabled there.
struct ScriptGates {
    errexit: bool,
    nounset: bool,
}

fn script_gates(path: &Path, kind: ScriptKind, content: &str) -> ScriptGates {
    let inc_bash = path.to_string_lossy().ends_with(".inc.bash");
    let mut gates = ScriptGates {
        errexit: inc_bash,
        nounset: inc_bash,
    };
    if kind == ScriptKind::Shell {
        for raw in content.lines() {
            let flags = shell::set_flags(raw);
            gates.errexit |= flags.errexit;
            gates.nounset |= flags.nounset;
        }
    }
    gates
}

/// Conservative, function-scoped tracking of arrays known to be empty on the
/// current straight-line path. Any control-flow boundary (a conditional, loop,
/// function body, group, or subshell) makes every tracked array ambiguous, so a
/// bare `"${arr[@]}"` fires only when the array is provably empty with nothing
/// between its `arr=()` assignment and the expansion.
#[derive(Default)]
struct EmptyArrayTracker {
    known_empty: HashSet<String>,
}

impl EmptyArrayTracker {
    fn scan_line(
        &mut self,
        path: &Path,
        line_number: usize,
        line: &str,
        suppressed: bool,
        diag: &mut DiagnosticCollector,
    ) {
        if shell::opens_control_flow(line) {
            self.known_empty.clear();
            return;
        }
        // Interleave assignments and expansions in source order so a reset later
        // on the line cannot retroactively make an earlier expansion look empty.
        enum Event {
            Assign { name: String, empty: bool },
            Expand { name: String, span: shell::Span },
        }
        let mut events: Vec<(usize, Event)> = Vec::new();
        for assignment in shell::array_assignments(line) {
            events.push((
                assignment.offset,
                Event::Assign {
                    name: assignment.name,
                    empty: assignment.empty,
                },
            ));
        }
        for (name, span) in shell::unguarded_array_expansions(line) {
            events.push((span.start, Event::Expand { name, span }));
        }
        events.sort_by_key(|(offset, _)| *offset);
        for (_, event) in events {
            match event {
                Event::Assign { name, empty } => {
                    if empty {
                        self.known_empty.insert(name);
                    } else {
                        self.known_empty.remove(&name);
                    }
                }
                Event::Expand { name, span } => {
                    if suppressed || !self.known_empty.contains(&name) {
                        continue;
                    }
                    let evidence = line.get(span.start..span.end).unwrap_or(name.as_str());
                    diag.report_at_with(
                        LintRule::Bash32Incompatible,
                        path,
                        &format!(
                            "{}:{line_number}: Bash 3.2 aborts under 'set -u' on the unguarded empty-array expansion {evidence}",
                            path.display()
                        ),
                        DiagnosticMetadata::at_line(line_number)
                            .with_evidence(evidence)
                            .with_suggestion(
                                "guard the expansion, e.g. ${arr[@]+\"${arr[@]}\"}, or seed the array before use",
                            ),
                    );
                }
            }
        }
    }
}

struct HeredocState {
    delimiter: String,
    quoted: bool,
}

fn heredoc_state(line: &str) -> Option<HeredocState> {
    let captures = HEREDOC.captures(line)?;
    let quoted = captures.get(1).is_some() || captures.get(2).is_some();
    let delimiter = captures
        .get(1)
        .or_else(|| captures.get(2))
        .or_else(|| captures.get(3))?
        .as_str()
        .to_string();
    Some(HeredocState { delimiter, quoted })
}

fn closes_heredoc(line: &str, delimiter: &str) -> bool {
    line.trim() == delimiter
}

/// A multi-line inline awk program (`awk 'BEGIN {` ... `}'`) accumulated across
/// raw lines until its single quote closes.
struct AwkInline {
    command_line_number: usize,
    text: String,
}

/// An awk program supplied through a `-f -`/stdin heredoc, accumulated until the
/// heredoc delimiter closes.
struct AwkHeredoc {
    command_line: String,
    command_line_number: usize,
    body: String,
    body_base_line: usize,
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
        let lines: Vec<&str> = content.lines().collect();
        // G008 (owned separately) inspects the whole shell script once.
        if kind == ScriptKind::Shell && g008_shell_script(&path) {
            validate_gh_inline(&path, &content, diag);
        }

        // A standalone awk file is one complete program; analyze it whole so a
        // regex literal is attributed to its own line and the awk lexer sees the
        // full multi-line context.
        if kind == ScriptKind::Awk {
            if scope.portability {
                emit_awk(&path, 0, 1, "", &content, diag, &lines);
            }
            continue;
        }
        if kind == ScriptKind::Other {
            continue;
        }

        let gates = script_gates(&path, kind, &content);
        let mut heredoc: Option<HeredocState> = None;
        let mut awk_heredoc: Option<AwkHeredoc> = None;
        let mut awk_inline: Option<AwkInline> = None;
        let mut continuation = String::new();
        let mut empty = EmptyArrayTracker::default();
        for (index, raw) in lines.iter().enumerate() {
            let continued = raw.trim_end().ends_with('\\');
            if continued {
                if !continuation.is_empty() {
                    continuation.push(' ');
                }
                continuation.push_str(raw.trim_end().trim_end_matches('\\'));
                continue;
            }
            let line = if continuation.is_empty() {
                (*raw).to_string()
            } else {
                continuation.push(' ');
                continuation.push_str(raw);
                std::mem::take(&mut continuation)
            };
            let line_number = index + 1;

            // Accumulate a multi-line inline awk program until its quote closes.
            if let Some(accum) = &mut awk_inline {
                accum.text.push('\n');
                accum.text.push_str(&line);
                if !shell::continues_single_quote(&accum.text) {
                    let accum = awk_inline.take().expect("just matched Some");
                    if scope.portability {
                        analyze_awk_text(
                            &path,
                            accum.command_line_number,
                            &accum.text,
                            diag,
                            &lines,
                        );
                    }
                }
                continue;
            }

            if let Some(active) = &heredoc {
                if closes_heredoc(&line, &active.delimiter) {
                    if let Some(accum) = awk_heredoc.take() {
                        if scope.portability {
                            emit_awk(
                                &path,
                                accum.command_line_number,
                                accum.body_base_line,
                                &accum.command_line,
                                &accum.body,
                                diag,
                                &lines,
                            );
                        }
                    }
                    heredoc = None;
                    continue;
                }
                if let Some(accum) = &mut awk_heredoc {
                    if !accum.body.is_empty() {
                        accum.body.push('\n');
                    }
                    accum.body.push_str(&line);
                } else if scope.portability && !active.quoted {
                    // An unquoted shell heredoc still performs parameter
                    // expansion, so the G009 replacement hazard applies.
                    validate_bash_replacement(&path, line_number, &line, diag, &lines);
                }
                continue;
            }

            if line.trim_start().starts_with('#') {
                continue;
            }

            if scope.portability {
                validate_bash_replacement(&path, line_number, &line, diag, &lines);
                validate_bash4_constructs(&path, line_number, &line, diag, &lines);
                if gates.errexit {
                    validate_command_condition(&path, line_number, &line, diag, &lines);
                }
                if gates.nounset {
                    let suppressed = line_waived(&lines, line_number, "lint-bash32: ok");
                    empty.scan_line(&path, line_number, &line, suppressed, diag);
                }
            }

            // Detect awk invocations and heredocs opened on this line.
            let is_awk = AWK_COMMAND.is_match(&line);
            let inline = is_awk.then(|| shell::inline_awk_program(&line)).flatten();
            let opened = heredoc_state(&line);
            if inline.is_some() {
                if shell::continues_single_quote(&line) && opened.is_none() {
                    awk_inline = Some(AwkInline {
                        command_line_number: line_number,
                        text: line.clone(),
                    });
                } else if scope.portability {
                    analyze_awk_text(&path, line_number, &line, diag, &lines);
                }
            }
            if let Some(state) = opened {
                // A heredoc is the awk program only when awk reads its program
                // from stdin (`-f -`); otherwise the heredoc is input data.
                if is_awk && shell::awk_program_from_stdin(&line) {
                    awk_heredoc = Some(AwkHeredoc {
                        command_line: line.clone(),
                        command_line_number: line_number,
                        body: String::new(),
                        body_base_line: line_number + 1,
                    });
                }
                heredoc = Some(state);
            }
        }
    }
}

fn line_waived(lines: &[&str], line_number: usize, marker: &str) -> bool {
    let index = line_number.saturating_sub(1);
    let current = lines.get(index).copied().unwrap_or("");
    let previous = index
        .checked_sub(1)
        .and_then(|i| lines.get(i).copied())
        .unwrap_or("");
    shell::reasoned_comment_marker(current, previous, marker)
}

fn validate_bash_replacement(
    path: &Path,
    line_number: usize,
    line: &str,
    diag: &mut DiagnosticCollector,
    lines: &[&str],
) {
    if line_waived(lines, line_number, "lint-renderer-safe: ok") {
        return;
    }
    for span in shell::hazardous_replacements(line) {
        let evidence = &line[span.start..span.end];
        diag.report_at_with(
            LintRule::BashReplacementUnsafe,
            path,
            &format!(
                "{}:{line_number}: unsafe Bash pattern-substitution replacement can reinterpret '&' as the match",
                path.display()
            ),
            DiagnosticMetadata::at_line(line_number)
                .with_evidence(evidence)
                .with_suggestion(
                    "quote the replacement inside the expansion (${var//pat/\"$rep\"}) or escape '&'",
                ),
        );
    }
}

fn validate_bash4_constructs(
    path: &Path,
    line_number: usize,
    line: &str,
    diag: &mut DiagnosticCollector,
    lines: &[&str],
) {
    if line_waived(lines, line_number, "lint-bash32: ok") {
        return;
    }
    for construct in shell::bash4_constructs(line) {
        let evidence = line
            .get(construct.span.start..construct.span.end)
            .unwrap_or(line);
        diag.report_at_with(
            LintRule::Bash32Incompatible,
            path,
            &format!(
                "{}:{line_number}: Bash 3.2 incompatible {}",
                path.display(),
                construct.label
            ),
            DiagnosticMetadata::at_line(line_number)
                .with_evidence(evidence)
                .with_suggestion("use a construct available in macOS Bash 3.2"),
        );
    }
}

fn validate_command_condition(
    path: &Path,
    line_number: usize,
    line: &str,
    diag: &mut DiagnosticCollector,
    lines: &[&str],
) {
    if line_waived(lines, line_number, "lint-bash32: ok") {
        return;
    }
    for span in shell::command_conditions(line) {
        let evidence = line.get(span.start..span.end).unwrap_or(line);
        diag.report_at_with(
            LintRule::Bash32Incompatible,
            path,
            &format!(
                "{}:{line_number}: Bash 3.2 aborts under 'set -e' when a 'command <cmd>' condition fails",
                path.display()
            ),
            DiagnosticMetadata::at_line(line_number)
                .with_evidence(evidence)
                .with_suggestion("wrap the probe in a subshell: if ( command <cmd> ); then"),
        );
    }
}

/// Analyze a single complete awk invocation whose program is inline on
/// `command_line` (a logical line or a joined multi-line accumulation).
fn analyze_awk_text(
    path: &Path,
    command_line_number: usize,
    command_line: &str,
    diag: &mut DiagnosticCollector,
    lines: &[&str],
) {
    let Some((program, content_offset)) = shell::inline_awk_program(command_line) else {
        return;
    };
    let base_line = command_line_number
        + command_line[..content_offset]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count();
    emit_awk(
        path,
        command_line_number,
        base_line,
        command_line,
        &program,
        diag,
        lines,
    );
}

/// Emit G011 findings for one analyzed awk invocation. Option-supplied operands
/// (`-F`, `-v FS=`, or a `-v` value used as a regex) are reported at the command
/// line; program-body operands are reported at their own line.
fn emit_awk(
    path: &Path,
    command_line_number: usize,
    program_base_line: usize,
    command_line: &str,
    program: &str,
    diag: &mut DiagnosticCollector,
    lines: &[&str],
) {
    let marker = "lint-awk-multibyte-regex: ok";
    let analysis = shell::analyze_awk(command_line, program);
    let command_waived =
        command_line_number != 0 && line_waived(lines, command_line_number, marker);
    for evidence in analysis.option_evidence {
        if command_waived {
            continue;
        }
        emit_awk_finding(path, command_line_number, &evidence, diag);
    }
    for (offset, evidence) in analysis.program_findings {
        let report_line = program_base_line + offset;
        if command_waived || line_waived(lines, report_line, marker) {
            continue;
        }
        emit_awk_finding(path, report_line, &evidence, diag);
    }
}

fn emit_awk_finding(
    path: &Path,
    line_number: usize,
    evidence: &str,
    diag: &mut DiagnosticCollector,
) {
    diag.report_at_with(
        LintRule::AwkRegexNonascii,
        path,
        &format!(
            "{}:{line_number}: non-ASCII awk regex operand is locale-dependent and not portable",
            path.display()
        ),
        DiagnosticMetadata::at_line(line_number)
            .with_evidence(evidence)
            .with_suggestion(
                "use an ASCII regex or a byte-oriented match; keep display text separate",
            ),
    );
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
            .then(|| heredoc_state(line).map(|state| state.delimiter))
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
    fn skill_invocation_rule_accepts_tokenized_tools_and_only_operative_live_prose() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all(".claude/skills/demo").unwrap();

        let run_case = |allowed_tools: &str, body: &str| {
            fs::write(
                ".claude/skills/demo/SKILL.md",
                format!(
                    "---\nname: demo\ndescription: Use when validating Skill invocation contracts\n{allowed_tools}\n---\n{body}\n"
                ),
            )
            .unwrap();
            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_skill_contracts(&mut diag, &ExcludeSet::default(), false);
            diag.diagnostics()
                .iter()
                .filter(|item| item.rule == LintRule::SkillInvokeMissing)
                .map(|item| {
                    (
                        item.message.clone(),
                        item.location.map(|span| span.start().line_number()),
                        item.suggestion.clone(),
                    )
                })
                .collect::<Vec<_>>()
        };

        for allowed_tools in [
            "allowed-tools: Skill Bash",
            "allowed-tools: Skill(child), Bash",
            "allowed-tools: [\"Skill\", \"Bash\"] # canonical flow list",
            "allowed-tools:\n  - \"Skill\" # delegation is permitted\n  - Bash",
            "allowed-tools: >-\n  Skill Bash",
            "allowed-tools: |-\n  Skill Bash",
        ] {
            assert_eq!(
                run_case(allowed_tools, "Describe the child workflow."),
                vec![(
                    ".claude/skills/demo/SKILL.md: allowed-tools includes Skill but the body has no explicit Skill tool invocation step".to_string(),
                    None,
                    Some(S058_MISSING_STEP_SUGGESTION.to_string()),
                )],
                "{allowed_tools} must engage the Skill gate"
            );
            assert!(
                run_case(allowed_tools, "Use the Skill tool to invoke the child.").is_empty(),
                "{allowed_tools} must accept an operative clear step"
            );
        }

        for body in [
            "Launch each child skill with the Skill tool.",
            "For each child, invoke the Skill tool with its name.",
            "Invoke the **Skill** tool with the child name.",
            "Use the Skill tool to invoke the child.",
            "Invoke `/child` with the Skill tool.",
        ] {
            assert!(
                run_case("allowed-tools: Skill, Bash", body).is_empty(),
                "{body:?} must be a clear Skill-tool invocation"
            );
        }

        for body in [
            "Do not Invoke the Skill tool under any circumstance.",
            "The Skill tool is available.",
            "> Invoke the Skill tool for the child.",
            "<!-- Invoke the Skill tool for the child. -->",
            "```text\nInvoke the Skill tool for the child.\n```",
        ] {
            let findings = run_case("allowed-tools: Skill", body);
            assert_eq!(findings.len(), 1, "{body:?} must not satisfy the gate");
            assert_eq!(findings[0].1, None);
            assert_eq!(
                findings[0].2.as_deref(),
                Some(S058_MISSING_STEP_SUGGESTION),
                "{body:?} must retain the file-level fixed suggestion"
            );
        }

        for allowed_tools in [
            "allowed-tools: Read, Write",
            "allowed-tools: SkillFoo",
            "allowed-tools: { tool: Skill }",
            "allowed-tools: Skill\nbad: [unclosed",
        ] {
            assert!(
                run_case(allowed_tools, "INVOKE `/child` directly.").is_empty(),
                "{allowed_tools} must leave the S058 gate closed"
            );
        }

        fs::write(
            ".claude/skills/demo/SKILL.md",
            "---\n- allowed-tools: Skill\n---\nINVOKE `/child` directly.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_contracts(&mut diag, &ExcludeSet::default(), false);
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::SkillInvokeMissing),
            "non-mapping frontmatter must leave the S058 gate closed"
        );

        for body in [
            "Use the Skill tool to invoke the child.\n> INVOKE `/child` directly.",
            "Use the Skill tool to invoke the child.\n<!-- INVOKE `/child` directly. -->",
            "Use the Skill tool to invoke the child.\n\"INVOKE `/child` directly.\"",
        ] {
            assert!(
                run_case("allowed-tools: Skill", body).is_empty(),
                "{body:?} must not revive an invocation from excluded prose"
            );
        }

        let findings = run_case(
            "allowed-tools: Skill",
            "Use the Skill tool to invoke the child.\nINVOKE `/child` directly.",
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].1, Some(7));
        assert_eq!(
            findings[0].2.as_deref(),
            Some(S058_AMBIGUOUS_INVOKE_SUGGESTION)
        );
        assert!(findings[0].0.contains(":7: ambiguous skill invocation"));
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

    fn s060_findings(command: &str) -> Vec<crate::diagnostic::Diagnostic> {
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_awk_fields(Path::new("skills/demo/SKILL.md"), 7, command, &mut diag);
        diag.diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::AwkFieldRef)
            .cloned()
            .collect()
    }

    fn s061_findings(command: &str) -> Vec<crate::diagnostic::Diagnostic> {
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_grep_probe(Path::new("skills/demo/SKILL.md"), 7, command, &mut diag);
        diag.diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::UnsafeGrepProbe)
            .cloned()
            .collect()
    }

    fn assert_s060(command: &str) {
        let findings = s060_findings(command);
        assert_eq!(findings.len(), 1, "expected S060 for {command:?}");
        assert_eq!(findings[0].location, Some(SourceSpan::line(7)));
        assert_eq!(
            findings[0].suggestion.as_deref(),
            Some("move the awk parsing into a shipped script")
        );
        assert!(findings[0].message.contains("bare awk positional field"));
    }

    fn assert_no_s060(command: &str) {
        assert!(
            s060_findings(command).is_empty(),
            "unexpected S060 for {command:?}"
        );
    }

    fn assert_s061(command: &str, message_fragment: &str, suggestion: &str) {
        let findings = s061_findings(command);
        assert_eq!(findings.len(), 1, "expected S061 for {command:?}");
        assert_eq!(findings[0].location, Some(SourceSpan::line(7)));
        assert!(
            findings[0].message.contains(message_fragment),
            "message {:?} missing {message_fragment:?} for {command:?}",
            findings[0].message
        );
        assert_eq!(findings[0].suggestion.as_deref(), Some(suggestion));
    }

    fn assert_no_s061(command: &str) {
        assert!(
            s061_findings(command).is_empty(),
            "unexpected S061 for {command:?}: {:?}",
            s061_findings(command)
                .iter()
                .map(|item| &item.message)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn awk_field_ref_covers_substitution_variants_and_families() {
        assert_s060("first=$(awk '{print $1}' data.txt)");
        assert_s060("first=`awk '{print $1}' f`");
        assert_s060("gawk '{print $1}' data.txt");
        assert_s060("awk -W posix '{print $1}' data.txt");
        assert_s060("awk -Wposix '{print $1}' data.txt");
        assert_s060("/usr/bin/awk '{print $1}' data.txt");
        assert_s060("(awk '{print $1}' data.txt)");

        assert_no_s060("first=$(awk 'BEGIN { print v }' f)");
        assert_no_s060("echo $1");
        assert_no_s060("notawk '{print $1}' data.txt");
        assert_no_s060(r#"cmd="$awk"; $cmd '{print $1}' data.txt"#);
    }

    #[test]
    fn grep_probe_arg_feeders_and_wrappers() {
        assert_no_s061("git ls-files | xargs grep -l pattern");
        assert_no_s061("git ls-files | parallel grep pat");
        assert_s061(
            "timeout 5 grep pat",
            "grep-family probe has no explicit path and may block on stdin",
            "add an explicit search path or pipe/redirect input",
        );
        assert_s061(
            "xargs grep -l pat ../up",
            "grep-family path ascends through a parent directory",
            "use a repository-contained path",
        );
    }

    #[test]
    fn grep_probe_option_values_are_not_paths() {
        assert_no_s061("command grep -e '../escape' log.txt");
        assert_no_s061("command grep --regexp=../x f");
        assert_s061(
            "grep needle ../shared/config",
            "grep-family path ascends through a parent directory",
            "use a repository-contained path",
        );
    }

    #[test]
    fn grep_probe_redirection_aware_path_and_stdin() {
        assert_s061(
            "rg pattern > out.txt",
            "grep-family probe has no explicit path and may block on stdin",
            "add an explicit search path or pipe/redirect input",
        );
        assert_s061(
            "rg needle 2>&1",
            "grep-family probe has no explicit path and may block on stdin",
            "add an explicit search path or pipe/redirect input",
        );
        assert_no_s061("command grep pat < input.txt");
        assert_no_s061("command grep pat </dev/null");
        assert_no_s061("command grep pat file > out");
        // Output redirections do not feed stdin; bare `grep` still uses the bare arm.
        assert_s061(
            "grep pat file > out",
            "bare top-level grep in a shell fence",
            "prefix top-level grep with command or feed it through a pipe",
        );
    }

    #[test]
    fn grep_probe_recognizes_path_and_command_position_forms() {
        assert_s061(
            "matches=$(rg needle)",
            "grep-family probe has no explicit path and may block on stdin",
            "add an explicit search path or pipe/redirect input",
        );
        assert_s061(
            "(rg needle)",
            "grep-family probe has no explicit path and may block on stdin",
            "add an explicit search path or pipe/redirect input",
        );
        assert_s061(
            "/usr/bin/grep needle",
            "grep-family probe has no explicit path and may block on stdin",
            "add an explicit search path or pipe/redirect input",
        );
        assert_no_s061("/usr/bin/grep needle file.txt");
        assert_no_s061("mygrep needle");
        assert_no_s061(r#"cmd=grep; $cmd needle"#);
        assert_s061(
            "grep needle",
            "bare top-level grep in a shell fence",
            "prefix top-level grep with command or feed it through a pipe",
        );
    }

    #[test]
    fn awk_and_grep_waivers_require_reasons() {
        assert_no_s060(
            "awk '{print $1}' data.txt # lint-skill-awk-field-ref: ok reviewed exception",
        );
        assert_s060("awk '{print $1}' data.txt # lint-skill-awk-field-ref: ok");
        assert_no_s061("grep needle # lint-bare-grep-probe: ok reviewed exception");
        assert_s061(
            "grep needle # lint-bare-grep-probe: ok",
            "bare top-level grep in a shell fence",
            "prefix top-level grep with command or feed it through a pipe",
        );
        // Each marker suppresses only its own rule.
        assert_s061(
            "grep needle # lint-skill-awk-field-ref: ok reviewed",
            "bare top-level grep in a shell fence",
            "prefix top-level grep with command or feed it through a pipe",
        );
        assert_s060("awk '{print $1}' f # lint-bare-grep-probe: ok reviewed");
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
        assert_eq!(
            awk_programs("awk -W posix '{print $1}' data.txt"),
            ["{print $1}"]
        );
        assert_eq!(
            awk_programs("awk -Wposix '{print $1}' data.txt"),
            ["{print $1}"]
        );
        assert_eq!(
            awk_programs("first=$(awk '{print $1}' data.txt)"),
            ["{print $1}"]
        );
        assert_eq!(
            awk_programs("notawk '{print $1}' data.txt"),
            Vec::<String>::new()
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

    fn bash32_lines(content: &str) -> Vec<String> {
        run_script_contracts(content, "scripts/fixture.inc.bash")
            .into_iter()
            .filter(|item| item.rule == LintRule::Bash32Incompatible)
            .map(|item| item.message.clone())
            .collect()
    }

    fn run_script_contracts(content: &str, path: &str) -> Vec<crate::diagnostic::Diagnostic> {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        if let Some(parent) = Path::new(path).parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_script_contracts(&mut diag, &ExcludeSet::default(), true);
        diag.diagnostics().to_vec()
    }

    #[test]
    #[serial_test::serial]
    fn bash32_matrix_matches_bash32_syntax_and_ignores_supported_and_inert_forms() {
        // The unavailable-syntax/builtin/option matrix, probe-verified against
        // GNU Bash 3.2.57. `.inc.bash` enables both the errexit and nounset
        // gates because a sourced library inherits its caller's options.
        let content = r#"# shellcheck shell=bash
# declare -A documentation is ignored
declare -A seen=()
typeset -x -A legacy=()
declare -g GLOBAL=1
mapfile -t rows < input
readarray -t more < input
printf '%s' "${NAME^^}" "${NAME^}" "${NAME,,}" "${NAME,}"
declare -n ref=target
local -x -n inner=target
cmd &>>log
coproc WORKER { cat; }
arr=(a b); printf '%s' "${arr[-1]}"
printf '%s' {1..10..2}
case x in x) : ;& esac
case y in y) : ;;& esac
echo hi |& cat
shopt -s globstar
wait -n
if command grep -q needle file; then :; fi
if ( command grep -q needle file ); then :; fi
if command -v tool >/dev/null; then :; fi
safe="${MYVAR//[^A-Za-z0-9_-]/_}"
printf '%s\n' "a;; b" || cmd1 && cmd2
declare -A reviewed=() # lint-bash32: ok intentional compatibility shim
declare -A no_reason=() # lint-bash32: ok
"#;
        let findings = bash32_lines(content);
        for label in [
            "declare -A associative arrays",
            "typeset -A associative arrays",
            "declare -g global variable",
            "mapfile/readarray",
            "parameter case conversion",
            "declare -n nameref",
            "local -n nameref",
            "&>> append-all redirection",
            "coproc",
            "negative array index",
            "stepped brace expansion",
            ";& case fallthrough",
            ";;& case fallthrough",
            "|& pipe shorthand",
            "shopt -s globstar",
            "wait -n",
        ] {
            assert!(
                findings.iter().any(|message| message.contains(label)),
                "missing fixture for {label}: {findings:?}"
            );
        }
        // The `if command <cmd>` errexit hazard fires exactly once (line 20) and
        // is relabeled — it is no longer described as unavailable syntax.
        let command_conditions: Vec<_> = findings
            .iter()
            .filter(|message| message.contains("'command <cmd>' condition"))
            .collect();
        assert_eq!(command_conditions.len(), 1, "{findings:?}");
        assert!(command_conditions[0].contains(":20:"));
        // Supported Bash 3.2 forms and inert text stay clean.
        for clean in [":2:", ":21:", ":22:", ":23:", ":24:"] {
            assert!(
                !findings.iter().any(|message| message.contains(clean)),
                "unexpected finding on {clean}: {findings:?}"
            );
        }
        // A reasoned waiver silences its own construct; a reasonless one does not.
        assert!(!findings.iter().any(|message| message.contains(":25:")));
        assert!(findings.iter().any(|message| message.contains(":26:")));
    }

    #[test]
    #[serial_test::serial]
    fn bash32_command_condition_is_errexit_gated() {
        // Differential pair: the same `if command grep` construct fires only
        // when the file lexically enables errexit.
        let with_errexit =
            "#!/usr/bin/env bash\nset -e\nif command grep -q x /etc/hosts; then :; fi\n";
        assert!(
            run_script_contracts(with_errexit, "scripts/errexit.sh")
                .iter()
                .any(|item| item.rule == LintRule::Bash32Incompatible
                    && item.message.contains("'command <cmd>' condition"))
        );
        let no_errexit = "#!/usr/bin/env bash\nif command grep -q x /etc/hosts; then :; fi\n";
        assert!(
            !run_script_contracts(no_errexit, "scripts/plain.sh")
                .iter()
                .any(|item| item.rule == LintRule::Bash32Incompatible)
        );
    }

    #[test]
    #[serial_test::serial]
    fn bash32_empty_array_is_nounset_gated_and_conservative() {
        // Under `set -u`, only a provably empty, unguarded, straight-line
        // expansion fires; guards, reassignment, and control flow stay clean.
        let gated = r#"#!/usr/bin/env bash
set -u
items=()
printf '%s\n' "${items[@]}"
seeded=(one)
printf '%s\n' "${seeded[@]}"
guarded=()
if [ -e marker ]; then :; fi
printf '%s\n' "${guarded[@]}"
refilled=()
refilled+=(x)
printf '%s\n' "${refilled[@]}"
printf '%s\n' ${maybe[@]+"${maybe[@]}"}
suppressed=()
printf '%s\n' "${suppressed[@]}" # lint-bash32: ok deliberately optional
"#;
        let findings: Vec<_> = run_script_contracts(gated, "scripts/nounset.sh")
            .into_iter()
            .filter(|item| item.rule == LintRule::Bash32Incompatible)
            .map(|item| item.message)
            .collect();
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(findings[0].contains(":4:"), "{findings:?}");

        // Without nounset the identical straight-line hazard stays clean.
        let ungated = "#!/usr/bin/env bash\nitems=()\nprintf '%s\\n' \"${items[@]}\"\n";
        assert!(
            !run_script_contracts(ungated, "scripts/no-nounset.sh")
                .iter()
                .any(|item| item.rule == LintRule::Bash32Incompatible)
        );
    }

    #[test]
    #[serial_test::serial]
    fn awk_reports_regex_operands_not_display_text_or_ascii_regexes() {
        let content = r#"# awk -v label='テスト' 'BEGIN { print label }'
awk -v label='テスト' 'BEGIN { print label }'
awk 'BEGIN { printf "テスト\n" }'
awk -v re='テスト' '$0 ~ re'
awk -F '—' '{ print $1 }'
awk -F ',' '{ print $1 }'
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
  if ($0 ~ /ASCII/) print "テスト"
}' | cat
awk -v reviewed='テスト' '$0 ~ reviewed' # lint-awk-multibyte-regex: ok reviewed shim
awk -v shown='テスト' 'BEGIN { print shown }'
"#;
        let findings: Vec<_> = run_script_contracts(content, "scripts/awk.sh")
            .into_iter()
            .filter(|item| item.rule == LintRule::AwkRegexNonascii)
            .map(|item| item.message)
            .collect();
        for line in [4, 5, 11, 14, 15, 16, 17] {
            assert!(
                findings
                    .iter()
                    .any(|message| message.contains(&format!(":{line}:"))),
                "missing awk finding on line {line}: {findings:?}"
            );
        }
        for line in [1, 2, 3, 6, 8, 18, 20, 21] {
            assert!(
                !findings
                    .iter()
                    .any(|message| message.contains(&format!(":{line}:"))),
                "unexpected awk finding on line {line}: {findings:?}"
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn awk_field_separator_operand_fires_in_standalone_awk_files() {
        let content = "BEGIN { FS = \"—\" }\n/ASCII/ { print }\ngsub(\"—\", \"-\")\n";
        let findings: Vec<_> = run_script_contracts(content, "scripts/rules.awk")
            .into_iter()
            .filter(|item| item.rule == LintRule::AwkRegexNonascii)
            .map(|item| item.message)
            .collect();
        // FS assignment (line 1) and gsub operand (line 3) fire; the ASCII
        // pattern on line 2 stays clean.
        assert!(findings.iter().any(|message| message.contains(":1:")));
        assert!(findings.iter().any(|message| message.contains(":3:")));
        assert!(!findings.iter().any(|message| message.contains(":2:")));
    }

    #[test]
    #[serial_test::serial]
    fn portability_diagnostics_carry_structured_locations_and_evidence() {
        let content = "#!/usr/bin/env bash\nout=${text//TOKEN/$rep}\ndeclare -A m\n";
        let diagnostics = run_script_contracts(content, "scripts/meta.sh");
        let replacement = diagnostics
            .iter()
            .find(|item| item.rule == LintRule::BashReplacementUnsafe)
            .expect("G009 fires");
        assert!(replacement.location.is_some(), "G009 has a source location");
        assert!(replacement.evidence.is_some(), "G009 carries evidence");
        assert!(
            replacement.suggestion.is_some(),
            "G009 carries a suggestion"
        );
        let bash32 = diagnostics
            .iter()
            .find(|item| item.rule == LintRule::Bash32Incompatible)
            .expect("G010 fires");
        assert!(bash32.location.is_some());
        assert!(bash32.evidence.is_some());
    }

    #[test]
    #[serial_test::serial]
    fn empty_array_does_not_fire_on_quoted_or_conditional_initializers() {
        // Reviewer #1: a quoted-literal initializer is non-empty; reviewer #4: a
        // `&&`-guarded reset and an intra-line reset-after-use are ambiguous.
        let content = r#"# shellcheck shell=bash
literal=("one")
printf '%s\n' "${literal[@]}"
guarded=(a b)
[[ -n "$FOO" ]] && guarded=()
printf '%s\n' "${guarded[@]}"
reset=(a b); printf '%s\n' "${reset[@]}"; reset=()
"#;
        assert!(
            !run_script_contracts(content, "scripts/arrays.inc.bash")
                .iter()
                .any(|item| item.rule == LintRule::Bash32Incompatible),
            "no empty-array false positive"
        );
    }

    #[test]
    #[serial_test::serial]
    fn stray_apostrophe_and_string_hash_do_not_disable_lint() {
        // Reviewer #2: an apostrophe in a trailing comment must not swallow the
        // following line. Reviewer #3: a `#` inside a string must not waive.
        let content = r##"# shellcheck shell=bash
awk '{ print }'  # don't touch this line
declare -A swallowed
notice="# lint-bash32: ok this is not a real waiver"; declare -A m
"##;
        let findings: Vec<_> = run_script_contracts(content, "scripts/tricky.inc.bash")
            .into_iter()
            .filter(|item| item.rule == LintRule::Bash32Incompatible)
            .map(|item| item.message)
            .collect();
        assert!(
            findings.iter().any(|m| m.contains(":3:")),
            "line 3 not swallowed: {findings:?}"
        );
        assert!(
            findings.iter().any(|m| m.contains(":4:")),
            "string # did not waive: {findings:?}"
        );
    }

    #[test]
    #[serial_test::serial]
    fn awk_data_heredoc_is_not_analyzed_as_a_program() {
        // Reviewer #5: `-f file` reads the program from a file; the heredoc is
        // input data and must not be analyzed as awk source.
        let content = "awk -f transform.awk <<DATA\nrow with — dash\nDATA\n";
        assert!(
            !run_script_contracts(content, "scripts/data.sh")
                .iter()
                .any(|item| item.rule == LintRule::AwkRegexNonascii),
            "heredoc data must not be read as an awk program"
        );
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
             Image md ![architecture](docs/missing-image.md) ok.\n\
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

    #[test]
    #[serial_test::serial]
    fn l005_skips_percent_encoded_external_destinations() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::write(
            "CLAUDE.md",
            "Encoded [ex](http%3A%2F%2Fexample.com/page.md) ok.\n\
             Proto [cdn](%2F%2Fcdn.example/x.md) ok.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_markdown_links(&mut diag, &ExcludeSet::default());
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::BrokenMarkdownLink)
        );
    }

    #[test]
    #[serial_test::serial]
    fn l005_resolves_source_relative_without_root_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all("nested").unwrap();
        fs::write("present.md", "root\n").unwrap();
        fs::write("nested/CLAUDE.md", "See [details](present.md).\n").unwrap();
        let config = LintConfig {
            instruction_files: vec!["nested/CLAUDE.md".into()],
            ..LintConfig::default()
        };
        let mut diag = all_enabled_with(config);
        validate_markdown_links(&mut diag, &ExcludeSet::default());
        let broken: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::BrokenMarkdownLink)
            .collect();
        assert_eq!(broken.len(), 1);
        assert_eq!(broken[0].evidence.as_deref(), Some("present.md"));
        assert!(broken[0].location.is_some());
        assert_eq!(
            broken[0].suggestion.as_deref(),
            Some(SUGGEST_CREATE_OR_CORRECT)
        );
    }
}
