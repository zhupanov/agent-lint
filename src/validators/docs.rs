use crate::config::ExcludeSet;
use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata, SourceSpan};
use crate::live_instructions::example_scopes_for;
use crate::markdown::MarkdownDocument;
use crate::markdown_refs::{is_external_or_fragment_destination, percent_decode_once};
use crate::repo_path::{
    PathProbe, ResolutionBase, normalize_separators, normalized_target_key,
    probe_contains_parent_segment, resolve_repo_path,
};
use crate::rules::LintRule;
use crate::unfinished_work::find_first_unfinished_work_marker;
use regex::Regex;
use std::collections::HashSet;
use std::fs;
use std::ops::Range;
use std::path::Path;
use std::sync::LazyLock;

static RE_DOCS_PATH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^docs/[a-zA-Z0-9._/-]+\.md$").unwrap());

const D001_SUGGESTION: &str = "create the canonical document or correct this reference";
const D002_SUGGESTION: &str = "split detailed guidance into referenced documents";

/// One candidate `docs/...md` reference inside the Canonical sources section.
struct DocsRefHit {
    byte_range: Range<usize>,
    /// Authored path spelling used for evidence and the human message.
    evidence: String,
    /// Path passed to the repository-safe resolver (fragment already stripped;
    /// link destinations are percent-decoded once).
    resolve_raw: String,
}

/// D001: docs file references from the root CLAUDE.md Canonical sources section.
pub fn validate_docs_references(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    if exclude.is_excluded("CLAUDE.md") {
        return;
    }
    let claude_md = Path::new("CLAUDE.md");
    if !claude_md.is_file() {
        return;
    }

    let content = match fs::read_to_string(claude_md) {
        Ok(c) => c,
        Err(_) => return,
    };

    let document = MarkdownDocument::parse(&content);
    let Some((section_start, section_end)) = canonical_sources_section_bounds(&document) else {
        return;
    };

    let mut hits = collect_docs_ref_hits(&document, &content, section_start, section_end);
    hits.sort_by_key(|hit| hit.byte_range.start);

    let mut seen = HashSet::new();
    for hit in hits {
        let key = if probe_contains_parent_segment(&hit.resolve_raw) {
            format!("unsafe:{}", normalize_separators(&hit.resolve_raw))
        } else {
            normalized_target_key(claude_md, &hit.resolve_raw, ResolutionBase::RepositoryRoot)
                .unwrap_or_else(|| normalize_separators(&hit.resolve_raw))
        };
        if !seen.insert(key) {
            continue;
        }
        let outcome = if probe_contains_parent_segment(&hit.resolve_raw) {
            PathProbe::Rejected
        } else {
            resolve_repo_path(claude_md, &hit.resolve_raw, ResolutionBase::RepositoryRoot)
        };
        match outcome {
            PathProbe::File(_) => continue,
            PathProbe::Missing(_) | PathProbe::Directory(_) | PathProbe::Rejected => {}
        }
        let metadata = SourceSpan::from_byte_range(&content, hit.byte_range.clone())
            .map_or_else(DiagnosticMetadata::default, |location| {
                DiagnosticMetadata::default().with_location(location)
            })
            .with_evidence(&hit.evidence)
            .with_suggestion(D001_SUGGESTION);
        diag.report_at_with(
            LintRule::DocsRefMissing,
            claude_md,
            &format!(
                "docs reference in CLAUDE.md canonical sources not found on disk: {}",
                hit.evidence
            ),
            metadata,
        );
    }
}

/// D002: CLAUDE.md size limit (500 lines). Advisory project-maintainability check.
pub fn validate_claudemd_size(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    if exclude.is_excluded("CLAUDE.md") {
        return;
    }
    let claude_md = Path::new("CLAUDE.md");
    if !claude_md.is_file() {
        return;
    }

    let content = match fs::read_to_string(claude_md) {
        Ok(c) => c,
        Err(_) => return,
    };

    let line_count = content.lines().count();
    if line_count > 500 {
        diag.report_at_with(
            LintRule::ClaudemdTooLarge,
            claude_md,
            &format!(
                "CLAUDE.md exceeds 500 lines ({line_count} lines); consider splitting into referenced documents"
            ),
            DiagnosticMetadata::default()
                .with_evidence(format!("{line_count} lines"))
                .with_suggestion(D002_SUGGESTION),
        );
    }
}

/// D003: TODO/FIXME/HACK/XXX markers in CLAUDE.md.
pub fn validate_claudemd_todos(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    if exclude.is_excluded("CLAUDE.md") {
        return;
    }
    let claude_md = Path::new("CLAUDE.md");
    if !claude_md.is_file() {
        return;
    }

    let content = match fs::read_to_string(claude_md) {
        Ok(c) => c,
        Err(_) => return,
    };

    let Some(hit) = find_first_unfinished_work_marker(&content) else {
        return;
    };
    diag.report_at_with(
        LintRule::TodoInDocs,
        claude_md,
        &format!(
            "CLAUDE.md contains {} marker; remove before publishing",
            hit.marker
        ),
        hit.metadata(),
    );
}

/// X002–X005: fence / XML structure checks for CLAUDE.md.
pub fn validate_claudemd_structure(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    if exclude.is_excluded("CLAUDE.md") {
        return;
    }
    let claude_md = Path::new("CLAUDE.md");
    if !claude_md.is_file() {
        return;
    }
    let content = match fs::read_to_string(claude_md) {
        Ok(c) => c,
        Err(_) => return,
    };
    diag.with_subject_path(claude_md, |diag| {
        super::markdown_structure::check_markdown_structure("CLAUDE.md", &content, diag);
    });
}

/// Inclusive start line / exclusive end line of the Canonical sources section.
fn canonical_sources_section_bounds(document: &MarkdownDocument) -> Option<(usize, usize)> {
    let start = document.headings().iter().find(|heading| {
        heading.level == 2
            && heading
                .text
                .trim()
                .eq_ignore_ascii_case("canonical sources")
    })?;
    let end = document
        .headings()
        .iter()
        .find(|heading| heading.line > start.line && heading.level <= 2)
        .map(|heading| heading.line)
        .unwrap_or(usize::MAX);
    Some((start.line, end))
}

fn line_in_section(line: usize, section_start: usize, section_end: usize) -> bool {
    line > section_start && line < section_end
}

fn collect_docs_ref_hits(
    document: &MarkdownDocument,
    content: &str,
    section_start: usize,
    section_end: usize,
) -> Vec<DocsRefHit> {
    let example_lines: HashSet<usize> = document
        .body_prose()
        .iter()
        .zip(example_scopes_for(document))
        .filter_map(|(line, is_example)| is_example.then_some(line.line))
        .collect();
    let live_prose_lines: HashSet<usize> =
        document.body_prose().iter().map(|line| line.line).collect();

    let mut hits = Vec::new();

    for link in document.links() {
        if !line_in_section(link.line, section_start, section_end)
            || !live_prose_lines.contains(&link.line)
            || example_lines.contains(&link.line)
            || is_external_or_fragment_destination(&link.raw_destination)
        {
            continue;
        }
        let decoded = percent_decode_once(&link.raw_destination);
        let Some(path) = classify_docs_path(&decoded) else {
            continue;
        };
        hits.push(DocsRefHit {
            byte_range: link.destination_byte_range.clone(),
            evidence: path.clone(),
            resolve_raw: path,
        });
    }

    for code in document.inline_code() {
        if !line_in_section(code.start_line, section_start, section_end)
            || !live_prose_lines.contains(&code.start_line)
            || example_lines.contains(&code.start_line)
        {
            continue;
        }
        let Some(path) = classify_docs_path(&code.raw_literal) else {
            continue;
        };
        hits.push(DocsRefHit {
            byte_range: code.literal_byte_range.clone(),
            evidence: path.clone(),
            resolve_raw: path,
        });
    }

    for (line, is_example) in document
        .body_prose()
        .iter()
        .zip(example_scopes_for(document))
    {
        if is_example || !line_in_section(line.line, section_start, section_end) {
            continue;
        }
        hits.extend(plain_docs_tokens(content, line.line, &line.text));
    }

    hits
}

fn classify_docs_path(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || !trimmed.starts_with("docs/") {
        return None;
    }
    let without_fragment = trimmed.split_once('#').map_or(trimmed, |(path, _)| path);
    let normalized = normalize_separators(without_fragment);
    if RE_DOCS_PATH.is_match(&normalized) {
        Some(normalized)
    } else {
        None
    }
}

fn plain_docs_tokens(content: &str, line_number: usize, masked_text: &str) -> Vec<DocsRefHit> {
    let Some(line_start) = byte_offset_for_line(content, line_number) else {
        return Vec::new();
    };
    // `str::lines` / body_prose strip a trailing CR; match that view for column
    // alignment while byte offsets stay relative to `line_start`.
    let original_line = content[line_start..]
        .split_once('\n')
        .map_or(&content[line_start..], |(line, _)| line);
    let original_line = original_line.strip_suffix('\r').unwrap_or(original_line);
    let original_chars: Vec<char> = original_line.chars().collect();
    let masked_chars: Vec<char> = masked_text.chars().collect();
    if original_chars.len() != masked_chars.len() {
        // Masking preserves Unicode columns; a length mismatch means this line
        // should not be scanned for plain tokens.
        return Vec::new();
    }

    let mut hits = Vec::new();
    let mut index = 0;
    while index < masked_chars.len() {
        if masked_chars[index].is_whitespace() {
            index += 1;
            continue;
        }
        let start = index;
        while index < masked_chars.len() && !masked_chars[index].is_whitespace() {
            index += 1;
        }
        let mut token_end = index;
        while token_end > start
            && matches!(
                masked_chars[token_end - 1],
                ')' | ']' | '}' | '>' | ',' | '.' | ';' | ':' | '!' | '?'
            )
        {
            token_end -= 1;
        }
        if token_end == start {
            continue;
        }
        // Skip spans altered by inline-code / link-destination masking so those
        // references are owned exclusively by the structured node extractors.
        if masked_chars[start..token_end] != original_chars[start..token_end] {
            continue;
        }
        let token: String = original_chars[start..token_end].iter().collect();
        let Some(path) = classify_docs_path(&token) else {
            continue;
        };
        let byte_start = line_start
            + original_chars[..start]
                .iter()
                .map(|ch| ch.len_utf8())
                .sum::<usize>();
        let byte_end = line_start
            + original_chars[..token_end]
                .iter()
                .map(|ch| ch.len_utf8())
                .sum::<usize>();
        hits.push(DocsRefHit {
            byte_range: byte_start..byte_end,
            evidence: path.clone(),
            resolve_raw: path,
        });
    }
    hits
}

fn byte_offset_for_line(content: &str, line_number: usize) -> Option<usize> {
    if line_number == 0 {
        return None;
    }
    if line_number == 1 {
        return Some(0);
    }
    let mut line = 1usize;
    for (index, byte) in content.bytes().enumerate() {
        if byte == b'\n' {
            line += 1;
            if line == line_number {
                return Some(index + 1);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::SourceSpan;

    #[test]
    fn canonical_section_exact_heading_only() {
        let document = MarkdownDocument::parse(
            "## Canonical sources\n- docs/foo.md\n## Canonical sources and more\n- docs/bar.md\n",
        );
        let bounds = canonical_sources_section_bounds(&document).unwrap();
        assert_eq!(bounds, (1, 3));
    }

    #[test]
    fn canonical_section_case_insensitive_and_nested() {
        let document = MarkdownDocument::parse(
            "## Canonical Sources\n### Nested\n- docs/foo.md\n## Other\n- docs/bar.md\n",
        );
        let bounds = canonical_sources_section_bounds(&document).unwrap();
        assert_eq!(bounds, (1, 4));
    }

    #[test]
    fn canonical_section_stops_at_level_one() {
        let document =
            MarkdownDocument::parse("## Canonical sources\n- docs/foo.md\n# Top\n- docs/bar.md\n");
        let bounds = canonical_sources_section_bounds(&document).unwrap();
        assert_eq!(bounds, (1, 3));
    }

    #[test]
    fn prefixed_heading_is_not_canonical_sources() {
        let document =
            MarkdownDocument::parse("## Canonical sources list\n- docs/foo.md\n## Other\n");
        assert!(canonical_sources_section_bounds(&document).is_none());
    }

    #[test]
    fn classify_docs_path_left_boundary() {
        assert_eq!(
            classify_docs_path("docs/intro.md"),
            Some("docs/intro.md".into())
        );
        assert_eq!(
            classify_docs_path("docs/sub/architecture.md#frag"),
            Some("docs/sub/architecture.md".into())
        );
        assert_eq!(classify_docs_path("website/docs/intro.md"), None);
        assert_eq!(classify_docs_path("mydocs/foo.md"), None);
        assert_eq!(classify_docs_path("./docs/foo.md"), None);
    }

    #[test]
    #[serial_test::serial]
    fn d001_subdirectory_docs_reference() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("docs/sub").unwrap();
        std::fs::write("docs/sub/architecture.md", "# Arch\n").unwrap();
        std::fs::write(
            "CLAUDE.md",
            "# Claude\n## Canonical sources\n- docs/sub/architecture.md\n## Other\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_docs_references(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn d001_valid_docs_reference() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("docs").unwrap();
        std::fs::write("docs/architecture.md", "# Arch\n").unwrap();
        std::fs::write(
            "CLAUDE.md",
            "# Claude\n## Canonical sources\n- docs/architecture.md\n## Other\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_docs_references(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn d001_missing_docs_reference() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::write(
            "CLAUDE.md",
            "# Claude\n## Canonical sources\n- docs/nonexistent.md\n## Other\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_docs_references(&mut diag, &crate::config::ExcludeSet::default());
        let findings: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|d| d.rule == LintRule::DocsRefMissing)
            .collect();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("not found on disk"));
        assert_eq!(findings[0].evidence.as_deref(), Some("docs/nonexistent.md"));
        assert_eq!(findings[0].suggestion.as_deref(), Some(D001_SUGGESTION));
        assert_eq!(findings[0].location, Some(SourceSpan::range(3, 3, 3, 22)));
    }

    #[test]
    #[serial_test::serial]
    fn d001_fenced_example_is_clean() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::write(
            "CLAUDE.md",
            "# Claude\n## Canonical sources\n\n```markdown\n- docs/missing-example.md\n```\n\n## Other\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_docs_references(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn d001_parent_directory_docs_path_never_emits() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("website/docs").unwrap();
        std::fs::write("website/docs/intro.md", "# Intro\n").unwrap();
        std::fs::write(
            "CLAUDE.md",
            "# Claude\n## Canonical sources\n- website/docs/intro.md\n- mydocs/foo.md\n- website/docs/missing.md\n- docs/missing.md\n## Other\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_docs_references(&mut diag, &crate::config::ExcludeSet::default());
        let findings: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|d| d.rule == LintRule::DocsRefMissing)
            .collect();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].evidence.as_deref(), Some("docs/missing.md"));
    }

    #[test]
    #[serial_test::serial]
    fn d001_link_inline_plain_and_dedupe() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::write(
            "CLAUDE.md",
            "# Claude\n## Canonical sources\nSee [arch](docs/arch.md#frag) and `docs/arch.md` plus docs/arch.md again.\n## Other\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_docs_references(&mut diag, &crate::config::ExcludeSet::default());
        let findings: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|d| d.rule == LintRule::DocsRefMissing)
            .collect();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].evidence.as_deref(), Some("docs/arch.md"));
    }

    #[test]
    #[serial_test::serial]
    fn d001_percent_encoded_link() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("docs").unwrap();
        std::fs::write("docs/my file.md", "# Doc\n").unwrap();
        std::fs::write(
            "CLAUDE.md",
            "# Claude\n## Canonical sources\n- [doc](docs/my%20file.md)\n## Other\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_docs_references(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(
            diag.diagnostics()
                .iter()
                .filter(|d| d.rule == LintRule::DocsRefMissing)
                .count(),
            0
        );
    }

    #[test]
    #[serial_test::serial]
    fn d001_symlink_directory_and_parent_emit() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("docs/folder.md").unwrap();
        std::fs::write("docs/real.md", "ok\n").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("real.md", "docs/link.md").unwrap();
        }
        std::fs::write(
            "CLAUDE.md",
            "# Claude\n## Canonical sources\n- docs/folder.md\n- docs/link.md\n- docs/../docs/real.md\n## Other\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_docs_references(&mut diag, &crate::config::ExcludeSet::default());
        let findings: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|d| d.rule == LintRule::DocsRefMissing)
            .collect();
        assert!(
            findings
                .iter()
                .any(|f| f.evidence.as_deref() == Some("docs/folder.md")),
            "directory named *.md must emit D001"
        );
        assert!(
            findings
                .iter()
                .any(|f| f.evidence.as_deref() == Some("docs/../docs/real.md")),
            "parent-segment path must emit D001"
        );
        #[cfg(unix)]
        {
            assert!(
                findings
                    .iter()
                    .any(|f| f.evidence.as_deref() == Some("docs/link.md")),
                "symlink component must emit D001"
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn d001_crlf_plain_token() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::write(
            "CLAUDE.md",
            "# Claude\r\n## Canonical sources\r\n- docs/missing.md\r\n## Other\r\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_docs_references(&mut diag, &crate::config::ExcludeSet::default());
        let findings: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|d| d.rule == LintRule::DocsRefMissing)
            .collect();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].evidence.as_deref(), Some("docs/missing.md"));
    }

    #[test]
    #[serial_test::serial]
    fn d001_image_destination_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::write(
            "CLAUDE.md",
            "# Claude\n## Canonical sources\n![alt](docs/missing.md)\n## Other\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_docs_references(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(
            diag.diagnostics()
                .iter()
                .filter(|d| d.rule == LintRule::DocsRefMissing)
                .count(),
            0
        );
    }

    #[test]
    #[serial_test::serial]
    fn d001_no_claude_md_silent() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_docs_references(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn d001_skips_blockquote_and_example_heading() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::write(
            "CLAUDE.md",
            "# Claude\n## Canonical sources\n> docs/quoted.md\n### Example\ndocs/example.md\n## Other\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_docs_references(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(
            diag.diagnostics()
                .iter()
                .filter(|d| d.rule == LintRule::DocsRefMissing)
                .count(),
            0
        );
    }

    // D002: claudemd-too-large
    #[test]
    #[serial_test::serial]
    fn test_d002_claudemd_too_large() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let content = "line\n".repeat(501);
        std::fs::write("CLAUDE.md", &content).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_claudemd_size(&mut diag, &crate::config::ExcludeSet::default());
        let findings: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|d| d.rule == LintRule::ClaudemdTooLarge)
            .collect();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].evidence.as_deref(), Some("501 lines"));
        assert_eq!(findings[0].suggestion.as_deref(), Some(D002_SUGGESTION));
        assert!(findings[0].location.is_none());
    }

    #[test]
    #[serial_test::serial]
    fn test_d002_boundaries() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        for (count, expect) in [
            (0usize, false),
            (1, false),
            (499, false),
            (500, false),
            (501, true),
        ] {
            let content = if count == 0 {
                String::new()
            } else {
                "line\n".repeat(count)
            };
            std::fs::write("CLAUDE.md", &content).unwrap();
            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_claudemd_size(&mut diag, &crate::config::ExcludeSet::default());
            let hit = diag
                .diagnostics()
                .iter()
                .any(|d| d.rule == LintRule::ClaudemdTooLarge);
            assert_eq!(hit, expect, "line count {count}");
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_d002_final_newline_semantics() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        // 500 lines with final terminator: "line\n" * 500 => 500 lines via str::lines
        std::fs::write("CLAUDE.md", "line\n".repeat(500)).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_claudemd_size(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|d| d.rule == LintRule::ClaudemdTooLarge)
        );

        // 501 Unicode lines without a final newline
        let mut content = "λ\n".repeat(500);
        content.push('μ');
        std::fs::write("CLAUDE.md", content).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_claudemd_size(&mut diag, &crate::config::ExcludeSet::default());
        let findings: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|d| d.rule == LintRule::ClaudemdTooLarge)
            .collect();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].evidence.as_deref(), Some("501 lines"));
    }

    // D003: todo-in-docs
    #[test]
    #[serial_test::serial]
    fn test_d003_todo_outside_fence() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::write("CLAUDE.md", "# Docs\nTODO: finish this section\n").unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_claudemd_todos(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("TODO")));
    }

    #[test]
    #[serial_test::serial]
    fn test_d003_todo_inside_fence_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::write(
            "CLAUDE.md",
            "# Docs\n\n```bash\n# TODO: this is in a code block\n```\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_claudemd_todos(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("TODO")));
    }

    #[test]
    #[serial_test::serial]
    fn test_d003_todo_in_nested_fence_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        // 4-backtick fence containing 3-backtick line with TODO — should not trigger
        std::fs::write(
            "CLAUDE.md",
            "# Docs\n\n````\n```\n# TODO: nested fence content\n```\n````\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_claudemd_todos(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag.errors().iter().any(|e| e.contains("TODO")),
            "TODO inside nested 4-backtick fence should not trigger D003"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_d003_inline_code_prose_is_clean() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::write(
            "CLAUDE.md",
            "# Docs\nThe literal marker `TODO` is prohibited in committed instructions.\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_claudemd_todos(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_d003_reports_structured_marker_only() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::write(
            "CLAUDE.md",
            "# Docs\nTODO: finish release instructions\nTODO: second\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_claudemd_todos(&mut diag, &crate::config::ExcludeSet::default());
        let findings: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|d| d.rule == LintRule::TodoInDocs)
            .collect();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].evidence.as_deref(), Some("TODO"));
        assert_eq!(
            findings[0].location,
            Some(crate::diagnostic::SourceSpan::range(2, 1, 2, 5))
        );
        assert!(!findings[0].message.contains("finish release"));
    }

    #[test]
    #[serial_test::serial]
    fn test_d003_no_claudemd_silent() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_claudemd_todos(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }
}
