use crate::config::ExcludeSet;
use crate::context::LintMode;
use crate::diagnostic::DiagnosticCollector;
use crate::rules::LintRule;
use regex::Regex;
use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use walkdir::WalkDir;

static RE_SCRIPT_PUB: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\$\{CLAUDE_PLUGIN_ROOT\}/(scripts|skills|\.claude/skills)/[a-zA-Z0-9._/-]+\.sh")
        .unwrap()
});
static RE_SCRIPT_PRIV: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\$PWD/\.claude/skills/[a-zA-Z0-9._/-]+\.sh").unwrap());
pub(super) static RE_SCRIPT_PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\$\{CLAUDE_PLUGIN_ROOT_PLACEHOLDER:-\$PWD\}/\.claude/skills/[a-zA-Z0-9._/-]+\.sh")
        .unwrap()
});
pub(super) static RE_SCRIPT_DIR_REF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\$SCRIPT_DIR/[a-zA-Z0-9._-]+\.sh").unwrap());
pub(super) static RE_SCRIPTS_PATH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(^|[^a-zA-Z0-9._/-])scripts/[a-zA-Z0-9._-]+\.sh").unwrap());
pub(super) static RE_SCRIPTS_EXTRACT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"scripts/[a-zA-Z0-9._-]+\.sh").unwrap());

/// Directory patterns for Plugin mode script discovery (V10, --list-scripts).
pub const PLUGIN_SCRIPT_DIRS: &[&str] =
    &["scripts", "skills/*/scripts", ".claude/skills/*/scripts"];

/// Directory patterns for Basic mode script discovery (V10-adapted, --list-scripts).
pub const BASIC_SCRIPT_DIRS: &[&str] = &[".claude/skills/*/scripts"];

static RE_FULL_HASH_COMMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[[:space:]]*#").unwrap());

/// Strip `#` comments from content that uses `#` as its comment character
/// (YAML, Makefile, shell). Drops full-comment lines and trailing comments,
/// respecting single/double quotes. Shared by the YAML-workflow and Makefile
/// reference extraction in G004 (dead scripts) and V9 (script references).
pub(super) fn strip_yaml_comments(content: &str) -> String {
    content
        .lines()
        .filter(|line| !RE_FULL_HASH_COMMENT.is_match(line))
        .map(strip_trailing_yaml_comment)
        .collect::<Vec<_>>()
        .join("\n")
}

fn strip_trailing_yaml_comment(line: &str) -> String {
    let mut in_quote: Option<char> = None;
    let mut prev_was_ws = false;
    let mut skip_next = false;

    for (byte_pos, ch) in line.char_indices() {
        if skip_next {
            skip_next = false;
            prev_was_ws = ch.is_whitespace();
            continue;
        }
        match in_quote {
            Some(q) => {
                if q == '"' && ch == '\\' {
                    skip_next = true;
                } else if q == '\'' && ch == '\'' {
                    let rest = &line[byte_pos + ch.len_utf8()..];
                    if rest.starts_with('\'') {
                        skip_next = true;
                    } else {
                        in_quote = None;
                    }
                } else if ch == q {
                    in_quote = None;
                }
            }
            None => {
                if ch == '"' || ch == '\'' {
                    in_quote = Some(ch);
                } else if ch == '#' && prev_was_ws {
                    return line[..byte_pos].trim_end().to_string();
                }
            }
        }
        prev_was_ws = ch.is_whitespace();
    }

    line.to_string()
}

/// Read the repo-root `Makefile` and any root-level `*.mk` files and return
/// their `#`-comment-stripped contents. Used by G004 (dead scripts) and V9
/// (script references) so Make-target invocations like `bash scripts/foo.sh`
/// and `${CLAUDE_PLUGIN_ROOT}/scripts/foo.sh` are recognised as references.
pub(super) fn collect_makefile_contents(exclude: &ExcludeSet) -> Vec<String> {
    let mut candidates: Vec<PathBuf> = vec![PathBuf::from("Makefile")];
    if let Ok(entries) = fs::read_dir(".") {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.ends_with(".mk") {
                    candidates.push(path);
                }
            }
        }
    }
    let mut out = Vec::new();
    for path in candidates {
        let display = path.display().to_string();
        if exclude.is_excluded(&display) {
            continue;
        }
        if let Ok(content) = fs::read_to_string(&path) {
            out.push(strip_yaml_comments(&content));
        }
    }
    out
}

/// V9: Script reference integrity.
pub fn validate_script_references(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let mut seen = HashSet::new();

    for dir in &["skills", ".claude/skills"] {
        let base = Path::new(dir);
        if !base.is_dir() {
            continue;
        }
        for entry in WalkDir::new(base).into_iter().flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let display_path = path.display().to_string();
            if exclude.is_excluded(&display_path) {
                continue;
            }
            let content = match fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            for cap in RE_SCRIPT_PUB.find_iter(&content) {
                let reference = cap.as_str().to_string();
                if seen.insert(reference.clone()) {
                    let rel = reference.replace("${CLAUDE_PLUGIN_ROOT}/", "");
                    if !Path::new(&rel).is_file() {
                        diag.report(
                            LintRule::ScriptRefMissing,
                            &format!(
                                "script reference missing on disk: {reference} (expected {rel})"
                            ),
                        );
                    }
                }
            }

            for cap in RE_SCRIPT_PRIV.find_iter(&content) {
                let reference = cap.as_str().to_string();
                if seen.insert(reference.clone()) {
                    let rel = reference.replace("$PWD/", "");
                    if !Path::new(&rel).is_file() {
                        diag.report(
                            LintRule::ScriptRefMissing,
                            &format!(
                                "script reference missing on disk: {reference} (expected {rel})"
                            ),
                        );
                    }
                }
            }

            for cap in RE_SCRIPT_PLACEHOLDER.find_iter(&content) {
                let reference = cap.as_str().to_string();
                if seen.insert(reference.clone()) {
                    let rel = reference.replace("${CLAUDE_PLUGIN_ROOT_PLACEHOLDER:-$PWD}/", "");
                    if !Path::new(&rel).is_file() {
                        diag.report(
                            LintRule::ScriptRefMissing,
                            &format!(
                                "script reference missing on disk: {reference} (expected {rel})"
                            ),
                        );
                    }
                }
            }
        }
    }

    // Also scan the Makefile and any *.mk so Make-target invocations are
    // validated for existence (V9). Comments are stripped first so
    // commented-out references are not mistaken for live invocations.
    for content in collect_makefile_contents(exclude) {
        for cap in RE_SCRIPT_PUB.find_iter(&content) {
            let reference = cap.as_str().to_string();
            if seen.insert(reference.clone()) {
                let rel = reference.replace("${CLAUDE_PLUGIN_ROOT}/", "");
                if !Path::new(&rel).is_file() {
                    diag.report(
                        LintRule::ScriptRefMissing,
                        &format!("script reference missing on disk: {reference} (expected {rel})"),
                    );
                }
            }
        }
        for cap in RE_SCRIPTS_PATH.find_iter(&content) {
            if let Some(m) = RE_SCRIPTS_EXTRACT.find(cap.as_str()) {
                let reference = m.as_str().to_string();
                if seen.insert(reference.clone()) && !Path::new(&reference).is_file() {
                    diag.report(
                        LintRule::ScriptRefMissing,
                        &format!("script reference missing on disk: {reference}"),
                    );
                }
            }
        }
    }
}

/// V9-adapted: Script reference integrity for private .claude/skills/ only.
pub fn validate_private_script_references(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let mut seen = HashSet::new();
    let base = Path::new(".claude/skills");
    if !base.is_dir() {
        return;
    }

    for entry in WalkDir::new(base).into_iter().flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let display_path = path.display().to_string();
        if exclude.is_excluded(&display_path) {
            continue;
        }
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        for cap in RE_SCRIPT_PRIV.find_iter(&content) {
            let reference = cap.as_str().to_string();
            if seen.insert(reference.clone()) {
                let rel = reference.replace("$PWD/", "");
                if !Path::new(&rel).is_file() {
                    diag.report(
                        LintRule::ScriptRefMissing,
                        &format!("script reference missing on disk: {reference} (expected {rel})"),
                    );
                }
            }
        }

        for cap in RE_SCRIPT_PLACEHOLDER.find_iter(&content) {
            let reference = cap.as_str().to_string();
            if seen.insert(reference.clone()) {
                let rel = reference.replace("${CLAUDE_PLUGIN_ROOT_PLACEHOLDER:-$PWD}/", "");
                if !Path::new(&rel).is_file() {
                    diag.report(
                        LintRule::ScriptRefMissing,
                        &format!("script reference missing on disk: {reference} (expected {rel})"),
                    );
                }
            }
        }
    }
}

/// V10: Executability -- every .sh file under scripts/, skills/*/scripts/,
/// and .claude/skills/*/scripts/ must be chmod +x.
#[cfg(unix)]
pub fn validate_executability(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    check_executability_in_dirs(
        &["scripts", "skills/*/scripts", ".claude/skills/*/scripts"],
        diag,
        exclude,
    );
}

#[cfg(not(unix))]
pub fn validate_executability(_diag: &mut DiagnosticCollector, _exclude: &ExcludeSet) {}

/// V10-adapted: Executability for private .claude/skills/*/scripts/*.sh only.
#[cfg(unix)]
pub fn validate_private_executability(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    check_executability_in_dirs(&[".claude/skills/*/scripts"], diag, exclude);
}

#[cfg(not(unix))]
pub fn validate_private_executability(_diag: &mut DiagnosticCollector, _exclude: &ExcludeSet) {}

/// Expand glob-like directory patterns into concrete directory paths.
/// Supports multiple `*` wildcards (e.g., `skills/*/nested/*/scripts`).
pub fn expand_script_dirs(patterns: &[&str]) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for pattern in patterns {
        if pattern.contains('*') {
            let segments: Vec<&str> = pattern.split('/').collect();
            let mut candidates = vec![PathBuf::new()];
            for seg in &segments {
                let mut next = Vec::new();
                if *seg == "*" {
                    for base in &candidates {
                        let read_dir = if base.as_os_str().is_empty() {
                            fs::read_dir(".")
                        } else {
                            fs::read_dir(base)
                        };
                        if let Ok(entries) = read_dir {
                            for entry in entries.flatten() {
                                if entry.path().is_dir() {
                                    let child = if base.as_os_str().is_empty() {
                                        PathBuf::from(entry.file_name())
                                    } else {
                                        base.join(entry.file_name())
                                    };
                                    next.push(child);
                                }
                            }
                        }
                    }
                } else {
                    for base in &candidates {
                        let child = if base.as_os_str().is_empty() {
                            PathBuf::from(seg)
                        } else {
                            base.join(seg)
                        };
                        if child.is_dir() {
                            next.push(child);
                        }
                    }
                }
                candidates = next;
            }
            for c in candidates {
                if c.is_dir() {
                    dirs.push(c);
                }
            }
        } else {
            let dir = Path::new(pattern);
            if dir.is_dir() {
                dirs.push(dir.to_path_buf());
            }
        }
    }
    dirs
}

#[cfg(unix)]
pub fn check_executability_in_dirs(
    patterns: &[&str],
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
) {
    for dir in expand_script_dirs(patterns) {
        check_sh_executability(&dir, diag, exclude);
    }
}

/// Collect all .sh script paths for the given lint mode.
/// Returns sorted, deduplicated repo-relative paths.
pub fn collect_script_paths(mode: LintMode, exclude: &ExcludeSet) -> Vec<String> {
    let patterns = match mode {
        LintMode::Plugin => PLUGIN_SCRIPT_DIRS,
        LintMode::Basic => BASIC_SCRIPT_DIRS,
    };
    let mut paths = BTreeSet::new();
    for dir in expand_script_dirs(patterns) {
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.ends_with(".sh") {
                    let display = path.display().to_string();
                    if !exclude.is_excluded(&display) {
                        paths.insert(display);
                    }
                }
            }
        }
    }
    paths.into_iter().collect()
}

#[cfg(unix)]
fn check_sh_executability(dir: &Path, diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    use std::os::unix::fs::PermissionsExt;

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) if n.ends_with(".sh") => n.to_string(),
            _ => continue,
        };

        let display_path = path.display().to_string();
        if exclude.is_excluded(&display_path) {
            continue;
        }

        if let Ok(meta) = path.metadata() {
            if meta.permissions().mode() & 0o111 == 0 {
                diag.report(
                    LintRule::ScriptNotExecutable,
                    &format!("script not executable: {}", path.display()),
                );
                let _ = name;
            }
        }
    }
}
