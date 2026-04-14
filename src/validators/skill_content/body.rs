use crate::diagnostic::DiagnosticCollector;
use crate::frontmatter;
use crate::rules::LintRule;
use crate::validators::skills::SkillInfo;
use regex::Regex;
use std::sync::LazyLock;

use super::RE_BACKSLASH_PATH;

const MAX_BODY_LINES: usize = 500;
const BODY_NO_REFS_THRESHOLD: usize = 300;

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
