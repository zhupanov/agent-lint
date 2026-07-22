//! Detection and activation for platform-specific and shared agent surfaces.
//!
//! Unique platform and shared surfaces are observed independently. Optional
//! `agent-lint.toml` overrides resolve only platform activation, leaving shared
//! observations intact.

use crate::config::{ExcludeSet, PlatformOverrides};
use crate::traversal;
use std::path::{Component, Path};

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
        || !cursor_rule_candidates(exclude).is_empty()
        || has_matching_file(".cursor/agents", exclude, |path| {
            path.extension().and_then(|ext| ext.to_str()) == Some("md")
        })
        || !cursor_skill_candidates(exclude).is_empty()
}

/// Return the included Cursor-only skill files anywhere in the repository.
///
/// The result is the unique Cursor activation surface: shared `.agents/skills`
/// files deliberately do not appear here, because they must not infer Cursor.
pub(crate) fn cursor_skill_candidates(exclude: &ExcludeSet) -> Vec<traversal::WalkEntry> {
    skill_candidates(exclude, is_cursor_skill_path)
}

/// Return the included shared Agent Skills files anywhere in the repository.
pub(crate) fn agent_skill_candidates(exclude: &ExcludeSet) -> Vec<traversal::WalkEntry> {
    skill_candidates(exclude, is_agent_skill_path)
}

/// Return the normalized Cursor runtime skill inventory.
///
/// Cursor loads both `.cursor/skills/` and `.agents/skills/` skill roots,
/// including roots below monorepo packages. Walking the repository once keeps
/// overlapping roots deduplicated and gives every Cursor consumer the same
/// deterministic, exclusion-aware scope.
pub(crate) fn cursor_runtime_skill_candidates(exclude: &ExcludeSet) -> Vec<traversal::WalkEntry> {
    skill_candidates(exclude, |path| {
        is_cursor_skill_path(path) || is_agent_skill_path(path)
    })
}

pub(crate) fn is_cursor_skill_path(path: &Path) -> bool {
    is_skill_path_below(path, ".cursor")
}

fn is_agent_skill_path(path: &Path) -> bool {
    is_skill_path_below(path, ".agents")
}

fn skill_candidates(
    exclude: &ExcludeSet,
    includes: impl Fn(&Path) -> bool,
) -> Vec<traversal::WalkEntry> {
    traversal::recursive_files(Path::new("."), Path::new("."), Some(exclude))
        .entries
        .into_iter()
        .filter(|entry| includes(&entry.path))
        .collect()
}

fn is_skill_path_below(path: &Path, surface_dir: &str) -> bool {
    path.file_name().and_then(|name| name.to_str()) == Some("SKILL.md")
        && path
            .components()
            .collect::<Vec<_>>()
            .windows(2)
            .any(|pair| {
                matches!(pair, [Component::Normal(surface), Component::Normal(skills)]
                if *surface == surface_dir && *skills == "skills")
            })
}

/// Return included Cursor project-rule candidates anywhere in the repository.
///
/// Cursor permits `.cursor/rules` directories below the repository root. This
/// is the single discovery contract consumed by platform detection and Cursor
/// validation, so an included candidate that activates Cursor is also the
/// candidate that validation receives. `recursive_files` supplies deterministic
/// ordering, exclusion handling, pruning, and no-follow-symlink behavior.
pub(crate) fn cursor_rule_candidates(exclude: &ExcludeSet) -> Vec<traversal::WalkEntry> {
    traversal::recursive_files(Path::new("."), Path::new("."), Some(exclude))
        .entries
        .into_iter()
        .filter(|entry| {
            is_beneath_cursor_rules(&entry.path)
                && matches!(
                    entry
                        .path
                        .extension()
                        .and_then(|extension| extension.to_str()),
                    Some("md" | "mdc")
                )
        })
        .collect()
}

fn is_beneath_cursor_rules(path: &Path) -> bool {
    let components: Vec<_> = path.components().collect();
    components.windows(2).any(|pair| {
        matches!(pair, [Component::Normal(cursor), Component::Normal(rules)]
            if *cursor == ".cursor" && *rules == "rules")
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
    !agent_skill_candidates(exclude).is_empty()
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
    fn cursor_rule_candidates_cover_nested_roots_and_ignore_non_candidates() {
        let _guard = CwdGuard::new();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        for path in [
            ".cursor/rules/root.mdc",
            ".cursor/rules/nested/root.md",
            "packages/api/.cursor/rules/api.mdc",
            "packages/web/.cursor/rules/web.md",
            ".cursor/rules/nested/.cursor/rules/overlap.mdc",
            "packages/api/.cursor/rules/ignored.txt",
            "docs/not-a-rule.md",
            "node_modules/pkg/.cursor/rules/dependency.mdc",
            "vendor/pkg/.cursor/rules/dependency.mdc",
            "target/pkg/.cursor/rules/dependency.mdc",
            "dist/pkg/.cursor/rules/dependency.mdc",
            "build/pkg/.cursor/rules/dependency.mdc",
        ] {
            let path = Path::new(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "rule").unwrap();
        }
        let exclude = ExcludeSet::new(&["packages/web/**".into()]).unwrap();

        let candidates = cursor_rule_candidates(&exclude);
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.display.as_str())
                .collect::<Vec<_>>(),
            [
                ".cursor/rules/nested/.cursor/rules/overlap.mdc",
                ".cursor/rules/nested/root.md",
                ".cursor/rules/root.mdc",
                "packages/api/.cursor/rules/api.mdc",
            ]
        );
        assert!(DetectedSurfaces::discover(&exclude).cursor);
    }

    #[test]
    #[serial_test::serial]
    fn cursor_skill_inventory_is_recursive_deduplicated_and_activation_aware() {
        let _guard = CwdGuard::new();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        for path in [
            ".cursor/skills/root/SKILL.md",
            ".cursor/skills/shared/SKILL.md",
            ".cursor/skills/group/nested/SKILL.md",
            "packages/api/.cursor/skills/api/SKILL.md",
            "packages/api/.agents/skills/shared/SKILL.md",
            ".agents/skills/root-shared/SKILL.md",
            "packages/web/.agents/skills/excluded/SKILL.md",
            "node_modules/pkg/.cursor/skills/ignored/SKILL.md",
            ".cursor/skills/group/.cursor/skills/overlap/SKILL.md",
        ] {
            let path = Path::new(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(
                path,
                "---\nname: test\ndescription: Test skill\n---\nBody\n",
            )
            .unwrap();
        }
        let exclude = ExcludeSet::new(&["packages/web/**".into()]).unwrap();

        assert_eq!(
            cursor_skill_candidates(&exclude)
                .iter()
                .map(|entry| entry.display.as_str())
                .collect::<Vec<_>>(),
            [
                ".cursor/skills/group/.cursor/skills/overlap/SKILL.md",
                ".cursor/skills/group/nested/SKILL.md",
                ".cursor/skills/root/SKILL.md",
                ".cursor/skills/shared/SKILL.md",
                "packages/api/.cursor/skills/api/SKILL.md",
            ]
        );
        assert_eq!(
            cursor_runtime_skill_candidates(&exclude)
                .iter()
                .map(|entry| entry.display.as_str())
                .collect::<Vec<_>>(),
            [
                ".agents/skills/root-shared/SKILL.md",
                ".cursor/skills/group/.cursor/skills/overlap/SKILL.md",
                ".cursor/skills/group/nested/SKILL.md",
                ".cursor/skills/root/SKILL.md",
                ".cursor/skills/shared/SKILL.md",
                "packages/api/.agents/skills/shared/SKILL.md",
                "packages/api/.cursor/skills/api/SKILL.md",
            ]
        );
        assert!(DetectedSurfaces::discover(&exclude).cursor);
        assert!(DetectedSurfaces::discover(&exclude).agent_skills);
    }

    #[test]
    #[serial_test::serial]
    fn nested_shared_skills_do_not_infer_cursor() {
        let _guard = CwdGuard::new();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        let path = Path::new("packages/api/.agents/skills/shared/SKILL.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path,
            "---\nname: shared\ndescription: Shared skill\n---\nBody\n",
        )
        .unwrap();

        assert_eq!(
            DetectedSurfaces::discover(&ExcludeSet::default()),
            DetectedSurfaces {
                cursor: false,
                codex: false,
                claude_md: false,
                agents_md: false,
                agent_skills: true,
            }
        );
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn cursor_rule_candidates_do_not_follow_symlinks() {
        use std::os::unix::fs::symlink;

        let _guard = CwdGuard::new();
        let tmp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(outside.path().join(".cursor/rules")).unwrap();
        std::fs::write(outside.path().join(".cursor/rules/rule.mdc"), "rule").unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        symlink(outside.path(), "linked").unwrap();

        assert!(cursor_rule_candidates(&ExcludeSet::default()).is_empty());
        assert!(!DetectedSurfaces::discover(&ExcludeSet::default()).cursor);
    }

    #[test]
    #[serial_test::serial]
    fn md_only_candidates_activate_cursor_and_respect_overrides() {
        let _guard = CwdGuard::new();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("packages/api/.cursor/rules").unwrap();
        std::fs::write("packages/api/.cursor/rules/not-a-rule.md", "text").unwrap();

        let detected = DetectedSurfaces::discover(&ExcludeSet::default());
        assert!(detected.cursor);
        assert!(
            !detected
                .resolve(PlatformOverrides {
                    cursor: Some(false),
                    codex: None,
                })
                .cursor
        );
        assert!(
            detected
                .resolve(PlatformOverrides {
                    cursor: Some(true),
                    codex: None,
                })
                .cursor
        );
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
