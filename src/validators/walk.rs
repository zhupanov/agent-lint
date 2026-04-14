use crate::config::ExcludeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Shallow iteration of subdirectories within `base`, skipping entries that
/// don't pass the filter. Returns `(entry_path, dir_name)` pairs.
///
/// - `display_prefix`: used to build the exclude-check path (e.g., "skills" or ".claude/skills")
/// - `skip_shared`: when true, skips the "shared" subdirectory
pub fn read_subdirs(
    base: &Path,
    display_prefix: &str,
    exclude: &ExcludeSet,
    skip_shared: bool,
) -> Vec<(PathBuf, String)> {
    if !base.is_dir() {
        return Vec::new();
    }
    let entries = match fs::read_dir(base) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut result = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if skip_shared && name == "shared" {
            continue;
        }
        // Build a representative path for exclude checking (e.g., "skills/foo/SKILL.md")
        let check_path = format!("{display_prefix}/{name}/SKILL.md");
        if exclude.is_excluded(&check_path) {
            continue;
        }
        result.push((path, name));
    }
    result
}

/// Shallow iteration of files within `base` matching the given extension.
/// Returns `(entry_path, file_name)` pairs.
///
/// - `display_prefix`: used to build the exclude-check path
#[allow(dead_code)]
pub fn read_files(
    base: &Path,
    display_prefix: &str,
    exclude: &ExcludeSet,
    extension: &str,
) -> Vec<(PathBuf, String)> {
    if !base.is_dir() {
        return Vec::new();
    }
    let entries = match fs::read_dir(base) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut result = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) if n.ends_with(extension) => n.to_string(),
            _ => continue,
        };
        let check_path = format!("{display_prefix}/{name}");
        if exclude.is_excluded(&check_path) {
            continue;
        }
        result.push((path, name));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_subdirs_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let result = read_subdirs(tmp.path(), "test", &ExcludeSet::default(), false);
        assert!(result.is_empty());
    }

    #[test]
    fn read_subdirs_skips_files() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("file.txt"), "").unwrap();
        std::fs::create_dir(tmp.path().join("subdir")).unwrap();
        let result = read_subdirs(tmp.path(), "test", &ExcludeSet::default(), false);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1, "subdir");
    }

    #[test]
    fn read_subdirs_skips_shared() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("shared")).unwrap();
        std::fs::create_dir(tmp.path().join("other")).unwrap();
        let result = read_subdirs(tmp.path(), "test", &ExcludeSet::default(), true);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1, "other");
    }

    #[test]
    fn read_subdirs_includes_shared_when_not_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("shared")).unwrap();
        std::fs::create_dir(tmp.path().join("other")).unwrap();
        let result = read_subdirs(tmp.path(), "test", &ExcludeSet::default(), false);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn read_subdirs_nonexistent_dir() {
        let result = read_subdirs(
            Path::new("/nonexistent/path"),
            "test",
            &ExcludeSet::default(),
            false,
        );
        assert!(result.is_empty());
    }

    #[test]
    fn read_files_filters_by_extension() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("agent.md"), "content").unwrap();
        std::fs::write(tmp.path().join("readme.txt"), "content").unwrap();
        let result = read_files(tmp.path(), "test", &ExcludeSet::default(), ".md");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1, "agent.md");
    }

    #[test]
    fn read_files_skips_directories() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("subdir.md")).unwrap();
        std::fs::write(tmp.path().join("file.md"), "content").unwrap();
        let result = read_files(tmp.path(), "test", &ExcludeSet::default(), ".md");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1, "file.md");
    }

    #[test]
    fn read_files_nonexistent_dir() {
        let result = read_files(
            Path::new("/nonexistent/path"),
            "test",
            &ExcludeSet::default(),
            ".md",
        );
        assert!(result.is_empty());
    }
}
