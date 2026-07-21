//! Shared prompt-content checks for live instruction documents.
//!
//! These checks intentionally inspect prose only. Code fences frequently contain
//! examples of wording that should not be treated as live instructions.

use crate::config::ExcludeSet;
use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata};
use crate::live_instructions::{InstructionSurfaceKind, LiveInstructionDocument};
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

/// One shared prompt-content pass for a validation run.
///
/// Surface owners submit typed documents as they discover them. The pass does
/// not walk the repository or decide platform activation, and it analyzes each
/// normalized subject path at most once.
#[derive(Default)]
pub(crate) struct PromptContentPass {
    seen: HashSet<String>,
}

impl PromptContentPass {
    pub(crate) fn validate(
        &mut self,
        document: &LiveInstructionDocument<'_>,
        diag: &mut DiagnosticCollector,
    ) {
        let path = crate::config::normalize_path(&document.subject_path().to_string_lossy());
        if !self.seen.insert(path.clone()) {
            return;
        }

        // Every current Q001-Q003 rule applies to every live-instruction kind.
        // Keeping the typed context here makes later applicability decisions
        // explicit instead of requiring path-string inference.
        let _surface_kind = document.surface_kind();
        diag.with_subject_path(document.subject_path(), |diag| {
            check_generic_filler(&path, document, diag);
            check_negative_only(&path, document, diag);
            check_weak_critical_language(&path, document, diag);
        });
    }
}

#[cfg(test)]
fn validate_body(path: &str, body: &str, diag: &mut DiagnosticCollector) {
    let markdown = MarkdownDocument::parse_body(body);
    let document =
        LiveInstructionDocument::new(Path::new(path), InstructionSurfaceKind::Skill, &markdown);
    PromptContentPass::default().validate(&document, diag);
}

/// Run the shared body checks for the root `CLAUDE.md`, then compare its prose
/// with `README.md` when both files exist.
#[cfg(test)]
pub(crate) fn validate_claude_md(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let mut prompt_pass = PromptContentPass::default();
    validate_claude_md_with_prompt_pass(diag, exclude, &mut prompt_pass);
}

pub(crate) fn validate_claude_md_with_prompt_pass(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    prompt_pass: &mut PromptContentPass,
) {
    const CLAUDE_MD: &str = "CLAUDE.md";
    if exclude.is_excluded(CLAUDE_MD) || !Path::new(CLAUDE_MD).is_file() {
        return;
    }

    let Ok(claude) = fs::read_to_string(CLAUDE_MD) else {
        return;
    };
    let markdown = MarkdownDocument::parse_body(&claude);
    let document = LiveInstructionDocument::new(
        Path::new(CLAUDE_MD),
        InstructionSurfaceKind::ClaudeProject,
        &markdown,
    );
    diag.with_subject_path(CLAUDE_MD, |diag| {
        prompt_pass.validate(&document, diag);

        let Ok(readme) = fs::read_to_string("README.md") else {
            return;
        };
        check_readme_overlap(&claude, &readme, diag);
    });
}

fn check_generic_filler(
    path: &str,
    document: &LiveInstructionDocument<'_>,
    diag: &mut DiagnosticCollector,
) {
    for line in document.prose_lines() {
        let normalized = line.text.to_ascii_lowercase();
        if let Some(phrase) = GENERIC_FILLER_PHRASES
            .iter()
            .find(|phrase| normalized.contains(**phrase))
        {
            diag.report_with(
                LintRule::PromptGenericFiller,
                &format!(
                    "{path}: generic filler instruction '{phrase}' adds no actionable guidance"
                ),
                DiagnosticMetadata::at_line(line.line),
            );
            return;
        }
    }
}

fn check_negative_only(
    path: &str,
    document: &LiveInstructionDocument<'_>,
    diag: &mut DiagnosticCollector,
) {
    let lines = document.prose_lines();
    for (index, line) in lines.iter().enumerate() {
        let normalized = line.text.to_ascii_lowercase();
        if !contains_phrase(&normalized, NEGATIVE_INSTRUCTIONS) {
            continue;
        }

        let start = index.saturating_sub(NEGATIVE_WINDOW);
        let end = (index + NEGATIVE_WINDOW + 1).min(lines.len());
        let has_alternative = lines[start..end].iter().any(|nearby| {
            contains_phrase(&nearby.text.to_ascii_lowercase(), POSITIVE_ALTERNATIVES)
        });
        if !has_alternative {
            diag.report_with(
                LintRule::PromptNegativeOnly,
                &format!(
                    "{path}: negative instruction lacks a positive alternative (add instead, rather, or prefer within {NEGATIVE_WINDOW} lines)"
                ),
                DiagnosticMetadata::at_line(line.line),
            );
            return;
        }
    }
}

fn check_weak_critical_language(
    path: &str,
    document: &LiveInstructionDocument<'_>,
    diag: &mut DiagnosticCollector,
) {
    let mut section_level: Option<usize> = None;

    for prose_line in document.prose_lines() {
        let line_number = prose_line.line;
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
                &prose_line.text.to_ascii_lowercase(),
                &["should", "try to", "consider", "maybe"],
            )
        }) {
            diag.report_with(
                LintRule::PromptWeakCritical,
                &format!(
                    "{path}: weak language in a critical/important section; use a concrete requirement instead"
                ),
                DiagnosticMetadata::at_line(line_number),
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
    use crate::config::LintConfig;
    use crate::context::{LintContext, LintMode, ManifestState};
    use crate::platforms::ValidationTargets;

    fn context(root: &Path, mode: LintMode) -> LintContext {
        LintContext {
            base_path: root.to_path_buf(),
            mode,
            plugin_json: ManifestState::Missing,
            marketplace_json: ManifestState::Missing,
            hooks_json: ManifestState::Missing,
            settings_json: ManifestState::Missing,
            settings_local_json: ManifestState::Missing,
        }
    }

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

    #[test]
    #[serial_test::serial]
    fn agents_and_cursor_rules_receive_source_aware_prompt_diagnostics_in_both_modes() {
        for mode in [LintMode::Basic, LintMode::Plugin] {
            let tmp = tempfile::tempdir().unwrap();
            let _guard = crate::test_helpers::CwdGuard::new();
            std::env::set_current_dir(tmp.path()).unwrap();
            std::fs::create_dir_all("nested").unwrap();
            std::fs::create_dir_all(".cursor/rules/nested").unwrap();
            std::fs::write("AGENTS.md", "Never invent test results.\n").unwrap();
            std::fs::write("nested/AGENTS.md", "Be helpful when responding.\n").unwrap();
            std::fs::write(
                ".cursor/rules/example.mdc",
                "---\ndescription: Be helpful in metadata\nalwaysApply: true\n---\nNever invent test results.\n",
            )
            .unwrap();
            std::fs::write(
                ".cursor/rules/nested/example.md",
                "Be concise when responding.\n",
            )
            .unwrap();
            std::fs::write(".cursorrules", "Never invent test results.\n").unwrap();
            std::fs::write("notes.md", "Never invent test results.\n").unwrap();

            let mut diag = DiagnosticCollector::new_all_enabled();
            super::super::run_all_with_targets(
                &context(tmp.path(), mode),
                &mut diag,
                &ExcludeSet::default(),
                ValidationTargets {
                    cursor: true,
                    codex: false,
                    agents_md: true,
                    agent_skills: false,
                },
            );

            let prompt_diagnostics: Vec<_> = diag
                .diagnostics()
                .iter()
                .filter(|item| {
                    matches!(
                        item.rule,
                        LintRule::PromptGenericFiller
                            | LintRule::PromptNegativeOnly
                            | LintRule::PromptWeakCritical
                    )
                })
                .collect();
            for expected in [
                "AGENTS.md",
                "nested/AGENTS.md",
                ".cursor/rules/example.mdc",
                ".cursor/rules/nested/example.md",
                ".cursorrules",
            ] {
                assert!(
                    prompt_diagnostics
                        .iter()
                        .any(|item| { item.subject_path.as_deref() == Some(Path::new(expected)) }),
                    "{mode:?}: missing prompt diagnostic for {expected}: {prompt_diagnostics:?}"
                );
            }
            assert!(
                !prompt_diagnostics
                    .iter()
                    .any(|item| { item.subject_path.as_deref() == Some(Path::new("notes.md")) })
            );
            let mdc = prompt_diagnostics
                .iter()
                .find(|item| {
                    item.subject_path.as_deref() == Some(Path::new(".cursor/rules/example.mdc"))
                })
                .unwrap();
            assert_eq!(
                mdc.location.unwrap().start().line_number(),
                5,
                "frontmatter removal must preserve original source lines"
            );
            assert_eq!(
                prompt_diagnostics
                    .iter()
                    .filter(|item| {
                        item.subject_path.as_deref() == Some(Path::new(".cursor/rules/example.mdc"))
                    })
                    .count(),
                1,
                "frontmatter text must not be linted"
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn code_and_quoted_examples_are_not_live_prose() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::write(
            "AGENTS.md",
            "# Examples\n`Never invent output.`\n> Be helpful in this quoted example.\n```text\nBe concise.\n```\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        super::super::run_all_with_targets(
            &context(tmp.path(), LintMode::Basic),
            &mut diag,
            &ExcludeSet::default(),
            ValidationTargets {
                agents_md: true,
                ..ValidationTargets::default()
            },
        );

        assert!(!diag.diagnostics().iter().any(|item| matches!(
            item.rule,
            LintRule::PromptGenericFiller
                | LintRule::PromptNegativeOnly
                | LintRule::PromptWeakCritical
        )));
    }

    #[test]
    #[serial_test::serial]
    fn cursor_disable_does_not_disable_agents_prompt_checks() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".cursor/rules").unwrap();
        std::fs::write("AGENTS.md", "Never invent output.\n").unwrap();
        std::fs::write(".cursor/rules/example.md", "Never invent output.\n").unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        super::super::run_all_with_targets(
            &context(tmp.path(), LintMode::Basic),
            &mut diag,
            &ExcludeSet::default(),
            ValidationTargets {
                cursor: false,
                agents_md: true,
                ..ValidationTargets::default()
            },
        );

        let prompt_diagnostics: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::PromptNegativeOnly)
            .collect();
        assert_eq!(prompt_diagnostics.len(), 1);
        assert_eq!(
            prompt_diagnostics[0].subject_path.as_deref(),
            Some(Path::new("AGENTS.md"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn exclusions_and_structured_per_file_suppression_apply_to_prompt_rules() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("nested").unwrap();
        std::fs::create_dir_all("excluded").unwrap();
        std::fs::write("AGENTS.md", "Never invent output.\n").unwrap();
        std::fs::write("nested/AGENTS.md", "Never invent output.\n").unwrap();
        std::fs::write("excluded/AGENTS.md", "Never invent output.\n").unwrap();
        std::fs::write(
            "agent-lint.toml",
            "[[lint.overrides]]\nfiles = [\"nested/AGENTS.md\"]\nsuppress = [\"Q002\"]\nreason = \"legacy nested instructions\"\n",
        )
        .unwrap();
        let config = LintConfig::load(tmp.path()).unwrap();
        let exclude = ExcludeSet::new(&["excluded/**".into()]).unwrap();
        let mut diag = DiagnosticCollector::with_config(config);

        super::super::run_all_with_targets(
            &context(tmp.path(), LintMode::Basic),
            &mut diag,
            &exclude,
            ValidationTargets {
                agents_md: true,
                ..ValidationTargets::default()
            },
        );

        let q002: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::PromptNegativeOnly)
            .collect();
        assert_eq!(q002.len(), 1);
        assert_eq!(
            q002[0].subject_path.as_deref(),
            Some(Path::new("AGENTS.md"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn shared_prompt_pass_analyzes_overlapping_surface_once() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".cursor/rules").unwrap();
        std::fs::write(".cursor/rules/AGENTS.md", "Never invent output.\n").unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        super::super::run_all_with_targets(
            &context(tmp.path(), LintMode::Basic),
            &mut diag,
            &ExcludeSet::default(),
            ValidationTargets {
                cursor: true,
                agents_md: true,
                ..ValidationTargets::default()
            },
        );

        assert_eq!(
            diag.diagnostics()
                .iter()
                .filter(|item| {
                    item.rule == LintRule::PromptNegativeOnly
                        && item.subject_path.as_deref()
                            == Some(Path::new(".cursor/rules/AGENTS.md"))
                })
                .count(),
            1
        );
    }
}
