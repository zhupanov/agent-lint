use crate::config::ExcludeSet;
use crate::diagnostic::DiagnosticCollector;
use crate::frontmatter;
use crate::rules::LintRule;
use crate::traversal;
use crate::validators::skills::SkillInfo;
use regex::Regex;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

use super::contains_backslash_path;

const MAX_BODY_LINES: usize = 500;
const BODY_NO_REFS_THRESHOLD: usize = 300;
const BODY_NO_WORKFLOW_THRESHOLD: usize = 300;
const BODY_NO_EXAMPLES_THRESHOLD: usize = 200;

// S037: Explicit repository-relative path tokens with a filename extension.
// The leading boundary prevents a URL tail such as `example.test/config.json`
// from being treated as a repository path.
static RE_BODY_FILE_PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?:^|[^A-Za-z0-9_./:-])(?:\$\{CLAUDE_PLUGIN_ROOT\}/|\./)?(?:[A-Za-z0-9][A-Za-z0-9._-]*/)+[A-Za-z0-9][A-Za-z0-9._-]*\.[A-Za-z0-9]{1,16}(?:$|[^A-Za-z0-9_-])"#,
    )
    .unwrap()
});

// S037: Reference directories may contain extensionless supporting files.
static RE_BODY_REFERENCE_DIRECTORY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?:^|[^A-Za-z0-9_./:-])(?:\$\{CLAUDE_PLUGIN_ROOT\}/)?(?:scripts|shared|references|assets|examples|templates)/[^\s/]+"#,
    )
    .unwrap()
});

// S037: Bare Markdown filenames remain useful references to the canonical
// split target. Other bare extensions are intentionally not accepted.
static RE_BARE_MARKDOWN_FILE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:^|[^A-Za-z0-9_./:-])[A-Za-z0-9][A-Za-z0-9._-]*\.md\b"#).unwrap()
});

// S038: Time-sensitive
static RE_YEAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b20[2-3][0-9]\b").unwrap());

// S041: Fork-no-task
static RE_IMPERATIVE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(run|execute|create|build|generate|invoke|call|launch|start|perform|apply|install|deploy|write|implement|analyze|audit|check|collect|compare|compile|convert|describe|diagnose|document|evaluate|examine|explain|extract|find|fix|format|gather|identify|inspect|lint|list|locate|measure|merge|output|parse|produce|read|refactor|rename|replace|report|research|resolve|return|review|scan|search|summarize|test|update|validate|verify)\b",
    )
    .unwrap()
});

// S046: Workflow structure
static RE_WORKFLOW_STRUCTURE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?m)^\s*(?:\*\*Step \d+|#{2,3} Step\b|- \[[ xX]\]|#{2,3} (?:Workflow|Process|Steps)\b)",
    )
    .unwrap()
});

// S046: Numbered list items (counted separately — need 3+ contiguous)
// Accepts both `1.` and `1)` CommonMark ordered-list markers.
static RE_NUMBERED_LIST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*\d+[.)]\s").unwrap());

// S047: Example patterns (singular and plural heading/marker forms)
static RE_EXAMPLE_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*(?:#{2,3} (?:Examples?|Usage|Templates?|Formats?)\b|\*\*(?:Examples?|Inputs?|Outputs?)(?:\s*\d*)?:\*\*)").unwrap()
});

// S051/S052: Script file reference (narrower than RE_BODY_FILE_REF — excludes .md, shared/, ${CLAUDE_PLUGIN_ROOT})
static RE_SCRIPT_FILE_REF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\.sh\b|\.py\b|\.js\b|\.ts\b|scripts/").unwrap());

// S051: Dependency keywords (case-insensitive)
static RE_DEPS_KEYWORDS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:pip3?\s+install|npm\s+install|brew\s+install|apt\s+install|cargo\s+install|\brequires\b|\bdependencies\b|\bprerequisite\b|\binstall\b|requirements\.txt|package\.json|Cargo\.toml|(?m)^#{2,3}\s+(?:Dependencies|Requirements|Prerequisites|Setup)\b)").unwrap()
});

// S052: Verification keywords (case-insensitive)
static RE_VERIFY_KEYWORDS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:\bverify\b|\bvalidate\b|\bcheck\b|\btest\b|\bconfirm\b|if\s+.*\bfails\b|if\s+.*\berrors\b|validation\s+passes|run\s+.*\bagain\b|\brepeat\b|\bre-?run\b|(?m)^#{2,3}\s+(?:Verify|Validation|Testing)\b)").unwrap()
});

// S053: Synonym groups for terminology consistency
// Each entry: (group label, &[single-token lowercase members])
#[rustfmt::skip]
const SYNONYM_GROUPS: &[(&str, &[&str])] = &[
    ("endpoint/route/URL",             &["endpoint", "route", "url"]),
    ("field/element/control",          &["field", "element", "control", "widget"]),
    ("extract/retrieve/fetch",         &["extract", "retrieve", "fetch", "pull"]),
    ("function/method/routine",        &["function", "method", "routine", "procedure"]),
    ("exception/failure/fault",        &["exception", "failure", "fault"]),
    ("configuration/settings/preferences", &["configuration", "settings", "preferences"]),
    ("execute/invoke/launch",          &["execute", "invoke", "launch"]),
    ("component/module/package",       &["component", "module", "package"]),
];

// S055: Python statement-level `try:` and `except` (both required)
static RE_PY_TRY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^\s*try\s*:").unwrap());
static RE_PY_EXCEPT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^\s*except\b").unwrap());

// S055: Shell error handling patterns (set -e, set -o errexit, trap, || exit/return,
// compound `|| { ...; exit|return; }`, and `if ! cmd` negated-command guards)
static RE_SH_ERROR_HANDLING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?m)(^\s*set\s+-[^\s]*e[^\s]*(\s|$)|^\s*set\s+-o\s+errexit\b|^\s*trap\b|\|\|\s*(exit|return)|\|\|\s*\{[^}]*\b(?:exit|return)\b|^\s*if\s+!\s)",
    )
    .unwrap()
});

const SCRIPT_MIN_LINES: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScriptKind {
    Shell,
    Python,
}

// S056: Or-chain detection — 3+ alternatives via comma-list-with-or or 2+ bare "or" occurrences
static RE_OR_CHAIN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?:\w[\w `'-]*,\s*\w[\w `'-]*(?:,\s*\w[\w `'-]*)*,?\s+or\s+\w|(?:.*\bor\b){2})",
    )
    .unwrap()
});

// S056: Suppress when line has conditional framing or recommendation keywords
static RE_ALTERNATIVES_SUPPRESS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:^\s*(?:if|when)\b|\b(?:prefer|recommend(?:ed)?|by default|default)\b)")
        .unwrap()
});

// S056: A choice must be explicitly framed as selecting alternatives rather
// than merely mentioning alternatives in ordinary prose.
static RE_CHOICE_CUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:use|using|choose|choosing|pick|select|option|options|alternatively|either|tool|tools|library|libraries|approach|approaches|method|methods)\b").unwrap()
});

static RE_MARKDOWN_HEADING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s{0,3}#{1,6}\s+").unwrap());
static RE_MARKDOWN_LIST_ITEM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?:[-+*]|\d+[.)])\s+").unwrap());

// S057: Magic number assignment pattern (identifier = digits)
static RE_MAGIC_ASSIGN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[A-Za-z_][A-Za-z0-9_]*\s*=\s*(\d+)").unwrap());

// S057: Preceding-line comment detection (anchored to line start)
static RE_COMMENT_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?:#|//|--(?:\s|$))").unwrap());

// S057: Well-known values that don't need documentation
const WELL_KNOWN_VALUES: &[u64] = &[
    0, 1, 80, 443, 8080, 8443, 3000, 30, 60, 120, 300, 1024, 2048, 4096,
];

pub(super) fn check_body_content(
    info: &SkillInfo,
    plugin_mode: bool,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
) {
    // S020: empty body
    if info.body.trim().is_empty() {
        diag.report(
            LintRule::BodyEmpty,
            &format!("{}: no content after frontmatter", info.path),
        );
        return; // No point checking other body rules
    }

    // S019: body too long
    let line_count = info.body.lines().count();
    if line_count > MAX_BODY_LINES {
        diag.report(
            LintRule::BodyTooLong,
            &format!(
                "{}: body exceeds 500 lines ({} lines)",
                info.path, line_count
            ),
        );
    }

    // S021: consecutive bash code blocks
    check_consecutive_bash(info, diag);

    // S022: backslash paths. The shared matcher excludes short escape pairs
    // and named TeX command pairs while retaining path-like multi-segment runs.
    // Only check outside code fences to reduce false positives
    for line in crate::fence::lines_outside_fences(&info.body) {
        if contains_backslash_path(line) {
            diag.report(
                LintRule::BackslashPath,
                &format!(
                    "{}: Windows-style backslash path detected; use forward slashes",
                    info.path
                ),
            );
            break; // Report once per file
        }
    }

    // S037: body-no-refs (plugin-only) -- body > 300 lines with no file references
    if plugin_mode
        && line_count > BODY_NO_REFS_THRESHOLD
        && !body_has_file_reference(&info.body, info.document.links())
    {
        diag.report(
            LintRule::BodyNoRefs,
            &format!(
                "{}: body exceeds 300 lines ({}) with no file references; consider splitting into reference files",
                info.path, line_count
            ),
        );
    }

    // S038: time-sensitive (plugin-only) -- date/year patterns in authored prose
    if plugin_mode {
        for line in info.document.body_prose() {
            if RE_YEAR.is_match(&line.text) {
                diag.report(
                    LintRule::TimeSensitive,
                    &format!(
                        "{}: body contains date/year pattern that may become outdated",
                        info.path
                    ),
                );
                break; // Report once per file
            }
        }
    }

    // S041: fork-no-task -- context: fork set but no task instructions in body
    if frontmatter::get_field(&info.fm_lines, "context").as_deref() == Some("fork")
        && !RE_IMPERATIVE.is_match(&info.body)
    {
        diag.report(
            LintRule::ForkNoTask,
            &format!(
                "{}: context: fork is set but body has no task instructions (fork subagent needs an actionable prompt)",
                info.path
            ),
        );
    }

    // S046: body-no-workflow (plugin-only) + S047: body-no-examples (plugin-only)
    // Single iteration through lines_outside_fences() when line_count > 200
    if plugin_mode && line_count > BODY_NO_EXAMPLES_THRESHOLD {
        let check_workflow = line_count > BODY_NO_WORKFLOW_THRESHOLD;
        let mut has_workflow = !check_workflow; // skip if below threshold
        let mut has_examples = false;
        let mut numbered_count: usize = 0;

        for line in crate::fence::lines_outside_fences(&info.body) {
            if !has_workflow {
                if RE_WORKFLOW_STRUCTURE.is_match(line) {
                    has_workflow = true;
                } else if RE_NUMBERED_LIST.is_match(line) {
                    numbered_count += 1;
                    if numbered_count >= 3 {
                        has_workflow = true;
                    }
                } else if !line.trim().is_empty() {
                    // Continuation lines (2+ spaces of indent) do not break
                    // a contiguous numbered sequence; reset only on flush-left
                    // or single-space non-matching non-empty lines.
                    let leading_spaces = line.chars().take_while(|c| *c == ' ').count();
                    if leading_spaces < 2 {
                        numbered_count = 0;
                    }
                }
            }
            if !has_examples && RE_EXAMPLE_PATTERN.is_match(line) {
                has_examples = true;
            }
            if has_workflow && has_examples {
                break;
            }
        }

        if !has_workflow {
            diag.report(
                LintRule::BodyNoWorkflow,
                &format!(
                    "{}: body exceeds {} lines ({}) with no workflow structure (steps, checklists, or numbered sequences)",
                    info.path, BODY_NO_WORKFLOW_THRESHOLD, line_count
                ),
            );
        }
        if !has_examples {
            diag.report(
                LintRule::BodyNoExamples,
                &format!(
                    "{}: body exceeds {} lines ({}) with no examples or templates",
                    info.path, BODY_NO_EXAMPLES_THRESHOLD, line_count
                ),
            );
        }
    }

    // S053: terminology consistency (plugin-only) — authored prose only
    if plugin_mode {
        check_terminology_consistency(info, diag);
    }

    // S051/S052: script-backed skill quality checks (plugin-only)
    // Intentionally scans full body INCLUDING code fences — dependency
    // declarations and verification steps are often in code blocks.
    if plugin_mode && is_script_backed(info) {
        if !RE_DEPS_KEYWORDS.is_match(&info.body) {
            diag.report(
                LintRule::ScriptDepsMissing,
                &format!(
                    "{}: script-backed skill lacks dependency/package documentation",
                    info.path
                ),
            );
        }
        if !RE_VERIFY_KEYWORDS.is_match(&info.body) {
            diag.report(
                LintRule::ScriptVerifyMissing,
                &format!(
                    "{}: script-backed skill lacks verification/validation steps",
                    info.path
                ),
            );
        }

        // S055: check actual script files for error handling patterns
        check_script_error_handling(info, diag, exclude);
    }

    // S056: body-no-default (plugin-only) — alternatives without stated default
    if plugin_mode {
        check_body_no_default(info, diag);
    }

    // S057: magic-number-undoc (plugin-only) — undocumented magic numbers in code blocks
    if plugin_mode {
        check_magic_numbers(info, diag);
    }
}

/// Whether a skill body explicitly directs readers to a supporting file.
///
/// This deliberately recognizes only repository-relative Markdown links and
/// path-shaped prose. The rule measures authored references, so fenced content
/// is included just as it was before S037 used this predicate.
fn body_has_file_reference(body: &str, links: &[crate::markdown::MarkdownLink]) -> bool {
    links
        .iter()
        .any(|link| link_destination_is_repository_relative(&link.destination))
        || RE_BODY_FILE_PATH.is_match(body)
        || RE_BODY_REFERENCE_DIRECTORY.is_match(body)
        || RE_BARE_MARKDOWN_FILE.is_match(body)
}

fn link_destination_is_repository_relative(destination: &str) -> bool {
    let destination = strip_link_query_or_fragment(destination);
    !destination.is_empty() && !destination.starts_with('/') && !has_uri_scheme(destination)
}

fn strip_link_query_or_fragment(destination: &str) -> &str {
    match destination.find(['?', '#']) {
        Some(index) => &destination[..index],
        None => destination,
    }
}

fn has_uri_scheme(value: &str) -> bool {
    let Some((scheme, _)) = value.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    matches!(chars.next(), Some(character) if character.is_ascii_alphabetic())
        && chars.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
        })
}

/// A skill is "script-backed" if it has a non-empty `scripts/` subdirectory
/// or its body references script file extensions (.sh, .py, .js, .ts).
fn is_script_backed(info: &SkillInfo) -> bool {
    info.has_scripts_dir || RE_SCRIPT_FILE_REF.is_match(&info.body)
}

fn check_terminology_consistency(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    // Collect words from the shared authored-prose view into a set.
    let mut words = HashSet::new();
    for line in info.document.body_prose() {
        for token in line
            .text
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
        {
            if !token.is_empty() {
                words.insert(token.to_string());
            }
        }
    }

    for (group_name, members) in SYNONYM_GROUPS {
        let mut found: Vec<&str> = members
            .iter()
            .filter(|m| words.contains(**m))
            .copied()
            .collect();
        if found.len() >= 3 {
            found.sort_unstable();
            diag.report(
                LintRule::TerminologyInconsistent,
                &format!(
                    "{}: uses 3+ variants from the same synonym group ({}): {}; pick one term and use it consistently",
                    info.path,
                    group_name,
                    found.join(", ")
                ),
            );
        }
    }
}

fn check_body_no_default(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    let mut paragraph = Vec::new();
    let mut previous_line = None;
    let source_lines: Vec<_> = info.document.content().lines().collect();
    for line in info.document.body_prose() {
        let source_is_blank = source_lines
            .get(line.line - 1)
            .is_some_and(|source| source.trim().is_empty());
        let starts_group = source_is_blank
            || RE_MARKDOWN_HEADING.is_match(&line.text)
            || RE_MARKDOWN_LIST_ITEM.is_match(&line.text)
            || previous_line.is_some_and(|previous| line.line != previous + 1);
        if starts_group && !paragraph.is_empty() {
            if paragraph_has_unframed_choice(&paragraph) {
                report_body_no_default(info, diag);
                return;
            }
            paragraph.clear();
        }
        if !line.text.trim().is_empty() {
            paragraph.push(line.text.as_str());
        }
        previous_line = Some(line.line);
    }
    if paragraph_has_unframed_choice(&paragraph) {
        report_body_no_default(info, diag);
    }
}

fn paragraph_has_unframed_choice(paragraph: &[&str]) -> bool {
    !paragraph
        .iter()
        .any(|line| RE_ALTERNATIVES_SUPPRESS.is_match(line))
        && paragraph
            .iter()
            .any(|line| RE_CHOICE_CUE.is_match(line) && RE_OR_CHAIN.is_match(line))
}

fn report_body_no_default(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    diag.report(
        LintRule::BodyNoDefault,
        &format!(
            "{}: body lists multiple alternatives without stating a default; \
             pick a recommended option or add conditional context",
            info.path
        ),
    );
}

fn check_consecutive_bash(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    let body_line_offset = info.fm_lines.len() + 2;
    for (first, second) in crate::fence::consecutive_bash_pairs(&info.body) {
        let first = first + body_line_offset;
        let second = second + body_line_offset;
        diag.report(
            LintRule::ConsecutiveBash,
            &format!(
                "{}:{first}: consecutive bash tool-call fences at lines {first} and {second}; combine them or add a reason-bearing lint-consecutive-bash waiver",
                info.path,
            ),
        );
    }
}

/// S055: Recursively check shell/Python scripts under the skill's `scripts/`
/// directory for error handling patterns. Each finding is owned by the script
/// path (`report_at`), not `SKILL.md`.
fn check_script_error_handling(
    info: &SkillInfo,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
) {
    let scripts_dir = match Path::new(&info.path).parent().map(|p| p.join("scripts")) {
        Some(d) if d.is_dir() => d,
        _ => return,
    };

    for entry in traversal::recursive_files(&scripts_dir, Path::new("."), Some(exclude)).entries {
        // Extension-based classification needs no I/O; shebang classification
        // reads the file. Defer the read until we know the candidate is in scope
        // or is extensionless.
        let extension_kind = classify_by_extension(&entry.path);
        if entry.path.extension().is_some() && extension_kind.is_none() {
            continue;
        }

        let content = match fs::read_to_string(&entry.path) {
            Ok(c) => c,
            Err(_) => continue, // best-effort: unreadable scripts are skipped
        };

        let kind = match extension_kind {
            Some(kind) => kind,
            None => match classify_shebang(content.lines().next().unwrap_or("")) {
                Some(kind) => kind,
                None => continue,
            },
        };

        // Skip trivially small scripts (< 5 non-empty lines)
        let nonempty_lines = content.lines().filter(|l| !l.trim().is_empty()).count();
        if nonempty_lines < SCRIPT_MIN_LINES {
            continue;
        }

        let has_handling = match kind {
            ScriptKind::Python => has_python_error_handling(&content),
            ScriptKind::Shell => RE_SH_ERROR_HANDLING.is_match(&content),
        };

        if !has_handling {
            diag.report_at(
                LintRule::ScriptErrhandMissing,
                &entry.display,
                &format!(
                    "{}: lacks error handling (try/except for Python, set -e/trap/|| exit for shell)",
                    entry.display
                ),
            );
        }
    }
}

fn has_python_error_handling(content: &str) -> bool {
    RE_PY_TRY.is_match(content) && RE_PY_EXCEPT.is_match(content)
}

/// Case-insensitive extension classification (`.sh`/`.bash` → shell, `.py` →
/// Python). Returns `None` when the path has a non-matching extension or no
/// extension (callers then consult the shebang).
fn classify_by_extension(path: &Path) -> Option<ScriptKind> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "sh" | "bash" => Some(ScriptKind::Shell),
        "py" => Some(ScriptKind::Python),
        _ => None,
    }
}

fn classify_shebang(first_line: &str) -> Option<ScriptKind> {
    let interpreter = shebang_interpreter(first_line)?;
    match interpreter {
        "sh" | "bash" | "dash" | "ksh" | "zsh" => Some(ScriptKind::Shell),
        "python" | "python2" | "python3" => Some(ScriptKind::Python),
        _ => None,
    }
}

fn shebang_interpreter(first_line: &str) -> Option<&str> {
    let rest = first_line.strip_prefix("#!")?.trim();
    let mut parts = rest.split_whitespace();
    let first = parts.next()?;
    let basename = Path::new(first).file_name()?.to_str()?;
    if basename == "env" {
        let mut next = parts.next()?;
        if next == "-S" {
            next = parts.next()?;
        }
        Path::new(next).file_name()?.to_str()
    } else {
        Some(basename)
    }
}

/// S057: Check for undocumented magic numbers in code blocks.
/// Iterates lines inside code fences, looking for identifier assignments
/// with numeric literals that are not in the well-known values list and
/// lack a justification comment on the same or preceding line.
fn check_magic_numbers(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    use crate::fence::{CodeFenceTracker, LineClass};

    let mut tracker = CodeFenceTracker::new();
    let mut prev_is_comment = false;

    for line in info.body.lines() {
        let class = tracker.process_line(line);

        match class {
            LineClass::Delimiter => {
                prev_is_comment = false;
            }
            LineClass::Outside => {
                prev_is_comment = false;
            }
            LineClass::Inside => {
                let trimmed = line.trim();

                // Skip comment lines — they can't contain undocumented assignments
                if RE_COMMENT_LINE.is_match(trimmed) {
                    prev_is_comment = true;
                    continue;
                }

                for caps in RE_MAGIC_ASSIGN.captures_iter(trimmed) {
                    if caps.get(0).is_some_and(|matched| {
                        matched.start() > 0 && trimmed.as_bytes()[matched.start() - 1] == b'-'
                    }) {
                        continue;
                    }
                    if let Some(num_match) = caps.get(1) {
                        // Float guard: skip if the character after the digits is '.' or 'e'/'E'
                        let after_pos = num_match.end();
                        if after_pos < trimmed.len() {
                            let next_char = trimmed.as_bytes()[after_pos];
                            if next_char == b'.' || next_char == b'e' || next_char == b'E' {
                                continue;
                            }
                        }

                        // Parse the number and check against allowlist
                        if let Ok(value) = num_match.as_str().parse::<u64>() {
                            if !WELL_KNOWN_VALUES.contains(&value) {
                                // Check for same-line trailing comment
                                let rest = trimmed[after_pos..].trim_start();
                                let has_trailing_comment = rest.starts_with('#')
                                    || rest.starts_with("//")
                                    || rest.starts_with("--");

                                if !has_trailing_comment && !prev_is_comment {
                                    // Extract the matched assignment for the diagnostic
                                    let assign_match = caps.get(0).unwrap().as_str();
                                    diag.report(
                                        LintRule::MagicNumberUndoc,
                                        &format!(
                                            "{}: undocumented magic number in code block: `{}`; \
                                             add a comment explaining why this value was chosen",
                                            info.path, assign_match
                                        ),
                                    );
                                    return; // Report once per file
                                }
                            }
                        }
                    }
                }

                prev_is_comment = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::MarkdownDocument;

    fn has_reference(body: &str) -> bool {
        let document = MarkdownDocument::parse_body(body);
        body_has_file_reference(body, document.links())
    }

    #[test]
    fn s037_reference_predicate_accepts_explicit_paths_and_relative_links() {
        for body in [
            "Read references/config.json before continuing.",
            "Validate schemas/input.yaml before use.",
            "Copy assets/template.txt into the output.",
            "Apply references/policy.",
            "Open [the schema](references/config.toml?raw=1#contents).",
            "See guide.md for the full procedure.",
            "See *guide.md* for the full procedure.",
            "Run ${CLAUDE_PLUGIN_ROOT}/tools/check.rb.",
            "Inspect templates/default before writing output.",
        ] {
            assert!(has_reference(body), "expected reference in: {body}");
        }
    }

    #[test]
    fn magic_assignment_pattern_iterates_each_assignment() {
        for (line, expected) in [
            ("enabled=1 timeout=47", vec!["enabled=1", "timeout=47"]),
            ("--max-count=50 timeout=47", vec!["count=50", "timeout=47"]),
        ] {
            let matches: Vec<_> = RE_MAGIC_ASSIGN
                .find_iter(line)
                .map(|matched| matched.as_str())
                .collect();
            assert_eq!(matches, expected);
        }
    }

    #[test]
    fn s037_reference_predicate_rejects_urls_fragments_and_extension_prose() {
        for body in [
            "No supporting files are needed.",
            "Open [the remote schema](https://example.test/config.json).",
            "Open [this section](#configuration).",
            "This skill supports JSON input and version .json terminology.",
            "```text\nword-ending-in-.json\n```",
            "See https://example.test/config.md for remote documentation.",
        ] {
            assert!(!has_reference(body), "unexpected reference in: {body}");
        }
    }
}
