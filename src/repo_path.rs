//! Repository-safe path resolution shared by I003, D005, L005, and S062.
//!
//! Callers choose a resolution base. The resolver never follows a symlink and
//! never discloses an outside-repository canonical path in its result.

use std::fs;
use std::path::{Component, Path, PathBuf};

/// Where a reference is resolved from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionBase {
    /// Join with the repository root (current working directory).
    RepositoryRoot,
    /// Join with the directory that owns `source`.
    SourceRelative,
}

/// Outcome of a repository-safe existence probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathProbe {
    /// Path is a regular file inside the repository with no symlink components.
    File(PathBuf),
    /// Path is a non-symlink directory inside the repository.
    Directory(PathBuf),
    /// Lexically safe repository-relative path that does not exist.
    Missing(PathBuf),
    /// Absolute, escaping, symlink-containing, non-UTF-8, or canonically outside.
    Rejected,
}

/// Replace Windows separators before classification or probing.
pub fn normalize_separators(raw: &str) -> String {
    raw.replace('\\', "/")
}

/// Strip one `#fragment` and one `::symbol` suffix, then normalize separators.
pub fn normalize_path_probe(raw: &str) -> String {
    let without_fragment = raw.split_once('#').map_or(raw, |(path, _)| path);
    let without_symbol = without_fragment
        .split_once("::")
        .map_or(without_fragment, |(path, _)| path);
    normalize_separators(without_symbol)
}

/// Lexically normalize a repository-relative path.
///
/// Rejects absolute paths and `..` components that escape the repository root.
/// Safe in-repository `..` segments are collapsed.
pub fn normalize_repo_relative(path: &Path) -> Option<PathBuf> {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !result.pop() {
                    return None;
                }
            }
            Component::Normal(value) => {
                value.to_str()?;
                result.push(value);
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(result)
}

/// Whether the authored probe still contains a parent-directory segment.
///
/// I003/D005 keep the #241 policy of treating any `..` in the probe as unsafe
/// even when lexical collapse would remain inside the repository.
pub fn probe_contains_parent_segment(probe: &str) -> bool {
    Path::new(probe)
        .components()
        .any(|component| component == Component::ParentDir)
}

/// Resolve `raw` against `source` using `base`, then probe without following
/// symlinks. On success the returned path is repository-relative.
pub fn resolve_repo_path(source: &Path, raw: &str, base: ResolutionBase) -> PathProbe {
    let probe = normalize_path_probe(raw);
    if probe.is_empty() {
        return PathProbe::Rejected;
    }
    let candidate = Path::new(&probe);
    if candidate.is_absolute() {
        return PathProbe::Rejected;
    }
    let joined = match base {
        ResolutionBase::RepositoryRoot => PathBuf::from(&probe),
        ResolutionBase::SourceRelative => source
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(&probe),
    };
    let Some(normalized) = normalize_repo_relative(&joined) else {
        return PathProbe::Rejected;
    };
    if normalized.as_os_str().is_empty() {
        return PathProbe::Rejected;
    }
    probe_normalized(&normalized)
}

fn probe_normalized(normalized: &Path) -> PathProbe {
    let mut current = PathBuf::new();
    for component in normalized.components() {
        let Component::Normal(part) = component else {
            return PathProbe::Rejected;
        };
        current.push(part);
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(_) => return PathProbe::Missing(normalized.to_path_buf()),
        };
        if metadata.file_type().is_symlink() {
            return PathProbe::Rejected;
        }
    }
    let metadata = match fs::symlink_metadata(normalized) {
        Ok(metadata) => metadata,
        Err(_) => return PathProbe::Missing(normalized.to_path_buf()),
    };
    if metadata.file_type().is_symlink() {
        return PathProbe::Rejected;
    }
    if !stays_inside_repository(normalized) {
        return PathProbe::Rejected;
    }
    if metadata.is_file() {
        PathProbe::File(normalized.to_path_buf())
    } else if metadata.is_dir() {
        PathProbe::Directory(normalized.to_path_buf())
    } else {
        PathProbe::Rejected
    }
}

fn stays_inside_repository(path: &Path) -> bool {
    let Ok(root) = std::env::current_dir().and_then(|dir| dir.canonicalize()) else {
        return false;
    };
    let Ok(canonical) = path.canonicalize() else {
        return false;
    };
    canonical.starts_with(&root)
}

/// Dedupe key for equivalent slash/backslash, fragment, and symbol spellings.
pub fn normalized_target_key(source: &Path, raw: &str, base: ResolutionBase) -> Option<String> {
    let probe = normalize_path_probe(raw);
    if probe.is_empty() || Path::new(&probe).is_absolute() {
        return None;
    }
    let joined = match base {
        ResolutionBase::RepositoryRoot => PathBuf::from(&probe),
        ResolutionBase::SourceRelative => source
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(&probe),
    };
    normalize_repo_relative(&joined).map(|path| path.to_string_lossy().replace('\\', "/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    #[serial_test::serial]
    fn source_relative_does_not_fall_back_to_root() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all("nested").unwrap();
        fs::write("present.md", "root\n").unwrap();
        let outcome = resolve_repo_path(
            Path::new("nested/AGENTS.md"),
            "present.md",
            ResolutionBase::SourceRelative,
        );
        assert_eq!(
            outcome,
            PathProbe::Missing(PathBuf::from("nested/present.md"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn root_relative_finds_repository_file() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all("docs").unwrap();
        fs::write("docs/present.md", "ok\n").unwrap();
        assert_eq!(
            resolve_repo_path(
                Path::new("nested/AGENTS.md"),
                "docs/present.md",
                ResolutionBase::RepositoryRoot,
            ),
            PathProbe::File(PathBuf::from("docs/present.md"))
        );
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn rejects_ancestor_symlink_components() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all("outside").unwrap();
        fs::write("outside/present.md", "secret\n").unwrap();
        fs::create_dir_all("docs").unwrap();
        std::os::unix::fs::symlink("../outside", "docs/external").unwrap();
        assert_eq!(
            resolve_repo_path(
                Path::new("AGENTS.md"),
                "docs/external/present.md",
                ResolutionBase::RepositoryRoot,
            ),
            PathProbe::Rejected
        );
    }

    #[test]
    fn normalizes_separators_and_suffixes() {
        assert_eq!(
            normalize_path_probe(r"docs\file.md#section::symbol"),
            "docs/file.md"
        );
        assert!(probe_contains_parent_segment("docs/../outside.md"));
        assert!(!probe_contains_parent_segment("docs/outside.md"));
    }
}
