use crate::config::ExcludeSet;
use crate::diagnostic::DiagnosticCollector;
use crate::markdown::MarkdownDocument;
use crate::rules::LintRule;
use crate::traversal;
use crate::validators::skills::SkillInfo;
use regex::Regex;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

/// S048: denylist for non-descriptive reference file names in skill directories.
/// Matches generic stems (doc, file, ref, data, info, tmp, test) with optional
/// digits, single letters (case-insensitive), and pure numeric names — all with .md extension.
static RE_GENERIC_REF_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i:^(?:doc|file|ref|data|info|tmp|test)\d*|^[a-z]|^\d+)\.md$").unwrap()
});

const REF_NO_TOC_THRESHOLD: usize = 100;

/// Build a regex matching `${CLAUDE_PLUGIN_ROOT}/<base_dir>/shared/<path>.md` references.
pub(super) fn shared_ref_regex(base_dir: &str) -> Regex {
    Regex::new(&format!(
        r"\$\{{CLAUDE_PLUGIN_ROOT\}}/{}/shared/[a-zA-Z0-9._/-]+\.md",
        regex::escape(base_dir)
    ))
    .unwrap()
}

/// S029: Check for deeply nested shared markdown references.
/// Matches `${CLAUDE_PLUGIN_ROOT}/<base_dir>/shared/*.md` references.
pub(super) fn validate_nested_references(
    base_dir: &str,
    skills: &[SkillInfo],
    diag: &mut DiagnosticCollector,
) {
    let shared_dir = Path::new(base_dir).join("shared");
    if !shared_dir.is_dir() {
        return;
    }

    let re_shared = shared_ref_regex(base_dir);

    // Cache: which shared .md files are nested (avoids re-reading files from disk)
    let mut checked: HashSet<String> = HashSet::new();
    let mut nested: HashSet<String> = HashSet::new();

    for info in skills {
        // Find shared-md references in this skill's body
        for cap in re_shared.find_iter(&info.body) {
            let reference = cap.as_str();
            let rel = reference.replace("${CLAUDE_PLUGIN_ROOT}/", "");
            let rel_path = Path::new(&rel);

            if !rel_path.is_file() {
                continue; // S008 handles missing refs
            }

            // Check the file once for nesting, cache result
            if !checked.contains(&rel) {
                checked.insert(rel.clone());
                if let Ok(content) = fs::read_to_string(rel_path) {
                    if re_shared.is_match(&content) {
                        nested.insert(rel.clone());
                    }
                }
            }

            // Report for every referencing skill (not just the first)
            if nested.contains(&rel) {
                diag.report_at(
                    LintRule::NestedRefDeep,
                    &info.path,
                    &format!(
                        "{}: references {} which itself references other shared .md files (keep references one level deep)",
                        info.path, reference
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
        if docs.is_empty() {
            continue;
        }

        // Check each file in scripts/
        for script_entry in traversal::shallow_files(&scripts_dir, Path::new("."), None).entries {
            let script_path = script_entry.path;
            let script_name = match script_path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };

            let display_path = format!("{base_dir}/{dir_name}/scripts/{script_name}");
            if exclude.is_excluded(&display_path) {
                continue;
            }

            // Referenced when the file name appears in any skill-local .md with
            // a leading character boundary (avoids dry-run.sh shadowing run.sh).
            if !docs
                .iter()
                .any(|doc| name_referenced_with_boundary(doc, &script_name))
            {
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
    traversal::recursive_files(skill_dir, Path::new("."), None)
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

/// True when `name` appears in `content` and the preceding character (if any)
/// is not `[A-Za-z0-9_.-]`.
fn name_referenced_with_boundary(content: &str, name: &str) -> bool {
    let mut start = 0;
    while let Some(offset) = content[start..].find(name) {
        let abs = start + offset;
        let boundary_ok = abs == 0
            || content[..abs]
                .chars()
                .next_back()
                .is_some_and(|prev| !is_name_boundary_char(prev));
        if boundary_ok {
            return true;
        }
        start = abs + name.len().max(1);
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

    let re_shared = shared_ref_regex(base_dir);

    let mut checked: HashSet<String> = HashSet::new();

    for info in skills {
        for cap in re_shared.find_iter(&info.body) {
            let reference = cap.as_str();
            let rel = reference.replace("${CLAUDE_PLUGIN_ROOT}/", "");

            if !checked.insert(rel.clone()) {
                continue;
            }

            if exclude.is_excluded(&rel) {
                continue;
            }

            let rel_path = Path::new(&rel);
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
                            &rel,
                            &format!(
                                "{}: references {} ({} lines) which has no headings for navigation",
                                info.path, reference, line_count
                            ),
                        );
                    }
                }
            }
        }
    }
}

/// S048: Detect non-descriptive reference file names in skill directories.
/// Walks each skill subdirectory (non-recursive), skips SKILL.md and subdirectories
/// (e.g., scripts/), and flags `.md` files whose names match the generic denylist.
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

        for file_entry in traversal::shallow_files(&path, Path::new("."), None).entries {
            let file_path = file_entry.path;
            let file_name = match file_path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };

            if file_name == "SKILL.md" {
                continue;
            }

            let display_path = format!("{base_dir}/{dir_name}/{file_name}");
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
