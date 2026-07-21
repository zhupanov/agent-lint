//! Shared prompt-content checks for live instruction documents.
//!
//! These checks intentionally inspect prose only. Code fences frequently contain
//! examples of wording that should not be treated as live instructions.

use crate::config::ExcludeSet;
use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata, SourceSpan};
use crate::live_instructions::{InstructionSurfaceKind, LiveInstructionDocument};
use crate::markdown::MarkdownDocument;
use crate::rules::LintRule;
use crate::validators::common::NEVER_INVENT_PROHIBITION;
use regex::Regex;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

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

/// Explicit unbounded-retry forms accepted by Q005. Keep these narrow: a
/// generic `until` expression also describes many finite, non-retry workflows.
const UNBOUNDED_RETRY_PATTERNS: &[&str] = &[
    r"\bcontinue\s+indefinitely\b",
    r"\bloop\s+forever\b",
    r"\bretry\s+as\s+many\s+times\s+as\s+(?:needed|necessary)\b",
    r"\bkeep\s+trying\s+until\s+(?:it\s+)?succeeds\b",
    r"\bretry\s+until\s+(?:success|it\s+succeeds)\b",
    r"\bdo\s+not\s+stop\s+until\s+(?:it\s+)?(?:(?:the\s+)?(?:task|operation|command|tool\s+call|test\s+suite)\s+)?(?:succeeds|passes|works|is\s+(?:complete|completed|resolved))\b",
];

static UNBOUNDED_RETRY_REGEXES: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    UNBOUNDED_RETRY_PATTERNS
        .iter()
        .map(|pattern| Regex::new(pattern).expect("Q005 retry pattern is valid"))
        .collect()
});

static APPLICABLE_RETRY_BOUNDS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"\b(?:at\s+most|no\s+more\s+than|up\s+to|within|after|for|(?:a\s+)?maximum\s+of)\s+\d+\s+(?:attempts?|retries|tries|tool[ -]?calls?|steps?)\b",
        r"\b\d+\s+(?:attempts?|retries|tries|tool[ -]?calls?|steps?)\s+(?:maximum|max)\b",
        r"\b(?:timeout|time(?:\s|-)?limit)\s*(?:of|:|is)?\s*\d+\s*(?:milliseconds?|seconds?|minutes?|hours?|ms|secs?|mins?|hrs?)\b",
        r"\bwithin\s+\d+\s*(?:milliseconds?|seconds?|minutes?|hours?|ms|secs?|mins?|hrs?)\b",
        r"\b(?:token|cost)\s+budget\s*(?:of|:|is)?\s*(?:\$?\d[\d,.]*|\d+[kKmM]?)\b",
        r"\b(?:at\s+most|no\s+more\s+than|up\s+to)\s+(?:\$?\d[\d,.]*|\d+[kKmM]?)\s+(?:tokens?|dollars?|usd)\b",
        r"\b(?:deadline\s*(?:of|:|is)?|by)\s+(?:\d{4}-\d{2}-\d{2}|\d{1,2}:\d{2}\s*(?:am|pm)?|end\s+of\s+(?:day|week)|(?:monday|tuesday|wednesday|thursday|friday|saturday|sunday))\b",
        r"\b(?:if|when)\s+(?:it|the\s+(?:retry|operation|task|command|tool\s+call))\s+fails?\s*,?\s*(?:stop|abort|return|report|escalate|surface|give\s+up|fall\s+back)\b",
        r"\b(?:on|upon)\s+failure\s*,?\s*(?:stop|abort|return|report|escalate|surface|give\s+up|fall\s+back)\b",
        r"\botherwise\s*,?\s*(?:stop|abort|return|report|escalate|surface|give\s+up|fall\s+back)\b",
    ]
    .into_iter()
    .map(|pattern| Regex::new(pattern).expect("Q005 bound pattern is valid"))
    .collect()
});

static OPERATIVE_RETRY_SETUP_CLAUSE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:when|if|after|before)\b[^,]*,\s*(?:continue|loop|retry|keep trying|do not stop)\b",
    )
    .expect("Q005 setup-clause pattern is valid")
});

static OPERATIVE_RETRY_SUBJECT_DIRECTIVE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:you|agents?|assistant|model)\s+(?:must|should|shall|will)\s+(?:continue|loop|retry|keep trying|not stop)\b",
    )
    .expect("Q005 subject directive pattern is valid")
});

static UNBOUNDED_RETRY_PROHIBITION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:do\s+not|don't|never|avoid)\s+(?:keep\s+trying|retrying|retry|continuing|looping)\b",
    )
    .expect("Q005 prohibition pattern is valid")
});

static PRECISE_SAFETY_PROHIBITIONS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        // Disclosure or logging of secrets, credentials, tokens, and private data.
        r"^(?:don't|do not|never|avoid)\s+(?:(?:ever|intentionally|knowingly)\s+)?(?:expos(?:e|ing)|reveal(?:ing)?|disclos(?:e|ing)|leak(?:ing)?|log(?:ging)?|print(?:ing)?|echo(?:ing)?|publish(?:ing)?|shar(?:e|ing)|transmit(?:ting)?|send(?:ing)?|commit(?:ting)?)(?:\s+(?:or|and)\s+(?:expos(?:e|ing)|reveal(?:ing)?|disclos(?:e|ing)|leak(?:ing)?|log(?:ging)?|print(?:ing)?|echo(?:ing)?|publish(?:ing)?|shar(?:e|ing)|transmit(?:ting)?|send(?:ing)?|commit(?:ting)?))?\s+(?:(?:any|the|a|an|our|user|users'|customer|customers'|private|sensitive|personal|authentication|api)\s+){0,3}(?:secrets?|credentials?|tokens?|passwords?|private data|personal data|customer data|user data|personally identifiable information|pii|api keys?)\b",
        // Attempts to bypass approval, authorization, permissions, or access controls.
        r"^(?:don't|do not|never|avoid)\s+(?:(?:ever|intentionally|knowingly)\s+)?(?:bypass(?:ing)?|circumvent(?:ing)?|evad(?:e|ing)|skip(?:ping)?|ignor(?:e|ing)|overrid(?:e|ing)|disabl(?:e|ing))\s+(?:(?:any|required|explicit|the|an?|user|security)\s+){0,3}(?:authorization|approval|permissions?|authentication|access controls?|security controls?)\b",
        r"^(?:don't|do not|never|avoid)\s+(?:(?:ever|intentionally|knowingly)\s+)?(?:proceed(?:ing)?|act(?:ing)?|execut(?:e|ing)|continu(?:e|ing)|mak(?:e|ing) changes?)\b(?:\s+[[:alnum:]_-]+){0,3}\s+without\s+(?:(?:required|explicit|the|an?|user)\s+){0,2}(?:authorization|approval|permission)\b",
        // Direct destructive or irreversible mutations.
        r"^(?:don't|do not|never|avoid)\s+(?:(?:ever|intentionally|knowingly)\s+)?(?:delet(?:e|ing)|destroy(?:ing)?|eras(?:e|ing)|wip(?:e|ing)|overwrit(?:e|ing)|truncat(?:e|ing)|drop(?:ping)?|force[- ]push(?:ing)?|reformat(?:ting)?)\b",
        r"^(?:don't|do not|never|avoid)\s+(?:(?:ever|intentionally|knowingly)\s+)?(?:remov(?:e|ing)|reset(?:ting)?|discard(?:ing)?)\s+(?:(?:any|all|the|a|an|our|user|customer|local|remote|uncommitted)\s+){0,3}(?:files?|directories|data|records?|resources?|branches|commits?|changes?|work)\b",
        r"^(?:don't|do not|never|avoid)\s+(?:(?:ever|intentionally|knowingly)\s+)?(?:run(?:ning)?|execut(?:e|ing)|perform(?:ing)?)\s+(?:(?:any|an?|the|potentially)\s+){0,2}(?:destructive(?:\s+or\s+irreversible)?|irreversible)\s+(?:actions?|commands?|operations?)\b",
        // Additional evidence-integrity language beyond A013's shared contract.
        r"^(?:don't|do not|never|avoid)\s+(?:(?:ever|intentionally|knowingly)\s+)?(?:inventing|fabricating|guessing|falsif(?:y|ying)|forg(?:e|ing)|mak(?:e|ing) up)\b",
        // Explicit legal, regulatory, privacy, and security policy constraints.
        r"^(?:don't|do not|never|avoid)\s+(?:(?:ever|intentionally|knowingly)\s+)?(?:violat(?:e|ing)|breach(?:ing)?|contraven(?:e|ing)|disregard(?:ing)?|break(?:ing)?)\s+(?:(?:any|an?|the|explicit|applicable|legal|security|privacy)\s+){0,3}(?:laws?|regulations?|legal(?:/security)? polic(?:y|ies)|security polic(?:y|ies)|privacy polic(?:y|ies)|compliance requirements?)\b",
    ]
    .into_iter()
    .map(|pattern| Regex::new(pattern).expect("Q002 safety pattern is valid"))
    .collect()
});

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

        // Every current Q001-Q005 rule applies to every live-instruction kind.
        // Keeping the typed context here makes later applicability decisions
        // explicit instead of requiring path-string inference.
        let _surface_kind = document.surface_kind();
        diag.with_subject_path(document.subject_path(), |diag| {
            check_generic_filler(&path, document, diag);
            check_negative_only(&path, document, diag);
            check_weak_critical_language(&path, document, diag);
            check_unbounded_retry(&path, document, diag);
        });
    }
}

fn check_unbounded_retry(
    path: &str,
    document: &LiveInstructionDocument<'_>,
    diag: &mut DiagnosticCollector,
) {
    if document.has_outer_execution_bound() {
        return;
    }

    let example_scopes = example_scopes(document);
    for scope in retry_instruction_scopes(document, &example_scopes) {
        let normalized_scope = scope
            .iter()
            .map(|line| line.text.to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join(" ");
        if APPLICABLE_RETRY_BOUNDS
            .iter()
            .any(|pattern| pattern.is_match(&normalized_scope))
        {
            continue;
        }

        for line in scope {
            let mut sentence_offset = 0;
            for sentence in line.text.split_inclusive(['.', '!', '?', ';']) {
                let normalized_sentence = sentence.to_ascii_lowercase();
                if !is_operative_retry_instruction(&normalized_sentence)
                    || explicitly_prohibits_unbounded_retry(&normalized_sentence)
                {
                    sentence_offset += sentence.chars().count();
                    continue;
                }
                let Some(matched) = UNBOUNDED_RETRY_REGEXES
                    .iter()
                    .find_map(|pattern| pattern.find(&normalized_sentence))
                else {
                    sentence_offset += sentence.chars().count();
                    continue;
                };

                let start_column =
                    sentence_offset + normalized_sentence[..matched.start()].chars().count() + 1;
                let end_column =
                    sentence_offset + normalized_sentence[..matched.end()].chars().count() + 1;
                let evidence = sentence.trim();
                diag.report_with(
                    LintRule::PromptUnboundedRetry,
                    &format!(
                        "{path}: unbounded retry or continuation instruction; add an explicit bound or concrete failure outcome"
                    ),
                    DiagnosticMetadata::default()
                        .with_location(SourceSpan::range(
                            line.line,
                            start_column,
                            line.line,
                            end_column,
                        ))
                        .with_evidence(evidence)
                        .with_suggestion(
                            "Add an explicit attempt, step, tool-call, timeout, token/cost budget, deadline, or concrete failure outcome.",
                        ),
                );
                return;
            }
        }
    }
}

fn retry_instruction_scopes<'a>(
    document: &'a LiveInstructionDocument<'_>,
    example_scopes: &[bool],
) -> Vec<Vec<&'a crate::markdown::MarkdownProseLine>> {
    let heading_lines: HashSet<_> = document
        .headings()
        .iter()
        .map(|heading| heading.line)
        .collect();
    let mut scopes = Vec::new();
    let mut scope = Vec::new();
    let mut previous_line = None;

    for (line, is_example) in document.prose_lines().iter().zip(example_scopes) {
        let starts_list_item = line.text.trim_start().starts_with(['-', '*', '+']);
        let boundary = line.text.trim().is_empty()
            || *is_example
            || heading_lines.contains(&line.line)
            || previous_line.is_some_and(|previous| line.line > previous + 1)
            || (starts_list_item && !scope.is_empty());
        if boundary && !scope.is_empty() {
            scopes.push(std::mem::take(&mut scope));
        }
        if !line.text.trim().is_empty() && !*is_example && !heading_lines.contains(&line.line) {
            scope.push(line);
        }
        previous_line = Some(line.line);
    }
    if !scope.is_empty() {
        scopes.push(scope);
    }
    scopes
}

fn is_operative_retry_instruction(sentence: &str) -> bool {
    let sentence = sentence.trim().trim_start_matches(|character: char| {
        character.is_whitespace()
            || matches!(character, '#' | '-' | '*' | '+' | '(' | ')' | '[' | ']')
            || character.is_ascii_digit()
            || character == '.'
    });
    let directive = [
        "continue",
        "loop",
        "retry",
        "keep trying",
        "do not stop",
        "please continue",
        "please retry",
    ];
    directive.iter().any(|prefix| sentence.starts_with(prefix))
        || OPERATIVE_RETRY_SETUP_CLAUSE.is_match(sentence)
        || OPERATIVE_RETRY_SUBJECT_DIRECTIVE.is_match(sentence)
}

fn explicitly_prohibits_unbounded_retry(sentence: &str) -> bool {
    UNBOUNDED_RETRY_PROHIBITION.is_match(sentence.trim())
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
    let example_scopes = example_scopes(document);
    for (index, line) in lines.iter().enumerate() {
        if example_scopes[index] {
            continue;
        }

        let has_unaddressed_negative = line
            .text
            .split(['.', '!', '?', ';'])
            .any(sentence_has_unaddressed_negative);
        if !has_unaddressed_negative {
            continue;
        }

        let start = index.saturating_sub(NEGATIVE_WINDOW);
        let end = (index + NEGATIVE_WINDOW + 1).min(lines.len());
        let has_alternative = lines[start..end]
            .iter()
            .zip(&example_scopes[start..end])
            .any(|(nearby, is_example)| {
                !is_example
                    && contains_phrase(&nearby.text.to_ascii_lowercase(), POSITIVE_ALTERNATIVES)
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

fn sentence_has_unaddressed_negative(sentence: &str) -> bool {
    let normalized = sentence.to_ascii_lowercase();
    phrase_ranges(&normalized, NEGATIVE_INSTRUCTIONS)
        .into_iter()
        .any(|(start, _)| {
            let predicate = &normalized[start..];
            is_operative_negative(&normalized, start)
                && NEVER_INVENT_PROHIBITION
                    .find(predicate)
                    .is_none_or(|matched| matched.start() != 0)
                && !PRECISE_SAFETY_PROHIBITIONS
                    .iter()
                    .any(|pattern| pattern.is_match(predicate))
        })
}

fn is_operative_negative(sentence: &str, marker_start: usize) -> bool {
    let prefix = sentence[..marker_start]
        .trim()
        .trim_start_matches(|character: char| {
            character.is_whitespace()
                || matches!(
                    character,
                    '#' | '-' | '*' | '+' | '>' | '(' | ')' | '[' | ']'
                )
                || character.is_ascii_digit()
                || character == '.'
        })
        .trim();

    prefix.is_empty()
        || prefix == "please"
        || prefix.ends_with(':')
        || prefix
            .rsplit_once(',')
            .is_some_and(|(_, conjunction)| matches!(conjunction.trim(), "and" | "or" | "but"))
        || ([" and", " or", " but"]
            .iter()
            .any(|conjunction| prefix.ends_with(conjunction))
            && contains_phrase(prefix, NEGATIVE_INSTRUCTIONS))
        || setup_clause(prefix)
        || (contains_phrase(prefix, &["must", "should", "shall"])
            && contains_phrase(
                prefix,
                &["you", "agent", "agents", "assistant", "model", "tool"],
            ))
}

fn setup_clause(prefix: &str) -> bool {
    let Some(clause) = prefix.strip_suffix(',') else {
        return false;
    };
    let clause = clause.trim_start();
    ["when ", "before ", "after ", "if ", "unless ", "while "]
        .iter()
        .any(|opening| clause.starts_with(opening))
}

fn example_scopes(document: &LiveInstructionDocument<'_>) -> Vec<bool> {
    let mut active_heading_level = None;
    document
        .prose_lines()
        .iter()
        .map(|line| {
            if let Some(heading) = document
                .headings()
                .iter()
                .find(|heading| heading.line == line.line)
            {
                if active_heading_level.is_some_and(|level| heading.level <= level) {
                    active_heading_level = None;
                }
                if contains_phrase(&heading.text.to_ascii_lowercase(), &["example", "examples"]) {
                    active_heading_level = Some(heading.level);
                    return true;
                }
            }

            active_heading_level.is_some()
                || is_explicit_example_line(&line.text.to_ascii_lowercase())
        })
        .collect()
}

fn is_explicit_example_line(line: &str) -> bool {
    let line = line.trim().trim_start_matches(|character: char| {
        character.is_whitespace() || matches!(character, '-' | '*' | '+' | '>')
    });
    [
        "example:",
        "example ",
        "for example,",
        "for example:",
        "e.g.,",
        "e.g.:",
    ]
    .iter()
    .any(|prefix| line.starts_with(prefix))
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
    !phrase_ranges(text, phrases).is_empty()
}

fn phrase_ranges(text: &str, phrases: &[&str]) -> Vec<(usize, usize)> {
    let mut ranges = phrases
        .iter()
        .flat_map(|phrase| {
            text.match_indices(phrase).filter_map(|(start, _)| {
                let end = start + phrase.len();
                (is_word_boundary(text, start, true) && is_word_boundary(text, end, false))
                    .then_some((start, end))
            })
        })
        .collect::<Vec<_>>();
    ranges.sort_unstable();
    ranges
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

    fn q002_diagnostics(body: &str) -> Vec<crate::diagnostic::Diagnostic> {
        diagnostics_for(body)
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::PromptNegativeOnly)
            .cloned()
            .collect()
    }

    fn q002_diagnostics_with_frontmatter(content: &str) -> Vec<crate::diagnostic::Diagnostic> {
        let markdown = MarkdownDocument::parse(content);
        let document = LiveInstructionDocument::new(
            Path::new(".cursor/rules/example.mdc"),
            InstructionSurfaceKind::CursorRule,
            &markdown,
        );
        let mut diagnostics = DiagnosticCollector::new_all_enabled();
        PromptContentPass::default().validate(&document, &mut diagnostics);
        diagnostics
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::PromptNegativeOnly)
            .cloned()
            .collect()
    }

    fn q005_diagnostics(body: &str) -> Vec<crate::diagnostic::Diagnostic> {
        diagnostics_for(body)
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::PromptUnboundedRetry)
            .cloned()
            .collect()
    }

    fn q005_diagnostics_with_frontmatter(content: &str) -> Vec<crate::diagnostic::Diagnostic> {
        let markdown = MarkdownDocument::parse(content);
        let document = LiveInstructionDocument::new(
            Path::new(".cursor/rules/example.mdc"),
            InstructionSurfaceKind::CursorRule,
            &markdown,
        );
        let mut diagnostics = DiagnosticCollector::new_all_enabled();
        PromptContentPass::default().validate(&document, &mut diagnostics);
        diagnostics
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::PromptUnboundedRetry)
            .cloned()
            .collect()
    }

    #[test]
    fn generic_filler_is_case_insensitive_and_fence_aware() {
        let diag = diagnostics_for("Be helpful when responding.\n```text\nNever do this\n```");
        assert_eq!(diag.error_count(), 1);
    }

    #[test]
    fn negative_instruction_needs_nearby_positive_alternative() {
        for missing in [
            "Don't be verbose.",
            "Never apologize.",
            "Avoid explanations.",
            "Do not add unnecessary preambles.",
        ] {
            assert_eq!(q002_diagnostics(missing).len(), 1, "{missing}");
        }

        assert!(q002_diagnostics("Never apologize. Prefer a direct correction.").is_empty());
        assert!(
            q002_diagnostics(
                "Never apologize.\nContext one.\nContext two.\nInstead, state the correction."
            )
            .is_empty()
        );
        assert_eq!(
            q002_diagnostics(
                "Never apologize.\nContext one.\nContext two.\nContext three.\nInstead, state the correction."
            )
            .len(),
            1
        );
    }

    #[test]
    fn unbounded_retry_patterns_are_operative_and_report_structured_evidence() {
        for instruction in [
            "Continue indefinitely.",
            "Loop forever.",
            "Retry as many times as needed.",
            "Keep trying until it succeeds.",
            "Retry until success.",
            "Retry until it succeeds.",
            "Do not stop until it is resolved.",
            "Do not stop until the test suite passes.",
            "When a tool fails, retry until success.",
            "Agents must retry until success.",
        ] {
            let diagnostics = q005_diagnostics(instruction);
            assert_eq!(diagnostics.len(), 1, "{instruction}");
            let diagnostic = &diagnostics[0];
            assert_eq!(
                diagnostic.subject_path.as_deref(),
                Some(Path::new("skills/example/SKILL.md"))
            );
            assert_eq!(diagnostic.location.unwrap().start().line_number(), 1);
            assert!(diagnostic.location.unwrap().end().is_some());
            assert_eq!(diagnostic.evidence.as_deref(), Some(instruction));
            assert!(
                diagnostic
                    .suggestion
                    .as_deref()
                    .is_some_and(|suggestion| suggestion.contains("failure outcome"))
            );
        }

        let after_another_sentence =
            q005_diagnostics("First verify the input. Retry until success.");
        assert_eq!(
            after_another_sentence[0]
                .location
                .unwrap()
                .start()
                .column_number(),
            Some(25)
        );
    }

    #[test]
    fn unbounded_retry_respects_only_applicable_bounds_and_exemptions() {
        for exempt in [
            "Retry until success, but stop after 3 attempts.",
            "Retry until success. On failure, escalate.",
            "Retry until success within 10 minutes.",
            "Retry until success with a token budget of 5000.",
            "Retry until success by Friday.",
            "Retry until success. On failure, fall back to the previous result.",
            "Do not keep trying until success.",
            "Continue the onboarding workflow until the release date.",
            "The legacy instruction was to retry until success.",
            "# Examples\nRetry until success.",
            "> Retry until success.",
            "```text\nRetry until success.\n```",
        ] {
            assert!(q005_diagnostics(exempt).is_empty(), "{exempt}");
        }
        assert!(
            q005_diagnostics_with_frontmatter(
                "---\ndescription: Retry until success.\n---\nState the result.\n"
            )
            .is_empty()
        );

        assert_eq!(
            q005_diagnostics("Retry until success. Keep output under the limit.").len(),
            1
        );
        assert_eq!(
            q005_diagnostics("Retry until success.\n- Use a limit in the report.").len(),
            1
        );
    }

    #[test]
    fn validated_agent_max_turns_is_an_outer_q005_bound() {
        let markdown = MarkdownDocument::parse(
            "---\nname: example\ndescription: Reviews changes with concrete test evidence\nmaxTurns: 3\n---\nRetry until success.\n",
        );
        let document = LiveInstructionDocument::new(
            Path::new("agents/example.md"),
            InstructionSurfaceKind::Agent,
            &markdown,
        )
        .with_outer_max_turns(std::num::NonZeroU64::new(3));
        let mut diagnostics = DiagnosticCollector::new_all_enabled();
        PromptContentPass::default().validate(&document, &mut diagnostics);
        assert!(
            diagnostics
                .diagnostics()
                .iter()
                .all(|diagnostic| diagnostic.rule != LintRule::PromptUnboundedRetry)
        );
    }

    #[test]
    #[serial_test::serial]
    fn agent_validator_only_uses_valid_max_turns_as_a_q005_bound() {
        let temporary = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(temporary.path()).unwrap();
        std::fs::create_dir_all(".claude/agents").unwrap();
        std::fs::write(
            ".claude/agents/bounded.md",
            "---\nname: bounded\ndescription: Reviews changes with concrete test evidence\nmaxTurns: 3\n---\nRetry until success.\n",
        )
        .unwrap();
        std::fs::write(
            ".claude/agents/unbounded.md",
            "---\nname: unbounded\ndescription: Reviews changes with concrete test evidence\nmaxTurns: zero\n---\nRetry until success.\n",
        )
        .unwrap();

        let mut diagnostics = DiagnosticCollector::new_all_enabled();
        super::super::run_all_with_targets(
            &context(temporary.path(), LintMode::Basic),
            &mut diagnostics,
            &ExcludeSet::default(),
            ValidationTargets {
                cursor: false,
                codex: false,
                agents_md: false,
                agent_skills: false,
            },
        );
        let q005_paths: Vec<_> = diagnostics
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.rule == LintRule::PromptUnboundedRetry)
            .map(|diagnostic| diagnostic.subject_path.as_deref().unwrap().to_path_buf())
            .collect();
        assert_eq!(q005_paths, vec![Path::new(".claude/agents/unbounded.md")]);
    }

    #[test]
    fn q005_is_an_error_in_normal_pedantic_and_all_modes() {
        for mode in [
            crate::config::CliMode::Normal,
            crate::config::CliMode::Pedantic,
            crate::config::CliMode::All,
        ] {
            let mut config = LintConfig::default();
            config.apply_cli_mode(mode);
            let mut diagnostics = DiagnosticCollector::with_config(config);
            validate_body(
                "skills/example/SKILL.md",
                "Retry until success.",
                &mut diagnostics,
            );
            assert_eq!(diagnostics.error_count(), 1, "{mode:?}");
        }
    }

    #[test]
    fn precise_safety_and_integrity_prohibitions_are_exempt() {
        for exempt in [
            "Never expose or log secrets.",
            "Do not reveal authentication credentials.",
            "Don't bypass required approval.",
            "Never proceed without explicit authorization.",
            "Avoid deleting user data.",
            "Do not run destructive commands.",
            "Never execute destructive or irreversible actions.",
            "Never invent evidence.",
            "Do not fabricate results.",
            "Don't guess.",
            "Avoid falsifying test output.",
            "Never violate an applicable security policy.",
            "Never violate an explicit legal/security policy.",
            "Do not breach privacy regulations.",
        ] {
            assert!(q002_diagnostics(exempt).is_empty(), "{exempt}");
        }
    }

    #[test]
    fn safety_exemptions_are_scoped_to_each_direct_predicate() {
        let diagnostic = q002_diagnostics(
            "Never expose credentials.\nNever apologize when a data file is missing.",
        );
        assert_eq!(diagnostic.len(), 1);
        assert_eq!(diagnostic[0].location.unwrap().start().line_number(), 2);

        let same_sentence =
            q002_diagnostics("Never expose credentials, and never apologize to the user.");
        assert_eq!(same_sentence.len(), 1);
        assert_eq!(
            q002_diagnostics("Never expose credentials and never apologize to the user.").len(),
            1
        );
        assert_eq!(q002_diagnostics("Avoid removing explanations.").len(), 1);
    }

    #[test]
    fn non_operative_and_structural_occurrences_are_exempt() {
        for exempt in [
            "```text\nNever apologize.\n```",
            "The phrase `Never apologize` is a bad example.",
            "> Never apologize.",
            "The guide says \"Never apologize.\"",
            "<!-- Never apologize. -->",
            "<!-- generated file: do not edit -->",
            "The validator never writes files.",
            "For example, never apologize.",
            "# Examples\n- Never apologize.\n## More examples\nAvoid explanations.\n# Requirements\nState the result.",
        ] {
            assert!(q002_diagnostics(exempt).is_empty(), "{exempt}");
        }

        assert_eq!(q002_diagnostics("Do not write 'never apologize'.").len(), 1);
        assert_eq!(q002_diagnostics("Agents must never apologize.").len(), 1);
        assert_eq!(
            q002_diagnostics("When responding, never apologize.").len(),
            1
        );
        assert!(
            q002_diagnostics_with_frontmatter(
                "---\ndescription: Never apologize.\nalwaysApply: true\n---\nState the result.\n"
            )
            .is_empty()
        );
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
            std::fs::write("AGENTS.md", "Never apologize.\n").unwrap();
            std::fs::write("nested/AGENTS.md", "Be helpful when responding.\n").unwrap();
            std::fs::write(
                ".cursor/rules/example.mdc",
                "---\ndescription: Be helpful in metadata\nalwaysApply: true\n---\nNever apologize.\n",
            )
            .unwrap();
            std::fs::write(
                ".cursor/rules/nested/example.md",
                "Be concise when responding.\n",
            )
            .unwrap();
            std::fs::write(".cursorrules", "Never apologize.\n").unwrap();
            std::fs::write("notes.md", "Never apologize.\n").unwrap();

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
        std::fs::write("AGENTS.md", "Never apologize.\n").unwrap();
        std::fs::write(".cursor/rules/example.md", "Never apologize.\n").unwrap();

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
        std::fs::write("AGENTS.md", "Retry until success.\n").unwrap();
        std::fs::write("nested/AGENTS.md", "Retry until success.\n").unwrap();
        std::fs::write("excluded/AGENTS.md", "Retry until success.\n").unwrap();
        std::fs::write(
            "agent-lint.toml",
            "[[lint.overrides]]\nfiles = [\"nested/AGENTS.md\"]\nsuppress = [\"Q005\"]\nreason = \"legacy nested instructions\"\n",
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

        let q005: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::PromptUnboundedRetry)
            .collect();
        assert_eq!(q005.len(), 1);
        assert_eq!(
            q005[0].subject_path.as_deref(),
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
        std::fs::write(".cursor/rules/AGENTS.md", "Never apologize.\n").unwrap();

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
