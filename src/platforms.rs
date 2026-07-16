//! Detection and activation for optional agent platforms.
//!
//! A platform is observed from its on-disk surfaces, then independently
//! activated by its optional `agent-lint.toml` override. Keeping those two
//! operations separate lets configuration change validator policy without
//! changing what the repository contains.

use crate::config::{ExcludeSet, PlatformOverrides};
use std::path::Path;
use walkdir::{DirEntry, WalkDir};

const IGNORED_DIRECTORY_NAMES: &[&str] =
    &[".git", "node_modules", "vendor", "target", "dist", "build"];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlatformDetection {
    pub cursor: bool,
    pub codex: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ActivePlatforms {
    pub cursor: bool,
    pub codex: bool,
}

impl PlatformDetection {
    /// Discover supported platform surfaces in the current repository.
    pub fn discover(exclude: &ExcludeSet) -> Self {
        Self {
            cursor: cursor_surface_exists(exclude),
            codex: codex_surface_exists(exclude),
        }
    }

    pub fn activate(self, overrides: PlatformOverrides) -> ActivePlatforms {
        ActivePlatforms {
            cursor: overrides.cursor.unwrap_or(self.cursor),
            codex: overrides.codex.unwrap_or(self.codex),
        }
    }
}

impl ActivePlatforms {
    pub fn any(self) -> bool {
        self.cursor || self.codex
    }
}

fn cursor_surface_exists(exclude: &ExcludeSet) -> bool {
    is_included_file(".cursorrules", exclude)
        || is_included_file(".cursor/hooks.json", exclude)
        || is_included_file(".cursor/environment.json", exclude)
        || has_matching_file(".cursor/rules", exclude, |path| {
            matches!(
                path.extension().and_then(|ext| ext.to_str()),
                Some("md" | "mdc")
            )
        })
        || has_matching_file(".cursor/agents", exclude, |path| {
            path.extension().and_then(|ext| ext.to_str()) == Some("md")
        })
        || has_matching_file(".cursor/skills", exclude, |path| {
            path.file_name().and_then(|name| name.to_str()) == Some("SKILL.md")
        })
}

fn codex_surface_exists(exclude: &ExcludeSet) -> bool {
    is_included_file(".codex/config.toml", exclude)
        || is_included_file(".codex-plugin/plugin.json", exclude)
        || is_included_file("AGENTS.override.md", exclude)
        || has_matching_file(".", exclude, |path| {
            path.file_name().and_then(|name| name.to_str()) == Some("AGENTS.md")
        })
        || has_matching_file(".agents/skills", exclude, |path| {
            path.file_name().and_then(|name| name.to_str()) == Some("SKILL.md")
        })
}

fn is_included_file(path: &str, exclude: &ExcludeSet) -> bool {
    Path::new(path).is_file() && !exclude.is_excluded(path)
}

fn has_matching_file(base: &str, exclude: &ExcludeSet, matches: impl Fn(&Path) -> bool) -> bool {
    WalkDir::new(base)
        .into_iter()
        .filter_entry(should_descend)
        .flatten()
        .any(|entry| {
            entry.file_type().is_file()
                && matches(entry.path())
                && !exclude.is_excluded(&display_path(entry.path()))
        })
}

/// Skip repository metadata, dependency trees, and conventional build output.
pub fn should_descend(entry: &DirEntry) -> bool {
    !IGNORED_DIRECTORY_NAMES.contains(&entry.file_name().to_string_lossy().as_ref())
}

fn display_path(path: &Path) -> String {
    path.strip_prefix(".")
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::CwdGuard;

    #[test]
    #[serial_test::serial]
    fn discovers_every_supported_surface() {
        let cases = [
            (".cursorrules", true, false),
            (".cursor/rules/project.md", true, false),
            (".cursor/rules/project.mdc", true, false),
            (".cursor/hooks.json", true, false),
            (".cursor/agents/reviewer.md", true, false),
            (".cursor/environment.json", true, false),
            (".cursor/skills/reviewer/SKILL.md", true, false),
            (".codex/config.toml", false, true),
            (".codex-plugin/plugin.json", false, true),
            ("AGENTS.md", false, true),
            ("nested/AGENTS.md", false, true),
            ("AGENTS.override.md", false, true),
            (".agents/skills/reviewer/SKILL.md", false, true),
        ];

        for (path, cursor, codex) in cases {
            let _guard = CwdGuard::new();
            let tmp = tempfile::tempdir().unwrap();
            std::env::set_current_dir(tmp.path()).unwrap();
            let path = Path::new(path);
            std::fs::create_dir_all(path.parent().unwrap_or_else(|| Path::new("."))).unwrap();
            std::fs::write(path, "surface").unwrap();

            assert_eq!(
                PlatformDetection::discover(&ExcludeSet::default()),
                PlatformDetection { cursor, codex },
                "failed to detect {}",
                path.display()
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn ignores_git_and_excluded_nested_agents_files() {
        let _guard = CwdGuard::new();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".git/vendor").unwrap();
        std::fs::write(".git/vendor/AGENTS.md", "ignored").unwrap();
        assert_eq!(
            PlatformDetection::discover(&ExcludeSet::default()),
            PlatformDetection::default()
        );

        std::fs::create_dir_all("vendor").unwrap();
        std::fs::write("vendor/AGENTS.md", "ignored dependency").unwrap();
        assert_eq!(
            PlatformDetection::discover(&ExcludeSet::default()),
            PlatformDetection::default()
        );

        std::fs::create_dir_all("generated").unwrap();
        std::fs::write("generated/AGENTS.md", "excluded generated file").unwrap();
        let exclude = ExcludeSet::new(&["generated/**".into()]).unwrap();
        assert_eq!(
            PlatformDetection::discover(&exclude),
            PlatformDetection::default()
        );
    }

    #[test]
    fn overrides_change_activation_without_changing_observation() {
        let detected = PlatformDetection {
            cursor: true,
            codex: true,
        };
        let active = detected.activate(PlatformOverrides {
            cursor: Some(false),
            codex: Some(true),
        });
        assert_eq!(
            detected,
            PlatformDetection {
                cursor: true,
                codex: true
            }
        );
        assert_eq!(
            active,
            ActivePlatforms {
                cursor: false,
                codex: true
            }
        );
    }
}
