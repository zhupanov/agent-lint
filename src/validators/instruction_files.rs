//! Platform-neutral validation for shared `AGENTS.md` instruction files.

use crate::config::ExcludeSet;
use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata, SourceSpan};
use crate::fence::{CodeFenceTracker, LineClass};
use crate::live_instructions::{InstructionSurfaceKind, LiveInstructionDocument};
use crate::markdown::MarkdownDocument;
use crate::rules::LintRule;
use crate::sensitive::contains_possible_secret;
use crate::traversal;
use crate::validators::common::{
    classify_inline_code_path, is_unsafe_inline_code_path_probe, normalize_inline_code_path_probe,
};
use std::ops::Range;
use std::path::Path;

const CODEX_DEFAULT_MAX_BYTES: usize = 32_768;
const CODEX_HARD_MAX_BYTES: usize = 100_000;

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
    let codex_max_bytes = codex_active.then(|| project_doc_max_bytes(exclude));
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
            if let Some(max_bytes) = codex_max_bytes {
                validate_codex_rules(diag, exclude, &display, &content, max_bytes);
            }
        });
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
    if contains_possible_secret(content) {
        diag.report(
            LintRule::InstructionFileSecret,
            &format!("{display} contains a potential hardcoded secret/API key"),
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

fn validate_codex_rules(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    display: &str,
    content: &str,
    max_bytes: usize,
) {
    if content.len() > CODEX_HARD_MAX_BYTES {
        diag.report(
            LintRule::CodexAgentsTooLarge,
            &format!(
                "{display} exceeds Codex's {CODEX_HARD_MAX_BYTES}-byte hard limit ({} bytes)",
                content.len()
            ),
        );
    }
    if content.len() > max_bytes {
        diag.report(LintRule::CodexAgentsDocLimit, &format!("{display} exceeds Codex's effective project document limit of {max_bytes} bytes ({} bytes)", content.len()));
    }
    if agents_conflicts_with_config(content, exclude) {
        diag.report(
            LintRule::CodexAgentsConfigConflict,
            &format!("{display} explicitly contradicts a value in .codex/config.toml"),
        );
    }
}

fn project_doc_max_bytes(exclude: &ExcludeSet) -> usize {
    if exclude.is_excluded(".codex/config.toml") {
        return CODEX_DEFAULT_MAX_BYTES;
    }
    std::fs::read_to_string(".codex/config.toml")
        .ok()
        .and_then(|content| content.parse::<toml::Value>().ok())
        .and_then(|value| {
            value
                .get("project_doc_max_bytes")
                .and_then(toml::Value::as_integer)
        })
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
        .unwrap_or(CODEX_DEFAULT_MAX_BYTES)
}

fn validate_inline_paths(
    diag: &mut DiagnosticCollector,
    agents_path: &Path,
    display: &str,
    content: &str,
) {
    for reference in backtick_tokens(content) {
        if !classify_inline_code_path(reference.value).is_repository_path() {
            continue;
        }
        let probe = normalize_inline_code_path_probe(reference.value);
        let candidate = agents_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(probe);
        if is_unsafe_inline_code_path_probe(&candidate) || !candidate.exists() {
            let metadata = SourceSpan::from_byte_range(content, reference.range)
                .map_or_else(DiagnosticMetadata::default, |location| {
                    DiagnosticMetadata::default().with_location(location)
                })
                .with_evidence(reference.value);
            diag.report_with(
                LintRule::InstructionFilePathMissing,
                &format!(
                    "{display} references missing inline-code path `{}`",
                    reference.value
                ),
                metadata,
            );
            break;
        }
    }
}

struct BacktickToken<'a> {
    value: &'a str,
    range: Range<usize>,
}

fn backtick_tokens(content: &str) -> Vec<BacktickToken<'_>> {
    let mut result = Vec::new();
    let mut offset = 0;
    let mut fences = CodeFenceTracker::new();
    for raw_line in content.split_inclusive('\n') {
        let without_newline = raw_line.strip_suffix('\n').unwrap_or(raw_line);
        let line = without_newline
            .strip_suffix('\r')
            .unwrap_or(without_newline);
        if fences.process_line(line) == LineClass::Outside {
            backtick_tokens_in_line(line, offset, &mut result);
        }
        offset += raw_line.len();
    }
    result
}

fn backtick_tokens_in_line<'a>(line: &'a str, offset: usize, result: &mut Vec<BacktickToken<'a>>) {
    let mut line_offset = 0;
    while let Some(relative_start) = line[line_offset..].find('`') {
        let raw_start = line_offset + relative_start + 1;
        let Some(relative_end) = line[raw_start..].find('`') else {
            break;
        };
        let raw_end = raw_start + relative_end;
        let raw = &line[raw_start..raw_end];
        let token = raw.trim();
        if !token.is_empty() && !token.contains(char::is_whitespace) {
            let leading_whitespace_bytes = raw.len() - raw.trim_start().len();
            let token_start = offset + raw_start + leading_whitespace_bytes;
            result.push(BacktickToken {
                value: token,
                range: token_start..token_start + token.len(),
            });
        }
        line_offset = raw_end + 1;
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
            let normalized = normalize_prose(&mask_markdown_links(evidence));
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
    let mut matched = false;
    while !rest.is_empty() {
        if matched {
            if let Some(after_and) = rest.strip_prefix("and ") {
                rest = after_and.trim_start();
            } else if rest == "and" {
                return false;
            }
        }
        let Some(phrase) = GENERIC_GUIDANCE_PHRASES
            .iter()
            .copied()
            .find(|phrase| rest == *phrase || rest.starts_with(&format!("{phrase} ")))
        else {
            return false;
        };
        matched = true;
        rest = rest[phrase.len()..].trim_start();
    }
    matched
}

fn normalize_prose(text: &str) -> String {
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

fn agents_conflicts_with_config(content: &str, exclude: &ExcludeSet) -> bool {
    if exclude.is_excluded(".codex/config.toml") {
        return false;
    }
    let Ok(config) = std::fs::read_to_string(".codex/config.toml") else {
        return false;
    };
    let Ok(value) = config.parse::<toml::Value>() else {
        return false;
    };
    for key in ["approval_policy", "sandbox_mode", "project_doc_max_bytes"] {
        let Some(config_value) = value.get(key) else {
            continue;
        };
        let config_value = config_value.to_string().trim_matches('"').to_string();
        for line in content.lines() {
            let normalized = line.replace(['`', '"', '\''], "");
            let Some((mentioned_key, mentioned_value)) = normalized.split_once('=') else {
                continue;
            };
            if mentioned_key.trim() == key && mentioned_value.trim() != config_value {
                return true;
            }
        }
    }
    false
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
    fn shared_rules_run_without_codex_policy() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("nested/generic").unwrap();
        std::fs::write(
            "AGENTS.md",
            format!(
                "# Instructions\ntoken = sk-12345678901234567890\nSee `missing.md`.\n{}",
                "x".repeat(CODEX_DEFAULT_MAX_BYTES)
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
            LintRule::CodexAgentsTooLarge
                | LintRule::CodexAgentsDocLimit
                | LintRule::CodexAgentsConfigConflict
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
    fn codex_policy_uses_effective_limit_and_config() {
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
        std::fs::write(
            "nested/AGENTS.md",
            format!(
                "# Instructions\napproval_policy = \"on-request\"\n{}",
                "x".repeat(CODEX_HARD_MAX_BYTES + 1)
            ),
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents_files(&mut diag, &ExcludeSet::default(), true);

        assert!(diag.diagnostics().iter().any(|item| {
            item.rule == LintRule::CodexAgentsDocLimit && item.message.contains("nested/AGENTS.md")
        }));
        assert!(
            diag.diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::CodexAgentsTooLarge)
        );
        assert!(
            diag.diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::CodexAgentsConfigConflict)
        );
    }

    #[test]
    #[serial_test::serial]
    fn excluded_codex_config_does_not_affect_agents_policy() {
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

        assert!(!diag.diagnostics().iter().any(|item| matches!(
            item.rule,
            LintRule::CodexAgentsDocLimit | LintRule::CodexAgentsConfigConflict
        )));
    }
}
