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
//! Inside a Git repository, Git's own ignore decision also applies: a path Git
//! ignores is not authored configuration of the repository being linted, so no
//! rule sees it (issue #674). The decision is Git's, asked of `git
//! check-ignore`, so every ignore source counts -- `.gitignore` at the root and
//! in any directory along the way, `.git/info/exclude`, `core.excludesFile` --
//! and a tracked file is never ignored, however the patterns read. Outside a
//! Git repository nothing has an ignore status and every path is walked as
//! before.

use crate::config::{ExcludeSet, normalize_path};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
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
    retain_paths_git_does_not_ignore(&mut report.entries);
    report
        .entries
        .sort_by(|left, right| left.display.cmp(&right.display));
    report
}

/// Decisions already obtained for one analysis root. Overlapping walks ask
/// about the same path repeatedly, and a path's ignore status cannot change
/// within a run, so each one is paid for once. The root is part of the key so
/// a process that moves between repositories (the test suite) never reuses
/// another root's answers.
static GIT_IGNORE_DECISIONS: Mutex<Option<(PathBuf, HashMap<String, bool>)>> = Mutex::new(None);

/// Drop the entries Git ignores, asking Git itself.
///
/// Outside a Git work tree nothing has an ignore status, and a path outside
/// the analysis root cannot be asked about, so both are kept.
fn retain_paths_git_does_not_ignore(entries: &mut Vec<WalkEntry>) {
    if entries.is_empty() {
        return;
    }
    let Some(root) = git_work_tree_root() else {
        return;
    };
    let candidates: Vec<Option<String>> = entries
        .iter()
        .map(|entry| repository_relative(&root, &entry.path))
        .collect();

    let mut guard = GIT_IGNORE_DECISIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if guard
        .as_ref()
        .is_none_or(|(cached_root, _)| *cached_root != root)
    {
        *guard = Some((root, HashMap::new()));
    }
    let (_, decisions) = guard.as_mut().expect("the cache was just initialized");

    let unknown: Vec<String> = candidates
        .iter()
        .flatten()
        .filter(|path| !decisions.contains_key(*path))
        .cloned()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    if !unknown.is_empty() {
        let ignored = git_ignored_paths(&unknown);
        for path in unknown {
            let is_ignored = ignored.contains(&path);
            decisions.insert(path, is_ignored);
        }
    }

    let mut keep = candidates.iter().map(|candidate| match candidate {
        Some(path) => !decisions.get(path).copied().unwrap_or(false),
        None => true,
    });
    entries.retain(|_| keep.next().unwrap_or(true));
}

/// The working directory when it is inside a Git work tree, which is the
/// analysis root every walk resolves against.
fn git_work_tree_root() -> Option<PathBuf> {
    let root = std::env::current_dir().ok()?;
    // `.git` is a directory in a normal clone and a file in a linked worktree
    // or submodule; both are work trees Git can answer for.
    root.join(".git").exists().then_some(root)
}

/// `path` spelled relative to the analysis root, or `None` when it lies
/// outside it and Git therefore has no opinion about it.
fn repository_relative(root: &Path, path: &Path) -> Option<String> {
    let relative = if path.is_absolute() {
        path.strip_prefix(root).ok()?
    } else {
        path
    };
    let normalized = normalize_path(&relative.to_string_lossy());
    (!normalized.is_empty() && normalized != ".").then_some(normalized)
}

/// The subset of `paths` Git ignores.
///
/// `git check-ignore` applies every ignore source and consults the index, so a
/// tracked file is never ignored however the patterns read. An unavailable or
/// failing Git yields no decision, leaving every path walked exactly as it was
/// before ignore handling existed.
fn git_ignored_paths(paths: &[String]) -> HashSet<String> {
    let mut payload = Vec::new();
    for path in paths {
        payload.extend_from_slice(path.as_bytes());
        payload.push(0);
    }

    let Ok(mut child) = Command::new("git")
        .args(["check-ignore", "--stdin", "-z"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return HashSet::new();
    };
    let Some(mut stdin) = child.stdin.take() else {
        return HashSet::new();
    };
    // Write from another thread: a large query would otherwise fill the
    // child's output pipe while this one is still writing, and both block.
    let writer = std::thread::spawn(move || stdin.write_all(&payload));
    let output = child.wait_with_output();
    let _ = writer.join();
    let Ok(output) = output else {
        return HashSet::new();
    };
    // 0 reports ignored paths, 1 reports none; anything else (128, no work
    // tree) is not a usable answer.
    if !matches!(output.status.code(), Some(0 | 1)) {
        return HashSet::new();
    }
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
        .map(|record| String::from_utf8_lossy(record).into_owned())
        .collect()
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

    /// Build a repository whose files are ignored through every source, and
    /// return the paths that should survive discovery. `track` is force-added
    /// so it is tracked *and* matched by a pattern.
    fn ignore_corpus(root: &Path) {
        fs::create_dir_all(root.join("sub/gen")).unwrap();
        fs::create_dir_all(root.join("tools/sub")).unwrap();
        fs::create_dir_all(root.join("excl")).unwrap();
        fs::create_dir_all(root.join("plain")).unwrap();
        fs::create_dir_all(root.join("forced")).unwrap();
        for relative in [
            "tools/sub/AGENTS.md",
            "tools/CLAUDE.md",
            "sub/gen/AGENTS.md",
            "excl/AGENTS.md",
            "plain/AGENTS.md",
            "forced/AGENTS.md",
        ] {
            fs::write(root.join(relative), "body\n").unwrap();
        }
    }

    const IGNORED_CORPUS_PATHS: &[&str] = &[
        "tools/sub/AGENTS.md",
        "tools/CLAUDE.md",
        "sub/gen/AGENTS.md",
        "excl/AGENTS.md",
    ];

    fn run_git(root: &Path, arguments: &[&str]) {
        let output = Command::new("git")
            .args(arguments)
            .current_dir(root)
            .output()
            .expect("git runs for the traversal fixture");
        assert!(
            output.status.success(),
            "git {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// Issue #674: every Git ignore source removes a path from discovery, and
    /// a tracked file stays discoverable even when a pattern also matches it.
    #[test]
    #[serial_test::serial]
    fn walks_apply_every_git_ignore_source() {
        let tmp = tempfile::tempdir().unwrap();
        // Resolve symlinks (`/var` -> `/private/var` on macOS) so the walked
        // paths and the working directory share one spelling.
        let root = tmp.path().canonicalize().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        ignore_corpus(&root);
        crate::test_helpers::init_git_repository(&root);
        fs::write(root.join(".gitignore"), "tools/\nforced/\n").unwrap();
        fs::write(root.join("sub/.gitignore"), "gen/\n").unwrap();
        fs::write(root.join(".git/info/exclude"), "excl/\n").unwrap();
        run_git(&root, &["add", "--force", "forced/AGENTS.md"]);
        std::env::set_current_dir(&root).unwrap();

        let report = recursive_files(Path::new("."), Path::new("."), None);
        let displays: Vec<_> = report
            .entries
            .iter()
            .map(|entry| entry.display.as_str())
            .collect();
        for ignored in IGNORED_CORPUS_PATHS {
            assert!(
                !displays.contains(ignored),
                "{ignored} is git-ignored but still walked: {displays:?}"
            );
        }
        assert!(
            displays.contains(&"plain/AGENTS.md"),
            "an ordinary file must stay discoverable: {displays:?}"
        );
        assert!(
            displays.contains(&"forced/AGENTS.md"),
            "a tracked file is never ignored, whatever the patterns say: {displays:?}"
        );
    }

    /// The same tree with no Git repository keeps the unfiltered behavior.
    #[test]
    #[serial_test::serial]
    fn walks_without_a_git_repository_filter_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        ignore_corpus(&root);
        fs::write(root.join(".gitignore"), "tools/\n").unwrap();
        fs::write(root.join("sub/.gitignore"), "gen/\n").unwrap();
        std::env::set_current_dir(&root).unwrap();

        let report = recursive_files(Path::new("."), Path::new("."), None);
        let displays: Vec<_> = report
            .entries
            .iter()
            .map(|entry| entry.display.as_str())
            .collect();
        for walked in IGNORED_CORPUS_PATHS {
            if *walked == "excl/AGENTS.md" {
                continue; // that source needs a `.git` directory to exist
            }
            assert!(
                displays.contains(walked),
                "{walked} has no ignore status without Git: {displays:?}"
            );
        }
    }

    /// A walk rooted below an ignored directory is filtered too: Git's answer
    /// accounts for ancestors, which is how skill and agent discovery starts
    /// inside `.claude/`.
    #[test]
    #[serial_test::serial]
    fn walks_rooted_inside_an_ignored_directory_are_filtered() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        fs::create_dir_all(root.join(".claude/skills/local")).unwrap();
        fs::create_dir_all(root.join(".claude/skills/shipped")).unwrap();
        fs::write(root.join(".claude/skills/local/SKILL.md"), "body\n").unwrap();
        fs::write(root.join(".claude/skills/shipped/SKILL.md"), "body\n").unwrap();
        crate::test_helpers::init_git_repository(&root);
        std::env::set_current_dir(&root).unwrap();

        fs::write(root.join(".gitignore"), ".claude/skills/local/\n").unwrap();
        let report = shallow_directories(Path::new(".claude/skills"), Path::new("."), None);
        assert_eq!(
            report
                .entries
                .iter()
                .map(|entry| &entry.display)
                .collect::<Vec<_>>(),
            [".claude/skills/shipped"],
            "a directly named ignored skill is dropped"
        );

        // The whole surface is ignored one level above the walk root.
        let nested = tempfile::tempdir().unwrap();
        let nested_root = nested.path().canonicalize().unwrap();
        fs::create_dir_all(nested_root.join(".claude/skills/shipped")).unwrap();
        fs::write(
            nested_root.join(".claude/skills/shipped/SKILL.md"),
            "body\n",
        )
        .unwrap();
        crate::test_helpers::init_git_repository(&nested_root);
        fs::write(nested_root.join(".gitignore"), ".claude/\n").unwrap();
        std::env::set_current_dir(&nested_root).unwrap();

        let report = shallow_directories(Path::new(".claude/skills"), Path::new("."), None);
        assert!(
            report.entries.is_empty(),
            "an ignored ancestor hides the whole walk: {:?}",
            report.entries
        );
    }

    #[test]
    fn repository_relative_paths_stay_inside_the_analysis_root() {
        let root = Path::new("/repo");
        assert_eq!(
            repository_relative(root, Path::new("/repo/skills/a/SKILL.md")),
            Some("skills/a/SKILL.md".to_string())
        );
        assert_eq!(
            repository_relative(root, Path::new("./skills/a/SKILL.md")),
            Some("skills/a/SKILL.md".to_string())
        );
        // Outside the analysis root, Git has no opinion to ask for.
        assert_eq!(
            repository_relative(root, Path::new("/elsewhere/a.md")),
            None
        );
        assert_eq!(repository_relative(root, Path::new("/repo")), None);
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
