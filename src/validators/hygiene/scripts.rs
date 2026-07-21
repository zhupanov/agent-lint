use crate::config::ExcludeSet;
use crate::context::LintMode;
use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata};
use crate::rules::LintRule;
use crate::script_paths::{
    Invocation, ScriptReference, extract_bare_script_references, extract_command_references,
};
use crate::traversal;
use regex::Regex;
use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// Directory patterns used by `--list-scripts` and conventional script
/// discovery.
pub const PLUGIN_SCRIPT_DIRS: &[&str] =
    &["scripts", "skills/*/scripts", ".claude/skills/*/scripts"];
pub const BASIC_SCRIPT_DIRS: &[&str] = &[".claude/skills/*/scripts"];

static RE_FULL_HASH_COMMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[[:space:]]*#").unwrap());

pub(super) fn strip_yaml_comments(content: &str) -> String {
    content
        .lines()
        .filter(|line| !RE_FULL_HASH_COMMENT.is_match(line))
        .map(strip_trailing_yaml_comment)
        .collect::<Vec<_>>()
        .join("\n")
}

fn strip_trailing_yaml_comment(line: &str) -> String {
    let mut quote = None;
    let mut previous_ws = false;
    for (index, character) in line.char_indices() {
        match quote {
            Some(q) if character == q => quote = None,
            Some(_) => {}
            None if matches!(character, '\'' | '"') => quote = Some(character),
            None if character == '#' && previous_ws => return line[..index].trim_end().to_string(),
            None => {}
        }
        previous_ws = character.is_whitespace();
    }
    line.to_string()
}

pub(super) fn collect_makefile_contents(exclude: &ExcludeSet) -> Vec<(String, String)> {
    let mut candidates = vec![PathBuf::from("Makefile")];
    for entry in traversal::shallow_files(Path::new("."), Path::new("."), None).entries {
        if entry
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".mk"))
        {
            candidates.push(entry.path);
        }
    }
    candidates
        .into_iter()
        .filter_map(|path| {
            let display = path.display().to_string();
            (!exclude.is_excluded(&display))
                .then(|| {
                    fs::read_to_string(path)
                        .ok()
                        .map(|content| (display, strip_yaml_comments(&content)))
                })
                .flatten()
        })
        .collect()
}

/// A source-owned reference. The source path remains the G002 diagnostic
/// subject, while the normalized target is consumed by G003/G004.
pub(crate) fn collect_references(
    mode: LintMode,
    exclude: &ExcludeSet,
) -> Vec<(String, ScriptReference)> {
    let mut sources = match mode {
        LintMode::Plugin => vec![
            "skills",
            "agents",
            ".claude/skills",
            ".claude/agents",
            "scripts",
            ".github/workflows",
        ],
        LintMode::Basic => vec![".claude/skills", ".claude/agents"],
    };
    let mut references = Vec::new();
    for dir in sources.drain(..) {
        let base = Path::new(dir);
        if !base.is_dir() {
            continue;
        }
        for entry in traversal::recursive_files(base, Path::new("."), Some(exclude)).entries {
            let source = entry.display;
            if exclude.is_excluded(&source) {
                continue;
            }
            let Ok(content) = fs::read_to_string(entry.path) else {
                continue;
            };
            let fragments = if source.ends_with(".md") {
                markdown_command_fragments(&content)
            } else {
                command_line_fragments(&strip_yaml_comments(&content))
            };
            for (line, fragment) in fragments {
                if !is_executable_fragment(&fragment) {
                    continue;
                }
                references.extend(
                    extract_command_references(&fragment, line)
                        .into_iter()
                        .map(|reference| (source.clone(), reference)),
                );
                references.extend(
                    extract_bare_script_references(&fragment, line)
                        .into_iter()
                        .map(|reference| (source.clone(), reference)),
                );
            }
        }
    }
    if mode == LintMode::Plugin {
        for (source, content) in collect_makefile_contents(exclude) {
            for (line, fragment) in command_line_fragments(&content) {
                if is_executable_fragment(&fragment) {
                    references.extend(
                        extract_command_references(&fragment, line)
                            .into_iter()
                            .map(|reference| (source.clone(), reference)),
                    );
                    references.extend(
                        extract_bare_script_references(&fragment, line)
                            .into_iter()
                            .map(|reference| (source.clone(), reference)),
                    );
                }
            }
        }
    }
    references
}

fn command_line_fragments(content: &str) -> Vec<(usize, String)> {
    content
        .lines()
        .enumerate()
        .map(|(index, line)| (index + 1, line.to_string()))
        .collect()
}

/// Markdown prose is not executable.  Inline code and balanced code fences
/// are explicit command positions and retain their source line for diagnostics.
fn markdown_command_fragments(content: &str) -> Vec<(usize, String)> {
    let mut fragments = Vec::new();
    for fence in crate::fence::markdown_fences(content) {
        fragments.extend(fence.body);
    }
    for (index, line) in content.lines().enumerate() {
        if line.trim_start().starts_with("Run ") || line.trim_start().starts_with("Execute ") {
            fragments.push((index + 1, line.trim_start().to_string()));
        }
        let mut rest = line;
        while let Some(start) = rest.find('`') {
            let after = &rest[start + 1..];
            let Some(end) = after.find('`') else {
                break;
            };
            fragments.push((index + 1, after[..end].to_string()));
            rest = &after[end + 1..];
        }
    }
    fragments
}

fn is_executable_fragment(fragment: &str) -> bool {
    let first = fragment
        .trim_start()
        .trim_matches('"')
        .split_whitespace()
        .next()
        .unwrap_or("");
    !matches!(
        first,
        "" | "#" | "echo" | "printf" | "cat" | "grep" | "sed" | "awk"
    )
}

/// G002: missing or unsafe script references. Dedupe includes the source path
/// so collector policy is applied independently for every source file.
#[cfg(test)]
pub fn validate_script_references(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    validate_script_references_for_mode(LintMode::Plugin, diag, exclude);
}

pub fn validate_script_references_for_mode(
    mode: LintMode,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
) {
    let mut seen = HashSet::new();
    for (source, reference) in collect_references(mode, exclude) {
        if !seen.insert((source.clone(), reference.path.clone())) {
            continue;
        }
        if reference.path.as_os_str().is_empty() || !reference.path.is_file() {
            let expected = reference.path.display().to_string();
            diag.report_at_with(
                LintRule::ScriptRefMissing,
                &source,
                &format!(
                    "script reference missing on disk or unsafe at line {}: {} (expected {})",
                    reference.line,
                    reference.reference,
                    if expected.is_empty() {
                        "an in-repository path"
                    } else {
                        &expected
                    }
                ),
                DiagnosticMetadata::default()
                    .with_evidence(reference.reference)
                    .with_suggestion("use a normalized path within the repository"),
            );
        }
    }
}

pub fn validate_private_script_references(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    validate_script_references_for_mode(LintMode::Basic, diag, exclude);
}

/// Direct invocation, not a conventional filename or directory, determines
/// G003 scope. Interpreter-launched and sourced files need no execute bit.
pub(crate) fn direct_script_paths(mode: LintMode, exclude: &ExcludeSet) -> Vec<PathBuf> {
    let mut paths = BTreeSet::new();
    for (_, reference) in collect_references(mode, exclude) {
        if reference.invocation == Invocation::Direct
            && !reference.path.as_os_str().is_empty()
            && reference.path.is_file()
        {
            paths.insert(reference.path);
        }
    }
    paths.into_iter().collect()
}

#[cfg(unix)]
#[cfg(test)]
pub fn validate_executability(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    validate_executability_for_mode(LintMode::Plugin, diag, exclude);
}

#[cfg(unix)]
pub fn validate_executability_for_mode(
    mode: LintMode,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
) {
    use std::os::unix::fs::PermissionsExt;
    for path in direct_script_paths(mode, exclude) {
        if let Ok(meta) = path.metadata()
            && meta.permissions().mode() & 0o111 == 0
        {
            diag.report_at_with(
                LintRule::ScriptNotExecutable,
                &path,
                &format!(
                    "directly executed script is not executable: {}",
                    path.display()
                ),
                DiagnosticMetadata::default()
                    .with_evidence(path.display().to_string())
                    .with_suggestion("run chmod +x on this file"),
            );
        }
    }
}

#[cfg(not(unix))]
pub fn validate_executability_for_mode(
    _mode: LintMode,
    _diag: &mut DiagnosticCollector,
    _exclude: &ExcludeSet,
) {
}

pub fn validate_private_executability(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    validate_executability_for_mode(LintMode::Basic, diag, exclude);
}

pub fn expand_script_dirs(patterns: &[&str]) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for pattern in patterns {
        let mut candidates = vec![PathBuf::new()];
        for segment in pattern.split('/') {
            let mut next = Vec::new();
            for base in &candidates {
                if segment == "*" {
                    let directory = if base.as_os_str().is_empty() {
                        Path::new(".")
                    } else {
                        base.as_path()
                    };
                    for entry in
                        traversal::shallow_directories(directory, Path::new("."), None).entries
                    {
                        next.push(if base.as_os_str().is_empty() {
                            PathBuf::from(entry.path.file_name().unwrap_or_default())
                        } else {
                            base.join(entry.path.file_name().unwrap_or_default())
                        });
                    }
                } else {
                    let child = if base.as_os_str().is_empty() {
                        PathBuf::from(segment)
                    } else {
                        base.join(segment)
                    };
                    if child.is_dir() {
                        next.push(child);
                    }
                }
            }
            candidates = next;
        }
        dirs.extend(candidates);
    }
    dirs
}

pub fn collect_script_paths(mode: LintMode, exclude: &ExcludeSet) -> Vec<String> {
    let patterns = match mode {
        LintMode::Plugin => PLUGIN_SCRIPT_DIRS,
        LintMode::Basic => BASIC_SCRIPT_DIRS,
    };
    let mut paths = BTreeSet::new();
    for dir in expand_script_dirs(patterns) {
        for entry in traversal::shallow_files(&dir, Path::new("."), None).entries {
            let display = entry.path.display().to_string();
            if is_supported_script_file(&entry.path) && !exclude.is_excluded(&display) {
                paths.insert(display);
            }
        }
    }
    paths.into_iter().collect()
}

fn is_supported_script_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        None | Some("sh" | "py" | "js" | "mjs" | "inc.bash")
    )
}
