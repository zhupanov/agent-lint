mod dead_scripts;
mod pwd;
pub(crate) mod scripts;
mod security_md;
mod todo;

pub use dead_scripts::validate_dead_scripts;
pub use pwd::validate_pwd_hygiene;
pub use scripts::collect_script_paths;
#[cfg(test)]
pub use scripts::validate_executability;
#[cfg(test)]
pub use scripts::validate_private_executability;
#[cfg(test)]
pub use scripts::validate_private_script_references;
#[cfg(test)]
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
        assert_eq!(
            diag.diagnostics()[0].rule,
            crate::rules::LintRule::HardcodedMachinePath
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_v8_reports_rule_specific_location_evidence_and_suggestion() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::create_dir_all("scripts").unwrap();
        std::fs::write("scripts/check.sh", "#!/bin/sh\n").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\n---\nRun $PWD/scripts/check.sh.\nRead $PWD/package.json.\n",
        )
        .unwrap();

        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_pwd_hygiene(&mut diag, &crate::config::ExcludeSet::default());

        assert_eq!(diag.diagnostics().len(), 2);
        let bundled = &diag.diagnostics()[0];
        assert_eq!(bundled.rule, crate::rules::LintRule::PwdInSkill);
        assert_eq!(
            bundled.subject_path.as_deref(),
            Some(std::path::Path::new("skills/my-skill/SKILL.md"))
        );
        assert_eq!(bundled.location.unwrap().start().line_number(), 4);
        assert_eq!(bundled.location.unwrap().start().column_number(), Some(5));
        assert_eq!(bundled.evidence.as_deref(), Some("$PWD/scripts/check.sh"));
        assert!(
            bundled
                .suggestion
                .as_deref()
                .unwrap()
                .contains("CLAUDE_PLUGIN_ROOT")
        );

        let ambiguous = &diag.diagnostics()[1];
        assert_eq!(ambiguous.rule, crate::rules::LintRule::HardcodedMachinePath);
        assert_eq!(ambiguous.location.unwrap().start().line_number(), 5);
        assert_eq!(ambiguous.location.unwrap().start().column_number(), Some(6));
        assert_eq!(ambiguous.evidence.as_deref(), Some("$PWD/package.json"));
        assert!(
            ambiguous
                .suggestion
                .as_deref()
                .unwrap()
                .contains("CLAUDE_PROJECT_DIR")
        );
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
        std::fs::create_dir_all("skills/example").unwrap();
        std::fs::write(
            "skills/example/SKILL.md",
            "Run ${CLAUDE_PLUGIN_ROOT}/scripts/test.sh\n",
        )
        .unwrap();
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
        std::fs::create_dir_all("skills/example").unwrap();
        std::fs::write(
            "skills/example/SKILL.md",
            "Run ${CLAUDE_PLUGIN_ROOT}/scripts/test.sh\n",
        )
        .unwrap();
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
        std::fs::write(
            ".claude/skills/my-skill/SKILL.md",
            "Run $PWD/.claude/skills/my-skill/scripts/helper.sh\n",
        )
        .unwrap();
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
        std::fs::write(
            ".claude/skills/my-skill/SKILL.md",
            "Run $PWD/.claude/skills/my-skill/scripts/helper.sh\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o644)).unwrap();

        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_private_executability(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("not executable"));
    }

    // G005 (V14): validate_security_md
    #[test]
    #[serial_test::serial]
    fn test_g005_security_md_present_at_root() {
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
    fn test_g005_security_md_present_in_github() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".github").unwrap();
        std::fs::write(".github/SECURITY.md", "# Security Policy\n").unwrap();

        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_security_md(&mut diag);
        assert_eq!(
            diag.error_count(),
            0,
            "a .github/SECURITY.md policy must satisfy G005"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_g005_security_md_present_in_docs() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("docs").unwrap();
        std::fs::write("docs/SECURITY.md", "# Security Policy\n").unwrap();

        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_security_md(&mut diag);
        assert_eq!(
            diag.error_count(),
            0,
            "a docs/SECURITY.md policy must satisfy G005"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_g005_security_md_multiple_locations_satisfy() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".github").unwrap();
        std::fs::create_dir_all("docs").unwrap();
        std::fs::write("SECURITY.md", "# Security Policy\n").unwrap();
        std::fs::write(".github/SECURITY.md", "# Security Policy\n").unwrap();
        std::fs::write("docs/SECURITY.md", "# Security Policy\n").unwrap();

        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_security_md(&mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_g005_security_md_missing_reports_with_suggestion() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_security_md(&mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("SECURITY.md"));

        let diagnostic = &diag.diagnostics()[0];
        assert_eq!(
            diagnostic.subject_path.as_deref(),
            Some(std::path::Path::new("SECURITY.md")),
            "the absent-resource subject is the logical root SECURITY.md"
        );
        let suggestion = diagnostic
            .suggestion
            .as_deref()
            .expect("G005 provides an actionable suggestion");
        for accepted in [".github/", "docs/"] {
            assert!(
                suggestion.contains(accepted),
                "suggestion must name the {accepted} location: {suggestion}"
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_g005_directory_named_security_md_does_not_satisfy() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        // A directory named SECURITY.md is not a committed policy file.
        std::fs::create_dir_all("SECURITY.md").unwrap();

        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_security_md(&mut diag);
        assert_eq!(
            diag.error_count(),
            1,
            "a directory named SECURITY.md must not satisfy G005"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_g005_wrong_case_does_not_satisfy() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        // Wrong-case name; GitHub requires the exact SECURITY.md spelling.
        std::fs::write("security.md", "# Security Policy\n").unwrap();

        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_security_md(&mut diag);
        assert_eq!(
            diag.error_count(),
            1,
            "a wrong-case security.md must not satisfy G005 even on a \
             case-insensitive filesystem"
        );
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn test_g005_symlink_does_not_satisfy() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        // A symlink to a real file is not a committed policy file.
        std::fs::write("policy.md", "# Security Policy\n").unwrap();
        std::os::unix::fs::symlink("policy.md", "SECURITY.md").unwrap();

        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_security_md(&mut diag);
        assert_eq!(
            diag.error_count(),
            1,
            "a symlinked SECURITY.md must not satisfy G005"
        );
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn test_g005_dangling_symlink_does_not_satisfy() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        // A dangling symlink (target absent) is rejected without traversal, so
        // rejection does not depend on the symlink target existing.
        std::os::unix::fs::symlink("nonexistent-target.md", "SECURITY.md").unwrap();

        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_security_md(&mut diag);
        assert_eq!(
            diag.error_count(),
            1,
            "a dangling symlinked SECURITY.md must not satisfy G005"
        );
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
            declared_hook_configs: vec![],
            settings_json: crate::context::ManifestState::Missing,
            settings_local_json: crate::context::ManifestState::Missing,
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
            declared_hook_configs: vec![],
            settings_json: crate::context::ManifestState::Missing,
            settings_local_json: crate::context::ManifestState::Missing,
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
            hooks_json: crate::context::ManifestState::parsed(hooks_val),
            declared_hook_configs: vec![],
            settings_json: crate::context::ManifestState::Missing,
            settings_local_json: crate::context::ManifestState::Missing,
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
            declared_hook_configs: vec![],
            settings_json: crate::context::ManifestState::parsed(settings_val),
            settings_local_json: crate::context::ManifestState::Missing,
        };
        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_dead_scripts(&ctx, &mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(
            diag.error_count(),
            0,
            "Script referenced in settings.json should not be reported as dead"
        );
    }

    // G004: Makefile references count as live invocations (not dead)
    #[test]
    #[serial_test::serial]
    fn test_v11_makefile_bare_reference_not_dead() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("scripts").unwrap();
        std::fs::write("scripts/used.sh", "#!/bin/bash\n").unwrap();
        std::fs::write("Makefile", "lint:\n\tbash scripts/used.sh\n").unwrap();

        let ctx = crate::context::LintContext {
            base_path: tmp.path().to_path_buf(),
            mode: crate::context::LintMode::Plugin,
            plugin_json: crate::context::ManifestState::Missing,
            marketplace_json: crate::context::ManifestState::Missing,
            hooks_json: crate::context::ManifestState::Missing,
            declared_hook_configs: vec![],
            settings_json: crate::context::ManifestState::Missing,
            settings_local_json: crate::context::ManifestState::Missing,
        };
        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_dead_scripts(&ctx, &mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(
            diag.error_count(),
            0,
            "Script invoked from Makefile should not be reported as dead"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_v11_makefile_qualified_reference_not_dead() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("scripts").unwrap();
        std::fs::write("scripts/used.sh", "#!/bin/bash\n").unwrap();
        std::fs::write(
            "Makefile",
            "lint:\n\t${CLAUDE_PLUGIN_ROOT}/scripts/used.sh\n",
        )
        .unwrap();

        let ctx = crate::context::LintContext {
            base_path: tmp.path().to_path_buf(),
            mode: crate::context::LintMode::Plugin,
            plugin_json: crate::context::ManifestState::Missing,
            marketplace_json: crate::context::ManifestState::Missing,
            hooks_json: crate::context::ManifestState::Missing,
            declared_hook_configs: vec![],
            settings_json: crate::context::ManifestState::Missing,
            settings_local_json: crate::context::ManifestState::Missing,
        };
        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_dead_scripts(&ctx, &mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v11_makefile_commented_reference_still_dead() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("scripts").unwrap();
        std::fs::write("scripts/orphan.sh", "#!/bin/bash\n").unwrap();
        // Reference lives only in a comment, which must be stripped.
        std::fs::write("Makefile", "# bash scripts/orphan.sh\nlint:\n\techo hi\n").unwrap();

        let ctx = crate::context::LintContext {
            base_path: tmp.path().to_path_buf(),
            mode: crate::context::LintMode::Plugin,
            plugin_json: crate::context::ManifestState::Missing,
            marketplace_json: crate::context::ManifestState::Missing,
            hooks_json: crate::context::ManifestState::Missing,
            declared_hook_configs: vec![],
            settings_json: crate::context::ManifestState::Missing,
            settings_local_json: crate::context::ManifestState::Missing,
        };
        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_dead_scripts(&ctx, &mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(
            diag.error_count(),
            1,
            "Script only referenced in a Makefile comment should still be dead"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_v11_mk_file_reference_not_dead() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("scripts").unwrap();
        std::fs::write("scripts/used.sh", "#!/bin/bash\n").unwrap();
        std::fs::write("tools.mk", "lint:\n\tbash scripts/used.sh\n").unwrap();

        let ctx = crate::context::LintContext {
            base_path: tmp.path().to_path_buf(),
            mode: crate::context::LintMode::Plugin,
            plugin_json: crate::context::ManifestState::Missing,
            marketplace_json: crate::context::ManifestState::Missing,
            hooks_json: crate::context::ManifestState::Missing,
            declared_hook_configs: vec![],
            settings_json: crate::context::ManifestState::Missing,
            settings_local_json: crate::context::ManifestState::Missing,
        };
        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_dead_scripts(&ctx, &mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    // V9: Makefile references validated for existence
    #[test]
    #[serial_test::serial]
    fn test_v9_makefile_bare_reference_present() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("scripts").unwrap();
        std::fs::write("scripts/helper.sh", "#!/bin/bash\n").unwrap();
        std::fs::write("Makefile", "lint:\n\tbash scripts/helper.sh\n").unwrap();

        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_script_references(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v9_makefile_bare_reference_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::write("Makefile", "lint:\n\tbash scripts/ghost.sh\n").unwrap();

        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_script_references(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("missing on disk"));
    }

    #[test]
    #[serial_test::serial]
    fn test_v9_makefile_qualified_reference_present() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("scripts").unwrap();
        std::fs::write("scripts/helper.sh", "#!/bin/bash\n").unwrap();
        std::fs::write(
            "Makefile",
            "lint:\n\t${CLAUDE_PLUGIN_ROOT}/scripts/helper.sh\n",
        )
        .unwrap();

        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_script_references(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn g002_suppression_is_scoped_to_each_source_file() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        for skill in ["a", "b"] {
            std::fs::create_dir_all(format!("skills/{skill}")).unwrap();
            std::fs::write(
                format!("skills/{skill}/SKILL.md"),
                "Run ${CLAUDE_PLUGIN_ROOT}/scripts/missing.py\n",
            )
            .unwrap();
        }
        std::fs::write(
            "agent-lint.toml",
            "[[lint.overrides]]\nfiles = [\"skills/a/SKILL.md\"]\nsuppress = [\"G002\"]\nreason = \"intentional first occurrence\"\n",
        )
        .unwrap();

        let config = crate::config::LintConfig::load(".").unwrap();
        let mut diag = crate::diagnostic::DiagnosticCollector::with_config(config);
        validate_script_references(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.suppressed_count(), 1);
        assert_eq!(diag.error_count(), 1);
        assert_eq!(
            diag.diagnostics()[0].subject_path.as_deref(),
            Some(std::path::Path::new("skills/b/SKILL.md"))
        );
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn g003_only_requires_directly_executed_non_shell_scripts() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("scripts").unwrap();
        std::fs::create_dir_all("skills/example").unwrap();
        std::fs::write("scripts/direct.py", "#!/usr/bin/env python3\n").unwrap();
        std::fs::write("scripts/interpreted.py", "print('ok')\n").unwrap();
        for path in ["scripts/direct.py", "scripts/interpreted.py"] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        std::fs::write(
            "skills/example/SKILL.md",
            "Run \"${CLAUDE_PROJECT_DIR}\"/scripts/direct.py\nRun python3 ${CLAUDE_PLUGIN_ROOT}/scripts/interpreted.py\n",
        )
        .unwrap();

        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_executability(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert_eq!(
            diag.diagnostics()[0].subject_path.as_deref(),
            Some(std::path::Path::new("scripts/direct.py"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn g004_agent_invocation_is_live_but_comments_and_self_references_are_not() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("scripts").unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::write("scripts/agent-live.sh", "#!/bin/sh\n").unwrap();
        std::fs::write(
            "scripts/comment-dead.sh",
            "# ${CLAUDE_PLUGIN_ROOT}/scripts/comment-dead.sh\n",
        )
        .unwrap();
        std::fs::write(
            "agents/reviewer.md",
            "Run ${CLAUDE_PLUGIN_ROOT}/scripts/agent-live.sh\n",
        )
        .unwrap();
        let ctx = crate::context::LintContext::new(tmp.path(), LintMode::Plugin);
        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_dead_scripts(&ctx, &mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert_eq!(
            diag.diagnostics()[0].subject_path.as_deref(),
            Some(std::path::Path::new("scripts/comment-dead.sh"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn g002_accepts_directories_and_reports_source_locations() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("scripts/nested").unwrap();
        std::fs::create_dir_all("skills/example").unwrap();
        std::fs::write(
            "skills/example/SKILL.md",
            "Run ${CLAUDE_PLUGIN_ROOT}/scripts/\nRun ${CLAUDE_PLUGIN_ROOT}/scripts/missing.sh\n",
        )
        .unwrap();

        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_script_references(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        let finding = &diag.diagnostics()[0];
        assert_eq!(
            finding.location,
            Some(crate::diagnostic::SourceSpan::line(2))
        );
        assert_eq!(
            finding.evidence.as_deref(),
            Some("${CLAUDE_PLUGIN_ROOT}/scripts/missing.sh")
        );
    }

    #[test]
    #[serial_test::serial]
    fn g002_keeps_distinct_escaping_references() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/example").unwrap();
        std::fs::write(
            "skills/example/SKILL.md",
            "Run ${CLAUDE_PLUGIN_ROOT}/../first.sh\nRun ${CLAUDE_PLUGIN_ROOT}/../../second.sh\n",
        )
        .unwrap();

        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_script_references(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 2);
        assert_eq!(
            diag.diagnostics()[0].location,
            Some(crate::diagnostic::SourceSpan::line(1))
        );
        assert_eq!(
            diag.diagnostics()[1].location,
            Some(crate::diagnostic::SourceSpan::line(2))
        );
    }

    #[test]
    #[serial_test::serial]
    fn g002_expands_supported_globs_and_skips_unsupported_ones() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("scripts").unwrap();
        std::fs::create_dir_all("skills/example").unwrap();
        std::fs::write("scripts/a.sh", "#!/bin/sh\n").unwrap();
        std::fs::write("scripts/b.sh", "#!/bin/sh\n").unwrap();
        std::fs::write(
            "skills/example/SKILL.md",
            "Run bash ${CLAUDE_PLUGIN_ROOT}/scripts/*.sh\nRun ${CLAUDE_PLUGIN_ROOT}/scripts/nope*.sh\nRun ${CLAUDE_PLUGIN_ROOT}/scripts/[ab].sh\n",
        )
        .unwrap();

        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_script_references(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert_eq!(
            diag.diagnostics()[0].evidence.as_deref(),
            Some("${CLAUDE_PLUGIN_ROOT}/scripts/nope*.sh")
        );

        let ctx = crate::context::LintContext::new(tmp.path(), LintMode::Plugin);
        let mut dead = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_dead_scripts(&ctx, &mut dead, &crate::config::ExcludeSet::default());
        assert_eq!(
            dead.error_count(),
            0,
            "matched glob invocation keeps every match live"
        );
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn g003_checks_each_direct_glob_match_and_env_prefixed_invocation() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("scripts").unwrap();
        std::fs::create_dir_all("skills/example").unwrap();
        for path in ["scripts/a.sh", "scripts/b.sh", "scripts/c.sh"] {
            std::fs::write(path, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        std::fs::write(
            "skills/example/SKILL.md",
            "Run ${CLAUDE_PLUGIN_ROOT}/scripts/*.sh\nRun FOO=1 ${CLAUDE_PLUGIN_ROOT}/scripts/c.sh\n",
        )
        .unwrap();

        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_executability(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 3);
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn yaml_only_treats_run_and_block_scalar_lines_as_commands() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("scripts").unwrap();
        std::fs::create_dir_all(".github/workflows").unwrap();
        for path in ["scripts/run.sh", "scripts/block.sh"] {
            std::fs::write(path, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        std::fs::write(
            ".github/workflows/ci.yml",
            "path: ${CLAUDE_PLUGIN_ROOT}/scripts/artifact.sh\nwith: ${CLAUDE_PLUGIN_ROOT}/scripts/also-artifact.sh\nrun: ${CLAUDE_PLUGIN_ROOT}/scripts/run.sh\nrun: |\n  ${CLAUDE_PLUGIN_ROOT}/scripts/block.sh\n",
        )
        .unwrap();

        let mut references = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_script_references(&mut references, &crate::config::ExcludeSet::default());
        assert_eq!(references.error_count(), 0);

        let mut executable = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_executability(&mut executable, &crate::config::ExcludeSet::default());
        assert_eq!(executable.error_count(), 2);
    }

    #[test]
    #[serial_test::serial]
    fn commands_are_script_reference_surfaces_in_public_and_private_modes() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("scripts").unwrap();
        std::fs::create_dir_all("commands").unwrap();
        std::fs::create_dir_all(".claude/commands").unwrap();
        std::fs::write("scripts/live.sh", "#!/bin/sh\n").unwrap();
        std::fs::write("commands/deploy.md", "Run ${CLAUDE_PLUGIN_ROOT}/scripts/live.sh\nRun ${CLAUDE_PLUGIN_ROOT}/scripts/missing.sh\n").unwrap();
        std::fs::write(
            ".claude/commands/private.md",
            "Run ${CLAUDE_PLUGIN_ROOT}/scripts/private-missing.sh\n",
        )
        .unwrap();

        let mut plugin = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_script_references(&mut plugin, &crate::config::ExcludeSet::default());
        assert_eq!(plugin.error_count(), 2);
        assert!(
            plugin
                .diagnostics()
                .iter()
                .any(|finding| finding.subject_path.as_deref()
                    == Some(std::path::Path::new("commands/deploy.md")))
        );

        let mut private = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_private_script_references(&mut private, &crate::config::ExcludeSet::default());
        assert_eq!(private.error_count(), 1);
        assert_eq!(
            private.diagnostics()[0].subject_path.as_deref(),
            Some(std::path::Path::new(".claude/commands/private.md"))
        );

        let ctx = crate::context::LintContext::new(tmp.path(), LintMode::Plugin);
        let mut dead = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_dead_scripts(&ctx, &mut dead, &crate::config::ExcludeSet::default());
        assert_eq!(dead.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn permission_rule_forms_mark_scripts_live() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("scripts").unwrap();
        std::fs::create_dir_all(".claude").unwrap();
        for path in [
            "scripts/wildcard.sh",
            "scripts/argument.sh",
            "scripts/bare.sh",
        ] {
            std::fs::write(path, "#!/bin/sh\n").unwrap();
        }
        std::fs::write(
            ".claude/settings.json",
            r#"{"permissions":{"allow":["Bash(scripts/wildcard.sh:*)","Bash(scripts/argument.sh --flag)","scripts/bare.sh"],"deny":["Bash(scripts/denied.sh:*)"]}}"#,
        )
        .unwrap();

        let ctx = crate::context::LintContext::new(tmp.path(), LintMode::Plugin);
        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_dead_scripts(&ctx, &mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn collect_script_paths_uses_the_shared_script_kind_matrix() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("scripts").unwrap();
        for path in [
            "shell.sh",
            "shell.bash",
            "library.inc.bash",
            "rules.awk",
            "tool.py",
            "tool.js",
            "tool.mjs",
            "extensionless",
            "readme.txt",
        ] {
            std::fs::write(format!("scripts/{path}"), "content\n").unwrap();
        }
        let paths = collect_script_paths(LintMode::Plugin, &crate::config::ExcludeSet::default());
        assert_eq!(paths.len(), 8);
        assert!(paths.iter().any(|path| path.ends_with("shell.bash")));
        assert!(paths.iter().any(|path| path.ends_with("library.inc.bash")));
        assert!(paths.iter().any(|path| path.ends_with("rules.awk")));
        assert!(!paths.iter().any(|path| path.ends_with("readme.txt")));
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

    #[test]
    #[serial_test::serial]
    fn test_g006_prose_about_todo_is_clean() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: desc\n---\nRemove any TODO or FIXME markers from generated output before returning it.\nDo not hack around the permission system.\n",
        )
        .unwrap();
        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_todo_in_skills(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_g007_lowercase_xxx_prose_is_clean() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::write(
            "agents/general.md",
            "---\nname: general\ndescription: desc\n---\nNever use xxx as a placeholder.\nReject output containing TODO, FIXME, HACK, or XXX markers.\n",
        )
        .unwrap();
        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_todo_in_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_g006_reports_structured_location_once() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: desc\n---\nIntro\n- [ ] FIXME: first\nTODO: second\n",
        )
        .unwrap();
        let mut diag = crate::diagnostic::DiagnosticCollector::new_all_enabled();
        validate_todo_in_skills(&mut diag, &crate::config::ExcludeSet::default());
        let findings: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|d| d.rule == crate::rules::LintRule::TodoInSkill)
            .collect();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].evidence.as_deref(), Some("FIXME"));
        assert_eq!(
            findings[0].location,
            Some(crate::diagnostic::SourceSpan::range(6, 7, 6, 12))
        );
        assert_eq!(
            findings[0].suggestion.as_deref(),
            Some(crate::unfinished_work::UNFINISHED_WORK_SUGGESTION)
        );
    }
}
