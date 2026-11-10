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
//!
//! Inside a Git repository, Git's own ignore rules also apply: `.gitignore` at
//! the root and in any directory along the walk, `.git/info/exclude`, and
//! `core.excludesFile`. A git-ignored instruction file, skill, or script asset
//! is not authored configuration of the repository being linted, so no rule
//! sees it (issue #674). Outside a Git repository those files have no ignore
//! status and are walked as before.

use crate::config::{ExcludeSet, normalize_path};
use ignore::{DirEntry, WalkBuilder};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Read-only directory facts exposed to specialized traversal policies.
///
/// Keeping the walker's entry type private makes this module the sole owner of
/// the recursive-walking implementation.
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

    let mut walker = WalkBuilder::new(base);
    walker
        .follow_links(false)
        // Hidden entries stay visible: `.claude/`, `.cursor/` and `.codex/`
        // are supported surfaces. Git's ignore sources are the only new
        // filter; `.ignore` and `.rgignore` are search-tool conventions that
        // say nothing about what the repository ships.
        .hidden(false)
        .ignore(false)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(true)
        .parents(true)
        .require_git(true)
        .filter_entry(move |entry| should_descend(DirectoryEntry(entry)));
    if depth == WalkDepth::Shallow {
        walker.max_depth(Some(1));
    }

    let mut report = WalkReport::default();
    for entry in walker.build() {
        match entry {
            // Depth 0 is `base` itself, which this walk never reports.
            Ok(entry) if entry.depth() > 0 && includes(&entry, kind) => {
                let path = entry.into_path();
                let display = display_path(root, &path);
                if exclude.is_none_or(|set| !set.is_excluded(&display)) {
                    report.entries.push(WalkEntry { path, display });
                }
            }
            Ok(_) => {}
            Err(error) => report.errors.push(WalkError {
                path: error_path(&error),
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

/// The path a walk error is about, when the walker recorded one.
///
/// Errors are wrapped in context layers (`WithDepth`, `WithLineNumber`,
/// `Partial`), so the path can sit below the outermost one.
fn error_path(error: &ignore::Error) -> Option<PathBuf> {
    match error {
        ignore::Error::WithPath { path, .. } => Some(path.clone()),
        ignore::Error::WithLineNumber { err, .. } | ignore::Error::WithDepth { err, .. } => {
            error_path(err)
        }
        ignore::Error::Partial(errors) => errors.iter().find_map(error_path),
        _ => None,
    }
}

fn includes(entry: &DirEntry, kind: EntryKind) -> bool {
    // A symlink is never followed, so its own file type decides: it is
    // neither a regular file nor a directory for selection purposes.
    match kind {
        EntryKind::Files => entry.file_type().is_some_and(|kind| kind.is_file()),
        EntryKind::Directories => entry.file_type().is_some_and(|kind| kind.is_dir()),
        EntryKind::All => true,
    }
}

/// Skip repository metadata, dependency trees, and conventional build output.
pub fn should_descend(entry: DirectoryEntry<'_>) -> bool {
    !IGNORED_DIRECTORY_NAMES.contains(&entry.file_name().to_string_lossy().as_ref())
}

/// Apply the normal repository pruning policy and omit conventional
/// interpreter/tool caches. Command-source discovery uses this because local
/// cache content is not authored executable configuration.
pub fn should_descend_without_cache(entry: DirectoryEntry<'_>) -> bool {
    should_descend(entry)
        && !CACHE_DIRECTORY_NAMES.contains(&entry.file_name().to_string_lossy().as_ref())
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

    /// Issue #674: every Git ignore source removes a path from discovery, and
    /// the same tree without Git keeps today's unfiltered behavior.
    #[test]
    fn walks_apply_every_git_ignore_source() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("sub/gen")).unwrap();
        fs::create_dir_all(root.join("tools/sub")).unwrap();
        fs::create_dir_all(root.join("excl")).unwrap();
        fs::create_dir_all(root.join("plain")).unwrap();
        for relative in [
            "tools/sub/AGENTS.md",
            "tools/CLAUDE.md",
            "sub/gen/AGENTS.md",
            "excl/AGENTS.md",
            "plain/AGENTS.md",
        ] {
            fs::write(root.join(relative), "body\n").unwrap();
        }

        let unfiltered = recursive_files(root, root, None);
        for ignored in [
            "tools/sub/AGENTS.md",
            "tools/CLAUDE.md",
            "sub/gen/AGENTS.md",
            "excl/AGENTS.md",
        ] {
            assert!(
                unfiltered
                    .entries
                    .iter()
                    .any(|entry| entry.display == ignored),
                "{ignored} is walked while the tree has no Git repository"
            );
        }

        fs::create_dir_all(root.join(".git/info")).unwrap();
        fs::write(root.join(".gitignore"), "tools/\n").unwrap();
        fs::write(root.join("sub/.gitignore"), "gen/\n").unwrap();
        fs::write(root.join(".git/info/exclude"), "excl/\n").unwrap();

        let filtered = recursive_files(root, root, None);
        let displays: Vec<_> = filtered
            .entries
            .iter()
            .map(|entry| entry.display.as_str())
            .collect();
        for ignored in [
            "tools/sub/AGENTS.md",
            "tools/CLAUDE.md",
            "sub/gen/AGENTS.md",
            "excl/AGENTS.md",
        ] {
            assert!(
                !displays.contains(&ignored),
                "{ignored} is git-ignored but still walked: {displays:?}"
            );
        }
        assert!(
            displays.contains(&"plain/AGENTS.md"),
            "a tracked file must stay discoverable: {displays:?}"
        );
    }

    #[test]
    fn walk_error_paths_unwrap_their_context_layers() {
        let wrapped = ignore::Error::WithDepth {
            depth: 2,
            err: Box::new(ignore::Error::Partial(vec![ignore::Error::WithPath {
                path: PathBuf::from("skills/unreadable"),
                err: Box::new(ignore::Error::InvalidDefinition),
            }])),
        };
        assert_eq!(
            error_path(&wrapped),
            Some(PathBuf::from("skills/unreadable"))
        );
        assert_eq!(error_path(&ignore::Error::InvalidDefinition), None);
    }

    /// A shallow walk of a subdirectory still honors the repository-root
    /// `.gitignore` above it, which is where skill discovery starts.
    #[test]
    fn shallow_walks_of_subdirectories_honor_parent_ignore_files() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join(".git/info")).unwrap();
        fs::create_dir_all(root.join(".claude/skills/local")).unwrap();
        fs::create_dir_all(root.join(".claude/skills/shipped")).unwrap();
        fs::write(root.join(".gitignore"), ".claude/skills/local/\n").unwrap();
        fs::write(root.join(".claude/skills/local/SKILL.md"), "body\n").unwrap();
        fs::write(root.join(".claude/skills/shipped/SKILL.md"), "body\n").unwrap();

        let report = shallow_directories(&root.join(".claude/skills"), root, None);
        assert_eq!(
            report
                .entries
                .iter()
                .map(|entry| &entry.display)
                .collect::<Vec<_>>(),
            [".claude/skills/shipped"]
        );
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
