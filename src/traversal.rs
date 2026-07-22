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
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::path::{Path, PathBuf};
use walkdir::{DirEntry, WalkDir};

/// Read-only directory facts exposed to specialized traversal policies.
///
/// Keeping the `walkdir` type private makes this module the sole owner of the
/// recursive-walking implementation.
#[derive(Clone, Copy)]
pub struct DirectoryEntry<'a>(&'a DirEntry);

impl<'a> DirectoryEntry<'a> {
    pub fn depth(self) -> usize {
        self.0.depth()
    }

    pub fn file_name(self) -> &'a std::ffi::OsStr {
        self.0.file_name()
    }
}

/// Directory names skipped by every recursive repository walk.
pub const IGNORED_DIRECTORY_NAMES: &[&str] =
    &[".git", "node_modules", "vendor", "target", "dist", "build"];

/// Conventional interpreter/tool cache directories excluded from skill script
/// asset discovery (S030). These are never shipped skill assets.
pub const CACHE_DIRECTORY_NAMES: &[&str] = &[
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".tox",
    ".eggs",
    ".cache",
];

/// File extensions treated as conventional bytecode/cache artifacts for S030.
pub const CACHE_FILE_EXTENSIONS: &[&str] = &["pyc", "pyo", "pyd"];

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
    entries_with_pruning(base, root, depth, kind, exclude, should_descend)
}

/// Walk `base` with a caller-provided directory-pruning policy.
///
/// This is for narrow domain contracts that intentionally differ from the
/// repository-wide recursive policy. The caller remains responsible for
/// pruning `.git` and any domain-specific directory trees.
pub fn entries_with_pruning(
    base: &Path,
    root: &Path,
    depth: WalkDepth,
    kind: EntryKind,
    exclude: Option<&ExcludeSet>,
    should_descend: fn(DirectoryEntry<'_>) -> bool,
) -> WalkReport {
    if !std::fs::symlink_metadata(base).is_ok_and(|metadata| metadata.file_type().is_dir()) {
        return WalkReport::default();
    }

    let mut walker = WalkDir::new(base).follow_links(false).min_depth(1);
    if depth == WalkDepth::Shallow {
        walker = walker.max_depth(1);
    }

    let mut report = WalkReport::default();
    for entry in walker
        .into_iter()
        .filter_entry(|entry| should_descend(DirectoryEntry(entry)))
    {
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

/// Recursively enumerate regular files while pruning only directories selected
/// by `should_descend`. This is for validators whose public contract must
/// include packaged or generated-looking files.
pub fn recursive_files_with_pruning(
    base: &Path,
    root: &Path,
    exclude: Option<&ExcludeSet>,
    should_descend: fn(DirectoryEntry<'_>) -> bool,
) -> WalkReport {
    entries_with_pruning(
        base,
        root,
        WalkDepth::Recursive,
        EntryKind::Files,
        exclude,
        should_descend,
    )
}

/// Sum byte sizes of regular files under `dir` for upload-limit accounting.
///
/// Unlike [`recursive_files`], this descends into conventional build and
/// dependency directories (`node_modules`, `vendor`, `target`, `dist`, `build`)
/// because those trees count toward platform upload size. It still never
/// enters `.git` and never follows directory symlinks. File symlinks contribute
/// the size of their target via followed metadata.
pub fn directory_byte_size(dir: &Path) -> u64 {
    if !dir.is_dir() {
        return 0;
    }

    let mut total = 0u64;
    let walker = WalkDir::new(dir).follow_links(false).min_depth(1);
    for entry in walker
        .into_iter()
        .filter_entry(|entry| entry.file_name().to_string_lossy().as_ref() != ".git")
    {
        let Ok(entry) = entry else {
            continue;
        };
        match entry.path().metadata() {
            Ok(meta) if meta.is_file() => {
                total = total.saturating_add(meta.len());
            }
            _ => {}
        }
    }
    total
}

fn includes(entry: &DirEntry, kind: EntryKind) -> bool {
    match kind {
        EntryKind::Files => entry.file_type().is_file(),
        EntryKind::Directories => entry.file_type().is_dir(),
        EntryKind::All => true,
    }
}

/// Skip repository metadata, dependency trees, and conventional build output.
pub fn should_descend(entry: DirectoryEntry<'_>) -> bool {
    !IGNORED_DIRECTORY_NAMES.contains(&entry.file_name().to_string_lossy().as_ref())
}

/// Skip only Git metadata. Specialized recursive validators use this when
/// files in packaged directories are part of their ownership contract.
pub fn should_descend_except_git(entry: DirectoryEntry<'_>) -> bool {
    entry.file_name() != ".git"
}

/// Skip Git metadata and conventional cache directories. Used by S030 so local
/// interpreter caches never participate in orphan discovery.
pub fn should_descend_except_git_and_cache(entry: DirectoryEntry<'_>) -> bool {
    should_descend_except_git(entry)
        && !CACHE_DIRECTORY_NAMES.contains(&entry.file_name().to_string_lossy().as_ref())
}

/// True when `path` is a conventional cache/build artifact (cache directory
/// component or bytecode extension). Symlink identity is irrelevant: callers
/// pass walk entries that already refused to follow directory symlinks.
pub fn is_cache_artifact(path: &Path) -> bool {
    if path.components().any(|component| {
        matches!(
            component,
            std::path::Component::Normal(name)
                if CACHE_DIRECTORY_NAMES.contains(&name.to_string_lossy().as_ref())
        )
    }) {
        return true;
    }
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            CACHE_FILE_EXTENSIONS
                .iter()
                .any(|candidate| ext.eq_ignore_ascii_case(candidate))
        })
}

/// Repository-root ignore matcher for skill script asset discovery.
///
/// When the analysis root is a Git repository, root `.gitignore` and
/// `.git/info/exclude` participate. Non-Git fixtures stay on conventional
/// cache exclusion only so Git and non-Git behavior stay deterministic.
pub struct SkillScriptNoiseFilter {
    gitignore: Option<Gitignore>,
}

impl SkillScriptNoiseFilter {
    pub fn discover() -> Self {
        if !Path::new(".git").exists() {
            return Self { gitignore: None };
        }

        let mut builder = GitignoreBuilder::new(".");
        let mut loaded = false;
        for candidate in [".gitignore", ".git/info/exclude"] {
            if Path::new(candidate).is_file() {
                // `add` returns Some(err) on failure; None means the file loaded.
                if builder.add(candidate).is_none() {
                    loaded = true;
                }
            }
        }
        let gitignore = if loaded { builder.build().ok() } else { None };
        Self { gitignore }
    }

    pub fn is_noise(&self, path: &Path, display: &str) -> bool {
        if is_cache_artifact(path) {
            return true;
        }
        self.gitignore.as_ref().is_some_and(|ignore| {
            ignore
                .matched_path_or_any_parents(display, false)
                .is_ignore()
        })
    }
}
pub fn display_path(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    normalize_path(&relative.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const SHALLOW_WALK_FILES: &[(&str, &str)] = &[
        ("traversal.rs", "definition site"),
        (
            "validators/agents.rs",
            "inventoried at adoption; matches landed discovery design",
        ),
        (
            "validators/skill_discovery.rs",
            "inventoried at adoption; matches landed discovery design",
        ),
        (
            "validators/hygiene/scripts.rs",
            "inventoried at adoption; matches landed discovery design",
        ),
        (
            "validators/hygiene/todo.rs",
            "inventoried at adoption; matches landed discovery design",
        ),
    ];

    #[test]
    fn shallow_walk_call_sites_are_pinned() {
        let actual: std::collections::BTreeSet<_> = crate::test_helpers::source_files()
            .into_iter()
            .filter_map(|(path, content)| content.contains("shallow_files").then_some(path))
            .collect();
        let expected: std::collections::BTreeSet<_> = SHALLOW_WALK_FILES
            .iter()
            .map(|(path, _reason)| (*path).to_owned())
            .collect();

        let unpinned: Vec<_> = actual.difference(&expected).cloned().collect();
        let stale: Vec<_> = expected.difference(&actual).cloned().collect();
        assert!(
            unpinned.is_empty(),
            "shallow_files call sites are not pinned: {unpinned:?}"
        );
        assert!(
            stale.is_empty(),
            "pinned shallow_files files no longer contain a call site: {stale:?}"
        );
    }

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
    fn cache_artifact_detection_covers_dirs_and_extensions() {
        assert!(is_cache_artifact(Path::new(
            "skills/x/scripts/__pycache__/mod.pyc"
        )));
        assert!(is_cache_artifact(Path::new("skills/x/scripts/helper.PYC")));
        assert!(!is_cache_artifact(Path::new("skills/x/scripts/helper.py")));
        assert!(!is_cache_artifact(Path::new("skills/x/scripts/run.sh")));
    }

    #[test]
    #[serial_test::serial]
    fn skill_script_noise_filter_is_deterministic_for_git_and_non_git() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all("skills/x/scripts").unwrap();
        fs::write("skills/x/scripts/noise.tmp", "tmp").unwrap();
        fs::write("skills/x/scripts/cache.pyc", "cache").unwrap();
        fs::write(".gitignore", "*.tmp\n").unwrap();

        let non_git = SkillScriptNoiseFilter::discover();
        assert!(non_git.is_noise(
            Path::new("skills/x/scripts/cache.pyc"),
            "skills/x/scripts/cache.pyc"
        ));
        assert!(!non_git.is_noise(
            Path::new("skills/x/scripts/noise.tmp"),
            "skills/x/scripts/noise.tmp"
        ));

        fs::create_dir_all(".git/info").unwrap();
        let git = SkillScriptNoiseFilter::discover();
        assert!(git.is_noise(
            Path::new("skills/x/scripts/cache.pyc"),
            "skills/x/scripts/cache.pyc"
        ));
        assert!(git.is_noise(
            Path::new("skills/x/scripts/noise.tmp"),
            "skills/x/scripts/noise.tmp"
        ));
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
    fn recursive_walk_does_not_follow_a_symlinked_root() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir(tmp.path().join("real")).unwrap();
        fs::write(tmp.path().join("real/AGENTS.md"), "").unwrap();
        symlink(tmp.path().join("real"), tmp.path().join("linked")).unwrap();

        let report = recursive_files(&tmp.path().join("linked"), tmp.path(), None);
        assert!(report.entries.is_empty());
        assert!(report.errors.is_empty());
    }

    #[test]
    fn directory_byte_size_counts_dist_skips_git_and_dir_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("dist")).unwrap();
        fs::create_dir_all(tmp.path().join(".git/objects")).unwrap();
        fs::create_dir_all(tmp.path().join("plain")).unwrap();
        fs::write(tmp.path().join("dist/a.bin"), vec![1u8; 100]).unwrap();
        fs::write(tmp.path().join("plain/b.bin"), vec![1u8; 50]).unwrap();
        fs::write(tmp.path().join(".git/objects/c.bin"), vec![1u8; 1000]).unwrap();

        assert_eq!(directory_byte_size(tmp.path()), 150);

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let outside = tempfile::tempdir().unwrap();
            fs::write(outside.path().join("big.bin"), vec![1u8; 500]).unwrap();
            symlink(outside.path(), tmp.path().join("linked-dist")).unwrap();
            // Directory symlink contents are not followed.
            assert_eq!(directory_byte_size(tmp.path()), 150);

            fs::write(tmp.path().join("target.bin"), vec![1u8; 25]).unwrap();
            symlink(tmp.path().join("target.bin"), tmp.path().join("alias.bin")).unwrap();
            // File symlink contributes target size (25) in addition to target.bin (25).
            assert_eq!(directory_byte_size(tmp.path()), 200);
        }
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
