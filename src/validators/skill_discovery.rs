//! Canonical Claude skill and command runtime discovery.
//!
//! All paths returned here are repository-relative, normalized, deduplicated,
//! sorted, and already filtered through the configured exclusion set.

use crate::config::ExcludeSet;
use crate::context::{LintContext, LintMode, ManifestState};
use crate::plugin_paths::safe_component_path;
use crate::traversal;
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone)]
pub(crate) struct SkillDiscovery {
    pub exported_skill_files: Vec<PathBuf>,
    pub private_skill_files: Vec<PathBuf>,
    pub active_command_files: Vec<PathBuf>,
    pub declared_skill_dirs: Vec<PathBuf>,
}

impl SkillDiscovery {
    pub(crate) fn from_context(ctx: &LintContext, exclude: &ExcludeSet) -> Self {
        let private_skill_files = skill_files_in_dir(Path::new(".claude/skills"), false, exclude);
        let mut active_command_files =
            markdown_files_in_dir(Path::new(".claude/commands"), exclude);
        if ctx.mode != LintMode::Plugin {
            return Self {
                exported_skill_files: Vec::new(),
                private_skill_files,
                active_command_files,
                declared_skill_dirs: Vec::new(),
            };
        }

        let (skills_declared, commands_declared, has_skills_field) = match &ctx.plugin_json {
            ManifestState::Parsed(manifest) => (
                declared_string_paths(manifest, "skills"),
                declared_string_paths(manifest, "commands"),
                manifest.get("skills").is_some(),
            ),
            _ => (Vec::new(), Vec::new(), false),
        };

        let mut exported = BTreeSet::new();
        for path in skill_files_in_dir(Path::new("skills"), true, exclude) {
            exported.insert(path);
        }
        let mut declared_skill_dirs = BTreeSet::new();
        for raw in skills_declared {
            let Some(dir) = safe_component_path(&raw) else {
                continue;
            };
            if !safe_existing_dir(&dir) {
                continue;
            }
            declared_skill_dirs.insert(dir.clone());
            for path in skill_files_in_dir(&dir, true, exclude) {
                exported.insert(path);
            }
        }
        // Claude only treats a root SKILL.md as a fallback when neither the
        // conventional directory nor a skills declaration is present.
        if !Path::new("skills").is_dir() && !has_skills_field {
            let root = PathBuf::from("SKILL.md");
            if safe_regular_file(&root) && !exclude.is_excluded("SKILL.md") {
                exported.insert(root);
            }
        }

        let command_roots =
            if commands_declared.is_empty() && !manifest_has_field(&ctx.plugin_json, "commands") {
                vec![PathBuf::from("commands")]
            } else {
                commands_declared
                    .into_iter()
                    .filter_map(|raw| safe_component_path(&raw))
                    .collect()
            };
        let mut commands = BTreeSet::new();
        for root in command_roots {
            if safe_regular_file(&root) {
                if root.extension().and_then(|value| value.to_str()) == Some("md")
                    && !exclude.is_excluded(&root.to_string_lossy())
                {
                    commands.insert(root);
                }
            } else if safe_existing_dir(&root) {
                commands.extend(markdown_files_in_dir(&root, exclude));
            }
        }
        active_command_files.extend(commands);
        active_command_files.sort();
        active_command_files.dedup();

        Self {
            exported_skill_files: exported.into_iter().collect(),
            private_skill_files,
            active_command_files,
            declared_skill_dirs: declared_skill_dirs.into_iter().collect(),
        }
    }
}

fn manifest_has_field(state: &ManifestState, field: &str) -> bool {
    matches!(state, ManifestState::Parsed(value) if value.get(field).is_some())
}

fn declared_string_paths(value: &Value, field: &str) -> Vec<String> {
    match value.get(field) {
        Some(Value::String(path)) => vec![path.clone()],
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

fn safe_existing_dir(path: &Path) -> bool {
    !has_symlink_component(path)
        && fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_dir())
}

fn safe_regular_file(path: &Path) -> bool {
    !has_symlink_component(path)
        && fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_file())
}

fn has_symlink_component(path: &Path) -> bool {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if fs::symlink_metadata(&current).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return true;
        }
    }
    false
}

fn skill_files_in_dir(dir: &Path, skip_shared: bool, exclude: &ExcludeSet) -> Vec<PathBuf> {
    traversal::shallow_directories(dir, Path::new("."), None)
        .entries
        .into_iter()
        .filter(|entry| {
            !(skip_shared && entry.path.file_name().and_then(|n| n.to_str()) == Some("shared"))
        })
        .map(|entry| entry.path.join("SKILL.md"))
        .filter(|path| safe_regular_file(path) && !exclude.is_excluded(&path.to_string_lossy()))
        .collect()
}

fn markdown_files_in_dir(dir: &Path, exclude: &ExcludeSet) -> Vec<PathBuf> {
    traversal::shallow_files(dir, Path::new("."), None)
        .entries
        .into_iter()
        .map(|entry| entry.path)
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("md"))
        .filter(|path| safe_regular_file(path) && !exclude.is_excluded(&path.to_string_lossy()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::CwdGuard;

    fn plugin_context() -> LintContext {
        LintContext::new(Path::new("."), LintMode::Plugin)
    }

    #[test]
    #[serial_test::serial]
    fn manifest_declared_skills_and_replacement_commands_are_runtime_exports() {
        let temp = tempfile::tempdir().unwrap();
        let _guard = CwdGuard::new();
        std::env::set_current_dir(temp.path()).unwrap();
        fs::create_dir_all(".claude-plugin").unwrap();
        fs::create_dir_all("custom/declared").unwrap();
        fs::create_dir_all("replacement").unwrap();
        fs::write(
            "custom/declared/SKILL.md",
            "---\nname: declared\ndescription: Use when testing discovery behavior\n---\n",
        )
        .unwrap();
        fs::write(
            "replacement/replaced.md",
            "---\ndescription: Use when testing command discovery\n---\n",
        )
        .unwrap();
        fs::write(
            "SKILL.md",
            "---\nname: root\ndescription: This root fallback is inactive\n---\n",
        )
        .unwrap();
        fs::write(
            ".claude-plugin/plugin.json",
            r#"{"skills":"./custom","commands":"./replacement"}"#,
        )
        .unwrap();

        let found = SkillDiscovery::from_context(&plugin_context(), &ExcludeSet::default());
        assert_eq!(
            found.exported_skill_files,
            vec![PathBuf::from("custom/declared/SKILL.md")]
        );
        assert_eq!(
            found.active_command_files,
            vec![PathBuf::from("replacement/replaced.md")]
        );
    }

    #[test]
    #[serial_test::serial]
    fn root_fallback_and_private_commands_are_discovered_without_default_trees() {
        let temp = tempfile::tempdir().unwrap();
        let _guard = CwdGuard::new();
        std::env::set_current_dir(temp.path()).unwrap();
        fs::create_dir_all(".claude-plugin").unwrap();
        fs::create_dir_all(".claude/commands").unwrap();
        fs::create_dir_all("commands").unwrap();
        fs::write(
            "SKILL.md",
            "---\nname: root\ndescription: Use when testing a root skill fallback\n---\n",
        )
        .unwrap();
        fs::write(
            "commands/published.md",
            "---\ndescription: Use when testing published commands\n---\n",
        )
        .unwrap();
        fs::write(
            ".claude/commands/private.md",
            "---\ndescription: Use when testing private commands\n---\n",
        )
        .unwrap();
        fs::write(".claude-plugin/plugin.json", "{}").unwrap();

        let found = SkillDiscovery::from_context(&plugin_context(), &ExcludeSet::default());
        assert_eq!(found.exported_skill_files, vec![PathBuf::from("SKILL.md")]);
        assert_eq!(
            found.active_command_files,
            vec![
                PathBuf::from(".claude/commands/private.md"),
                PathBuf::from("commands/published.md")
            ]
        );
    }
}
