mod agents;
mod docs;
mod email;
mod hooks;
pub mod hygiene;
mod manifest;
mod skill_content;
pub(crate) mod skills;
mod slack;
mod user_config;

use crate::context::{LintContext, LintMode};
use crate::diagnostic::DiagnosticCollector;

/// Run all validators appropriate for the current lint mode.
pub fn run_all(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    match ctx.mode {
        LintMode::Basic => run_basic(ctx, diag),
        LintMode::Plugin => run_plugin(ctx, diag),
    }
}

/// Basic mode: validate .claude/ contents only.
fn run_basic(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    // V4: settings.json hook paths
    hooks::validate_settings_hooks(ctx, diag);
    // V6-adapted: private SKILL.md frontmatter for .claude/skills/
    skills::validate_private_skill_frontmatter(diag);
    // V9-adapted: script ref integrity for $PWD/.claude/skills/ refs
    hygiene::validate_private_script_references(diag);
    // V10-adapted: executability for .claude/skills/*/scripts/*.sh
    hygiene::validate_private_executability(diag);
    // Skill content checks (both-mode subset: excludes S015, S016, S017, S029, S033)
    skill_content::validate_private_skill_content(diag);
}

/// Plugin mode: run all validators plus `.claude/` checks.
fn run_plugin(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    // Private .claude/ validators (also run in basic mode)
    skills::validate_private_skill_frontmatter(diag);
    hygiene::validate_private_script_references(diag);
    hygiene::validate_private_executability(diag);

    // V1: plugin.json
    manifest::validate_plugin_json(ctx, diag);
    // V2: marketplace.json
    manifest::validate_marketplace_json(ctx, diag);
    // V3: hooks/hooks.json
    hooks::validate_hooks_json(ctx, diag);
    // V4: settings.json hook paths
    hooks::validate_settings_hooks(ctx, diag);
    // V5: skills layout
    skills::validate_skills_layout(diag);
    // V6: SKILL.md frontmatter (public)
    skills::validate_skill_frontmatter(diag);
    // V7: agents frontmatter
    agents::validate_agents(diag);
    // V8: PWD hygiene
    hygiene::validate_pwd_hygiene(diag);
    // V9: script reference integrity
    hygiene::validate_script_references(diag);
    // V10: executability (generic, no hardcoded block-submodule-edit.sh)
    hygiene::validate_executability(diag);
    // V11: dead-script detection
    hygiene::validate_dead_scripts(diag);
    // V12: marketplace enriched metadata
    manifest::validate_marketplace_enriched(ctx, diag);
    // V13: plugin enriched metadata
    manifest::validate_plugin_enriched(ctx, diag);
    // V14: SECURITY.md presence
    hygiene::validate_security_md(diag);
    // V15: shared markdown reference integrity
    skills::validate_shared_md_references(diag);
    // V16: agent-template alignment
    agents::validate_agent_template_alignment(diag);
    // V17: email format
    email::validate_email_format(ctx, diag);
    // V18: userConfig structure
    user_config::validate_userconfig_structure(ctx, diag);
    // V19: Slack fallback consistency (larch-specific convention)
    slack::validate_slack_fallback_consistency(diag);
    // V20: userConfig→env mapping
    user_config::validate_userconfig_env_mapping(ctx, diag);
    // V21: agent-template count
    agents::validate_agent_template_count(diag);
    // V22: docs file references
    docs::validate_docs_references(diag);
    // V23: userConfig sensitive type
    user_config::validate_userconfig_sensitive_type(ctx, diag);
    // V24: userConfig title field
    user_config::validate_userconfig_title(ctx, diag);
    // V25: userConfig type field
    user_config::validate_userconfig_type(ctx, diag);
    // Skill content checks (all 26 rules including plugin-only)
    skill_content::validate_skill_content(diag);
    // Private skill content checks (both-mode subset)
    skill_content::validate_private_skill_content(diag);
    // D002: CLAUDE.md size
    docs::validate_claudemd_size(diag);
    // D003: TODO/FIXME in CLAUDE.md
    docs::validate_claudemd_todos(diag);
    // G006: TODO/FIXME in published skills
    hygiene::validate_todo_in_skills(diag);
    // G007: TODO/FIXME in agents
    hygiene::validate_todo_in_agents(diag);
}

#[cfg(test)]
mod tests {
    use super::*;
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
            repo_root: tmp.path().to_string_lossy().to_string(),
            mode: LintMode::Basic,
            plugin_json: ManifestState::Missing,
            marketplace_json: ManifestState::Missing,
            hooks_json: ManifestState::Missing,
            settings_json: ManifestState::Missing,
        };
        let mut diag = DiagnosticCollector::new();
        run_all(&ctx, &mut diag);
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
        let hooks_val = json!({"hooks": []});

        let ctx = LintContext {
            repo_root: tmp.path().to_string_lossy().to_string(),
            mode: LintMode::Plugin,
            plugin_json: ManifestState::Parsed(plugin_val),
            marketplace_json: ManifestState::Parsed(marketplace_val),
            hooks_json: ManifestState::Parsed(hooks_val),
            settings_json: ManifestState::Missing,
        };
        let mut diag = DiagnosticCollector::new();
        run_all(&ctx, &mut diag);

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
            repo_root: tmp.path().to_string_lossy().to_string(),
            mode: LintMode::Basic,
            plugin_json: ManifestState::Missing,
            marketplace_json: ManifestState::Missing,
            hooks_json: ManifestState::Missing,
            settings_json: ManifestState::Missing,
        };
        let mut diag = DiagnosticCollector::new();
        run_all(&ctx, &mut diag);
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
            repo_root: tmp.path().to_string_lossy().to_string(),
            mode: LintMode::Basic,
            plugin_json: ManifestState::Missing,
            marketplace_json: ManifestState::Missing,
            hooks_json: ManifestState::Missing,
            settings_json: ManifestState::Missing,
        };
        let mut diag = DiagnosticCollector::new();
        run_all(&ctx, &mut diag);
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
        };

        let ctx = LintContext {
            repo_root: tmp.path().to_string_lossy().to_string(),
            mode: LintMode::Basic,
            plugin_json: ManifestState::Missing,
            marketplace_json: ManifestState::Missing,
            hooks_json: ManifestState::Missing,
            settings_json: ManifestState::Missing,
        };
        let mut diag = DiagnosticCollector::with_config(config);
        run_all(&ctx, &mut diag);
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
        let hooks_val = json!({"hooks": []});

        let ctx = LintContext {
            repo_root: tmp.path().to_string_lossy().to_string(),
            mode: LintMode::Plugin,
            plugin_json: ManifestState::Parsed(plugin_val),
            marketplace_json: ManifestState::Parsed(marketplace_val),
            hooks_json: ManifestState::Parsed(hooks_val),
            settings_json: ManifestState::Missing,
        };
        let mut diag = DiagnosticCollector::new();
        run_all(&ctx, &mut diag);
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("first/second person")),
            "Plugin mode should fire S016 (desc-uses-person)"
        );
    }
}
