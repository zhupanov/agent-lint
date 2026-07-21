//! Typed adapter for repository files that contain live agent instructions.
//!
//! Surface owners remain responsible for discovery, activation, exclusions,
//! and structural validation. This adapter only exposes source-aware Markdown
//! facts to prompt-content rules.

use crate::markdown::{MarkdownDocument, MarkdownHeading, MarkdownProseLine};
use std::num::NonZeroU64;
use std::path::Path;

/// The live-instruction surface that supplied a Markdown document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstructionSurfaceKind {
    ClaudeProject,
    Skill,
    Agent,
    AgentsMd,
    CursorRule,
    CursorLegacyRule,
}

/// A repository-relative, typed view of live instruction prose.
pub struct LiveInstructionDocument<'a> {
    subject_path: &'a Path,
    surface_kind: InstructionSurfaceKind,
    markdown: &'a MarkdownDocument,
    outer_max_turns: Option<NonZeroU64>,
}

impl<'a> LiveInstructionDocument<'a> {
    pub fn new(
        subject_path: &'a Path,
        surface_kind: InstructionSurfaceKind,
        markdown: &'a MarkdownDocument,
    ) -> Self {
        Self {
            subject_path,
            surface_kind,
            markdown,
            outer_max_turns: None,
        }
    }

    /// Record a validated positive agent-level `maxTurns` bound supplied by the owning
    /// surface. Prompt-content rules can use this fact without reparsing or
    /// interpreting frontmatter themselves.
    pub fn with_outer_max_turns(mut self, max_turns: Option<NonZeroU64>) -> Self {
        self.outer_max_turns = max_turns;
        self
    }

    pub fn subject_path(&self) -> &Path {
        self.subject_path
    }

    pub fn surface_kind(&self) -> InstructionSurfaceKind {
        self.surface_kind
    }

    /// Markdown body after a complete frontmatter block, when the owning
    /// surface parsed frontmatter.
    #[allow(dead_code)] // adapter contract for surface-specific prompt rules
    pub fn body(&self) -> &str {
        self.markdown.body()
    }

    /// Live prose with frontmatter, fenced/indented code, inline code, and
    /// identifiable quoted examples removed. Line numbers refer to the
    /// original source file.
    pub fn prose_lines(&self) -> &[MarkdownProseLine] {
        self.markdown.body_prose()
    }

    /// Source-positioned headings used to determine Markdown sections.
    pub fn headings(&self) -> &[MarkdownHeading] {
        self.markdown.headings()
    }

    /// Whether each prose line belongs to an identifiable example scope.
    ///
    /// An Examples heading applies through the next heading of the same or
    /// higher level. Explicitly marked example lines are also excluded even
    /// outside such a section. Consumers use this shared source-aware fact
    /// when deciding whether prose is a live instruction.
    pub fn example_scopes(&self) -> Vec<bool> {
        let mut active_heading_level = None;
        self.prose_lines()
            .iter()
            .map(|line| {
                if let Some(heading) = self
                    .headings()
                    .iter()
                    .find(|heading| heading.line == line.line)
                {
                    if active_heading_level.is_some_and(|level| heading.level <= level) {
                        active_heading_level = None;
                    }
                    if is_example_heading(&heading.text) {
                        active_heading_level = Some(heading.level);
                        return true;
                    }
                }

                active_heading_level.is_some() || is_explicit_example_line(&line.text)
            })
            .collect()
    }

    pub fn has_outer_execution_bound(&self) -> bool {
        self.outer_max_turns.is_some()
    }
}

fn is_example_heading(text: &str) -> bool {
    text.split(|character: char| !character.is_alphanumeric())
        .any(|word| matches!(word.to_ascii_lowercase().as_str(), "example" | "examples"))
}

fn is_explicit_example_line(line: &str) -> bool {
    let line = line.trim().trim_start_matches(|character: char| {
        character.is_whitespace() || matches!(character, '-' | '*' | '+' | '>')
    });
    let lower = line.to_ascii_lowercase();
    [
        "example:",
        "example ",
        "for example,",
        "for example:",
        "e.g.,",
        "e.g.:",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
        || is_qualified_output_example_label(&lower)
}

fn is_qualified_output_example_label(lower: &str) -> bool {
    const QUALIFIERS: &[&str] = &[
        "bad",
        "good",
        "sample",
        "expected",
        "invalid",
        "wrong",
        "incorrect",
        "example",
    ];
    const OUTPUT_LABELS: &[&str] = &["output:", "response:", "answer:", "reply:"];
    QUALIFIERS.iter().any(|qualifier| {
        lower
            .strip_prefix(qualifier)
            .and_then(|suffix| suffix.strip_prefix(' '))
            .is_some_and(|suffix| OUTPUT_LABELS.iter().any(|label| suffix.starts_with(label)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_typed_source_aware_body() {
        let markdown = MarkdownDocument::parse(
            "---\ndescription: Be helpful\n---\n# Important\nUse `do not` as an example.\n",
        );
        let document = LiveInstructionDocument::new(
            Path::new(".cursor/rules/example.mdc"),
            InstructionSurfaceKind::CursorRule,
            &markdown,
        );

        assert_eq!(document.surface_kind(), InstructionSurfaceKind::CursorRule);
        assert_eq!(
            document.subject_path(),
            Path::new(".cursor/rules/example.mdc")
        );
        assert!(document.body().starts_with("# Important"));
        assert_eq!(document.prose_lines()[0].line, 4);
        assert!(!document.prose_lines()[1].text.contains("do not"));
    }

    #[test]
    fn identifies_heading_and_explicit_example_scopes() {
        let markdown = MarkdownDocument::parse(
            "# Examples\nUse a timeout of 10 minutes.\n## More examples\nExample: stop after failure.\n# Task\nUse a timeout of 5 minutes.\n",
        );
        let document = LiveInstructionDocument::new(
            Path::new("agents/example.md"),
            InstructionSurfaceKind::Agent,
            &markdown,
        );

        assert_eq!(
            document.example_scopes(),
            vec![true, true, true, true, false, false]
        );
    }

    #[test]
    fn identifies_qualified_output_labels_as_examples_without_hiding_bare_labels() {
        let markdown = MarkdownDocument::parse(
            "Bad output: Return only JSON.\nGood response: Respond in Markdown.\nOutput: Return only JSON.\n",
        );
        let document = LiveInstructionDocument::new(
            Path::new("AGENTS.md"),
            InstructionSurfaceKind::AgentsMd,
            &markdown,
        );

        assert_eq!(document.example_scopes(), vec![true, true, false]);
    }
}
