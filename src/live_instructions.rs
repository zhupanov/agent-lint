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

    pub fn has_outer_execution_bound(&self) -> bool {
        self.outer_max_turns.is_some()
    }
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
}
