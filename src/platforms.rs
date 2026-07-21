//! Detection and activation for platform-specific and shared agent surfaces.
//!
//! Unique platform and shared surfaces are observed independently. Optional
//! `agent-lint.toml` overrides resolve only platform activation, leaving shared
//! observations intact.

use crate::config::{ExcludeSet, PlatformOverrides};
use crate::traversal;
use std::path::Path;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DetectedSurfaces {
    pub cursor: bool,
    pub codex: bool,
    pub claude_md: bool,
    pub agents_md: bool,
    pub agent_skills: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ValidationTargets {
    pub cursor: bool,
    pub codex: bool,
    pub claude_md: bool,
    pub agents_md: bool,
    pub agent_skills: bool,
}

impl DetectedSurfaces {
    /// Discover supported platform-specific and shared surfaces.
    pub fn discover(exclude: &ExcludeSet) -> Self {
        Self {
            cursor: cursor_surface_exists(exclude),
            codex: codex_surface_exists(exclude),
            claude_md: is_included_file("CLAUDE.md", exclude),
            agents_md: agents_md_surface_exists(exclude),
            agent_skills: agent_skills_surface_exists(exclude),
        }
    }

    pub fn resolve(self, overrides: PlatformOverrides) -> ValidationTargets {
        ValidationTargets {
            cursor: overrides.cursor.unwrap_or(self.cursor),
            codex: overrides.codex.unwrap_or(self.codex),
            claude_md: self.claude_md,
            agents_md: self.agents_md,
            agent_skills: self.agent_skills,
        }
    }
}

impl ValidationTargets {
    pub fn has_work(self) -> bool {
        self.cursor || self.codex || self.claude_md || self.agents_md || self.agent_skills
    }
}

fn cursor_surface_exists(exclude: &ExcludeSet) -> bool {
    is_included_file(".cursorrules", exclude)
        || is_included_file(".cursor/mcp.json", exclude)
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
}

fn agents_md_surface_exists(exclude: &ExcludeSet) -> bool {
    has_matching_file(".", exclude, |path| {
        path.file_name().and_then(|name| name.to_str()) == Some("AGENTS.md")
    })
}

fn agent_skills_surface_exists(exclude: &ExcludeSet) -> bool {
    has_matching_file(".agents/skills", exclude, |path| {
        path.file_name().and_then(|name| name.to_str()) == Some("SKILL.md")
    })
}

fn is_included_file(path: &str, exclude: &ExcludeSet) -> bool {
    Path::new(path).is_file() && !exclude.is_excluded(path)
}

fn has_matching_file(base: &str, exclude: &ExcludeSet, matches: impl Fn(&Path) -> bool) -> bool {
    traversal::recursive_files(Path::new(base), Path::new("."), Some(exclude))
        .entries
        .iter()
        .any(|entry| matches(&entry.path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::CwdGuard;

    #[test]
    #[serial_test::serial]
    fn discovers_every_supported_surface() {
        let cases = [
            (".cursorrules", true, false, false, false, false),
            (".cursor/mcp.json", true, false, false, false, false),
            (".cursor/rules/project.md", true, false, false, false, false),
            (
                ".cursor/rules/project.mdc",
                true,
                false,
                false,
                false,
                false,
            ),
            (".cursor/hooks.json", true, false, false, false, false),
            (
                ".cursor/agents/reviewer.md",
                true,
                false,
                false,
                false,
                false,
            ),
            (".cursor/environment.json", true, false, false, false, false),
            (
                ".cursor/skills/reviewer/SKILL.md",
                true,
                false,
                false,
                false,
                false,
            ),
            (".codex/config.toml", false, true, false, false, false),
            (
                ".codex-plugin/plugin.json",
                false,
                true,
                false,
                false,
                false,
            ),
            ("CLAUDE.md", false, false, true, false, false),
            ("AGENTS.md", false, false, false, true, false),
            ("nested/AGENTS.md", false, false, false, true, false),
            ("AGENTS.override.md", false, true, false, false, false),
            (
                ".agents/skills/reviewer/SKILL.md",
                false,
                false,
                false,
                false,
                true,
            ),
        ];

        for (path, cursor, codex, claude_md, agents_md, agent_skills) in cases {
            let _guard = CwdGuard::new();
            let tmp = tempfile::tempdir().unwrap();
            std::env::set_current_dir(tmp.path()).unwrap();
            let path = Path::new(path);
            std::fs::create_dir_all(path.parent().unwrap_or_else(|| Path::new("."))).unwrap();
            std::fs::write(path, "surface").unwrap();

            assert_eq!(
                DetectedSurfaces::discover(&ExcludeSet::default()),
                DetectedSurfaces {
                    cursor,
                    codex,
                    claude_md,
                    agents_md,
                    agent_skills,
                },
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
            DetectedSurfaces::discover(&ExcludeSet::default()),
            DetectedSurfaces::default()
        );

        std::fs::create_dir_all("vendor").unwrap();
        std::fs::write("vendor/AGENTS.md", "ignored dependency").unwrap();
        assert_eq!(
            DetectedSurfaces::discover(&ExcludeSet::default()),
            DetectedSurfaces::default()
        );

        std::fs::create_dir_all("generated").unwrap();
        std::fs::write("generated/AGENTS.md", "excluded generated file").unwrap();
        let exclude = ExcludeSet::new(&["generated/**".into()]).unwrap();
        assert_eq!(
            DetectedSurfaces::discover(&exclude),
            DetectedSurfaces::default()
        );
    }

    #[test]
    fn overrides_change_activation_without_changing_observation() {
        let detected = DetectedSurfaces {
            cursor: true,
            codex: true,
            claude_md: true,
            agents_md: true,
            agent_skills: true,
        };
        let active = detected.resolve(PlatformOverrides {
            cursor: Some(false),
            codex: Some(true),
        });
        assert_eq!(
            detected,
            DetectedSurfaces {
                cursor: true,
                codex: true,
                claude_md: true,
                agents_md: true,
                agent_skills: true,
            }
        );
        assert_eq!(
            active,
            ValidationTargets {
                cursor: false,
                codex: true,
                claude_md: true,
                agents_md: true,
                agent_skills: true,
            }
        );
    }
}
