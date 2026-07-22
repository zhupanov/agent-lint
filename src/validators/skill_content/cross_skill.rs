use crate::config::ExcludeSet;
use crate::diagnostic::DiagnosticCollector;
use crate::markdown::MarkdownDocument;
use crate::rules::LintRule;
use crate::traversal;
use crate::validators::shared_md_refs::{contains_shared_md_ref, find_shared_md_refs};
use crate::validators::skills::SkillInfo;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

/// S048: denylist for non-descriptive reference file names in skill directories.
/// Matches generic stems (doc, file, ref, data, info, tmp, test) with optional
/// digits, single letters (case-insensitive), and pure numeric names — all with .md extension.
static RE_GENERIC_REF_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i:^(?:(?:doc|file|ref|data|info|tmp|test)\d*|[a-z]|\d+)\.md$)").unwrap()
});

const REF_NO_TOC_THRESHOLD: usize = 100;

/// S029: Check for deeply nested shared markdown references.
/// Matches `$CLAUDE_PLUGIN_ROOT/<base_dir>/shared/*.md` references.
pub(super) fn validate_nested_references(
    base_dir: &str,
    skills: &[SkillInfo],
    diag: &mut DiagnosticCollector,
) {
    let shared_dir = Path::new(base_dir).join("shared");
    if !shared_dir.is_dir() {
        return;
    }

    // Cache: which shared .md files are nested (avoids re-reading files from disk)
    let mut checked: HashSet<String> = HashSet::new();
    let mut nested: HashSet<String> = HashSet::new();

    for info in skills {
        // Find shared-md references in this skill's body
        for shared_ref in find_shared_md_refs(&info.body, base_dir) {
            let rel = &shared_ref.relative_path;
            let rel_path = Path::new(rel);

            if !rel_path.is_file() {
                continue; // S008 handles missing refs
            }

            // Check the file once for nesting, cache result
            if !checked.contains(rel) {
                checked.insert(rel.clone());
                if let Ok(content) = fs::read_to_string(rel_path) {
                    if contains_shared_md_ref(&content, base_dir) {
                        nested.insert(rel.clone());
                    }
                }
            }

            // Report for every referencing skill (not just the first)
            if nested.contains(rel) {
                diag.report_at(
                    LintRule::NestedRefDeep,
                    &info.path,
                    &format!(
                        "{}: references {} which itself references other shared .md files (keep references one level deep)",
                        info.path, shared_ref.reference
                    ),
                );
            }
        }
    }
}

/// S030: Detect orphaned files in skill scripts/ subdirectories.
pub(super) fn validate_orphaned_skill_files(
    base_dir: &str,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
) {
    let dir = Path::new(base_dir);
    if !dir.is_dir() {
        return;
    }

    for entry in traversal::shallow_directories(dir, Path::new("."), None).entries {
        let path = entry.path;
        let dir_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if dir_name == "shared" {
            continue;
        }

        let skill_path = format!("{base_dir}/{dir_name}/SKILL.md");
        if exclude.is_excluded(&skill_path) {
            continue;
        }

        let scripts_dir = path.join("scripts");
        if !scripts_dir.is_dir() {
            continue;
        }

        let docs = read_skill_markdown_docs(&path);

        let scripts = traversal::recursive_files_with_pruning(
            &scripts_dir,
            Path::new("."),
            None,
            traversal::should_descend_except_git,
        )
        .entries;
        let basename_counts = scripts.iter().fold(HashMap::new(), |mut counts, script| {
            let basename = script
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            *counts.entry(basename.into_owned()).or_insert(0usize) += 1;
            counts
        });

        for script in scripts {
            let script_relative = traversal::display_path(&path, &script.path);
            let script_name = script
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            let display_path = script.display;
            if exclude.is_excluded(&display_path) {
                continue;
            }

            let unique_basename = basename_counts.get(script_name.as_ref()) == Some(&1);
            if !docs.iter().any(|doc| {
                script_referenced(doc, &script_relative, script_name.as_ref(), unique_basename)
            }) {
                diag.report_at(
                    LintRule::OrphanedSkillFiles,
                    &display_path,
                    &format!(
                        "{}: not referenced from any .md under {base_dir}/{dir_name}",
                        display_path
                    ),
                );
            }
        }
    }
}

/// Read every `*.md` under a skill directory in deterministic sorted order.
fn read_skill_markdown_docs(skill_dir: &Path) -> Vec<String> {
    traversal::recursive_files_with_pruning(
        skill_dir,
        Path::new("."),
        None,
        traversal::should_descend_except_git,
    )
    .entries
    .into_iter()
    .filter(|entry| {
        entry
            .path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
    })
    .filter_map(|entry| fs::read_to_string(&entry.path).ok())
    .collect()
}

fn script_referenced(
    content: &str,
    relative_path: &str,
    basename: &str,
    unique_basename: bool,
) -> bool {
    token_referenced(content, relative_path, true)
        || (unique_basename && token_referenced(content, basename, true))
}

/// True when `token` appears in `content` with exact token boundaries. When
/// requested, reject a `/` continuation so a path to a child is not an
/// ownership reference to its parent file.
fn token_referenced(content: &str, token: &str, reject_slash_suffix: bool) -> bool {
    let mut start = 0;
    while let Some(offset) = content[start..].find(token) {
        let abs = start + offset;
        let leading_boundary_ok = abs == 0
            || content[..abs]
                .chars()
                .next_back()
                .is_some_and(|prev| !is_name_boundary_char(prev));
        let end = abs + token.len();
        let trailing_boundary_ok = end == content.len()
            || content[end..].chars().next().is_some_and(|next| {
                !is_name_boundary_char(next) && (!reject_slash_suffix || next != '/')
            });
        if leading_boundary_ok && trailing_boundary_ok {
            return true;
        }
        start = end;
    }
    false
}

fn is_name_boundary_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')
}

/// S036: Check that referenced shared .md files > 100 lines have headings (TOC).
/// Only runs in plugin mode (called from validate_skill_content).
pub(super) fn validate_ref_no_toc(
    base_dir: &str,
    skills: &[SkillInfo],
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
) {
    let shared_dir = Path::new(base_dir).join("shared");
    if !shared_dir.is_dir() {
        return;
    }

    let mut checked: HashSet<String> = HashSet::new();

    for info in skills {
        for shared_ref in find_shared_md_refs(&info.body, base_dir) {
            let rel = &shared_ref.relative_path;

            if !checked.insert(rel.clone()) {
                continue;
            }

            if exclude.is_excluded(rel) {
                continue;
            }

            let rel_path = Path::new(rel);
            if !rel_path.is_file() {
                continue;
            }

            if let Ok(content) = fs::read_to_string(rel_path) {
                let line_count = content.lines().count();
                if line_count > REF_NO_TOC_THRESHOLD {
                    let document = MarkdownDocument::parse(&content);
                    let has_headings = !document.headings().is_empty();
                    if !has_headings {
                        diag.report_at(
                            LintRule::RefNoToc,
                            rel,
                            &format!(
                                "{}: references {} ({} lines) which has no headings for navigation",
                                info.path, shared_ref.reference, line_count
                            ),
                        );
                    }
                }
            }
        }
    }
}

/// S048: Detect non-descriptive reference file names in skill directories.
/// Recursively walks skill-local Markdown outside `scripts/`, excluding
/// `SKILL.md`, and flags names matching the generic denylist.
pub(super) fn validate_generic_ref_names(
    base_dir: &str,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
) {
    let dir = Path::new(base_dir);
    if !dir.is_dir() {
        return;
    }

    for entry in traversal::shallow_directories(dir, Path::new("."), None).entries {
        let path = entry.path;
        let dir_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if dir_name == "shared" {
            continue;
        }

        let skill_path = format!("{base_dir}/{dir_name}/SKILL.md");
        if exclude.is_excluded(&skill_path) {
            continue;
        }

        for file_entry in traversal::recursive_files_with_pruning(
            &path,
            Path::new("."),
            None,
            should_descend_skill_reference_directory,
        )
        .entries
        {
            let file_path = file_entry.path;
            let file_name = match file_path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };

            if file_name == "SKILL.md" {
                continue;
            }

            let display_path = file_entry.display;
            if exclude.is_excluded(&display_path) {
                continue;
            }

            if RE_GENERIC_REF_NAME.is_match(&file_name) {
                diag.report_at(
                    LintRule::RefNameGeneric,
                    &display_path,
                    &format!(
                        "{}: non-descriptive reference file name (use a descriptive name like 'form-validation-rules.md')",
                        display_path
                    ),
                );
            }
        }
    }
}

fn should_descend_skill_reference_directory(entry: &walkdir::DirEntry) -> bool {
    traversal::should_descend_except_git(entry)
        && !(entry.depth() == 1 && entry.file_name() == "scripts")
}
