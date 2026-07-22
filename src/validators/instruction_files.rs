//! Platform-neutral validation for shared `AGENTS.md` instruction files.

use crate::config::ExcludeSet;
use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata, SourceSpan};
use crate::live_instructions::{InstructionSurfaceKind, LiveInstructionDocument};
use crate::markdown::MarkdownDocument;
use crate::markdown_refs::{
    MarkdownRefKind, SUGGEST_CREATE_OR_CORRECT, SUGGEST_REPLACE_SYMLINK, markdown_references,
};
use crate::repo_path::{
    PathProbe, ResolutionBase, normalize_path_probe, normalize_separators, normalized_target_key,
    probe_contains_parent_segment, resolve_repo_path,
};
use crate::rules::LintRule;
use crate::sensitive::find_instruction_secret;
use crate::traversal;
use crate::validators::codex_config::{self, ProjectDocumentSettings};
use crate::validators::common::classify_inline_code_path;
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

const I002_SUGGESTION: &str =
    "replace the literal with an environment-variable or secret-store reference";

/// Validate every included `AGENTS.md`, applying Codex policy only when Codex is active.
#[cfg(test)]
pub fn validate_agents_files(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    codex_active: bool,
) {
    let mut prompt_pass = super::prompt_content::PromptContentPass::default();
    validate_agents_files_with_prompt_pass(diag, exclude, codex_active, &mut prompt_pass);
}

pub(crate) fn validate_agents_files_with_prompt_pass(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    codex_active: bool,
    prompt_pass: &mut super::prompt_content::PromptContentPass,
) {
    for entry in traversal::recursive_files(Path::new("."), Path::new("."), Some(exclude)).entries {
        if entry
            .path
            .file_name()
            .is_none_or(|name| name != "AGENTS.md")
        {
            continue;
        }
        let path = &entry.path;
        let display = entry.display;
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        let markdown = MarkdownDocument::parse(&content);
        let prompt_document = LiveInstructionDocument::new(
            Path::new(&display),
            InstructionSurfaceKind::AgentsMd,
            &markdown,
        );

        diag.with_subject_path(&display, |diag| {
            validate_shared_rules(diag, path, &display, &content, &prompt_document);
            prompt_pass.validate(&prompt_document, diag);
        });
    }
    if codex_active {
        validate_selected_codex_project_documents(diag, exclude);
    }
}

fn validate_shared_rules(
    diag: &mut DiagnosticCollector,
    path: &Path,
    display: &str,
    content: &str,
    document: &LiveInstructionDocument<'_>,
) {
    if content.trim().is_empty() {
        diag.report(
            LintRule::InstructionFileEmpty,
            &format!("{display} is empty or whitespace-only"),
        );
        return;
    }
    if let Some(finding) = find_instruction_secret(content) {
        let mut metadata = DiagnosticMetadata::default()
            .with_evidence(finding.evidence.as_str())
            .with_suggestion(I002_SUGGESTION);
        if let Some(location) = SourceSpan::from_byte_range(content, finding.location_range) {
            metadata = metadata.with_location(location);
        }
        diag.report_with(
            LintRule::InstructionFileSecret,
            &format!("{display} contains a potential hardcoded secret/API key"),
            metadata,
        );
    }
    validate_inline_paths(diag, path, display, content);
    if let Some(finding) = generic_only_guidance(content, document) {
        let metadata = SourceSpan::from_byte_range(content, finding.range.clone())
            .map_or_else(DiagnosticMetadata::default, |location| {
                DiagnosticMetadata::default().with_location(location)
            })
            .with_evidence(finding.evidence)
            .with_suggestion("add concrete project commands, paths, or constraints");
        diag.report_with(
            LintRule::InstructionFileGenericGuidance,
            &format!("{display} contains only generic guidance; add project-specific commands, paths, or constraints"),
            metadata,
        );
    }
}

#[derive(Debug)]
struct ProjectDocument {
    path: PathBuf,
    display: String,
    directory: PathBuf,
    content: String,
    byte_len: usize,
}

/// Codex chooses one project document in each directory. Discovery deliberately
/// uses the shared walker: it filters exclusions before selection, yields only
/// regular files, and never follows links outside the repository.
fn selected_codex_project_documents(
    exclude: &ExcludeSet,
    fallback_filenames: &[String],
) -> Vec<ProjectDocument> {
    let mut candidates: BTreeMap<PathBuf, Vec<traversal::WalkEntry>> = BTreeMap::new();
    for entry in traversal::recursive_files(Path::new("."), Path::new("."), Some(exclude)).entries {
        let Some(file_name) = entry.path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if file_name == "AGENTS.override.md"
            || file_name == "AGENTS.md"
            || fallback_filenames
                .iter()
                .any(|fallback| fallback == file_name)
        {
            let directory = entry
                .path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf();
            candidates.entry(directory).or_default().push(entry);
        }
    }

    candidates
        .into_values()
        .filter_map(|entries| {
            let selected = ["AGENTS.override.md", "AGENTS.md"]
                .into_iter()
                .map(str::to_owned)
                .chain(
                    fallback_filenames
                        .iter()
                        .filter(|name| is_safe_project_document_filename(name))
                        .cloned(),
                )
                .find_map(|name| {
                    entries.iter().find(|entry| {
                        entry
                            .path
                            .file_name()
                            .is_some_and(|file_name| file_name == std::ffi::OsStr::new(&name))
                    })
                })?;
            let bytes = std::fs::read(&selected.path).ok()?;
            let content = String::from_utf8(bytes).ok()?;
            Some(ProjectDocument {
                directory: selected.path.parent()?.to_path_buf(),
                path: selected.path.clone(),
                display: selected.display.clone(),
                byte_len: content.len(),
                content,
            })
        })
        .collect()
}

fn is_safe_project_document_filename(name: &str) -> bool {
    let path = Path::new(name);
    !name.is_empty()
        && path.components().count() == 1
        && matches!(path.components().next(), Some(Component::Normal(_)))
}

fn validate_selected_codex_project_documents(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let Some(settings) = codex_config::project_document_settings(exclude) else {
        return;
    };
    let documents = selected_codex_project_documents(exclude, &settings.fallback_filenames);
    let by_directory: BTreeMap<_, _> = documents
        .iter()
        .map(|document| (document.directory.clone(), document))
        .collect();

    for document in &documents {
        let chain = active_project_document_chain(document, &by_directory);
        let used = chain
            .iter()
            .take_while(|active| active.path != document.path)
            .fold(0usize, |used, active| used.saturating_add(active.byte_len))
            .min(settings.max_bytes);
        let remaining = settings.max_bytes.saturating_sub(used);
        let visible = document.byte_len.min(remaining);
        if document.byte_len > remaining {
            let metadata = DiagnosticMetadata::default()
                .with_evidence(format!("{used}/{} bytes", settings.max_bytes))
                .with_suggestion(
                    "reduce the active project-document chain or raise project_doc_max_bytes",
                )
                .with_related_subjects(chain.iter().map(|document| document.display.as_str()));
            diag.report_at_with(
                LintRule::CodexProjectDocBudget,
                &document.display,
                &format!(
                    "{} is partially or wholly omitted by Codex's cumulative project-document budget ({used}/{} bytes used; {} bytes in this document)",
                    document.display, settings.max_bytes, document.byte_len
                ),
                metadata,
            );
        }
        if visible > 0 {
            validate_project_document_conflicts(diag, document, &settings, visible);
        }
    }
}

fn active_project_document_chain<'a>(
    document: &'a ProjectDocument,
    by_directory: &BTreeMap<PathBuf, &'a ProjectDocument>,
) -> Vec<&'a ProjectDocument> {
    let mut directories = Vec::new();
    let mut current = Some(document.directory.as_path());
    while let Some(directory) = current {
        directories.push(directory.to_path_buf());
        current = directory.parent().filter(|parent| *parent != directory);
    }
    directories.reverse();
    directories
        .into_iter()
        .filter_map(|directory| by_directory.get(&directory).copied())
        .collect()
}

fn validate_inline_paths(
    diag: &mut DiagnosticCollector,
    agents_path: &Path,
    display: &str,
    content: &str,
) {
    let mut seen = BTreeSet::new();
    for reference in markdown_references(content) {
        if reference.kind != MarkdownRefKind::InlineCode {
            continue;
        }
        let classified = normalize_separators(&reference.raw);
        if !classify_inline_code_path(&classified).is_repository_path() {
            continue;
        }
        let probe = normalize_path_probe(&reference.raw);
        let rejected_parent = probe_contains_parent_segment(&probe);
        let key = if rejected_parent {
            format!("unsafe:{probe}")
        } else {
            normalized_target_key(agents_path, &reference.raw, ResolutionBase::SourceRelative)
                .unwrap_or_else(|| probe.clone())
        };
        if !seen.insert(key) {
            continue;
        }
        let outcome = if rejected_parent {
            PathProbe::Rejected
        } else {
            resolve_repo_path(agents_path, &reference.raw, ResolutionBase::SourceRelative)
        };
        let suggestion = match &outcome {
            PathProbe::File(_) | PathProbe::Directory(_) => continue,
            PathProbe::Missing(_) => SUGGEST_CREATE_OR_CORRECT,
            PathProbe::Rejected => SUGGEST_REPLACE_SYMLINK,
        };
        let metadata = SourceSpan::from_byte_range(content, reference.byte_range.clone())
            .map_or_else(DiagnosticMetadata::default, |location| {
                DiagnosticMetadata::default().with_location(location)
            })
            .with_evidence(&reference.raw)
            .with_suggestion(suggestion);
        diag.report_with(
            LintRule::InstructionFilePathMissing,
            &format!(
                "{display} references missing inline-code path `{}`",
                reference.raw
            ),
            metadata,
        );
    }
}

/// Exact generic phrases owned by I004. Longer phrases are listed first so
/// conjunction matching prefers complete phrases over accidental prefixes.
const GENERIC_GUIDANCE_PHRASES: &[&str] = &[
    "follow best practices",
    "write good code",
    "be helpful",
    "be accurate",
];

struct GenericGuidanceFinding {
    range: Range<usize>,
    evidence: String,
}

struct ProseClause {
    range: Range<usize>,
    normalized: String,
    evidence: String,
}

fn generic_only_guidance(
    content: &str,
    document: &LiveInstructionDocument<'_>,
) -> Option<GenericGuidanceFinding> {
    let headings = document.headings();
    let example_scopes = document.example_scopes();
    let mut clauses = Vec::new();

    for (line, is_example) in document.prose_lines().iter().zip(example_scopes) {
        if is_example || headings.iter().any(|heading| heading.line == line.line) {
            continue;
        }
        let Some(line_start) = line_start_offset(content, line.line) else {
            continue;
        };
        let (marker_bytes, unmarked) = strip_leading_list_marker(&line.text);
        collect_prose_clauses(line_start + marker_bytes, unmarked, &mut clauses);
    }

    let operative: Vec<_> = clauses
        .into_iter()
        .filter(|clause| !clause.normalized.is_empty())
        .collect();
    if operative.is_empty()
        || !operative
            .iter()
            .all(|clause| is_generic_conjunction(&clause.normalized))
    {
        return None;
    }
    let first = &operative[0];
    Some(GenericGuidanceFinding {
        range: first.range.clone(),
        evidence: first.evidence.clone(),
    })
}

fn collect_prose_clauses(line_start: usize, line: &str, clauses: &mut Vec<ProseClause>) {
    let mut index = 0;
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    while index < chars.len() {
        while index < chars.len() && chars[index].1.is_whitespace() {
            index += 1;
        }
        if index >= chars.len() {
            break;
        }
        let clause_start = chars[index].0;
        let mut clause_end = clause_start;
        let mut end_index = index;
        while end_index < chars.len() {
            let (offset, ch) = chars[end_index];
            if is_clause_terminator(&chars, end_index) {
                clause_end = offset;
                end_index += 1;
                break;
            }
            clause_end = offset + ch.len_utf8();
            end_index += 1;
        }
        let raw = &line[clause_start..clause_end];
        let evidence = raw.trim();
        if !evidence.is_empty() {
            let leading = raw.len() - raw.trim_start().len();
            let trailing = raw.len() - raw.trim_end().len();
            let normalized = normalize_generic_prose(&mask_markdown_links(evidence));
            clauses.push(ProseClause {
                range: (line_start + clause_start + leading)..(line_start + clause_end - trailing),
                normalized,
                evidence: evidence.to_string(),
            });
        }
        index = end_index;
    }
}

fn is_clause_terminator(chars: &[(usize, char)], index: usize) -> bool {
    match chars.get(index).map(|(_, character)| *character) {
        Some('!' | '?' | ';') => true,
        Some('.') => chars
            .get(index + 1)
            .is_none_or(|(_, next)| next.is_whitespace()),
        _ => false,
    }
}

fn is_generic_conjunction(normalized: &str) -> bool {
    let mut rest = normalized.trim();
    if rest.is_empty() {
        return false;
    }
    loop {
        let Some(phrase) = GENERIC_GUIDANCE_PHRASES.iter().copied().find(|phrase| {
            rest.strip_prefix(phrase).is_some_and(|suffix| {
                suffix.is_empty()
                    || suffix.starts_with(',')
                    || suffix.starts_with(char::is_whitespace)
            })
        }) else {
            return false;
        };
        rest = &rest[phrase.len()..];
        if rest.is_empty() {
            return true;
        }
        if let Some(after_comma) = rest.strip_prefix(',') {
            rest = after_comma.trim_start();
            if let Some(after_and) = rest.strip_prefix("and ") {
                rest = after_and.trim_start();
            }
            if rest.is_empty() {
                return false;
            }
            continue;
        }
        let after_space = rest.trim_start();
        let Some(after_and) = after_space.strip_prefix("and ") else {
            return false;
        };
        rest = after_and.trim_start();
        if rest.is_empty() {
            return false;
        }
    }
}

/// Normalize generic-guidance prose while retaining comma separators. I004's
/// grammar distinguishes `be helpful, be accurate` from unseparated adjacent
/// phrases, so a punctuation-erasing normalizer is not sufficient here.
fn normalize_generic_prose(text: &str) -> String {
    let mut normalized = String::new();
    let mut last_was_space = true;
    for character in text.chars() {
        if character.is_ascii_alphanumeric() {
            normalized.push(character.to_ascii_lowercase());
            last_was_space = false;
        } else if character.is_alphanumeric() {
            for lower in character.to_lowercase() {
                normalized.push(lower);
            }
            last_was_space = false;
        } else if character == ',' {
            while normalized.ends_with(' ') {
                normalized.pop();
            }
            normalized.push(',');
            last_was_space = false;
        } else if !last_was_space {
            normalized.push(' ');
            last_was_space = true;
        }
    }
    normalized.trim().to_string()
}

fn mask_markdown_links(text: &str) -> String {
    let mut chars: Vec<char> = text.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '!' && chars.get(index + 1) == Some(&'[') {
            if let Some(end) = link_span_end(&chars, index + 1) {
                chars[index..=end].fill(' ');
                index = end + 1;
                continue;
            }
        }
        if chars[index] == '[' {
            if let Some(end) = link_span_end(&chars, index) {
                chars[index..=end].fill(' ');
                index = end + 1;
                continue;
            }
        }
        if chars[index] == '<' {
            if let Some(end) = autolink_span_end(&chars, index) {
                chars[index..=end].fill(' ');
                index = end + 1;
                continue;
            }
        }
        index += 1;
    }
    chars.into_iter().collect()
}

fn link_span_end(chars: &[char], start: usize) -> Option<usize> {
    let mut index = start + 1;
    while index < chars.len() {
        if chars[index] == ']' {
            break;
        }
        if chars[index] == '[' {
            return None;
        }
        index += 1;
    }
    if index >= chars.len() || chars.get(index + 1) != Some(&'(') {
        return None;
    }
    index += 2;
    let mut depth = 1;
    while index < chars.len() {
        match chars[index] {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn autolink_span_end(chars: &[char], start: usize) -> Option<usize> {
    let body: String = chars
        .get(start + 1..)?
        .iter()
        .take_while(|character| **character != '>')
        .collect();
    if body.is_empty() || body.contains(char::is_whitespace) {
        return None;
    }
    let lower = body.to_ascii_lowercase();
    if !(lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("mailto:"))
    {
        return None;
    }
    Some(start + 1 + body.chars().count())
}

fn strip_leading_list_marker(text: &str) -> (usize, &str) {
    let trimmed_start = text.trim_start();
    let unmarked = trimmed_start
        .strip_prefix("- ")
        .or_else(|| trimmed_start.strip_prefix("* "))
        .or_else(|| trimmed_start.strip_prefix("+ "))
        .or_else(|| {
            let digits = trimmed_start
                .chars()
                .take_while(|character| character.is_ascii_digit())
                .count();
            if digits > 0 {
                trimmed_start
                    .get(digits..)
                    .and_then(|rest| rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")))
            } else {
                None
            }
        })
        .unwrap_or(trimmed_start);
    (text.len() - unmarked.len(), unmarked)
}

fn line_start_offset(content: &str, line_number: usize) -> Option<usize> {
    if line_number == 0 {
        return None;
    }
    let mut offset = 0;
    for (index, line) in content.split_inclusive('\n').enumerate() {
        if index + 1 == line_number {
            return Some(offset);
        }
        offset += line.len();
    }
    None
}

static PROJECT_DOCUMENT_ASSERTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^\s*(approval_policy|sandbox_mode|project_doc_max_bytes)\s*(?:=|:|\bis\b)\s*(.*?)\s*$")
        .expect("project-document assertion expression is valid")
});

fn validate_project_document_conflicts(
    diag: &mut DiagnosticCollector,
    document: &ProjectDocument,
    settings: &ProjectDocumentSettings,
    visible_bytes: usize,
) {
    let Some(config) = settings.config.as_ref() else {
        return;
    };
    // Codex only includes a prefix of each selected document. Never inspect
    // suffix bytes beyond that cutoff, and never treat a partial construct as a
    // complete assertion.
    let visible_bytes = floor_char_boundary(&document.content, visible_bytes);
    if visible_bytes == 0 {
        return;
    }
    let visible_content = &document.content[..visible_bytes];
    let markdown = MarkdownDocument::parse(visible_content);
    let live = LiveInstructionDocument::new(
        Path::new(&document.display),
        InstructionSurfaceKind::AgentsMd,
        &markdown,
    );
    for (line, is_example) in live.prose_lines().iter().zip(live.example_scopes()) {
        if is_example {
            continue;
        }
        let raw_line = visible_content
            .lines()
            .nth(line.line - 1)
            .unwrap_or_default();
        let Some(line_start) = line_start_offset(visible_content, line.line) else {
            continue;
        };
        let line_end = line_start + raw_line.len();
        if !line_wholly_visible(&document.content, visible_bytes, line_end) {
            continue;
        }
        let (marker_bytes, prose) = strip_leading_list_marker(raw_line);
        let Some(captures) = PROJECT_DOCUMENT_ASSERTION.captures(prose) else {
            continue;
        };
        let Some(key_match) = captures.get(1) else {
            continue;
        };
        let Some(value_match) = captures.get(2) else {
            continue;
        };
        let assertion_start = line_start + marker_bytes + key_match.start();
        let assertion_end = line_start + marker_bytes + value_match.end();
        if assertion_end > visible_bytes {
            continue;
        }
        if markdown.inline_code().iter().any(|code| {
            code.byte_range.start < assertion_end && assertion_start < code.byte_range.end
        }) || markdown.links().iter().any(|link| {
            link.byte_range.start < assertion_end && assertion_start < link.byte_range.end
        }) {
            continue;
        }
        let key = key_match.as_str().to_ascii_lowercase();
        let Some(config_value) = config.get(&key) else {
            continue;
        };
        let literal = value_match
            .as_str()
            .trim()
            .trim_end_matches(['.', '!', '?'])
            .trim_end();
        let Some(asserted_value) = parse_toml_scalar(literal) else {
            continue;
        };
        if asserted_value == *config_value {
            continue;
        }
        let value_offset = marker_bytes + value_match.start() + value_match.as_str().len()
            - value_match.as_str().trim_start().len();
        let value_start = line_start + value_offset;
        let value_end = value_start + literal.len();
        if value_end > visible_bytes {
            continue;
        }
        let metadata = SourceSpan::from_byte_range(visible_content, value_start..value_end)
            .map_or_else(DiagnosticMetadata::default, |location| {
                DiagnosticMetadata::default().with_location(location)
            })
            .with_evidence(&key)
            .with_suggestion("align this project instruction with .codex/config.toml or remove the runtime assertion");
        diag.report_at_with(
            LintRule::CodexProjectDocConflict,
            &document.display,
            &format!(
                "{} asserts {key} contrary to .codex/config.toml",
                document.display
            ),
            metadata,
        );
    }
}

fn floor_char_boundary(content: &str, index: usize) -> usize {
    if index >= content.len() {
        content.len()
    } else {
        let mut index = index;
        while index > 0 && !content.is_char_boundary(index) {
            index -= 1;
        }
        index
    }
}

/// A line is complete only when its content is wholly inside the visible prefix
/// and either the document ends there or the line terminator is visible.
fn line_wholly_visible(content: &str, visible_bytes: usize, line_end: usize) -> bool {
    if line_end > visible_bytes {
        return false;
    }
    if visible_bytes >= content.len() {
        return true;
    }
    matches!(content.as_bytes().get(line_end), Some(b'\n' | b'\r'))
}

fn parse_toml_scalar(literal: &str) -> Option<toml::Value> {
    let value = format!("value = {literal}").parse::<toml::Value>().ok()?;
    let value = value.get("value")?.clone();
    (value.is_str()
        || value.is_integer()
        || value.is_float()
        || value.is_bool()
        || value.is_datetime())
    .then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BARE_EXTENSION_FIXTURE: &str =
        include_str!("../../tests/fixtures/instruction_files/bare_extension/AGENTS.md");

    #[test]
    #[serial_test::serial]
    fn bare_extension_fixture_only_reports_the_neighboring_missing_path() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::write("AGENTS.md", BARE_EXTENSION_FIXTURE).unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents_files(&mut diag, &ExcludeSet::default(), false);

        let findings: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::InstructionFilePathMissing)
            .collect();
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].subject_path.as_deref(),
            Some(Path::new("AGENTS.md"))
        );
        assert_eq!(findings[0].evidence.as_deref(), Some("docs/missing.md"));
        assert_eq!(
            findings[0].location.map(|span| span.start().line_number()),
            Some(5)
        );
    }

    #[test]
    #[serial_test::serial]
    fn dotfile_references_are_existence_sensitive() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        for dotfile in [".env", ".gitignore", ".cursorrules", ".mcp.json"] {
            std::fs::write(dotfile, "present\n").unwrap();
            let content = format!("# Instructions\nUse `{dotfile}` for project settings.\n");
            let mut existing_diag = DiagnosticCollector::new_all_enabled();
            existing_diag.with_subject_path("AGENTS.md", |diag| {
                validate_inline_paths(diag, Path::new("AGENTS.md"), "AGENTS.md", &content)
            });
            assert!(
                existing_diag
                    .diagnostics()
                    .iter()
                    .all(|item| item.rule != LintRule::InstructionFilePathMissing),
                "existing {dotfile} should not report"
            );

            std::fs::remove_file(dotfile).unwrap();
            let mut missing_diag = DiagnosticCollector::new_all_enabled();
            missing_diag.with_subject_path("AGENTS.md", |diag| {
                validate_inline_paths(diag, Path::new("AGENTS.md"), "AGENTS.md", &content)
            });
            let finding = missing_diag
                .diagnostics()
                .iter()
                .find(|item| item.rule == LintRule::InstructionFilePathMissing)
                .unwrap_or_else(|| panic!("missing {dotfile} should report"));
            assert_eq!(
                finding.subject_path.as_deref(),
                Some(Path::new("AGENTS.md"))
            );
            assert_eq!(finding.evidence.as_deref(), Some(dotfile));
            assert!(finding.location.is_some());
        }
    }

    #[test]
    #[serial_test::serial]
    fn each_concrete_missing_path_shape_reports_i003() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        for reference in [
            "missing.md",
            "missing.ts",
            "docs/missing.md",
            "docs/missing.ts",
            "./missing",
            "nested/path/missing.json",
        ] {
            let content = format!("# Instructions\nSee `{reference}`.\n");
            let mut diag = DiagnosticCollector::new_all_enabled();
            diag.with_subject_path("AGENTS.md", |diag| {
                validate_inline_paths(diag, Path::new("AGENTS.md"), "AGENTS.md", &content)
            });

            let finding = diag
                .diagnostics()
                .iter()
                .find(|item| item.rule == LintRule::InstructionFilePathMissing)
                .unwrap_or_else(|| panic!("missing path {reference} should report"));
            assert_eq!(finding.evidence.as_deref(), Some(reference));
            assert!(finding.location.is_some());
        }
    }

    #[test]
    #[serial_test::serial]
    fn i003_normalizes_fragments_and_symbol_suffixes_before_probing() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir("docs").unwrap();
        std::fs::write("docs/README.md", "present\n").unwrap();
        std::fs::write("src.rs", "present\n").unwrap();
        let content = "# Instructions\nSee `docs/README.md#usage` and `src.rs::main`.\n";
        let mut diag = DiagnosticCollector::new_all_enabled();

        diag.with_subject_path("AGENTS.md", |diag| {
            validate_inline_paths(diag, Path::new("AGENTS.md"), "AGENTS.md", content)
        });

        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::InstructionFilePathMissing)
        );
    }

    #[test]
    #[serial_test::serial]
    fn i003_reports_parent_escaping_probes() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir("docs").unwrap();
        std::fs::write("outside.md", "present\n").unwrap();
        let content = "# Instructions\nSee `docs/../outside.md`.\n";
        let mut diag = DiagnosticCollector::new_all_enabled();

        diag.with_subject_path("AGENTS.md", |diag| {
            validate_inline_paths(diag, Path::new("AGENTS.md"), "AGENTS.md", content)
        });

        assert_eq!(
            diag.diagnostics()
                .iter()
                .find(|item| item.rule == LintRule::InstructionFilePathMissing)
                .and_then(|item| item.evidence.as_deref()),
            Some("docs/../outside.md")
        );
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn i003_reports_symlink_probes() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::write("present.md", "present\n").unwrap();
        std::os::unix::fs::symlink("present.md", "link.md").unwrap();
        let content = "# Instructions\nSee `link.md`.\n";
        let mut diag = DiagnosticCollector::new_all_enabled();

        diag.with_subject_path("AGENTS.md", |diag| {
            validate_inline_paths(diag, Path::new("AGENTS.md"), "AGENTS.md", content)
        });

        assert_eq!(
            diag.diagnostics()
                .iter()
                .find(|item| item.rule == LintRule::InstructionFilePathMissing)
                .and_then(|item| item.evidence.as_deref()),
            Some("link.md")
        );
    }

    #[test]
    #[serial_test::serial]
    fn i003_ignores_path_tokens_inside_fences() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let content = "# Instructions\n```\ndocs/missing.md\n```\n";
        let mut diag = DiagnosticCollector::new_all_enabled();

        diag.with_subject_path("AGENTS.md", |diag| {
            validate_inline_paths(diag, Path::new("AGENTS.md"), "AGENTS.md", content)
        });

        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::InstructionFilePathMissing)
        );
    }

    #[test]
    #[serial_test::serial]
    fn well_known_dot_directory_references_are_existence_sensitive() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        for directory in [
            ".claude",
            ".claude-plugin",
            ".github",
            ".vscode",
            ".codex",
            ".cursor",
            ".venv",
            ".husky",
            ".idea",
            ".devcontainer",
        ] {
            std::fs::create_dir(directory).unwrap();
            let content = format!("# Instructions\nUse `{directory}`.\n");
            let mut existing_diag = DiagnosticCollector::new_all_enabled();
            existing_diag.with_subject_path("AGENTS.md", |diag| {
                validate_inline_paths(diag, Path::new("AGENTS.md"), "AGENTS.md", &content)
            });
            assert!(
                !existing_diag
                    .diagnostics()
                    .iter()
                    .any(|item| { item.rule == LintRule::InstructionFilePathMissing })
            );

            std::fs::remove_dir(directory).unwrap();
            let mut missing_diag = DiagnosticCollector::new_all_enabled();
            missing_diag.with_subject_path("AGENTS.md", |diag| {
                validate_inline_paths(diag, Path::new("AGENTS.md"), "AGENTS.md", &content)
            });
            assert_eq!(
                missing_diag
                    .diagnostics()
                    .iter()
                    .find(|item| item.rule == LintRule::InstructionFilePathMissing)
                    .and_then(|item| item.evidence.as_deref()),
                Some(directory)
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn i003_safe_path_does_not_suppress_parent_traversing_duplicate() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::write("outside.md", "present\n").unwrap();
        let content = "See `outside.md` and `docs/../outside.md`.\n";
        let mut diag = DiagnosticCollector::new_all_enabled();
        diag.with_subject_path("AGENTS.md", |diag| {
            validate_inline_paths(diag, Path::new("AGENTS.md"), "AGENTS.md", content)
        });
        let findings: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::InstructionFilePathMissing)
            .collect();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].evidence.as_deref(), Some("docs/../outside.md"));
        assert_eq!(
            findings[0].suggestion.as_deref(),
            Some(SUGGEST_REPLACE_SYMLINK)
        );
    }

    #[test]
    #[serial_test::serial]
    fn i003_reports_every_distinct_path_including_double_backticks() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let content = "See `docs/first-missing.md` and ``docs/third-missing.md`` plus `docs/first-missing.md` again.\n";
        let mut diag = DiagnosticCollector::new_all_enabled();
        diag.with_subject_path("AGENTS.md", |diag| {
            validate_inline_paths(diag, Path::new("AGENTS.md"), "AGENTS.md", content)
        });
        let findings: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::InstructionFilePathMissing)
            .collect();
        assert_eq!(findings.len(), 2);
        assert_eq!(
            findings[0].evidence.as_deref(),
            Some("docs/first-missing.md")
        );
        assert_eq!(
            findings[1].evidence.as_deref(),
            Some("docs/third-missing.md")
        );
        assert_eq!(
            findings[0].suggestion.as_deref(),
            Some(SUGGEST_CREATE_OR_CORRECT)
        );
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn i003_rejects_ancestor_symlink_components() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("outside").unwrap();
        std::fs::write("outside/present.md", "ok\n").unwrap();
        std::fs::create_dir_all("docs").unwrap();
        std::os::unix::fs::symlink("../outside", "docs/external").unwrap();
        let content = "See `docs/external/present.md`.\n";
        let mut diag = DiagnosticCollector::new_all_enabled();
        diag.with_subject_path("AGENTS.md", |diag| {
            validate_inline_paths(diag, Path::new("AGENTS.md"), "AGENTS.md", content)
        });
        let finding = diag
            .diagnostics()
            .iter()
            .find(|item| item.rule == LintRule::InstructionFilePathMissing)
            .expect("ancestor symlink must report");
        assert_eq!(
            finding.evidence.as_deref(),
            Some("docs/external/present.md")
        );
        assert_eq!(finding.suggestion.as_deref(), Some(SUGGEST_REPLACE_SYMLINK));
    }

    #[test]
    #[serial_test::serial]
    fn shared_rules_run_without_codex_policy() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("nested/generic").unwrap();
        std::fs::write(
            "AGENTS.md",
            format!(
                "# Instructions\ntoken = sk-12345678901234567890\nSee `missing.md`.\n{}",
                "x".repeat(codex_config::CODEX_DEFAULT_PROJECT_DOC_MAX_BYTES)
            ),
        )
        .unwrap();
        std::fs::write("nested/AGENTS.md", " \n\t").unwrap();
        std::fs::write(
            "nested/generic/AGENTS.md",
            "Be helpful and write good code.",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents_files(&mut diag, &ExcludeSet::default(), false);

        for rule in [
            LintRule::InstructionFileEmpty,
            LintRule::InstructionFileSecret,
            LintRule::InstructionFilePathMissing,
            LintRule::InstructionFileGenericGuidance,
        ] {
            assert!(
                diag.diagnostics().iter().any(|item| item.rule == rule),
                "missing {}",
                rule.code()
            );
        }
        assert!(!diag.diagnostics().iter().any(|item| matches!(
            item.rule,
            LintRule::CodexProjectDocBudget | LintRule::CodexProjectDocConflict
        )));
    }

    fn report_i004(content: &str) -> Option<crate::diagnostic::Diagnostic> {
        let markdown = MarkdownDocument::parse(content);
        let document = LiveInstructionDocument::new(
            Path::new("AGENTS.md"),
            InstructionSurfaceKind::AgentsMd,
            &markdown,
        );
        let mut diag = DiagnosticCollector::new();
        diag.with_subject_path("AGENTS.md", |diag| {
            validate_shared_rules(
                diag,
                Path::new("AGENTS.md"),
                "AGENTS.md",
                content,
                &document,
            );
        });
        diag.diagnostics()
            .iter()
            .find(|item| item.rule == LintRule::InstructionFileGenericGuidance)
            .cloned()
    }

    #[test]
    fn i004_table_covers_exact_phrases_conjunctions_and_organization() {
        for content in [
            "Be helpful.",
            "BE ACCURATE!",
            "Write good code?",
            "Follow best practices",
            "Be helpful and write good code.",
            "Be helpful, be accurate, and follow best practices.",
            "Be helpful, be accurate, follow best practices.",
            "# Guidance\nBe helpful.\n",
            "Be helpful.\nBe accurate.\n",
            "- Be helpful\n",
            "1. Follow best practices\n",
        ] {
            let finding =
                report_i004(content).unwrap_or_else(|| panic!("expected I004 for {content:?}"));
            assert_eq!(
                LintRule::InstructionFileGenericGuidance.default_severity(),
                crate::rules::DefaultSeverity::Warning
            );
            assert_eq!(finding.severity, crate::diagnostic::Severity::Warning);
            assert_eq!(
                finding.suggestion.as_deref(),
                Some("add concrete project commands, paths, or constraints")
            );
            assert!(finding.location.is_some(), "missing span for {content:?}");
            assert!(
                finding
                    .evidence
                    .as_ref()
                    .is_some_and(|evidence| !evidence.is_empty())
            );
        }
    }

    #[test]
    fn i004_hard_negatives_stay_clean() {
        for content in [
            "Run cargo test before each commit and never modify generated protobufs.",
            "Follow best practices when updating Acme billing schema 17; preserve audit event order.",
            "Be helpfully specific about crate boundaries.",
            "Please be helpful about release notes and write changelog entries.",
            "Be helpful be accurate.",
            "Write good code follow best practices.",
            "Run cargo test before each commit.\n",
            "```\nBe helpful.\n```\n",
            "> Be helpful.\n",
            "The guide says \"Be helpful.\"\nThen run cargo test.\n",
            "See [Be helpful](./README.md) for tone.\nRun cargo test.\n",
            "## Examples\nBe helpful.\n## Real rules\nRun cargo test.\n",
            "For example, be helpful.\nRun cargo test.\n",
            " \n\t",
        ] {
            assert!(
                report_i004(content).is_none(),
                "unexpected I004 for {content:?}"
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn empty_agents_emits_only_i001() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::write("AGENTS.md", " \n\t\n").unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents_files(&mut diag, &ExcludeSet::default(), false);

        let rules: Vec<_> = diag.diagnostics().iter().map(|item| item.rule).collect();
        assert_eq!(rules, vec![LintRule::InstructionFileEmpty]);
    }

    #[test]
    fn i004_span_stays_aligned_when_earlier_multibyte_link_is_masked() {
        let content = "[文档](./README.md). Be helpful.\n";
        let finding = report_i004(content).expect("expected I004");
        assert_eq!(finding.evidence.as_deref(), Some("Be helpful"));
        let start = content.find("Be helpful").expect("clause text present");
        assert_eq!(
            finding.location,
            SourceSpan::from_byte_range(content, start..start + "Be helpful".len())
        );
    }

    #[test]
    #[serial_test::serial]
    fn i004_emits_once_with_first_clause_span() {
        let content = "Be helpful.\nWrite good code.\n";
        let finding = report_i004(content).expect("expected I004");
        assert_eq!(finding.evidence.as_deref(), Some("Be helpful"));
        assert_eq!(
            finding.location.map(|span| span.start().line_number()),
            Some(1)
        );
        assert_eq!(
            diag_rule_count(content, LintRule::InstructionFileGenericGuidance),
            1
        );
    }

    fn diag_rule_count(content: &str, rule: LintRule) -> usize {
        let markdown = MarkdownDocument::parse(content);
        let document = LiveInstructionDocument::new(
            Path::new("AGENTS.md"),
            InstructionSurfaceKind::AgentsMd,
            &markdown,
        );
        let mut diag = DiagnosticCollector::new();
        diag.with_subject_path("AGENTS.md", |diag| {
            validate_shared_rules(
                diag,
                Path::new("AGENTS.md"),
                "AGENTS.md",
                content,
                &document,
            );
        });
        diag.diagnostics()
            .iter()
            .filter(|item| item.rule == rule)
            .count()
    }

    #[test]
    #[serial_test::serial]
    fn i002_reports_unquoted_password_without_leaking_value() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let secret = "hunterhunter";
        std::fs::write(
            "AGENTS.md",
            format!("# Instructions\npassword = {secret}\nAlso see docs/setup.md and run `cargo test`.\n"),
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents_files(&mut diag, &ExcludeSet::default(), false);

        let finding = diag
            .diagnostics()
            .iter()
            .find(|item| item.rule == LintRule::InstructionFileSecret)
            .expect("I002 should fire");
        assert_eq!(finding.evidence.as_deref(), Some("password"));
        assert_eq!(finding.suggestion.as_deref(), Some(I002_SUGGESTION));
        assert_eq!(
            finding.location.map(|span| span.start().line_number()),
            Some(2)
        );
        assert!(!finding.message.contains(secret));

        let mut text = Vec::new();
        diag.render_text(&mut text);
        let text = String::from_utf8(text).unwrap();
        assert!(!text.contains(secret));
        assert!(!text.contains("password = "));

        let serialized = serde_json::json!({
            "message": finding.message,
            "evidence": finding.evidence,
            "suggestion": finding.suggestion,
        })
        .to_string();
        assert!(!serialized.contains(secret));
        assert!(!serialized.contains("password = "));
        assert!(serialized.contains("\"evidence\":\"password\""));
    }

    #[test]
    #[serial_test::serial]
    fn i002_emits_once_per_file_for_earliest_byte_match() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::write(
            "AGENTS.md",
            "# Instructions\npassword = first-secret\napi_key = second-secret\nsk-abcdefghijklmnopqrstuvwxyz\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents_files(&mut diag, &ExcludeSet::default(), false);
        let secrets: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::InstructionFileSecret)
            .collect();
        assert_eq!(secrets.len(), 1);
        assert_eq!(secrets[0].evidence.as_deref(), Some("password"));
    }

    #[test]
    #[serial_test::serial]
    fn codex_project_document_selection_applies_cumulative_budget_and_live_conflicts() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir(".codex").unwrap();
        std::fs::create_dir("nested").unwrap();
        std::fs::write(
            ".codex/config.toml",
            "project_doc_max_bytes = 100\napproval_policy = \"never\"\n",
        )
        .unwrap();
        std::fs::write("AGENTS.md", "a".repeat(70)).unwrap();
        std::fs::write(
            "nested/AGENTS.md",
            "approval_policy = \"on-request\"\n".to_string() + &"x".repeat(40),
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents_files(&mut diag, &ExcludeSet::default(), true);

        assert!(diag.diagnostics().iter().any(|item| {
            item.rule == LintRule::CodexProjectDocBudget
                && item.subject_path.as_deref() == Some(Path::new("nested/AGENTS.md"))
        }));
        assert!(
            diag.diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::CodexProjectDocConflict)
        );
    }

    #[test]
    #[serial_test::serial]
    fn excluded_codex_config_uses_default_budget_without_conflict_analysis() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir(".codex").unwrap();
        std::fs::write(
            ".codex/config.toml",
            "project_doc_max_bytes = 100\napproval_policy = \"never\"\n",
        )
        .unwrap();
        std::fs::write(
            "AGENTS.md",
            format!(
                "# Instructions\napproval_policy = \"on-request\"\n{}",
                "x".repeat(100)
            ),
        )
        .unwrap();
        let exclude = ExcludeSet::new(&[".codex/config.toml".into()]).unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents_files(&mut diag, &exclude, true);

        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|item| matches!(item.rule, LintRule::CodexProjectDocConflict))
        );
    }

    #[test]
    #[serial_test::serial]
    fn codex_project_document_precedence_exclusions_and_utf8_budget_are_deterministic() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("nested").unwrap();
        std::fs::create_dir_all(".codex").unwrap();
        std::fs::write(
            ".codex/config.toml",
            "project_doc_max_bytes = 4\nproject_doc_fallback_filenames = [\"PROJECT.md\"]\n",
        )
        .unwrap();
        std::fs::write("AGENTS.override.md", "ab").unwrap();
        std::fs::write("AGENTS.md", "this inactive sibling is deliberately long").unwrap();
        std::fs::write("nested/PROJECT.md", "éé").unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents_files(&mut diag, &ExcludeSet::default(), true);
        let budgets: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|finding| finding.rule == LintRule::CodexProjectDocBudget)
            .collect();
        assert_eq!(budgets.len(), 1);
        assert_eq!(
            budgets[0].subject_path.as_deref(),
            Some(Path::new("nested/PROJECT.md"))
        );
        assert_eq!(budgets[0].evidence.as_deref(), Some("2/4 bytes"));
        assert_eq!(
            budgets[0].related_subjects,
            vec![
                PathBuf::from("AGENTS.override.md"),
                PathBuf::from("nested/PROJECT.md")
            ]
        );

        let excluded_override = ExcludeSet::new(&["AGENTS.override.md".into()]).unwrap();
        let selected = selected_codex_project_documents(&excluded_override, &["PROJECT.md".into()]);
        assert!(
            selected
                .iter()
                .any(|document| document.display == "AGENTS.md")
        );
        assert!(
            !selected
                .iter()
                .any(|document| document.display == "AGENTS.override.md")
        );
    }

    #[test]
    #[serial_test::serial]
    fn codex_project_document_conflicts_only_scan_live_selected_prose() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".codex").unwrap();
        std::fs::write(
            ".codex/config.toml",
            "approval_policy = \"never\"\nsandbox_mode = \"workspace-write\"\nproject_doc_max_bytes = 4096\n",
        )
        .unwrap();
        std::fs::write(
            "AGENTS.md",
            "---\nsandbox_mode: \"read-only\"\n---\n```toml\nsandbox_mode = \"read-only\"\n```\n> sandbox_mode = \"read-only\"\n`sandbox_mode = \"read-only\"`\n[sandbox_mode = \"read-only\"](https://example.com)\n## Examples\n- approval_policy = \"on-request\"\n## Live\n- sandbox_mode: \"read-only\"\nproject_doc_max_bytes is 4096.\n",
        )
        .unwrap();
        std::fs::write(
            "AGENTS.override.md",
            "selected override without assertions\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents_files(&mut diag, &ExcludeSet::default(), true);
        assert!(
            diag.diagnostics()
                .iter()
                .all(|finding| finding.rule != LintRule::CodexProjectDocConflict)
        );

        std::fs::remove_file("AGENTS.override.md").unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents_files(&mut diag, &ExcludeSet::default(), true);
        let conflicts: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|finding| finding.rule == LintRule::CodexProjectDocConflict)
            .collect();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].evidence.as_deref(), Some("sandbox_mode"));
        assert_eq!(
            conflicts[0].suggestion.as_deref(),
            Some(
                "align this project instruction with .codex/config.toml or remove the runtime assertion"
            )
        );
    }

    #[test]
    #[serial_test::serial]
    fn cx045_only_scans_model_visible_project_document_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".codex").unwrap();
        std::fs::create_dir_all("nested").unwrap();

        // Zero budget: wholly omitted root document is CX040 only.
        std::fs::write(
            ".codex/config.toml",
            "project_doc_max_bytes = 0\napproval_policy = \"never\"\n",
        )
        .unwrap();
        std::fs::write("AGENTS.md", "approval_policy = \"on-request\"\n").unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents_files(&mut diag, &ExcludeSet::default(), true);
        assert!(diag.diagnostics().iter().any(|item| {
            item.rule == LintRule::CodexProjectDocBudget
                && item.subject_path.as_deref() == Some(Path::new("AGENTS.md"))
        }));
        assert!(
            diag.diagnostics()
                .iter()
                .all(|item| { item.rule != LintRule::CodexProjectDocConflict })
        );

        // Partial document: only conflicts wholly before the cutoff emit.
        std::fs::write(
            ".codex/config.toml",
            "project_doc_max_bytes = 31\napproval_policy = \"never\"\nsandbox_mode = \"workspace-write\"\n",
        )
        .unwrap();
        // First line is 30 content bytes + newline; second line is omitted.
        std::fs::write(
            "AGENTS.md",
            "approval_policy = \"on-request\"\nsandbox_mode = \"read-only\"\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents_files(&mut diag, &ExcludeSet::default(), true);
        let conflicts: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::CodexProjectDocConflict)
            .collect();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].evidence.as_deref(), Some("approval_policy"));

        // Clause split by the cutoff does not emit.
        std::fs::write(
            ".codex/config.toml",
            "project_doc_max_bytes = 20\napproval_policy = \"never\"\n",
        )
        .unwrap();
        std::fs::write("AGENTS.md", "approval_policy = \"on-request\"\n").unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents_files(&mut diag, &ExcludeSet::default(), true);
        assert!(
            diag.diagnostics()
                .iter()
                .any(|item| { item.rule == LintRule::CodexProjectDocBudget })
        );
        assert!(
            diag.diagnostics()
                .iter()
                .all(|item| { item.rule != LintRule::CodexProjectDocConflict })
        );

        // Cumulative chain: ancestor consumes budget; descendant conflict omitted.
        std::fs::write(
            ".codex/config.toml",
            "project_doc_max_bytes = 10\napproval_policy = \"never\"\n",
        )
        .unwrap();
        std::fs::write("AGENTS.md", "aaaaaaaaaa").unwrap();
        std::fs::write("nested/AGENTS.md", "approval_policy = \"on-request\"\n").unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents_files(&mut diag, &ExcludeSet::default(), true);
        assert!(diag.diagnostics().iter().any(|item| {
            item.rule == LintRule::CodexProjectDocBudget
                && item.subject_path.as_deref() == Some(Path::new("nested/AGENTS.md"))
        }));
        assert!(
            diag.diagnostics()
                .iter()
                .all(|item| { item.rule != LintRule::CodexProjectDocConflict })
        );

        // UTF-8 boundary immediately inside a multibyte scalar does not panic.
        std::fs::write(
            ".codex/config.toml",
            "project_doc_max_bytes = 2\napproval_policy = \"never\"\n",
        )
        .unwrap();
        std::fs::write("AGENTS.md", "éapproval_policy = \"on-request\"\n").unwrap();
        std::fs::remove_file("nested/AGENTS.md").unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents_files(&mut diag, &ExcludeSet::default(), true);
        assert!(
            diag.diagnostics()
                .iter()
                .all(|item| { item.rule != LintRule::CodexProjectDocConflict })
        );
    }

    #[test]
    #[serial_test::serial]
    fn invalid_project_document_config_skips_dependent_budget_and_conflict_findings() {
        for config in [
            "project_doc_max_bytes = -1\napproval_policy = \"never\"\n",
            "project_doc_fallback_filenames = [1]\napproval_policy = \"never\"\n",
            "project_doc_max_bytes = [invalid\n",
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let _guard = crate::test_helpers::CwdGuard::new();
            std::env::set_current_dir(tmp.path()).unwrap();
            std::fs::create_dir_all(".codex").unwrap();
            std::fs::write(".codex/config.toml", config).unwrap();
            std::fs::write(
                "AGENTS.md",
                "approval_policy = \"on-request\"\n".to_string() + &"x".repeat(40_000),
            )
            .unwrap();

            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_agents_files(&mut diag, &ExcludeSet::default(), true);
            assert!(diag.diagnostics().iter().all(|finding| !matches!(
                finding.rule,
                LintRule::CodexProjectDocBudget | LintRule::CodexProjectDocConflict
            )));
        }
    }
}
