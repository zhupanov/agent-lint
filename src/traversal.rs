//! Shared filesystem discovery policy.
//!
//! Traversal is deliberately best-effort: callers receive every readable entry
//! in deterministic repository-relative order plus any I/O errors encountered.
//! Validators that own an invalid-file diagnostic can report those errors using
//! their domain rule; optional discovery may safely use only `entries`.
//!
//! Recursive walks never follow symlinks and do not descend into repository
//! metadata, dependency trees, or conventional build output. Hidden directories
//! are otherwise included so supported surfaces such as `.claude/`, `.cursor/`,
//! and `.codex/` retain their normal coverage.

use crate::config::{ExcludeSet, normalize_path};
use std::path::{Path, PathBuf};
use walkdir::{DirEntry, WalkDir};

/// Directory names skipped by every recursive repository walk.
pub const IGNORED_DIRECTORY_NAMES: &[&str] =
    &[".git", "node_modules", "vendor", "target", "dist", "build"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkDepth {
    Shallow,
    Recursive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Files,
    Directories,
    All,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkEntry {
    pub path: PathBuf,
    /// Normalized, repository-relative path for diagnostics and exclusions.
    pub display: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkError {
    pub path: Option<PathBuf>,
    pub message: String,
}

#[derive(Debug, Default)]
pub struct WalkReport {
    pub entries: Vec<WalkEntry>,
    pub errors: Vec<WalkError>,
}

/// Walk `base` using the repository-wide discovery policy.
///
/// `root` determines the repository-relative display paths. Set it to `base`
/// for an absolute repository root, or `.` for normal validator paths.
pub fn entries(
    base: &Path,
    root: &Path,
    depth: WalkDepth,
    kind: EntryKind,
    exclude: Option<&ExcludeSet>,
) -> WalkReport {
    if !base.is_dir() {
        return WalkReport::default();
    }

    let mut walker = WalkDir::new(base).follow_links(false).min_depth(1);
    if depth == WalkDepth::Shallow {
        walker = walker.max_depth(1);
    }

    let mut report = WalkReport::default();
    for entry in walker.into_iter().filter_entry(should_descend) {
        match entry {
            Ok(entry) if includes(&entry, kind) => {
                let path = entry.into_path();
                let display = display_path(root, &path);
                if exclude.is_none_or(|set| !set.is_excluded(&display)) {
                    report.entries.push(WalkEntry { path, display });
                }
            }
            Ok(_) => {}
            Err(error) => report.errors.push(WalkError {
                path: error.path().map(Path::to_path_buf),
                message: error.to_string(),
            }),
        }
    }
    report
        .entries
        .sort_by(|left, right| left.display.cmp(&right.display));
    report
}

pub fn shallow_files(base: &Path, root: &Path, exclude: Option<&ExcludeSet>) -> WalkReport {
    entries(base, root, WalkDepth::Shallow, EntryKind::Files, exclude)
}

/// Shallow entries for callers whose domain policy selects both files and directories.
pub fn shallow_entries(base: &Path, root: &Path, exclude: Option<&ExcludeSet>) -> WalkReport {
    entries(base, root, WalkDepth::Shallow, EntryKind::All, exclude)
}

pub fn shallow_directories(base: &Path, root: &Path, exclude: Option<&ExcludeSet>) -> WalkReport {
    entries(
        base,
        root,
        WalkDepth::Shallow,
        EntryKind::Directories,
        exclude,
    )
}

pub fn recursive_files(base: &Path, root: &Path, exclude: Option<&ExcludeSet>) -> WalkReport {
    entries(base, root, WalkDepth::Recursive, EntryKind::Files, exclude)
}

fn includes(entry: &DirEntry, kind: EntryKind) -> bool {
    match kind {
        EntryKind::Files => entry.file_type().is_file(),
        EntryKind::Directories => entry.file_type().is_dir(),
        EntryKind::All => true,
    }
}

/// Skip repository metadata, dependency trees, and conventional build output.
pub fn should_descend(entry: &DirEntry) -> bool {
    !IGNORED_DIRECTORY_NAMES.contains(&entry.file_name().to_string_lossy().as_ref())
}

pub fn display_path(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    normalize_path(&relative.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn shallow_walk_is_sorted_and_does_not_recurse() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("z.md"), "").unwrap();
        fs::write(tmp.path().join("a.md"), "").unwrap();
        fs::create_dir(tmp.path().join("nested")).unwrap();
        fs::write(tmp.path().join("nested/inside.md"), "").unwrap();

        let report = shallow_files(tmp.path(), tmp.path(), None);
        assert_eq!(
            report
                .entries
                .iter()
                .map(|entry| &entry.display)
                .collect::<Vec<_>>(),
            ["a.md", "z.md"]
        );
        assert!(report.errors.is_empty());
    }

    #[test]
    fn recursive_walk_applies_exclusions_and_skips_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join(".git/private")).unwrap();
        fs::create_dir_all(tmp.path().join(".claude/skills/demo")).unwrap();
        fs::create_dir_all(tmp.path().join("skills/ignored")).unwrap();
        fs::write(tmp.path().join(".git/private/AGENTS.md"), "").unwrap();
        fs::write(tmp.path().join(".claude/skills/demo/SKILL.md"), "").unwrap();
        fs::write(tmp.path().join("skills/ignored/SKILL.md"), "").unwrap();
        let exclude = ExcludeSet::new(&["skills/ignored/**".to_string()]).unwrap();

        let report = recursive_files(tmp.path(), tmp.path(), Some(&exclude));
        assert_eq!(
            report
                .entries
                .iter()
                .map(|entry| &entry.display)
                .collect::<Vec<_>>(),
            [".claude/skills/demo/SKILL.md"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn recursive_walk_does_not_follow_symlinked_directories() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir(tmp.path().join("real")).unwrap();
        fs::write(tmp.path().join("real/AGENTS.md"), "").unwrap();
        symlink(tmp.path().join("real"), tmp.path().join("linked")).unwrap();

        let report = recursive_files(tmp.path(), tmp.path(), None);
        assert_eq!(
            report
                .entries
                .iter()
                .map(|entry| &entry.display)
                .collect::<Vec<_>>(),
            ["real/AGENTS.md"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn walk_returns_unreadable_directory_errors() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let unreadable = tmp.path().join("unreadable");
        fs::create_dir(&unreadable).unwrap();
        fs::write(unreadable.join("AGENTS.md"), "").unwrap();
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).unwrap();

        let report = recursive_files(tmp.path(), tmp.path(), None);

        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.path.as_deref() == Some(unreadable.as_path()))
        );
    }
}
