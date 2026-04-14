mod agents;
mod common;
mod docs;
mod email;
mod hooks;
pub mod hygiene;
mod manifest;
mod skill_content;
pub(crate) mod skills;
mod slack;
mod user_config;
mod walk;

use crate::config::ExcludeSet;
use crate::context::{LintContext, LintMode};
use crate::diagnostic::DiagnosticCollector;

/// Run all validators appropriate for the current lint mode.
pub fn run_all(ctx: &LintContext, diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    match ctx.mode {
        LintMode::Basic => run_basic(ctx, diag, exclude),
        LintMode::Plugin => run_plugin(ctx, diag, exclude),
    }
}

/// Basic mode: validate .claude/ contents only.
fn run_basic(ctx: &LintContext, diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    // V4: settings.json hook paths
    hooks::validate_settings_hooks(ctx, diag);
    // V6-adapted: private SKILL.md frontmatter for .claude/skills/
    skills::validate_private_skill_frontmatter(diag, exclude);
    // V9-adapted: script ref integrity for $PWD/.claude/skills/ refs
    hygiene::validate_private_script_references(diag, exclude);
    // V10-adapted: executability for .claude/skills/*/scripts/*.sh
    hygiene::validate_private_executability(diag, exclude);
    // Skill content checks (both-mode subset: excludes S015, S016, S017, S029, S033)
    skill_content::validate_private_skill_content(diag, exclude);
}

/// Plugin mode: run all validators plus `.claude/` checks.
fn run_plugin(ctx: &LintContext, diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    // Private .claude/ validators (also run in basic mode)
    skills::validate_private_skill_frontmatter(diag, exclude);
    hygiene::validate_private_script_references(diag, exclude);
    hygiene::validate_private_executability(diag, exclude);

    // V1: plugin.json
    manifest::validate_plugin_json(ctx, diag);
    // V2: marketplace.json
    manifest::validate_marketplace_json(ctx, diag);
    // V3: hooks/hooks.json
    hooks::validate_hooks_json(ctx, diag);
    // V4: settings.json hook paths
    hooks::validate_settings_hooks(ctx, diag);
    // V5: skills layout
    skills::validate_skills_layout(diag, exclude);
    // V6: SKILL.md frontmatter (public)
    skills::validate_skill_frontmatter(diag, exclude);
    // V7: agents frontmatter
    agents::validate_agents(diag, exclude);
    // V8: PWD hygiene
    hygiene::validate_pwd_hygiene(diag, exclude);
    // V9: script reference integrity
    hygiene::validate_script_references(diag, exclude);
    // V10: executability (generic, no hardcoded block-submodule-edit.sh)
    hygiene::validate_executability(diag, exclude);
    // V11: dead-script detection
    hygiene::validate_dead_scripts(ctx, diag, exclude);
    // V12: marketplace enriched metadata
    manifest::validate_marketplace_enriched(ctx, diag);
    // V13: plugin enriched metadata
    manifest::validate_plugin_enriched(ctx, diag);
    // V14: SECURITY.md presence
    hygiene::validate_security_md(diag);
    // V15: shared markdown reference integrity
    skills::validate_shared_md_references(diag, exclude);
    // V16: agent-template alignment
    agents::validate_agent_template_alignment(diag, exclude);
    // V17: email format
    email::validate_email_format(ctx, diag);
    // V18: userConfig structure
    user_config::validate_userconfig_structure(ctx, diag);
    // V19: Slack fallback consistency (larch-specific convention)
    slack::validate_slack_fallback_consistency(diag, exclude);
    // V20: userConfig→env mapping
    user_config::validate_userconfig_env_mapping(ctx, diag);
    // V21: agent-template count
    agents::validate_agent_template_count(diag, exclude);
    // V22: docs file references
    docs::validate_docs_references(diag, exclude);
    // V23: userConfig sensitive type
    user_config::validate_userconfig_sensitive_type(ctx, diag);
    // V24: userConfig title field
    user_config::validate_userconfig_title(ctx, diag);
    // V25: userConfig type field
    user_config::validate_userconfig_type(ctx, diag);
    // Skill content checks (all 26 rules including plugin-only)
    skill_content::validate_skill_content(diag, exclude);
    // Private skill content checks (both-mode subset)
    skill_content::validate_private_skill_content(diag, exclude);
    // D002: CLAUDE.md size
    docs::validate_claudemd_size(diag, exclude);
    // D003: TODO/FIXME in CLAUDE.md
    docs::validate_claudemd_todos(diag, exclude);
    // G006: TODO/FIXME in published skills
    hygiene::validate_todo_in_skills(diag, exclude);
    // G007: TODO/FIXME in agents
    hygiene::validate_todo_in_agents(diag, exclude);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ExcludeSet;
    use crate::context::ManifestState;
    use serde_json::json;

    // Integration test: Basic mode dispatches correct validators
    #[test]
    #[serial_test::serial]
    fn test_run_all_basic_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        // Create minimal .claude/ structure for Basic mode
        std::fs::create_dir_all(".claude/skills/my-skill").unwrap();
        std::fs::write(
            ".claude/skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A skill that does useful things for developers\n---\nBody content here\n",
        )
        .unwrap();

        let ctx = LintContext {
            base_path: tmp.path().to_path_buf(),
            mode: LintMode::Basic,
            plugin_json: ManifestState::Missing,
            marketplace_json: ManifestState::Missing,
            hooks_json: ManifestState::Missing,
            settings_json: ManifestState::Missing,
        };
        let mut diag = DiagnosticCollector::new();
        run_all(&ctx, &mut diag, &ExcludeSet::default());
        // Basic mode with valid .claude/ structure should pass
        assert_eq!(diag.error_count(), 0);
    }

    // Integration test: Plugin mode dispatches all 25 validators
    #[test]
    #[serial_test::serial]
    fn test_run_all_plugin_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        // Create minimal plugin structure
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::create_dir_all("scripts").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill that does useful things for developers\n---\nBody content here\n",
        )
        .unwrap();
        std::fs::write(
            "agents/general.md",
            "---\nname: general\ndescription: General reviewer for code quality analysis\n---\nDerived from skills/shared/reviewer-templates.md\n",
        )
        .unwrap();
        std::fs::write("SECURITY.md", "# Security\n").unwrap();

        let plugin_val = json!({
            "name": "test-plugin",
            "version": "1.0.0",
            "description": "Test",
            "author": {"email": "a@b.com"},
            "keywords": ["test"]
        });
        let marketplace_val = json!({
            "name": "test-mp",
            "owner": {"name": "owner", "email": "a@b.com"},
            "plugins": [{"name": "p", "source": "s", "category": "lint"}]
        });
        let hooks_val = json!({"hooks": [{"command": "echo test"}]});

        let ctx = LintContext {
            base_path: tmp.path().to_path_buf(),
            mode: LintMode::Plugin,
            plugin_json: ManifestState::Parsed(plugin_val),
            marketplace_json: ManifestState::Parsed(marketplace_val),
            hooks_json: ManifestState::Parsed(hooks_val),
            settings_json: ManifestState::Missing,
        };
        let mut diag = DiagnosticCollector::new();
        run_all(&ctx, &mut diag, &ExcludeSet::default());

        // There may be some errors (e.g., V16 template file missing, V21 count mismatch)
        // but the key test is that run_all completes without panic and dispatches validators.
        // Verify that plugin-mode-specific validators ran by checking for expected errors.
        let errors = diag.errors();
        // V16 should fire because reviewer-templates.md doesn't exist
        assert!(
            errors.iter().any(|e| e.contains("reviewer-templates.md")),
            "Expected V16 error for missing reviewer-templates.md, got: {errors:?}"
        );
    }

    // Integration test: Basic mode does NOT run plugin-only validators
    #[test]
    #[serial_test::serial]
    fn test_basic_mode_skips_plugin_validators() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        // No .claude/ structure at all
        let ctx = LintContext {
            base_path: tmp.path().to_path_buf(),
            mode: LintMode::Basic,
            plugin_json: ManifestState::Missing,
            marketplace_json: ManifestState::Missing,
            hooks_json: ManifestState::Missing,
            settings_json: ManifestState::Missing,
        };
        let mut diag = DiagnosticCollector::new();
        run_all(&ctx, &mut diag, &ExcludeSet::default());
        // Basic mode should not report errors about plugin.json, marketplace.json, etc.
        let errors = diag.errors();
        assert!(
            !errors.iter().any(|e| e.contains("plugin.json")),
            "Basic mode should not validate plugin.json"
        );
        assert!(
            !errors.iter().any(|e| e.contains("marketplace.json")),
            "Basic mode should not validate marketplace.json"
        );
        assert!(
            !errors.iter().any(|e| e.contains("agents/")),
            "Basic mode should not validate agents/"
        );
    }

    // Integration: run_all in basic mode fires skill content rules
    #[test]
    #[serial_test::serial]
    fn test_run_all_basic_fires_content_rules() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/skills/my-skill").unwrap();
        // Empty body should trigger S020
        std::fs::write(
            ".claude/skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A skill that does useful things for developers\n---\n",
        )
        .unwrap();

        let ctx = LintContext {
            base_path: tmp.path().to_path_buf(),
            mode: LintMode::Basic,
            plugin_json: ManifestState::Missing,
            marketplace_json: ManifestState::Missing,
            hooks_json: ManifestState::Missing,
            settings_json: ManifestState::Missing,
        };
        let mut diag = DiagnosticCollector::new();
        run_all(&ctx, &mut diag, &ExcludeSet::default());
        assert!(
            diag.errors().iter().any(|e| e.contains("no content")),
            "Basic mode should fire S020 (body-empty) on private skills"
        );
    }

    // Integration: run_all with config suppression
    #[test]
    #[serial_test::serial]
    fn test_run_all_with_config_suppression() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/skills/my-skill").unwrap();
        std::fs::write(
            ".claude/skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A skill that does useful things for developers\n---\n",
        )
        .unwrap();

        // Suppress S020 via config
        let config = crate::config::LintConfig {
            ignore: std::collections::HashSet::from([crate::rules::LintRule::BodyEmpty]),
            warn: std::collections::HashSet::new(),
            exclude: vec![],
        };

        let ctx = LintContext {
            base_path: tmp.path().to_path_buf(),
            mode: LintMode::Basic,
            plugin_json: ManifestState::Missing,
            marketplace_json: ManifestState::Missing,
            hooks_json: ManifestState::Missing,
            settings_json: ManifestState::Missing,
        };
        let mut diag = DiagnosticCollector::with_config(config);
        run_all(&ctx, &mut diag, &ExcludeSet::default());
        assert!(
            !diag.errors().iter().any(|e| e.contains("no content")),
            "S020 should be suppressed by config"
        );
        assert_eq!(diag.suppressed_count(), 1);
    }

    // Integration: plugin mode fires plugin-only rules
    #[test]
    #[serial_test::serial]
    fn test_run_all_plugin_fires_content_rules() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::create_dir_all("scripts").unwrap();
        // Skill with "you" in description — triggers S016 in plugin mode
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need to analyze code for issues\n---\nBody content here\n",
        )
        .unwrap();
        std::fs::write("SECURITY.md", "# Security\n").unwrap();

        let plugin_val = json!({
            "name": "test-plugin",
            "version": "1.0.0",
            "description": "Test",
            "author": {"email": "a@b.com"},
            "keywords": ["test"]
        });
        let marketplace_val = json!({
            "name": "test-mp",
            "owner": {"name": "owner", "email": "a@b.com"},
            "plugins": [{"name": "p", "source": "s", "category": "lint"}]
        });
        let hooks_val = json!({"hooks": [{"command": "echo test"}]});

        let ctx = LintContext {
            base_path: tmp.path().to_path_buf(),
            mode: LintMode::Plugin,
            plugin_json: ManifestState::Parsed(plugin_val),
            marketplace_json: ManifestState::Parsed(marketplace_val),
            hooks_json: ManifestState::Parsed(hooks_val),
            settings_json: ManifestState::Missing,
        };
        let mut diag = DiagnosticCollector::new();
        run_all(&ctx, &mut diag, &ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("first/second person")),
            "Plugin mode should fire S016 (desc-uses-person)"
        );
    }

    // ── Exclude integration tests ───────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_exclude_suppresses_skill_diagnostics() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/skills/excluded-skill").unwrap();
        std::fs::create_dir_all(".claude/skills/included-skill").unwrap();
        // Both skills have empty body (triggers S020)
        std::fs::write(
            ".claude/skills/excluded-skill/SKILL.md",
            "---\nname: excluded-skill\ndescription: A skill that does useful things for developers\n---\n",
        )
        .unwrap();
        std::fs::write(
            ".claude/skills/included-skill/SKILL.md",
            "---\nname: included-skill\ndescription: A skill that does useful things for developers\n---\n",
        )
        .unwrap();

        let ctx = LintContext {
            base_path: tmp.path().to_path_buf(),
            mode: LintMode::Basic,
            plugin_json: ManifestState::Missing,
            marketplace_json: ManifestState::Missing,
            hooks_json: ManifestState::Missing,
            settings_json: ManifestState::Missing,
        };

        // Without exclusion: both skills produce errors
        let mut diag_all = DiagnosticCollector::new();
        run_all(&ctx, &mut diag_all, &ExcludeSet::default());
        let all_errors = diag_all.errors();
        assert!(
            all_errors.iter().any(|e| e.contains("excluded-skill")),
            "Without exclusion, excluded-skill should produce errors"
        );
        assert!(
            all_errors.iter().any(|e| e.contains("included-skill")),
            "Without exclusion, included-skill should produce errors"
        );

        // With exclusion: excluded-skill is suppressed
        let exclude = ExcludeSet::new(&[".claude/skills/excluded-skill/**".to_string()]).unwrap();
        let mut diag_excl = DiagnosticCollector::new();
        run_all(&ctx, &mut diag_excl, &exclude);
        let excl_errors = diag_excl.errors();
        assert!(
            !excl_errors.iter().any(|e| e.contains("excluded-skill")),
            "With exclusion, excluded-skill should produce no errors, got: {excl_errors:?}"
        );
        assert!(
            excl_errors.iter().any(|e| e.contains("included-skill")),
            "With exclusion, included-skill should still produce errors"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_exclude_with_wildcard_pattern() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/skills/test-a").unwrap();
        std::fs::create_dir_all(".claude/skills/test-b").unwrap();
        std::fs::create_dir_all(".claude/skills/keep-c").unwrap();
        for name in &["test-a", "test-b", "keep-c"] {
            std::fs::write(
                format!(".claude/skills/{name}/SKILL.md"),
                format!("---\nname: {name}\ndescription: A skill that does useful things for developers\n---\n"),
            )
            .unwrap();
        }

        let ctx = LintContext {
            base_path: tmp.path().to_path_buf(),
            mode: LintMode::Basic,
            plugin_json: ManifestState::Missing,
            marketplace_json: ManifestState::Missing,
            hooks_json: ManifestState::Missing,
            settings_json: ManifestState::Missing,
        };

        // Exclude test-* skills
        let exclude = ExcludeSet::new(&[".claude/skills/test-*/SKILL.md".to_string()]).unwrap();
        let mut diag = DiagnosticCollector::new();
        run_all(&ctx, &mut diag, &exclude);
        let errors = diag.errors();
        assert!(
            !errors.iter().any(|e| e.contains("test-a")),
            "test-a should be excluded"
        );
        assert!(
            !errors.iter().any(|e| e.contains("test-b")),
            "test-b should be excluded"
        );
        assert!(
            errors.iter().any(|e| e.contains("keep-c")),
            "keep-c should NOT be excluded"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_exclude_agents_in_plugin_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::create_dir_all("scripts").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill that does useful things for developers\n---\nBody content here\n",
        )
        .unwrap();
        // Agent with missing frontmatter fields — will trigger A003 if not excluded
        std::fs::write(
            "agents/excluded.md",
            "---\nname: excluded\n---\nDerived from skills/shared/reviewer-templates.md\n",
        )
        .unwrap();
        std::fs::write(
            "agents/included.md",
            "---\nname: included\n---\nDerived from skills/shared/reviewer-templates.md\n",
        )
        .unwrap();
        std::fs::write("SECURITY.md", "# Security\n").unwrap();

        let plugin_val = json!({
            "name": "test-plugin",
            "version": "1.0.0",
            "description": "Test",
            "author": {"email": "a@b.com"},
            "keywords": ["test"]
        });
        let marketplace_val = json!({
            "name": "test-mp",
            "owner": {"name": "owner", "email": "a@b.com"},
            "plugins": [{"name": "p", "source": "s", "category": "lint"}]
        });
        let hooks_val = json!({"hooks": [{"command": "echo test"}]});

        let ctx = LintContext {
            base_path: tmp.path().to_path_buf(),
            mode: LintMode::Plugin,
            plugin_json: ManifestState::Parsed(plugin_val),
            marketplace_json: ManifestState::Parsed(marketplace_val),
            hooks_json: ManifestState::Parsed(hooks_val),
            settings_json: ManifestState::Missing,
        };

        // Exclude agents/excluded.md
        let exclude = ExcludeSet::new(&["agents/excluded.md".to_string()]).unwrap();
        let mut diag = DiagnosticCollector::new();
        run_all(&ctx, &mut diag, &exclude);
        let errors = diag.errors();
        // excluded.md should produce no diagnostics
        assert!(
            !errors.iter().any(|e| e.contains("agents/excluded.md")),
            "agents/excluded.md should be excluded from diagnostics, got: {errors:?}"
        );
        // included.md should still produce diagnostics (missing description)
        assert!(
            errors.iter().any(|e| e.contains("agents/included.md")),
            "agents/included.md should still produce errors"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_exclude_does_not_affect_fixed_path_validators() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude").unwrap();

        let ctx = LintContext {
            base_path: tmp.path().to_path_buf(),
            mode: LintMode::Basic,
            plugin_json: ManifestState::Missing,
            marketplace_json: ManifestState::Missing,
            hooks_json: ManifestState::Missing,
            settings_json: ManifestState::Missing,
        };

        // Even if we exclude everything, fixed-path validators should still work
        // (settings.json hooks validator runs in basic mode but has no effect without settings.json)
        let exclude = ExcludeSet::new(&["**/*".to_string()]).unwrap();
        let mut diag = DiagnosticCollector::new();
        run_all(&ctx, &mut diag, &exclude);
        // Should run without panic — fixed-path validators are unaffected
        // No errors expected since .claude/ exists but no skills
        assert_eq!(diag.error_count(), 0);
    }
}
