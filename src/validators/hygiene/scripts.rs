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
