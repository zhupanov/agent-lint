mod dead_scripts;
mod pwd;
pub(crate) mod scripts;
mod security_md;
mod todo;

pub use dead_scripts::validate_dead_scripts;
pub use pwd::validate_pwd_hygiene;
pub use scripts::collect_script_paths;
pub use scripts::validate_executability;
pub use scripts::validate_private_executability;
pub use scripts::validate_private_script_references;
pub use scripts::validate_script_references;
pub use security_md::validate_security_md;
pub use todo::validate_todo_in_agents;
pub use todo::validate_todo_in_skills;

#[cfg(test)]
mod tests {
    use super::scripts::expand_script_dirs;
    use super::*;
    use crate::context::LintMode;

    // V8: validate_pwd_hygiene
    #[test]
    #[serial_test::serial]
    fn test_v8_clean_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: s\n---\nUses ${CLAUDE_PLUGIN_ROOT}/scripts/foo.sh\n",
        )
        .unwrap();

        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_pwd_hygiene(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v8_pwd_violation() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\n---\nRun $PWD/scripts/foo.sh\n",
        )
        .unwrap();

        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_pwd_hygiene(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("$PWD"));
    }

    #[test]
    #[serial_test::serial]
    fn test_v8_hardcoded_path_violation() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\n---\nPath /Users/somebody/code\n",
        )
        .unwrap();

        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_pwd_hygiene(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("hardcoded path"));
    }

    // V10: validate_executability
    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn test_v10_executable_script() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("scripts").unwrap();
        let script = tmp.path().join("scripts/test.sh");
        std::fs::write(&script, "#!/bin/bash\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_executability(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn test_v10_non_executable_script() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("scripts").unwrap();
        let script = tmp.path().join("scripts/test.sh");
        std::fs::write(&script, "#!/bin/bash\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o644)).unwrap();

        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_executability(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("not executable"));
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn test_v10a_private_executable() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/skills/my-skill/scripts").unwrap();
        let script = tmp.path().join(".claude/skills/my-skill/scripts/helper.sh");
        std::fs::write(&script, "#!/bin/bash\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_private_executability(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn test_v10a_private_non_executable() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/skills/my-skill/scripts").unwrap();
        let script = tmp.path().join(".claude/skills/my-skill/scripts/helper.sh");
        std::fs::write(&script, "#!/bin/bash\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o644)).unwrap();

        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_private_executability(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("not executable"));
    }

    // V14: validate_security_md
    #[test]
    #[serial_test::serial]
    fn test_v14_security_md_present() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::write("SECURITY.md", "# Security Policy\n").unwrap();

        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_security_md(&mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v14_security_md_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_security_md(&mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("SECURITY.md"));
    }

    // V9: validate_script_references
    #[test]
    #[serial_test::serial]
    fn test_v9_valid_reference() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("scripts").unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write("scripts/helper.sh", "#!/bin/bash\n").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\n---\nRun ${CLAUDE_PLUGIN_ROOT}/scripts/helper.sh\n",
        )
        .unwrap();

        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_script_references(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v9_missing_reference() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\n---\nRun ${CLAUDE_PLUGIN_ROOT}/scripts/nonexistent.sh\n",
        )
        .unwrap();

        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_script_references(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("missing on disk"));
    }

    #[test]
    #[serial_test::serial]
    fn test_v9a_valid_private_reference() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/skills/my-skill/scripts").unwrap();
        std::fs::write(".claude/skills/my-skill/scripts/run.sh", "#!/bin/bash\n").unwrap();
        std::fs::write(
            ".claude/skills/my-skill/SKILL.md",
            "---\nname: my-skill\n---\nRun $PWD/.claude/skills/my-skill/scripts/run.sh\n",
        )
        .unwrap();

        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_private_script_references(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v9a_missing_private_reference() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/skills/my-skill").unwrap();
        std::fs::write(
            ".claude/skills/my-skill/SKILL.md",
            "---\nname: my-skill\n---\nRun $PWD/.claude/skills/my-skill/scripts/missing.sh\n",
        )
        .unwrap();

        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_private_script_references(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("missing on disk"));
    }

    // V11: validate_dead_scripts
    #[test]
    #[serial_test::serial]
    fn test_v11_referenced_script_not_dead() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("scripts").unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write("scripts/used.sh", "#!/bin/bash\n").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\n---\nRun ${CLAUDE_PLUGIN_ROOT}/scripts/used.sh\n",
        )
        .unwrap();

        let ctx = crate::context::LintContext {
            base_path: tmp.path().to_path_buf(),
            mode: crate::context::LintMode::Plugin,
            plugin_json: crate::context::ManifestState::Missing,
            marketplace_json: crate::context::ManifestState::Missing,
            hooks_json: crate::context::ManifestState::Missing,
            settings_json: crate::context::ManifestState::Missing,
        };
        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_dead_scripts(&ctx, &mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v11_unreferenced_dead_script() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("scripts").unwrap();
        std::fs::write("scripts/orphan.sh", "#!/bin/bash\n").unwrap();

        let ctx = crate::context::LintContext {
            base_path: tmp.path().to_path_buf(),
            mode: crate::context::LintMode::Plugin,
            plugin_json: crate::context::ManifestState::Missing,
            marketplace_json: crate::context::ManifestState::Missing,
            hooks_json: crate::context::ManifestState::Missing,
            settings_json: crate::context::ManifestState::Missing,
        };
        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_dead_scripts(&ctx, &mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("dead script"));
    }

    #[test]
    #[serial_test::serial]
    fn test_v11_script_referenced_in_hooks_json_not_dead() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("scripts").unwrap();
        std::fs::write("scripts/referenced.sh", "#!/bin/bash\n").unwrap();

        let hooks_val = serde_json::json!({
            "hooks": [{"command": "${CLAUDE_PLUGIN_ROOT}/scripts/referenced.sh"}]
        });
        let ctx = crate::context::LintContext {
            base_path: tmp.path().to_path_buf(),
            mode: crate::context::LintMode::Plugin,
            plugin_json: crate::context::ManifestState::Missing,
            marketplace_json: crate::context::ManifestState::Missing,
            hooks_json: crate::context::ManifestState::Parsed(hooks_val),
            settings_json: crate::context::ManifestState::Missing,
        };
        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_dead_scripts(&ctx, &mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(
            diag.error_count(),
            0,
            "Script referenced in hooks.json should not be reported as dead"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_v11_script_referenced_in_settings_json_not_dead() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("scripts").unwrap();
        std::fs::write("scripts/setup.sh", "#!/bin/bash\n").unwrap();

        let settings_val = serde_json::json!({
            "permissions": {"allow": ["scripts/setup.sh"]}
        });
        let ctx = crate::context::LintContext {
            base_path: tmp.path().to_path_buf(),
            mode: crate::context::LintMode::Plugin,
            plugin_json: crate::context::ManifestState::Missing,
            marketplace_json: crate::context::ManifestState::Missing,
            hooks_json: crate::context::ManifestState::Missing,
            settings_json: crate::context::ManifestState::Parsed(settings_val),
        };
        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_dead_scripts(&ctx, &mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(
            diag.error_count(),
            0,
            "Script referenced in settings.json should not be reported as dead"
        );
    }

    // expand_script_dirs tests
    #[test]
    #[serial_test::serial]
    fn test_expand_script_dirs_plain_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("scripts").unwrap();
        let dirs = expand_script_dirs(&["scripts"]);
        assert_eq!(dirs.len(), 1);
    }

    #[test]
    #[serial_test::serial]
    fn test_expand_script_dirs_glob() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/a/scripts").unwrap();
        std::fs::create_dir_all("skills/b/scripts").unwrap();
        let mut dirs = expand_script_dirs(&["skills/*/scripts"]);
        dirs.sort();
        assert_eq!(dirs.len(), 2);
    }

    #[test]
    #[serial_test::serial]
    fn test_expand_script_dirs_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        let dirs = expand_script_dirs(&["nonexistent"]);
        assert!(dirs.is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn test_expand_script_dirs_multi_glob() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/a/nested/x/scripts").unwrap();
        std::fs::create_dir_all("skills/b/nested/y/scripts").unwrap();
        std::fs::create_dir_all("skills/c/other/z/scripts").unwrap();

        let mut dirs = expand_script_dirs(&["skills/*/nested/*/scripts"]);
        dirs.sort();
        assert_eq!(dirs.len(), 2);
        assert!(dirs[0].ends_with("skills/a/nested/x/scripts"));
        assert!(dirs[1].ends_with("skills/b/nested/y/scripts"));
    }

    #[test]
    #[serial_test::serial]
    fn test_expand_script_dirs_glob_nonexistent_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        let dirs = expand_script_dirs(&["nonexistent/*/scripts"]);
        assert!(dirs.is_empty());
    }

    // collect_script_paths tests
    #[test]
    #[serial_test::serial]
    fn test_collect_script_paths_basic_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/skills/my-skill/scripts").unwrap();
        std::fs::write(".claude/skills/my-skill/scripts/run.sh", "#!/bin/bash\n").unwrap();
        std::fs::write(".claude/skills/my-skill/scripts/helper.sh", "#!/bin/bash\n").unwrap();
        std::fs::write(".claude/skills/my-skill/scripts/readme.txt", "text\n").unwrap();

        let paths = collect_script_paths(LintMode::Basic, &crate::config::ExcludeSet::default());
        assert_eq!(paths.len(), 2);
        assert!(paths[0].ends_with("helper.sh"));
        assert!(paths[1].ends_with("run.sh"));
    }

    #[test]
    #[serial_test::serial]
    fn test_collect_script_paths_plugin_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("scripts").unwrap();
        std::fs::create_dir_all("skills/foo/scripts").unwrap();
        std::fs::create_dir_all(".claude/skills/bar/scripts").unwrap();
        std::fs::write("scripts/install.sh", "#!/bin/bash\n").unwrap();
        std::fs::write("skills/foo/scripts/build.sh", "#!/bin/bash\n").unwrap();
        std::fs::write(".claude/skills/bar/scripts/run.sh", "#!/bin/bash\n").unwrap();

        let paths = collect_script_paths(LintMode::Plugin, &crate::config::ExcludeSet::default());
        assert_eq!(paths.len(), 3);
        assert!(paths.iter().any(|p| p.ends_with("install.sh")));
        assert!(paths.iter().any(|p| p.ends_with("build.sh")));
        assert!(paths.iter().any(|p| p.ends_with("run.sh")));
    }

    #[test]
    #[serial_test::serial]
    fn test_collect_script_paths_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        let paths = collect_script_paths(LintMode::Basic, &crate::config::ExcludeSet::default());
        assert!(paths.is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn test_collect_script_paths_basic_excludes_top_level_scripts() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("scripts").unwrap();
        std::fs::write("scripts/install.sh", "#!/bin/bash\n").unwrap();

        let paths = collect_script_paths(LintMode::Basic, &crate::config::ExcludeSet::default());
        assert!(paths.is_empty());
    }

    // G006: todo-in-skill
    #[test]
    #[serial_test::serial]
    fn test_g006_todo_in_skill_body() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: desc\n---\nTODO: implement this\n",
        )
        .unwrap();
        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_todo_in_skills(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("TODO")));
    }

    #[test]
    #[serial_test::serial]
    fn test_g006_todo_in_code_fence_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: desc\n---\n\n```bash\n# TODO: this is fine\n```\n",
        )
        .unwrap();
        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_todo_in_skills(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("TODO")));
    }

    #[test]
    #[serial_test::serial]
    fn test_g006_todo_in_nested_fence_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: desc\n---\n\n````\n```\n# TODO: nested\n```\n````\n",
        )
        .unwrap();
        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_todo_in_skills(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag.errors().iter().any(|e| e.contains("TODO")),
            "TODO inside nested 4-backtick fence should not trigger G006"
        );
    }

    // G007: todo-in-agent
    #[test]
    #[serial_test::serial]
    fn test_g007_todo_in_agent_body() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::write(
            "agents/general.md",
            "---\nname: general\ndescription: desc\n---\nFIXME: this needs work\n",
        )
        .unwrap();
        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_todo_in_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("FIXME")));
    }

    #[test]
    #[serial_test::serial]
    fn test_g007_todo_in_code_fence_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::write(
            "agents/general.md",
            "---\nname: general\ndescription: desc\n---\n\n```\n# FIXME: inside fence\n```\n",
        )
        .unwrap();
        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_todo_in_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("FIXME")));
    }

    #[test]
    #[serial_test::serial]
    fn test_g007_todo_in_nested_fence_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::write(
            "agents/general.md",
            "---\nname: general\ndescription: desc\n---\n\n````\n```\n# FIXME: nested\n```\n````\n",
        )
        .unwrap();
        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_todo_in_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag.errors().iter().any(|e| e.contains("FIXME")),
            "FIXME inside nested 4-backtick fence should not trigger G007"
        );
    }
}
