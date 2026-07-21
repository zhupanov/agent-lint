//! Shared prompt-content checks for live instruction documents.
//!
//! These checks intentionally inspect prose only. Code fences frequently contain
//! examples of wording that should not be treated as live instructions.

use crate::config::ExcludeSet;
use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata, SourceSpan};
use crate::live_instructions::{InstructionSurfaceKind, LiveInstructionDocument};
use crate::markdown::MarkdownDocument;
use crate::rules::LintRule;
use crate::traversal;
use crate::validators::common::{NEVER_INVENT_PROHIBITION, has_bound_or_fallback, sentence_ranges};
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
    r"\bretry\s+indefinitely\b",
    r"\bretry\s+as\s+many\s+times\s+as\s+(?:needed|necessary)\b",
    r"\b(?:keep\s+(?:retrying|trying)|retry|try\s+again|repeat|continue)\s+until\s+(?:success|(?:(?:it|the\s+(?:build|tests?|task|operation|command|tool\s+call|test\s+suite))\s+)?(?:succeeds|pass(?:es)?|works|is\s+(?:complete|completed|resolved)))\b",
    r"\bdo\s+not\s+(?:give\s+up|stop)\s+until\s+(?:it\s+)?(?:(?:the\s+)?(?:build|tests?|task|operation|command|tool\s+call|test\s+suite)\s+)?(?:succeeds|pass(?:es)?|works|is\s+(?:complete|completed|resolved))\b",
];

static UNBOUNDED_RETRY_REGEXES: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    UNBOUNDED_RETRY_PATTERNS
        .iter()
        .map(|pattern| Regex::new(pattern).expect("Q005 retry pattern is valid"))
        .collect()
});

static OPERATIVE_RETRY_SETUP_CLAUSE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:when|if|after|before)\b[^,]*,\s*(?:continue|loop|retry|keep (?:trying|retrying)|try again|repeat|do not (?:give up|stop))\b",
    )
    .expect("Q005 setup-clause pattern is valid")
});

static OPERATIVE_RETRY_SUBJECT_DIRECTIVE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:the\s+)?(?:you|agents?|assistant|model)\s+(?:must|should|shall|will)\s+(?:continue|loop|retry|keep (?:trying|retrying)|try again|repeat|not (?:give up|stop))\b",
    )
    .expect("Q005 subject directive pattern is valid")
});

static UNBOUNDED_RETRY_PROHIBITION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:do\s+not|don't|never|avoid)\s+(?:keep\s+trying|retrying|retry|continuing|looping)\b",
    )
    .expect("Q005 prohibition pattern is valid")
});

static EMPHASIS_LABEL_PREFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:important|note|warning)\s*:\s*").expect("Q005 emphasis-label regex is valid")
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
            check_output_conflict(&path, document, diag);
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

    let example_scopes = document.example_scopes();
    for scope in retry_instruction_scopes(document, &example_scopes) {
        let joined_scope = JoinedProseScope::new(&scope);
        if has_bound_or_fallback(&joined_scope.text) {
            continue;
        }

        for sentence_range in sentence_ranges(&joined_scope.text) {
            let sentence = &joined_scope.text[sentence_range.clone()];
            let normalized_sentence = sentence.to_ascii_lowercase();
            if !is_operative_retry_instruction(&normalized_sentence)
                || explicitly_prohibits_unbounded_retry(&normalized_sentence)
            {
                continue;
            }
            let Some(matched) = UNBOUNDED_RETRY_REGEXES
                .iter()
                .find_map(|pattern| pattern.find(&normalized_sentence))
            else {
                continue;
            };

            let matched_range =
                sentence_range.start + matched.start()..sentence_range.start + matched.end();
            let (start_line, start_column) = joined_scope.position_at(matched_range.start);
            let (end_line, end_column) = joined_scope.position_at(matched_range.end);
            diag.report_with(
                LintRule::PromptUnboundedRetry,
                &format!(
                    "{path}: unbounded retry or continuation instruction; add an explicit bound or concrete failure outcome"
                ),
                DiagnosticMetadata::default()
                    .with_location(SourceSpan::range(
                        start_line,
                        start_column,
                        end_line,
                        end_column,
                    ))
                    .with_evidence(sentence.trim())
                    .with_suggestion(
                        "Add an explicit attempt, step, tool-call, timeout, token/cost budget, deadline, or concrete failure outcome.",
                    ),
            );
        }
    }
}

/// Prose lines joined exactly as the sentence parser sees them, retaining the
/// original source coordinates for structured Q005 diagnostics.
struct JoinedProseScope<'a> {
    text: String,
    segments: Vec<(
        std::ops::Range<usize>,
        &'a crate::markdown::MarkdownProseLine,
    )>,
}

impl<'a> JoinedProseScope<'a> {
    fn new(lines: &[&'a crate::markdown::MarkdownProseLine]) -> Self {
        let mut text = String::new();
        let mut segments = Vec::with_capacity(lines.len());
        for line in lines {
            if !text.is_empty() {
                text.push(' ');
            }
            let start = text.len();
            text.push_str(&line.text);
            segments.push((start..text.len(), *line));
        }
        Self { text, segments }
    }

    fn position_at(&self, offset: usize) -> (usize, usize) {
        let (range, line) = self
            .segments
            .iter()
            .find(|(range, _)| range.start <= offset && offset <= range.end)
            .expect("Q005 match offset belongs to a source prose line");
        let local_offset = offset.saturating_sub(range.start).min(line.text.len());
        (line.line, line.text[..local_offset].chars().count() + 1)
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
    let mut sentence = sentence.trim().trim_start_matches(|character: char| {
        character.is_whitespace()
            || matches!(character, '#' | '-' | '*' | '+' | '(' | ')' | '[' | ']')
            || character.is_ascii_digit()
            || character == '.'
    });
    sentence = sentence.trim_start_matches("always ");
    if let Some(after_label) = EMPHASIS_LABEL_PREFIX.find(sentence) {
        sentence = &sentence[after_label.end()..];
    }
    let directive = [
        "continue",
        "loop",
        "retry",
        "keep trying",
        "keep retrying",
        "try again",
        "repeat",
        "do not give up",
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

/// Run the shared body checks for every included `CLAUDE.md`. Only the root
/// file participates in the root-README overlap check.
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
    for entry in traversal::recursive_files(Path::new("."), Path::new("."), Some(exclude)).entries {
        if entry
            .path
            .file_name()
            .is_none_or(|name| name != "CLAUDE.md")
        {
            continue;
        }
        let path = entry.display;
        let Ok(claude) = fs::read_to_string(&entry.path) else {
            continue;
        };
        let markdown = MarkdownDocument::parse_body(&claude);
        let document = LiveInstructionDocument::new(
            Path::new(&path),
            InstructionSurfaceKind::ClaudeProject,
            &markdown,
        );
        diag.with_subject_path(&path, |diag| {
            prompt_pass.validate(&document, diag);

            if path != "CLAUDE.md" {
                return;
            }
            let Ok(readme) = fs::read_to_string("README.md") else {
                return;
            };
            check_readme_overlap(&claude, &readme, diag);
        });
    }
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
    let example_scopes = document.example_scopes();
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

// ── Q006: mechanically incompatible output instructions ──────────────────
//
// Each operative output directive is modeled as a typed constraint and only
// mechanically incompatible constraints that share one response scope are
// reported. The rule never counts raw format keywords: a document may mention
// many formats as long as no two exclusive operative requirements collide in
// the same section. Conditional routing, examples, quoted/fenced text, and
// separately headed response modes are excluded before comparison.

/// A response output format recognized by the exclusive-format conflict class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum OutputFormat {
    Json,
    Markdown,
    Xml,
    Yaml,
    Html,
    Csv,
    PlainText,
}

impl OutputFormat {
    fn label(self) -> &'static str {
        match self {
            Self::Json => "JSON",
            Self::Markdown => "Markdown",
            Self::Xml => "XML",
            Self::Yaml => "YAML",
            Self::Html => "HTML",
            Self::Csv => "CSV",
            Self::PlainText => "plain text",
        }
    }

    fn sort_rank(self) -> u8 {
        match self {
            Self::Json => 0,
            Self::Markdown => 1,
            Self::Xml => 2,
            Self::Yaml => 3,
            Self::Html => 4,
            Self::Csv => 5,
            Self::PlainText => 6,
        }
    }
}

/// A response unit recognized by the size/shape conflict class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum ShapeUnit {
    Character,
    Word,
    Sentence,
    Paragraph,
    Line,
}

impl ShapeUnit {
    fn label(self) -> &'static str {
        match self {
            Self::Character => "character",
            Self::Word => "word",
            Self::Sentence => "sentence",
            Self::Paragraph => "paragraph",
            Self::Line => "line",
        }
    }

    /// Rank in the containment hierarchy character < word < sentence <
    /// paragraph. `Line` does not nest cleanly and only participates in
    /// same-unit comparisons.
    fn nesting_rank(self) -> Option<u8> {
        match self {
            Self::Character => Some(0),
            Self::Word => Some(1),
            Self::Sentence => Some(2),
            Self::Paragraph => Some(3),
            Self::Line => None,
        }
    }

    fn sort_rank(self) -> u8 {
        match self {
            Self::Character => 0,
            Self::Word => 1,
            Self::Sentence => 2,
            Self::Paragraph => 3,
            Self::Line => 4,
        }
    }
}

/// A response size/shape bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum ShapeBound {
    Exactly(u32),
    AtLeast(u32),
    AtMost(u32),
}

impl ShapeBound {
    /// The smallest count this bound permits.
    fn floor(self) -> u32 {
        match self {
            Self::Exactly(count) | Self::AtLeast(count) => count,
            Self::AtMost(_) => 0,
        }
    }

    /// The largest count this bound permits, or `None` when unbounded above.
    fn cap(self) -> Option<u32> {
        match self {
            Self::Exactly(count) | Self::AtMost(count) => Some(count),
            Self::AtLeast(_) => None,
        }
    }

    fn label(self) -> String {
        match self {
            Self::Exactly(count) => format!("exactly {count}"),
            Self::AtLeast(count) => format!("at least {count}"),
            Self::AtMost(count) => format!("at most {count}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct FormatConstraint {
    format: OutputFormat,
    exclusive: bool,
}

/// One typed operative output requirement located in the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum OutputRequirement {
    Format(FormatConstraint),
    Shape { unit: ShapeUnit, bound: ShapeBound },
}

impl OutputRequirement {
    /// A stable typed key used only to break source-position ties. Keeping it
    /// independent of display labels prevents wording changes from reordering
    /// machine-observable diagnostics.
    fn stable_identity(self) -> (u8, u8, u8, u32) {
        match self {
            Self::Format(FormatConstraint { format, exclusive }) => {
                (0, format.sort_rank(), u8::from(exclusive), 0)
            }
            Self::Shape { unit, bound } => match bound {
                ShapeBound::Exactly(count) => (1, unit.sort_rank(), 0, count),
                ShapeBound::AtLeast(count) => (1, unit.sort_rank(), 1, count),
                ShapeBound::AtMost(count) => (1, unit.sort_rank(), 2, count),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClauseClassification {
    /// An imperative or explicit constraint on the agent's own response.
    OperativeOutput,
    /// A terse, otherwise-subjectless shape instruction such as "Exactly one
    /// sentence." It is accepted only when the whole clause is the mandate.
    StandaloneShapeMandate,
    /// Descriptions of inputs, requests, source data, examples, or third
    /// parties, which never contribute Q006 constraints.
    NonOutput,
}

struct DetectedRequirement {
    line: usize,
    column: usize,
    scope: Option<usize>,
    requirement: OutputRequirement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ConflictCategory {
    Format,
    Shape,
}

const OUTPUT_FORMATS: &[(&str, OutputFormat)] = &[
    ("json", OutputFormat::Json),
    ("markdown", OutputFormat::Markdown),
    ("xml", OutputFormat::Xml),
    ("yaml", OutputFormat::Yaml),
    ("html", OutputFormat::Html),
    ("csv", OutputFormat::Csv),
    ("plain text", OutputFormat::PlainText),
    ("plaintext", OutputFormat::PlainText),
];

/// Verbs that mark a clause as an operative output instruction rather than a
/// passing mention of a format.
const OUTPUT_DIRECTIVE_VERBS: &[&str] = &[
    "respond", "reply", "answer", "return", "output", "produce", "emit", "print", "render",
    "format", "write", "provide", "give",
];

/// Additional imperative verbs that unambiguously constrain a response's
/// shape, but do not imply a format requirement on their own.
const SHAPE_DIRECTIVE_VERBS: &[&str] = &["include", "use"];

/// Directive phrases that frame the whole response as one format and therefore
/// make the requirement exclusive on their own.
const WHOLE_RESPONSE_DIRECTIVES: &[&str] = &[
    "respond in",
    "respond with",
    "respond using",
    "respond only in",
    "respond only with",
    "reply in",
    "reply with",
    "reply using",
    "answer in",
    "answer with",
    "answer using",
    "format your response",
    "format the response",
    "format your output",
    "format the output",
    "your response must be",
    "your output must be",
    "the response must be",
    "the output must be",
    "response must be",
    "output must be",
    "write your response",
    "return your response",
];

const EXCLUSIVITY_MARKERS: &[&str] = &[
    "only",
    "exclusively",
    "solely",
    "nothing but",
    "nothing else",
];

/// Words that introduce conditional routing when they lead a sentence. Matching
/// these only at the head (plus the narrow subordinate markers below) keeps
/// "For data requests ..." style delineation out of the conflict set without the
/// over-broad effect of dropping any sentence that merely contains `for` or `or`.
const CONDITIONAL_LEAD_INS: &[&str] = &[
    "if",
    "when",
    "whenever",
    "unless",
    "for",
    "where",
    "wherever",
    "given",
    "once",
    "depending",
    "otherwise",
    "either",
    "assuming",
    "provided",
    "should",
];

/// Multi-word conditional lead-ins checked against the head of a sentence.
const CONDITIONAL_LEAD_PHRASES: &[&str] = &["in case", "only if", "as long as", "in the case"];

/// Sentence-leading `In <X> mode` / `In <X> format` routing has the same
/// conditional meaning as the established `When` and `For` forms. The
/// vocabulary is deliberately limited to these routing nouns (plus `case`),
/// rather than treating arbitrary prepositional phrases as conditions.
const PREPOSITIONAL_CONDITION_NOUNS: &[&str] = &["mode", "format", "case"];

/// Subordinate-clause markers that make an instruction conditional even when it
/// does not lead the sentence. Kept deliberately narrow (surrounded by spaces)
/// so common prepositions do not silently drop real conflicts.
const CONDITIONAL_CLAUSE_MARKERS: &[&str] = &[
    " if ",
    " when ",
    " whenever ",
    " unless ",
    " otherwise ",
    " depending on ",
    " based on ",
    " as needed",
    " where ",
];

/// Adverbs, politeness words, and an optional second-person subject/modal that
/// may precede an imperative directive verb without changing that the clause is
/// an operative instruction.
const IMPERATIVE_LEAD_SKIP: &[&str] = &[
    "please", "always", "then", "first", "next", "finally", "also", "only", "kindly", "simply",
    "just", "you", "must", "should", "shall", "will", "can", "may",
];

/// Explicit references to the agent's own response, which mark a non-imperative
/// clause as still constraining output.
const AGENT_OUTPUT_SUBJECTS: &[&str] = &[
    "your response",
    "your output",
    "your reply",
    "your answer",
    "the response must",
    "the response should",
    "the output must",
    "the output should",
];

/// Label forms are operative only at the beginning of a clause. Keeping them
/// separate from prose subjects prevents `Bad output: ...` from becoming an
/// instruction merely because it contains the substring `output:`.
const AGENT_OUTPUT_LABELS: &[&str] = &["response:", "output:", "answer:", "reply:"];

/// A conservative, typed vocabulary for artifacts that are not the agent's
/// response. Q006 must not reinterpret instructions about these artifacts as
/// whole-response format or shape requirements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NonResponseArtifact {
    CommitMessage,
    PullRequestDescription,
    IssueDescription,
    Changelog,
    Documentation,
    File,
    Filename,
    Log,
    NamedPath,
}

impl NonResponseArtifact {
    fn phrases(self) -> &'static [&'static str] {
        match self {
            Self::CommitMessage => &["commit message", "commit messages"],
            Self::PullRequestDescription => &[
                "pull request description",
                "pull request descriptions",
                "pr description",
                "pr descriptions",
            ],
            Self::IssueDescription => &["issue description", "issue descriptions"],
            Self::Changelog => &["changelog"],
            Self::Documentation => &["documentation", "document", "documents"],
            Self::File => &["file", "files", "export file"],
            Self::Filename => &["filename", "filenames", "file name", "file names"],
            Self::Log => &["log", "logs"],
            Self::NamedPath => &[],
        }
    }
}

const NON_RESPONSE_ARTIFACTS: &[NonResponseArtifact] = &[
    NonResponseArtifact::CommitMessage,
    NonResponseArtifact::PullRequestDescription,
    NonResponseArtifact::IssueDescription,
    NonResponseArtifact::Changelog,
    NonResponseArtifact::Documentation,
    NonResponseArtifact::File,
    NonResponseArtifact::Filename,
    NonResponseArtifact::Log,
    NonResponseArtifact::NamedPath,
];

/// Markers that turn an otherwise agent-output-looking clause into a
/// description of a past response rather than a live instruction.
const HISTORICAL_OUTPUT_REFERENCES: &[&str] = &[
    "previous response",
    "prior response",
    "historical response",
    "earlier response",
    "last response",
    "previous output",
    "prior output",
    "historical output",
    "earlier output",
    "last output",
    "response from the previous",
    "output from the previous",
];

const SENTENCE_DELIMITERS: &[char] = &['.', '!', '?', ';'];
const CLAUSE_DELIMITERS: &[char] = &[','];

#[derive(Debug, Clone, Copy)]
enum ShapeBoundKind {
    AtLeast,
    AtMost,
    AtMostStrict,
    Exactly,
    ExactlyOne,
}

static SHAPE_PATTERNS: LazyLock<Vec<(Regex, ShapeBoundKind)>> = LazyLock::new(|| {
    const NUM: &str = r"(?:\d+|one|two|three|four|five|six|seven|eight|nine|ten)";
    const UNIT: &str = r"(?P<u>words?|sentences?|paragraphs?|lines?|characters?|chars?)";
    let anchored = |quantifier: &str| {
        Regex::new(&format!(r"(?i)\b(?:{quantifier})\s+(?P<n>{NUM})\s+{UNIT}"))
            .expect("Q006 shape pattern is valid")
    };
    vec![
        (
            anchored("at least|no fewer than|no less than|at a minimum of|a minimum of|minimum of"),
            ShapeBoundKind::AtLeast,
        ),
        (
            Regex::new(&format!(r"(?i)\b(?P<n>{NUM})\s+or\s+more\s+{UNIT}"))
                .expect("Q006 shape pattern is valid"),
            ShapeBoundKind::AtLeast,
        ),
        (
            anchored("at most|no more than|up to|at a maximum of|a maximum of|maximum of"),
            ShapeBoundKind::AtMost,
        ),
        (
            Regex::new(&format!(r"(?i)\b(?P<n>{NUM})\s+or\s+(?:fewer|less)\s+{UNIT}"))
                .expect("Q006 shape pattern is valid"),
            ShapeBoundKind::AtMost,
        ),
        (
            anchored("fewer than|less than|under"),
            ShapeBoundKind::AtMostStrict,
        ),
        (anchored("exactly|precisely"), ShapeBoundKind::Exactly),
        (
            Regex::new(&format!(
                r"(?i)\b(?:a single|one single|in a single|just one|only one|exactly one|precisely one|in one|to one)\s+{UNIT}"
            ))
            .expect("Q006 shape pattern is valid"),
            ShapeBoundKind::ExactlyOne,
        ),
    ]
});

#[derive(Debug, Clone, Copy)]
struct RecognizedShapeConstraint {
    start: usize,
    end: usize,
    unit: ShapeUnit,
    bound: ShapeBound,
}

fn check_output_conflict(
    path: &str,
    document: &LiveInstructionDocument<'_>,
    diag: &mut DiagnosticCollector,
) {
    let lines = document.prose_lines();
    let example = document.example_scopes();
    let mut requirements: Vec<DetectedRequirement> = Vec::new();

    for (index, line) in lines.iter().enumerate() {
        if example[index] {
            continue;
        }
        // Heading lines define response scopes; they are never operative output
        // instructions themselves.
        if document
            .headings()
            .iter()
            .any(|heading| heading.line == line.line)
        {
            continue;
        }
        let scope = scope_of(document, line.line);
        for (sentence_offset, sentence) in split_on(&line.text, SENTENCE_DELIMITERS) {
            // Conditionality is judged on the whole sentence first so a leading
            // "For X, ..." condition still guards the directive that follows it.
            if sentence_is_conditional(sentence) {
                continue;
            }
            for (clause_offset, clause) in split_on(sentence, CLAUSE_DELIMITERS) {
                if sentence_is_conditional(clause) {
                    continue;
                }
                for (requirement_offset, requirement) in detect_requirements(clause) {
                    requirements.push(DetectedRequirement {
                        line: line.line,
                        column: line.text[..sentence_offset + clause_offset + requirement_offset]
                            .chars()
                            .count()
                            + 1,
                        scope,
                        requirement,
                    });
                }
            }
        }
    }

    report_output_conflicts(path, &requirements, diag);
}

fn detect_requirements(clause: &str) -> Vec<(usize, OutputRequirement)> {
    let shapes = shape_constraints(clause);
    match classify_clause(clause, &shapes) {
        ClauseClassification::NonOutput => Vec::new(),
        ClauseClassification::OperativeOutput => {
            let mut requirements = Vec::new();
            if let Some(format) = format_constraint(clause) {
                requirements.push((0, OutputRequirement::Format(format)));
            }
            requirements.extend(shapes.into_iter().map(|shape| {
                (
                    shape.start,
                    OutputRequirement::Shape {
                        unit: shape.unit,
                        bound: shape.bound,
                    },
                )
            }));
            requirements
        }
        ClauseClassification::StandaloneShapeMandate => shapes
            .into_iter()
            .map(|shape| {
                (
                    shape.start,
                    OutputRequirement::Shape {
                        unit: shape.unit,
                        bound: shape.bound,
                    },
                )
            })
            .collect(),
    }
}

/// Classify the semantic subject of a clause before turning any numeric shape
/// phrase into a constraint. This is deliberately separate from bound parsing:
/// the same phrase can describe a request or input rather than the response.
fn classify_clause(clause: &str, shapes: &[RecognizedShapeConstraint]) -> ClauseClassification {
    let lower = clause.to_ascii_lowercase();
    if clause_states_output_directive(&lower) || clause_states_shape_directive(&lower) {
        return ClauseClassification::OperativeOutput;
    }
    if shapes.len() == 1 && standalone_shape_mandate(clause, shapes[0]) {
        return ClauseClassification::StandaloneShapeMandate;
    }
    ClauseClassification::NonOutput
}

fn format_constraint(clause: &str) -> Option<FormatConstraint> {
    let lower = clause.to_ascii_lowercase();
    // Only an imperative directive, or an explicit constraint on the agent's own
    // response, is an operative output requirement. This excludes input-format
    // descriptions ("Users reply in plain text") and label/condition-prefixed
    // routing ("Data requests: respond in JSON") even though both name a verb.
    if !clause_states_output_directive(&lower) {
        return None;
    }
    let mut formats: Vec<OutputFormat> = Vec::new();
    for &(token, format) in OUTPUT_FORMATS {
        if contains_phrase(&lower, &[token]) && !formats.contains(&format) {
            formats.push(format);
        }
    }
    // Exactly one distinct format keeps "respond in JSON or Markdown" and other
    // multi-format wording out of the exclusive-conflict class.
    if formats.len() != 1 {
        return None;
    }
    let exclusive = contains_phrase(&lower, EXCLUSIVITY_MARKERS)
        || contains_phrase(&lower, WHOLE_RESPONSE_DIRECTIVES);
    Some(FormatConstraint {
        format: formats[0],
        exclusive,
    })
}

/// Whether a lowercased clause states an operative output requirement: it is
/// imperative (its first significant word is a directive verb, allowing leading
/// adverbs and an optional `you [modal]` subject) or it explicitly constrains
/// the agent's own response.
fn clause_states_output_directive(lower: &str) -> bool {
    (clause_starts_with_directive(lower, OUTPUT_DIRECTIVE_VERBS)
        && !directive_targets_non_response_artifact(lower, OUTPUT_DIRECTIVE_VERBS))
        || clause_explicitly_constrains_agent_output(lower)
}

fn clause_states_shape_directive(lower: &str) -> bool {
    (clause_starts_with_directive(lower, SHAPE_DIRECTIVE_VERBS)
        && !directive_targets_non_response_artifact(lower, SHAPE_DIRECTIVE_VERBS))
        || clause_explicitly_constrains_agent_output(lower)
}

fn clause_starts_with_directive(lower: &str, directive_verbs: &[&str]) -> bool {
    let lead = lower.trim_start_matches(|character: char| {
        character.is_whitespace()
            || matches!(character, '#' | '-' | '*' | '+' | '>' | ')')
            || character.is_ascii_digit()
            || character == '.'
    });
    let mut words = lead.split_whitespace().peekable();
    while words
        .peek()
        .is_some_and(|word| IMPERATIVE_LEAD_SKIP.contains(word))
    {
        words.next();
    }
    if words
        .peek()
        .is_some_and(|word| directive_verbs.contains(word))
    {
        return true;
    }
    false
}

fn directive_targets_non_response_artifact(lower: &str, directive_verbs: &[&str]) -> bool {
    let Some(tail) = imperative_directive_tail(lower, directive_verbs) else {
        return false;
    };
    artifact_starts(tail)
        || [" in ", " into ", " to "]
            .iter()
            .filter_map(|marker| tail.find(marker).map(|index| &tail[index + marker.len()..]))
            .any(artifact_starts)
}

fn imperative_directive_tail<'a>(lower: &'a str, directive_verbs: &[&str]) -> Option<&'a str> {
    let lead = lower.trim_start_matches(|character: char| {
        character.is_whitespace()
            || matches!(character, '#' | '-' | '*' | '+' | '>' | ')')
            || character.is_ascii_digit()
            || character == '.'
    });
    let mut remaining = lead;
    while !remaining.is_empty() {
        remaining = remaining.trim_start();
        let word_end = remaining
            .find(char::is_whitespace)
            .unwrap_or(remaining.len());
        let word = &remaining[..word_end];
        let tail = &remaining[word_end..];
        if IMPERATIVE_LEAD_SKIP.contains(&word) {
            remaining = tail;
            continue;
        }
        if directive_verbs.contains(&word) {
            return Some(tail.trim_start());
        }
        return None;
    }
    None
}

fn artifact_starts(text: &str) -> bool {
    let text = text.trim_start();
    let text = text
        .strip_prefix("the ")
        .or_else(|| text.strip_prefix("a "))
        .or_else(|| text.strip_prefix("an "))
        .or_else(|| text.strip_prefix("your "))
        .unwrap_or(text);
    NON_RESPONSE_ARTIFACTS
        .iter()
        .any(|artifact| match artifact {
            NonResponseArtifact::NamedPath => starts_with_named_path(text),
            _ => artifact
                .phrases()
                .iter()
                .any(|phrase| starts_with_words(text, phrase)),
        })
}

fn starts_with_named_path(text: &str) -> bool {
    let first = text.split_whitespace().next().unwrap_or_default();
    first.contains('/')
        || first.contains('\\')
        || first.ends_with(".md")
        || first.ends_with(".mdx")
        || first.ends_with(".json")
        || first.ends_with(".toml")
        || matches!(first, "changelog" | "readme" | "agents.md" | "claude.md")
}

fn starts_with_words(text: &str, phrase: &str) -> bool {
    text == phrase
        || text.strip_prefix(phrase).is_some_and(|suffix| {
            suffix.starts_with(char::is_whitespace) || suffix.starts_with(',')
        })
}

fn clause_explicitly_constrains_agent_output(lower: &str) -> bool {
    !HISTORICAL_OUTPUT_REFERENCES
        .iter()
        .any(|reference| lower.contains(reference))
        && (AGENT_OUTPUT_SUBJECTS
            .iter()
            .any(|subject| lower.contains(subject))
            || clause_starts_with_agent_output_label(lower))
}

fn clause_starts_with_agent_output_label(lower: &str) -> bool {
    let lead = lower.trim_start_matches(|character: char| {
        character.is_whitespace()
            || matches!(character, '#' | '-' | '*' | '+' | '>' | ')')
            || character.is_ascii_digit()
            || character == '.'
    });
    AGENT_OUTPUT_LABELS
        .iter()
        .any(|label| lead.starts_with(label))
}

/// Every recognized size/shape bound in a clause, kept left to right and
/// non-overlapping so one clause can carry more than one requirement (e.g.
/// "exactly one sentence but at least three paragraphs").
fn shape_constraints(clause: &str) -> Vec<RecognizedShapeConstraint> {
    let mut matches: Vec<(usize, usize, ShapeUnit, ShapeBound)> = Vec::new();
    for (regex, kind) in SHAPE_PATTERNS.iter() {
        for captures in regex.captures_iter(clause) {
            let Some(unit) = captures.name("u").and_then(|unit| unit_of(unit.as_str())) else {
                continue;
            };
            let count = match kind {
                ShapeBoundKind::ExactlyOne => 1,
                _ => match captures
                    .name("n")
                    .and_then(|number| parse_count(number.as_str()))
                {
                    Some(count) => count,
                    None => continue,
                },
            };
            let bound = match kind {
                ShapeBoundKind::AtLeast => ShapeBound::AtLeast(count),
                ShapeBoundKind::AtMost => ShapeBound::AtMost(count),
                ShapeBoundKind::AtMostStrict => ShapeBound::AtMost(count.saturating_sub(1)),
                ShapeBoundKind::Exactly | ShapeBoundKind::ExactlyOne => ShapeBound::Exactly(count),
            };
            let Some(whole) = captures.get(0) else {
                continue;
            };
            if has_distributive_qualifier(&clause[whole.end()..]) {
                continue;
            }
            matches.push((whole.start(), whole.end(), unit, bound));
        }
    }
    // Earliest start first, longer match first on ties, so selection is
    // deterministic regardless of pattern order.
    matches.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
    let mut selected = Vec::new();
    let mut consumed_end = 0usize;
    for (start, end, unit, bound) in matches {
        if start < consumed_end {
            continue;
        }
        consumed_end = end;
        selected.push(RecognizedShapeConstraint {
            start,
            end,
            unit,
            bound,
        });
    }
    selected
}

fn has_distributive_qualifier(suffix: &str) -> bool {
    let suffix = suffix.trim_start();
    suffix == "apiece"
        || suffix.starts_with("apiece ")
        || suffix == "each"
        || suffix.starts_with("each ")
        || suffix.starts_with("per ")
        || suffix.starts_with("for each ")
}

fn standalone_shape_mandate(clause: &str, shape: RecognizedShapeConstraint) -> bool {
    let prefix = clause[..shape.start].trim();
    let suffix = clause[shape.end..].trim_matches(|character: char| {
        character.is_whitespace() || matches!(character, ':' | '.' | '!' | '?')
    });
    prefix.is_empty() && suffix.is_empty()
}

fn unit_of(raw: &str) -> Option<ShapeUnit> {
    match raw.to_ascii_lowercase().trim_end_matches('s') {
        "word" => Some(ShapeUnit::Word),
        "sentence" => Some(ShapeUnit::Sentence),
        "paragraph" => Some(ShapeUnit::Paragraph),
        "line" => Some(ShapeUnit::Line),
        "character" | "char" => Some(ShapeUnit::Character),
        _ => None,
    }
}

fn parse_count(raw: &str) -> Option<u32> {
    match raw.to_ascii_lowercase().as_str() {
        "one" => Some(1),
        "two" => Some(2),
        "three" => Some(3),
        "four" => Some(4),
        "five" => Some(5),
        "six" => Some(6),
        "seven" => Some(7),
        "eight" => Some(8),
        "nine" => Some(9),
        "ten" => Some(10),
        other => other.parse().ok(),
    }
}

fn scope_of(document: &LiveInstructionDocument<'_>, line_number: usize) -> Option<usize> {
    document
        .headings()
        .iter()
        .filter(|heading| heading.line < line_number)
        .map(|heading| heading.line)
        .max()
}

fn split_on<'a>(text: &'a str, delimiters: &'static [char]) -> Vec<(usize, &'a str)> {
    let mut pieces = Vec::new();
    let mut start = 0;
    for (index, character) in text.char_indices() {
        if delimiters.contains(&character) {
            push_trimmed_piece(text, start, index, &mut pieces);
            start = index + character.len_utf8();
        }
    }
    push_trimmed_piece(text, start, text.len(), &mut pieces);
    pieces
}

fn push_trimmed_piece<'a>(
    text: &'a str,
    start: usize,
    end: usize,
    pieces: &mut Vec<(usize, &'a str)>,
) {
    let raw = &text[start..end];
    let trimmed = raw.trim();
    if !trimmed.is_empty() {
        pieces.push((start + raw.len() - raw.trim_start().len(), trimmed));
    }
}

fn sentence_is_conditional(clause: &str) -> bool {
    let cleaned = clause
        .trim_start_matches(|character: char| {
            character.is_whitespace()
                || matches!(character, '#' | '-' | '*' | '+' | '>' | ')')
                || character.is_ascii_digit()
                || character == '.'
        })
        .to_ascii_lowercase();
    if cleaned
        .split_whitespace()
        .next()
        .is_some_and(|word| CONDITIONAL_LEAD_INS.contains(&word))
    {
        return true;
    }
    if CONDITIONAL_LEAD_PHRASES
        .iter()
        .any(|phrase| cleaned.starts_with(phrase))
    {
        return true;
    }
    let words = cleaned
        .split_whitespace()
        .take(4)
        .map(|word| word.trim_matches(|character: char| !character.is_alphanumeric()))
        .collect::<Vec<_>>();
    if words.first() == Some(&"in")
        && (words
            .get(2)
            .is_some_and(|word| PREPOSITIONAL_CONDITION_NOUNS.contains(word))
            || matches!(words.get(1), Some(&"a" | &"an" | &"the"))
                && words
                    .get(3)
                    .is_some_and(|word| PREPOSITIONAL_CONDITION_NOUNS.contains(word)))
    {
        return true;
    }
    CONDITIONAL_CLAUSE_MARKERS
        .iter()
        .any(|marker| cleaned.contains(marker))
}

fn report_output_conflicts(
    path: &str,
    requirements: &[DetectedRequirement],
    diag: &mut DiagnosticCollector,
) {
    // Recognition can overlap through synonymous syntax, but a source
    // constraint is unique by its source position, response scope, and typed
    // identity. Deduplicate those before forming pairs; different pairs which
    // share a scope or category must remain distinct findings.
    let mut unique = HashSet::new();
    let mut requirements = requirements
        .iter()
        .filter(|requirement| {
            unique.insert((
                requirement.line,
                requirement.column,
                requirement.scope,
                requirement.requirement,
            ))
        })
        .collect::<Vec<_>>();
    requirements.sort_by_key(|requirement| {
        (
            requirement.line,
            requirement.column,
            requirement.requirement.stable_identity(),
        )
    });

    for (index, first) in requirements.iter().enumerate() {
        for second in &requirements[index + 1..] {
            if first.scope != second.scope {
                continue;
            }
            let Some(category) = conflict_between(&first.requirement, &second.requirement) else {
                continue;
            };
            emit_output_conflict(path, first, second, category, diag);
        }
    }
}

fn conflict_between(a: &OutputRequirement, b: &OutputRequirement) -> Option<ConflictCategory> {
    match (a, b) {
        (OutputRequirement::Format(first), OutputRequirement::Format(second)) => {
            formats_conflict(*first, *second).then_some(ConflictCategory::Format)
        }
        (
            OutputRequirement::Shape {
                unit: unit_a,
                bound: bound_a,
            },
            OutputRequirement::Shape {
                unit: unit_b,
                bound: bound_b,
            },
        ) => {
            shapes_conflict(*unit_a, *bound_a, *unit_b, *bound_b).then_some(ConflictCategory::Shape)
        }
        _ => None,
    }
}

fn formats_conflict(a: FormatConstraint, b: FormatConstraint) -> bool {
    a.format != b.format && a.exclusive && b.exclusive
}

fn shapes_conflict(
    unit_a: ShapeUnit,
    bound_a: ShapeBound,
    unit_b: ShapeUnit,
    bound_b: ShapeBound,
) -> bool {
    if unit_a == unit_b {
        return ranges_disjoint(bound_a, bound_b);
    }
    match (unit_a.nesting_rank(), unit_b.nesting_rank()) {
        (Some(rank_a), Some(rank_b)) if rank_a != rank_b => {
            // A floor on the larger unit forces at least that many of the
            // smaller unit; a tighter cap on the smaller unit is unsatisfiable.
            let (larger, smaller) = if rank_a > rank_b {
                (bound_a, bound_b)
            } else {
                (bound_b, bound_a)
            };
            smaller.cap().is_some_and(|cap| larger.floor() > cap)
        }
        _ => false,
    }
}

fn ranges_disjoint(a: ShapeBound, b: ShapeBound) -> bool {
    b.cap().is_some_and(|cap| a.floor() > cap) || a.cap().is_some_and(|cap| b.floor() > cap)
}

fn emit_output_conflict(
    path: &str,
    first: &DetectedRequirement,
    second: &DetectedRequirement,
    category: ConflictCategory,
    diag: &mut DiagnosticCollector,
) {
    let subject = match category {
        ConflictCategory::Format => "output-format",
        ConflictCategory::Shape => "output-shape",
    };
    let descriptor_a = describe_requirement(&first.requirement);
    let descriptor_b = describe_requirement(&second.requirement);
    let line_a = first.line;
    let line_b = second.line;
    let message = format!(
        "{path}: incompatible {subject} instructions in the same response scope ({descriptor_a} at line {line_a}, {descriptor_b} at line {line_b}); clarify which single requirement applies"
    );
    let evidence = format!("line {line_a}: {descriptor_a}; line {line_b}: {descriptor_b}");
    diag.report_with(
        LintRule::PromptOutputConflict,
        &message,
        DiagnosticMetadata::at_point(line_a, first.column)
            .with_evidence(evidence)
            .with_suggestion(
                "Clarify which single output requirement applies, or separate the instructions by explicit condition or response mode; this rule does not choose between them.",
            ),
    );
}

fn describe_requirement(requirement: &OutputRequirement) -> String {
    match requirement {
        OutputRequirement::Format(constraint) => {
            let label = constraint.format.label();
            if constraint.exclusive {
                format!("exclusive {label} output")
            } else {
                format!("{label} output")
            }
        }
        OutputRequirement::Shape { unit, bound } => {
            let bound_label = bound.label();
            let unit_label = unit.label();
            format!("{bound_label} {unit_label}")
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
            "Retry indefinitely.",
            "Retry as many times as needed.",
            "Keep trying until it succeeds.",
            "Keep retrying until the build passes.",
            "Try again until it works.",
            "Continue until success.",
            "Repeat until the tests pass.",
            "Do not give up until it succeeds.",
            "Retry until success.",
            "Retry until it succeeds.",
            "Do not stop until it is resolved.",
            "Do not stop until the test suite passes.",
            "When a tool fails, retry until success.",
            "Agents must retry until success.",
            "The agent must retry until success.",
            "Always retry until success.",
            "IMPORTANT: Retry until success.",
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
            "Retry until success, up to 3 times.",
            "Retry until success (max 3 attempts).",
            "Retry until success or 3 attempts, whichever comes first.",
            "Retry until success. Stop after 3 attempts and report the remaining failure.",
            "Retry until success. Give up after 3 attempts and report the blocker.",
            "Retry until success. Set a limit of 5 attempts for the repair.",
            "Retry until success. On failure, escalate to the user with a summary.",
            "Retry until success. Upon failure, stop and report the blocker.",
            "Retry until success. When you cannot make progress, escalate to the user.",
            "Retry until success. Abort after 10 minutes and summarize progress.",
            "Retry until success. Retry at most 3 times, then report the failure.",
            "Retry until success. Make at most three attempts before reporting the failure.",
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
    fn unbounded_retry_handles_wrapped_prose_and_keeps_negated_descriptive_and_question_controls() {
        for wrapped in [
            "Retry until\nsuccess.",
            "- Keep trying until it\n  succeeds.",
        ] {
            assert_eq!(q005_diagnostics(wrapped).len(), 1, "{wrapped}");
        }
        for excluded in [
            "Do not retry until success.",
            "The legacy documentation said: retry until success.",
            "Should I retry until success?",
        ] {
            assert!(q005_diagnostics(excluded).is_empty(), "{excluded}");
        }
    }

    #[test]
    fn unbounded_retry_reports_each_sentence_in_source_order() {
        let diagnostics = q005_diagnostics("Retry until success.\n\nLoop forever.");
        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].location.unwrap().start().line_number(), 1);
        assert_eq!(diagnostics[1].location.unwrap().start().line_number(), 3);
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
    fn agent_validator_only_hands_strict_positive_max_turns_to_q005() {
        let temporary = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(temporary.path()).unwrap();
        std::fs::create_dir_all(".claude/agents").unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::write(
            ".claude/agents/bounded.md",
            "---\nname: bounded\ndescription: Reviews changes with concrete test evidence\nmaxTurns: 3\n---\nRetry until success.\n",
        )
        .unwrap();
        std::fs::write(
            "agents/plugin-bounded.md",
            "---\nname: plugin-bounded\ndescription: Reviews changes with concrete test evidence\nmaxTurns: 3\n---\nRetry until success.\n",
        )
        .unwrap();

        let invalid_documents = [
            ("duplicate.md", "maxTurns: 3\nmaxTurns: 4"),
            ("syntax-invalid.md", "maxTurns: 3\n\tbroken: YAML"),
            ("string.md", "maxTurns: \"3\""),
            ("boolean.md", "maxTurns: true"),
            ("sequence.md", "maxTurns: [3]"),
            ("zero.md", "maxTurns: 0"),
        ];
        for (name, frontmatter) in invalid_documents {
            std::fs::write(
                format!(".claude/agents/{name}"),
                format!(
                    "---\nname: invalid\ndescription: Reviews changes with concrete test evidence\n{frontmatter}\n---\nRetry until success.\n"
                ),
            )
            .unwrap();
        }
        std::fs::write(
            ".claude/agents/non-mapping.md",
            "---\n- maxTurns: 3\n---\nRetry until success.\n",
        )
        .unwrap();

        let mut diagnostics = DiagnosticCollector::new_all_enabled();
        super::super::run_all_with_targets(
            &context(temporary.path(), LintMode::Plugin),
            &mut diagnostics,
            &ExcludeSet::default(),
            ValidationTargets {
                cursor: false,
                codex: false,
                claude_md: false,
                agents_md: false,
                agent_skills: false,
            },
        );
        let q005_paths: std::collections::HashSet<_> = diagnostics
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.rule == LintRule::PromptUnboundedRetry)
            .map(|diagnostic| diagnostic.subject_path.as_deref().unwrap().to_path_buf())
            .collect();
        assert_eq!(
            q005_paths,
            [
                ".claude/agents/duplicate.md",
                ".claude/agents/syntax-invalid.md",
                ".claude/agents/non-mapping.md",
                ".claude/agents/string.md",
                ".claude/agents/boolean.md",
                ".claude/agents/sequence.md",
                ".claude/agents/zero.md",
            ]
            .into_iter()
            .map(Path::new)
            .map(Path::to_path_buf)
            .collect()
        );
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
                    claude_md: false,
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

    fn q006_diagnostics(body: &str) -> Vec<crate::diagnostic::Diagnostic> {
        diagnostics_for(body)
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::PromptOutputConflict)
            .cloned()
            .collect()
    }

    #[test]
    fn q006_reports_exclusive_format_conflict_with_structured_evidence() {
        let diagnostics = q006_diagnostics("Return only JSON.\nRespond in Markdown.\n");
        assert_eq!(diagnostics.len(), 1);
        // Primary location is the earlier (deterministic) constraint.
        assert_eq!(diagnostics[0].location.unwrap().start().line_number(), 1);
        // Both conflicting constraints are exposed structurally, not via message text.
        let evidence = diagnostics[0].evidence.as_deref().unwrap();
        assert!(evidence.starts_with("line 1:"), "{evidence}");
        assert!(evidence.contains("line 2:"), "{evidence}");
        assert!(evidence.contains("JSON"), "{evidence}");
        assert!(evidence.contains("Markdown"), "{evidence}");
        // The suggestion asks for clarification and does not choose a side.
        assert!(
            diagnostics[0]
                .suggestion
                .as_deref()
                .unwrap()
                .contains("Clarify")
        );
    }

    #[test]
    fn q006_reports_each_positive_class() {
        // Class 1: two exclusive formats.
        assert_eq!(
            q006_diagnostics("Output XML only.\nReturn only JSON.\n").len(),
            1
        );
        // Class 1: a trailing purpose phrase ("... for the API") must not mask a
        // real conflict — conditionality is judged at the sentence head only.
        assert_eq!(
            q006_diagnostics("Return only JSON, formatted for the API.\nRespond in Markdown.\n")
                .len(),
            1
        );
        // Class 2: incompatible size/shape across nesting units.
        assert_eq!(
            q006_diagnostics(
                "Answer in exactly one sentence.\nInclude at least three paragraphs.\n"
            )
            .len(),
            1
        );
        // Class 2: incompatible same-unit minimum/maximum.
        assert_eq!(
            q006_diagnostics("Use at most two sentences.\nWrite at least five sentences.\n").len(),
            1
        );
        // Class 2: two incompatible bounds inside a single clause.
        assert_eq!(
            q006_diagnostics("Respond in exactly one sentence but at least three paragraphs.\n")
                .len(),
            1
        );
    }

    #[test]
    fn q006_conflict_is_scoped_to_one_section() {
        // Same section: reported.
        assert_eq!(
            q006_diagnostics("# Output\nReturn only JSON.\nRespond in Markdown.\n").len(),
            1
        );
        // Separate headings defining distinct response modes: clean.
        assert!(
            q006_diagnostics(
                "## JSON responses\nReturn only JSON.\n## Markdown responses\nRespond in Markdown.\n"
            )
            .is_empty()
        );
    }

    #[test]
    fn q006_ignores_every_hard_negative_class() {
        for clean in [
            // Mere multi-format mention.
            "The pipeline reads JSON and emits Markdown examples.",
            // Non-exclusive alternatives.
            "Respond in JSON or Markdown.",
            // Two non-exclusive single mentions.
            "Return JSON.\nWe also emit Markdown.",
            // Conditional routing (semicolon and comma forms).
            "For data requests use JSON; for explanations use Markdown.",
            "For data, respond in JSON. For prose, respond in Markdown.",
            // Input-format versus output-format.
            "The input is JSON.\nRespond in Markdown.",
            // Fenced, inline-code, and quoted examples.
            "`Return only JSON.`\n> Respond in Markdown.\n```\nOutput XML only.\n```",
            // Explicit examples section.
            "# Examples\nReturn only JSON.\nRespond in Markdown.",
            // Compatible shape bounds (satisfiable together).
            "Write at least two paragraphs.\nInclude at least three sentences.",
            // Input-format described with a directive verb (subject is the user,
            // not the agent) — not an operative output requirement.
            "Users reply in plain text.\nRespond in Markdown.",
            "The user will respond in JSON.\nAlways format your response in Markdown.",
            // Label-routed response modes (delineation without a marker word).
            "- Data requests: respond only in JSON.\n- Explanations: respond only in Markdown.",
            "Data mode: respond only in JSON. Prose mode: respond only in Markdown.",
            // Shape phrases only constrain output when their clause is
            // classified as operative. These all describe another subject.
            "The input contains exactly one sentence.\nThe input contains at least three paragraphs.",
            "The request requires exactly one sentence.\nThe request requires at least three paragraphs.",
            "The source document has exactly one sentence.\nThe source document has at least three paragraphs.",
            "The previous assistant response contained exactly one sentence.\nThe historical report contained at least three paragraphs.",
            "Your response from the previous turn contained exactly one sentence.\nThe historical response contained at least three paragraphs.",
        ] {
            assert!(
                q006_diagnostics(clean).is_empty(),
                "expected no Q006 for {clean:?}"
            );
        }
    }

    #[test]
    fn q006_classifies_issue_239_mode_artifact_bound_and_example_cases() {
        for clean in [
            // A sentence-leading prepositional mode adjunct guards all of its
            // comma-separated directive clauses, with or without the comma.
            "In JSON mode, respond only with JSON; in chat mode, respond in Markdown.",
            "In JSON mode respond only with JSON; in chat mode respond in Markdown.",
            "In that case, return only JSON; in chat format, respond in Markdown.",
            // Direct objects and destinations in the typed artifact vocabulary
            // are not requirements on the response itself.
            "Write commit messages in plain text only.\nRespond in Markdown.",
            "Write documentation in Markdown only.\nReturn only JSON.",
            "Write the export file as JSON only.\nRespond in Markdown.",
            "Include at least three paragraphs in the PR description.\nAnswer in exactly one sentence.",
            // Per-item bounds are not whole-response bounds.
            "Use at most two sentences per paragraph.\nWrite at least five paragraphs.",
            "Use exactly one sentence for each bullet.\nInclude at least three sentences.",
            "Use at most two sentences apiece.\nWrite at least five sentences.",
            // Qualified labels are examples, while bare labels remain live.
            "Bad output: Return only JSON.\nGood output: Respond in Markdown.",
            "Sample output: Return only JSON.\nRespond in Markdown.",
            // #221 mixed descriptive/operative hard negative.
            "The input contains at most two sentences.\nInclude at least five sentences.",
        ] {
            assert!(
                q006_diagnostics(clean).is_empty(),
                "expected clean: {clean:?}"
            );
        }

        for operative in [
            "Return only JSON.\nRespond in Markdown.",
            "Use at most two sentences.\nWrite at least five sentences.",
            "Output: Return only JSON.\nResponse: Respond in Markdown.",
        ] {
            assert_eq!(
                q006_diagnostics(operative).len(),
                1,
                "expected conflict: {operative:?}"
            );
        }
    }

    #[test]
    fn q006_classifies_output_shape_clauses_before_comparing_bounds() {
        for (operative, descriptive) in [
            (
                "Answer in exactly one sentence.\nInclude at least three paragraphs.",
                "The input contains exactly one sentence.\nThe input contains at least three paragraphs.",
            ),
            (
                "Use at most two sentences.\nWrite at least five sentences.",
                "The request contains at most two sentences.\nThe request contains at least five sentences.",
            ),
            (
                "Exactly one sentence.\nAt least three paragraphs.",
                "The source text is exactly one sentence.\nThe source text has at least three paragraphs.",
            ),
            (
                "Response: exactly one sentence.\nOutput: at least three paragraphs.",
                "The prior response: exactly one sentence.\nThe historical output: at least three paragraphs.",
            ),
        ] {
            assert_eq!(q006_diagnostics(operative).len(), 1, "{operative:?}");
            assert!(q006_diagnostics(descriptive).is_empty(), "{descriptive:?}");
        }
    }

    #[test]
    fn q006_severity_follows_normal_pedantic_and_suppression() {
        let body = "Return only JSON.\nRespond in Markdown.\n";

        // Normal mode: the default-warning rule fires as a non-blocking warning.
        let mut normal = DiagnosticCollector::new();
        validate_body("skills/example/SKILL.md", body, &mut normal);
        assert_eq!(normal.warning_count(), 1);
        assert_eq!(normal.error_count(), 0);

        // Pedantic mode: promoted to a blocking error.
        let mut config = LintConfig::default();
        config.apply_cli_mode(crate::config::CliMode::Pedantic);
        let mut pedantic = DiagnosticCollector::with_config(config);
        validate_body("skills/example/SKILL.md", body, &mut pedantic);
        assert_eq!(pedantic.error_count(), 1);
        assert_eq!(pedantic.warning_count(), 0);

        // All mode: enabled as an error like every registered rule.
        let mut all_config = LintConfig::default();
        all_config.apply_cli_mode(crate::config::CliMode::All);
        let mut all = DiagnosticCollector::with_config(all_config);
        validate_body("skills/example/SKILL.md", body, &mut all);
        assert_eq!(all.error_count(), 1);

        // Suppression removes it entirely and is accounted for.
        let suppressed_config = LintConfig {
            suppress: HashSet::from([LintRule::PromptOutputConflict]),
            ..LintConfig::default()
        };
        let mut suppressed = DiagnosticCollector::with_config(suppressed_config);
        validate_body("skills/example/SKILL.md", body, &mut suppressed);
        assert_eq!(suppressed.error_count() + suppressed.warning_count(), 0);
        assert_eq!(suppressed.suppressed_count(), 1);
    }

    #[test]
    #[serial_test::serial]
    fn q006_runs_on_every_live_instruction_surface() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude/skills/example").unwrap();
        std::fs::create_dir_all(".claude/agents").unwrap();
        std::fs::create_dir_all(".cursor/rules").unwrap();

        let conflict = "Return only JSON.\nRespond in Markdown.\n";
        std::fs::write("CLAUDE.md", conflict).unwrap();
        std::fs::write("AGENTS.md", conflict).unwrap();
        std::fs::write(
            ".claude/skills/example/SKILL.md",
            format!(
                "---\nname: example\ndescription: Use when you need reliable output support\n---\n{conflict}"
            ),
        )
        .unwrap();
        std::fs::write(
            ".claude/agents/example.md",
            format!(
                "---\nname: example\ndescription: Reviews changes with concrete test evidence\n---\n{conflict}"
            ),
        )
        .unwrap();
        std::fs::write(
            ".cursor/rules/example.mdc",
            format!("---\ndescription: Enforces output rules\nalwaysApply: true\n---\n{conflict}"),
        )
        .unwrap();
        std::fs::write(".cursorrules", conflict).unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        super::super::run_all_with_targets(
            &context(tmp.path(), LintMode::Basic),
            &mut diag,
            &ExcludeSet::default(),
            ValidationTargets {
                cursor: true,
                codex: false,
                claude_md: false,
                agents_md: true,
                agent_skills: false,
            },
        );

        for expected in [
            "CLAUDE.md",
            "AGENTS.md",
            ".claude/skills/example/SKILL.md",
            ".claude/agents/example.md",
            ".cursor/rules/example.mdc",
            ".cursorrules",
        ] {
            assert_eq!(
                diag.diagnostics()
                    .iter()
                    .filter(|item| {
                        item.rule == LintRule::PromptOutputConflict
                            && item.subject_path.as_deref() == Some(Path::new(expected))
                    })
                    .count(),
                1,
                "missing Q006 for {expected}"
            );
        }
    }

    #[test]
    fn q006_reports_every_deterministic_format_conflict_pair_per_scope() {
        let diagnostics =
            q006_diagnostics("Output only JSON.\nReturn only XML.\nRespond only in YAML.\n");
        assert_eq!(diagnostics.len(), 3);
        let pairs = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.evidence.as_deref().unwrap())
            .collect::<Vec<_>>();
        assert!(pairs[0].starts_with("line 1:") && pairs[0].contains("line 2:"));
        assert!(pairs[1].starts_with("line 1:") && pairs[1].contains("line 3:"));
        assert!(pairs[2].starts_with("line 2:") && pairs[2].contains("line 3:"));
    }

    #[test]
    fn q006_reports_every_deterministic_shape_conflict_pair_per_scope() {
        let diagnostics = q006_diagnostics(
            "Answer in exactly one sentence.\nInclude exactly two sentences.\nUse exactly three sentences.\n",
        );
        assert_eq!(diagnostics.len(), 3);
        let pairs = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.evidence.as_deref().unwrap())
            .collect::<Vec<_>>();
        assert!(pairs[0].starts_with("line 1:") && pairs[0].contains("line 2:"));
        assert!(pairs[1].starts_with("line 1:") && pairs[1].contains("line 3:"));
        assert!(pairs[2].starts_with("line 2:") && pairs[2].contains("line 3:"));
    }

    #[test]
    fn q006_deduplicates_overlapping_shape_recognition() {
        // "exactly one" matches both the general exact pattern and the
        // specialized one-count pattern, but represents one source constraint.
        let diagnostics =
            q006_diagnostics("Use exactly one sentence.\nInclude at least three sentences.\n");
        assert_eq!(diagnostics.len(), 1);
    }

    fn q006_count_on_surface(kind: InstructionSurfaceKind, content: &str) -> usize {
        let markdown = MarkdownDocument::parse(content);
        let document = LiveInstructionDocument::new(Path::new("surface.md"), kind, &markdown);
        let mut diag = DiagnosticCollector::new_all_enabled();
        PromptContentPass::default().validate(&document, &mut diag);
        diag.diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::PromptOutputConflict)
            .count()
    }

    #[test]
    fn q006_conflict_and_hard_negative_hold_across_surfaces() {
        for kind in [
            InstructionSurfaceKind::ClaudeProject,
            InstructionSurfaceKind::Skill,
            InstructionSurfaceKind::Agent,
            InstructionSurfaceKind::AgentsMd,
            InstructionSurfaceKind::CursorRule,
            InstructionSurfaceKind::CursorLegacyRule,
        ] {
            // A real conflict fires on every live-instruction surface...
            assert_eq!(
                q006_count_on_surface(kind, "Return only JSON.\nRespond in Markdown.\n"),
                1,
                "{kind:?}"
            );
            // ...and conditional routing stays clean on every surface.
            assert_eq!(
                q006_count_on_surface(
                    kind,
                    "For data requests use JSON; for prose respond in Markdown.\n"
                ),
                0,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn q006_shape_classification_holds_across_surfaces() {
        for kind in [
            InstructionSurfaceKind::ClaudeProject,
            InstructionSurfaceKind::Skill,
            InstructionSurfaceKind::Agent,
            InstructionSurfaceKind::AgentsMd,
            InstructionSurfaceKind::CursorRule,
            InstructionSurfaceKind::CursorLegacyRule,
        ] {
            assert_eq!(
                q006_count_on_surface(
                    kind,
                    "Answer in exactly one sentence.\nInclude at least three paragraphs.\n",
                ),
                1,
                "{kind:?} operative shape",
            );
            assert_eq!(
                q006_count_on_surface(
                    kind,
                    "The input contains exactly one sentence.\nThe input contains at least three paragraphs.\n",
                ),
                0,
                "{kind:?} input description",
            );
        }
    }
}
