//! Shared prompt-content checks for `CLAUDE.md`, skill bodies, and agent bodies.
//!
//! These checks intentionally inspect prose only. Code fences frequently contain
//! examples of wording that should not be treated as live instructions.

use crate::config::ExcludeSet;
use crate::diagnostic::DiagnosticCollector;
use crate::markdown::MarkdownDocument;
use crate::rules::LintRule;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

const GENERIC_FILLER_PHRASES: &[&str] = &[
    "be helpful",
    "be accurate",
    "be concise",
    "follow instructions",
    "do your best",
    "be professional",
    "use best judgment",
    "provide high-quality",
];

const NEGATIVE_INSTRUCTIONS: &[&str] = &["don't", "do not", "never", "avoid"];
const POSITIVE_ALTERNATIVES: &[&str] = &["instead", "rather", "prefer"];
const NEGATIVE_WINDOW: usize = 3;
const README_OVERLAP_THRESHOLD: f64 = 0.4;
const MIN_SHARED_README_LINES: usize = 3;

/// Validate the prompt body of a skill or agent. Callers supply a body with
/// frontmatter already removed, so frontmatter values are never interpreted as
/// instructions.
pub(crate) fn validate_body(path: &str, body: &str, diag: &mut DiagnosticCollector) {
    let document = MarkdownDocument::parse_body(body);
    validate_document_body(path, &document, diag);
}

/// Validate a pre-parsed Markdown document without reparsing its body.
pub(crate) fn validate_document_body(
    path: &str,
    document: &MarkdownDocument,
    diag: &mut DiagnosticCollector,
) {
    let lines: Vec<_> = document.body_prose_lines().collect();
    check_generic_filler(path, &lines, diag);
    check_negative_only(path, &lines, diag);
    check_weak_critical_language(path, document, diag);
}

/// Run the shared body checks for the root `CLAUDE.md`, then compare its prose
/// with `README.md` when both files exist.
pub(crate) fn validate_claude_md(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    const CLAUDE_MD: &str = "CLAUDE.md";
    if exclude.is_excluded(CLAUDE_MD) || !Path::new(CLAUDE_MD).is_file() {
        return;
    }

    let Ok(claude) = fs::read_to_string(CLAUDE_MD) else {
        return;
    };
    diag.with_subject_path(CLAUDE_MD, |diag| {
        validate_body(CLAUDE_MD, &claude, diag);

        let Ok(readme) = fs::read_to_string("README.md") else {
            return;
        };
        check_readme_overlap(&claude, &readme, diag);
    });
}

fn check_generic_filler(path: &str, lines: &[&str], diag: &mut DiagnosticCollector) {
    for line in lines {
        let normalized = line.to_ascii_lowercase();
        if let Some(phrase) = GENERIC_FILLER_PHRASES
            .iter()
            .find(|phrase| normalized.contains(**phrase))
        {
            diag.report(
                LintRule::PromptGenericFiller,
                &format!(
                    "{path}: generic filler instruction '{phrase}' adds no actionable guidance"
                ),
            );
            return;
        }
    }
}

fn check_negative_only(path: &str, lines: &[&str], diag: &mut DiagnosticCollector) {
    for (index, line) in lines.iter().enumerate() {
        let normalized = line.to_ascii_lowercase();
        if !contains_phrase(&normalized, NEGATIVE_INSTRUCTIONS) {
            continue;
        }

        let start = index.saturating_sub(NEGATIVE_WINDOW);
        let end = (index + NEGATIVE_WINDOW + 1).min(lines.len());
        let has_alternative = lines[start..end]
            .iter()
            .any(|nearby| contains_phrase(&nearby.to_ascii_lowercase(), POSITIVE_ALTERNATIVES));
        if !has_alternative {
            diag.report(
                LintRule::PromptNegativeOnly,
                &format!(
                    "{path}: negative instruction lacks a positive alternative (add instead, rather, or prefer within {NEGATIVE_WINDOW} lines)"
                ),
            );
            return;
        }
    }
}

fn check_weak_critical_language(
    path: &str,
    document: &MarkdownDocument,
    diag: &mut DiagnosticCollector,
) {
    let mut section_level: Option<usize> = None;

    for (line_number, line) in document.body_prose_lines_with_numbers() {
        if let Some(heading) = document
            .headings()
            .iter()
            .find(|heading| heading.line == line_number)
        {
            let level = heading.level as usize;
            if section_level.is_some_and(|active| level <= active) {
                section_level = None;
            }
            if contains_word(&heading.text.to_ascii_lowercase(), "critical")
                || contains_word(&heading.text.to_ascii_lowercase(), "important")
            {
                section_level = Some(level);
            }
            continue;
        }

        if section_level.is_some_and(|_| {
            contains_phrase(
                &line.to_ascii_lowercase(),
                &["should", "try to", "consider", "maybe"],
            )
        }) {
            diag.report(
                LintRule::PromptWeakCritical,
                &format!(
                    "{path}: weak language in a critical/important section; use a concrete requirement instead"
                ),
            );
            return;
        }
    }
}

fn check_readme_overlap(claude: &str, readme: &str, diag: &mut DiagnosticCollector) {
    let claude_lines = normalized_line_set(claude);
    let readme_lines = normalized_line_set(readme);
    if claude_lines.is_empty() || readme_lines.is_empty() {
        return;
    }

    let shared = claude_lines.intersection(&readme_lines).count();
    let overlap = shared as f64 / claude_lines.len() as f64;
    if shared >= MIN_SHARED_README_LINES && overlap > README_OVERLAP_THRESHOLD {
        diag.report(
            LintRule::ClaudeReadmeDuplicate,
            &format!(
                "CLAUDE.md duplicates README.md content ({:.0}% of {} normalized prose lines overlap); keep project instructions concise and link to the README instead",
                overlap * 100.0,
                claude_lines.len()
            ),
        );
    }
}

fn normalized_line_set(content: &str) -> HashSet<String> {
    crate::fence::lines_outside_fences(content)
        .filter_map(normalize_line)
        .collect()
}

fn normalize_line(line: &str) -> Option<String> {
    let normalized = line
        .trim()
        .trim_start_matches(['#', '-', '*', '+', '>', ' ', '\t'])
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    (!normalized.is_empty()).then_some(normalized)
}

fn contains_phrase(text: &str, phrases: &[&str]) -> bool {
    phrases.iter().any(|phrase| {
        text.match_indices(phrase).any(|(start, _)| {
            let end = start + phrase.len();
            is_word_boundary(text, start, true) && is_word_boundary(text, end, false)
        })
    })
}

fn contains_word(text: &str, word: &str) -> bool {
    contains_phrase(text, &[word])
}

fn is_word_boundary(text: &str, index: usize, before: bool) -> bool {
    let adjacent = if before {
        text[..index].chars().next_back()
    } else {
        text[index..].chars().next()
    };
    !adjacent.is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{LintContext, LintMode, ManifestState};

    fn diagnostics_for(body: &str) -> DiagnosticCollector {
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_body("skills/example/SKILL.md", body, &mut diag);
        diag
    }

    #[test]
    fn generic_filler_is_case_insensitive_and_fence_aware() {
        let diag = diagnostics_for("Be helpful when responding.\n```text\nNever do this\n```");
        assert_eq!(diag.error_count(), 1);
    }

    #[test]
    fn negative_instruction_needs_nearby_positive_alternative() {
        let missing = diagnostics_for("Never invent test results.");
        assert_eq!(missing.error_count(), 1);

        let addressed =
            diagnostics_for("Never invent test results. Prefer reporting the missing evidence.");
        assert_eq!(addressed.error_count(), 0);
    }

    #[test]
    fn weak_language_only_counts_inside_critical_sections() {
        let critical = diagnostics_for("## Important behavior\nYou should verify the result.");
        assert_eq!(critical.error_count(), 1);

        let ordinary = diagnostics_for("## Notes\nYou should verify the result.");
        assert_eq!(ordinary.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn claudemd_readme_overlap_uses_normalized_prose_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::write(
            "CLAUDE.md",
            "# Project\n- Run cargo test\n- Run cargo fmt\n- Review diagnostics\n- Commit focused changes\n",
        )
        .unwrap();
        std::fs::write(
            "README.md",
            "# Project\nRun cargo test\nRun cargo fmt\nReview diagnostics\nCommit focused changes\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_claude_md(&mut diag, &ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
    }

    #[test]
    #[serial_test::serial]
    fn shared_check_runs_for_claude_skills_and_agents() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude/skills/example").unwrap();
        std::fs::create_dir_all(".claude/agents").unwrap();
        std::fs::write("CLAUDE.md", "Be helpful when responding.\n").unwrap();
        std::fs::write(
            ".claude/skills/example/SKILL.md",
            "---\nname: example\ndescription: Use when you need reliable test support\n---\nBe helpful when responding.\n",
        )
        .unwrap();
        std::fs::write(
            ".claude/agents/example.md",
            "---\nname: example\ndescription: Reviews changes with concrete test evidence\n---\nBe helpful when responding.\n",
        )
        .unwrap();

        let ctx = LintContext {
            base_path: tmp.path().to_path_buf(),
            mode: LintMode::Basic,
            plugin_json: ManifestState::Missing,
            marketplace_json: ManifestState::Missing,
            hooks_json: ManifestState::Missing,
            settings_json: ManifestState::Missing,
            settings_local_json: ManifestState::Missing,
        };
        let mut diag = DiagnosticCollector::new_all_enabled();
        super::super::run_all(&ctx, &mut diag, &ExcludeSet::default());
        assert_eq!(diag.error_count(), 3);
    }
}
