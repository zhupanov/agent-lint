//! Detection and activation for platform-specific and shared agent surfaces.
//!
//! Unique platform and shared surfaces are observed independently. Optional
//! `agent-lint.toml` overrides resolve only platform activation, leaving shared
//! observations intact.

use crate::config::{ExcludeSet, PlatformOverrides};
use crate::traversal;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

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
        || is_included_file("AGENTS.override.md", exclude)
        || !codex_plugin_manifests(exclude).is_empty()
}

/// Codex-recognized plugin manifest directory components, in the precedence
/// order Codex applies when more than one exists beneath a single plugin root.
pub(crate) const CODEX_PLUGIN_MANIFEST_DIRS: &[&str] =
    &[".codex-plugin", ".claude-plugin", ".cursor-plugin"];

/// One Codex plugin manifest selected for validation: exactly one per plugin
/// root, chosen by upstream precedence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodexPluginManifest {
    /// Physical path used to read the manifest file.
    pub path: PathBuf,
    /// Normalized, repository-relative path for diagnostics and exclusions.
    pub display: String,
}

/// Discover every Codex plugin root and select its effective manifest.
///
/// A plugin root is any directory that contains a recognized manifest directory
/// (`.codex-plugin`, `.claude-plugin`, or `.cursor-plugin`) holding a
/// `plugin.json`. Candidates are classified by their exact parent-directory
/// component, never by path-string suffix, so `my.codex-plugin/plugin.json`
/// (parent component `my.codex-plugin`) is not a recognized manifest and
/// establishes no plugin root. Each root selects the first existing manifest in
/// Codex precedence order and yields it once; a nested plugin root is
/// independent, never a mislocated copy of an ancestor.
///
/// This is the single discovery contract consumed by both platform detection
/// and Codex plugin validation, so an included manifest that activates Codex is
/// the same manifest validation receives. `recursive_files` supplies
/// deterministic repository-relative ordering, exclusion handling,
/// ignored-tree pruning, and no-follow-symlink behavior.
pub(crate) fn codex_plugin_manifests(exclude: &ExcludeSet) -> Vec<CodexPluginManifest> {
    let mut by_root: BTreeMap<PathBuf, (usize, traversal::WalkEntry)> = BTreeMap::new();
    for entry in traversal::recursive_files(Path::new("."), Path::new("."), Some(exclude)).entries {
        if entry
            .path
            .file_name()
            .is_none_or(|name| name != "plugin.json")
        {
            continue;
        }
        let Some(parent) = entry.path.parent() else {
            continue;
        };
        let Some(precedence) = parent
            .file_name()
            .and_then(|component| component.to_str())
            .and_then(|component| {
                CODEX_PLUGIN_MANIFEST_DIRS
                    .iter()
                    .position(|dir| *dir == component)
            })
        else {
            continue;
        };
        let root = parent.parent().map(Path::to_path_buf).unwrap_or_default();
        by_root
            .entry(root)
            .and_modify(|selected| {
                if precedence < selected.0 {
                    *selected = (precedence, entry.clone());
                }
            })
            .or_insert_with(|| (precedence, entry.clone()));
    }
    let mut manifests: Vec<CodexPluginManifest> = by_root
        .into_values()
        .map(|(_, entry)| CodexPluginManifest {
            path: entry.path,
            display: entry.display,
        })
        .collect();
    manifests.sort_by(|left, right| left.display.cmp(&right.display));
    manifests
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
    #[serial_test::serial]
    fn codex_activates_on_any_recognized_manifest_at_any_depth() {
        for path in [
            ".codex-plugin/plugin.json",
            "plugins/example/.codex-plugin/plugin.json",
            ".claude-plugin/plugin.json",
            "packages/api/.claude-plugin/plugin.json",
            "libs/widget/.cursor-plugin/plugin.json",
        ] {
            let _guard = CwdGuard::new();
            let tmp = tempfile::tempdir().unwrap();
            std::env::set_current_dir(tmp.path()).unwrap();
            let path = Path::new(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, r#"{"name":"x"}"#).unwrap();
            assert!(
                DetectedSurfaces::discover(&ExcludeSet::default()).codex,
                "manifest {} must activate Codex",
                path.display()
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn suffix_collision_manifests_do_not_activate_codex() {
        let _guard = CwdGuard::new();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        for path in [
            "my.codex-plugin/plugin.json",
            "x.claude-plugin/plugin.json",
            "y.cursor-plugin/plugin.json",
        ] {
            let path = Path::new(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, r#"{"name":"x"}"#).unwrap();
        }
        assert!(!DetectedSurfaces::discover(&ExcludeSet::default()).codex);
        assert!(codex_plugin_manifests(&ExcludeSet::default()).is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn codex_plugin_manifests_select_one_per_root_by_precedence() {
        let _guard = CwdGuard::new();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        // Root: all three present -> .codex-plugin wins.
        for dir in [".codex-plugin", ".claude-plugin", ".cursor-plugin"] {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(format!("{dir}/plugin.json"), r#"{"name":"x"}"#).unwrap();
        }
        // Nested root with only claude+cursor -> .claude-plugin wins.
        std::fs::create_dir_all("nested/.claude-plugin").unwrap();
        std::fs::write("nested/.claude-plugin/plugin.json", r#"{"name":"x"}"#).unwrap();
        std::fs::create_dir_all("nested/.cursor-plugin").unwrap();
        std::fs::write("nested/.cursor-plugin/plugin.json", r#"{"name":"x"}"#).unwrap();

        let selected: Vec<String> = codex_plugin_manifests(&ExcludeSet::default())
            .into_iter()
            .map(|manifest| manifest.display)
            .collect();
        assert_eq!(
            selected,
            vec![
                ".codex-plugin/plugin.json".to_string(),
                "nested/.claude-plugin/plugin.json".to_string(),
            ]
        );
    }

    #[test]
    #[serial_test::serial]
    fn codex_plugin_manifests_honor_exclusions() {
        let _guard = CwdGuard::new();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("vendored/.codex-plugin").unwrap();
        std::fs::write("vendored/.codex-plugin/plugin.json", r#"{"name":"x"}"#).unwrap();
        let exclude = ExcludeSet::new(&["vendored/**".to_string()]).unwrap();
        assert!(codex_plugin_manifests(&exclude).is_empty());
        assert!(!DetectedSurfaces::discover(&exclude).codex);
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
