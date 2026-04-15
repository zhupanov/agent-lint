use crate::diagnostic::DiagnosticCollector;
use crate::frontmatter;
use crate::rules::LintRule;
use crate::validators::skills::SkillInfo;
use regex::Regex;
use std::collections::HashSet;
use std::sync::LazyLock;

use super::RE_BACKSLASH_PATH;

const MAX_BODY_LINES: usize = 500;
const BODY_NO_REFS_THRESHOLD: usize = 300;
const BODY_NO_WORKFLOW_THRESHOLD: usize = 300;
const BODY_NO_EXAMPLES_THRESHOLD: usize = 200;

// S037: Body-no-refs
static RE_BODY_FILE_REF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\$\{CLAUDE_PLUGIN_ROOT\}|\.sh\b|\.md\b|\.py\b|\.js\b|\.ts\b|scripts/|shared/")
        .unwrap()
});

// S038: Time-sensitive
static RE_YEAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b20[2-3][0-9]\b").unwrap());

// S041: Fork-no-task
static RE_IMPERATIVE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(run|execute|create|build|generate|invoke|call|launch|start|perform|apply|install|deploy|write|implement)\b").unwrap()
});

// S046: Workflow structure
static RE_WORKFLOW_STRUCTURE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?m)^\s*(?:\*\*Step \d+|#{2,3} Step\b|- \[[ xX]\]|#{2,3} (?:Workflow|Process|Steps)\b)",
    )
    .unwrap()
});

// S046: Numbered list items (counted separately — need 3+ contiguous)
static RE_NUMBERED_LIST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*\d+\.\s").unwrap());

// S047: Example patterns
static RE_EXAMPLE_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*(?:#{2,3} (?:Example|Usage|Template|Format)\b|\*\*(?:Example|Input|Output)(?:\s*\d*)?:\*\*)").unwrap()
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

// S021: Consecutive bash
static RE_BASH_FENCE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^```(bash|sh|shell)\s*$").unwrap());

pub(super) fn check_body_content(
    info: &SkillInfo,
    plugin_mode: bool,
    diag: &mut DiagnosticCollector,
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

    // S022: backslash paths -- require path-like context to avoid false positives
    // on regex escapes (\s, \n, \t), LaTeX (\frac), etc.
    // Matches: C:\Users, \dir\file, path\to\something (letter, backslash, letter pattern)
    // Only check outside code fences to reduce false positives
    for line in crate::fence::lines_outside_fences(&info.body) {
        if RE_BACKSLASH_PATH.is_match(line) {
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
    if plugin_mode && line_count > BODY_NO_REFS_THRESHOLD && !RE_BODY_FILE_REF.is_match(&info.body)
    {
        diag.report(
            LintRule::BodyNoRefs,
            &format!(
                "{}: body exceeds 300 lines ({}) with no file references; consider splitting into reference files",
                info.path, line_count
            ),
        );
    }

    // S038: time-sensitive (plugin-only) -- date/year patterns outside code fences
    if plugin_mode {
        for line in crate::fence::lines_outside_fences(&info.body) {
            if RE_YEAR.is_match(line) {
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
                    numbered_count = 0;
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

    // S053: terminology consistency (plugin-only) — outside code fences only
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
    }
}

/// A skill is "script-backed" if it has a non-empty `scripts/` subdirectory
/// or its body references script file extensions (.sh, .py, .js, .ts).
fn is_script_backed(info: &SkillInfo) -> bool {
    info.has_scripts_dir || RE_SCRIPT_FILE_REF.is_match(&info.body)
}

fn check_terminology_consistency(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    // Collect all words outside code fences into a set
    let mut words = HashSet::new();
    for line in crate::fence::lines_outside_fences(&info.body) {
        for token in line.to_lowercase().split(|c: char| !c.is_alphanumeric()) {
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

fn check_consecutive_bash(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    use crate::fence::{CodeFenceTracker, LineClass};

    let mut tracker = CodeFenceTracker::new();
    let mut last_bash_end: Option<usize> = None;
    let mut fence_is_bash = false;

    for (i, line) in info.body.lines().enumerate() {
        let trimmed = line.trim_start();
        match tracker.process_line(line) {
            LineClass::Delimiter => {
                if !tracker.in_fence() {
                    // This delimiter just closed a fence
                    if fence_is_bash {
                        last_bash_end = Some(i);
                    }
                    fence_is_bash = false;
                } else {
                    // This delimiter just opened a fence
                    if RE_BASH_FENCE.is_match(trimmed) {
                        // Opening a bash fence -- check for consecutive
                        if let Some(prev_end) = last_bash_end {
                            let between_lines: Vec<&str> = info
                                .body
                                .lines()
                                .skip(prev_end + 1)
                                .take(i - prev_end - 1)
                                .collect();
                            let only_blank = between_lines.iter().all(|l| l.trim().is_empty());
                            if only_blank {
                                diag.report(
                                    LintRule::ConsecutiveBash,
                                    &format!(
                                        "{}: consecutive bash code blocks (lines {} and {}) could be combined into one",
                                        info.path, prev_end + 1, i + 1
                                    ),
                                );
                                return; // Report once per file
                            }
                        }
                        fence_is_bash = true;
                    } else {
                        fence_is_bash = false;
                    }
                }
            }
            LineClass::Inside | LineClass::Outside => {}
        }
    }
}
