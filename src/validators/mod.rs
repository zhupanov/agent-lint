mod agent_discovery;
mod agents;
mod claude_config;
mod codex_config;
mod codex_constants;
mod codex_surfaces;
mod common;
pub(crate) mod contracts;
mod cursor;
mod desc_overlap;
mod docs;
mod email;
mod hook_schema;
mod hooks;
pub mod hygiene;
mod instruction_files;
mod manifest;
mod markdown_structure;
mod mcp;
mod npm_scripts;
mod prompt_content;
pub(crate) mod shared_md_refs;
mod shell;
pub(crate) mod skill_content;
pub(crate) mod skill_discovery;
pub(crate) mod skills;
mod user_config;

use crate::config::ExcludeSet;
use crate::context::{LintContext, LintMode};
use crate::diagnostic::DiagnosticCollector;
use crate::platforms::ValidationTargets;

/// Run all validators appropriate for the current lint mode.
#[cfg(test)]
pub fn run_all(ctx: &LintContext, diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let targets = crate::platforms::DetectedSurfaces::discover(&ExcludeSet::default())
        .resolve(crate::config::PlatformOverrides::default());
    run_all_with_targets(ctx, diag, exclude, targets);
}

/// Run all validators using the explicitly resolved validation targets.
pub fn run_all_with_targets(
    ctx: &LintContext,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    targets: ValidationTargets,
) {
    match ctx.mode {
        LintMode::Basic => run_basic(ctx, diag, exclude, targets),
        LintMode::Plugin => run_plugin(ctx, diag, exclude, targets),
    }
}

/// Basic mode: validate .claude/ contents only.
fn run_basic(
    ctx: &LintContext,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    targets: ValidationTargets,
) {
    let mut prompt_pass = prompt_content::PromptContentPass::default();
    // V4: settings.json hook paths
    hooks::validate_settings_hooks(ctx, diag);
    // V27: settings.json hook schema
    hooks::validate_settings_schema(ctx, diag);
    // V28: settings.local.json validity + hook schema
    hooks::validate_settings_local(ctx, diag);
    mcp::validate_mcp_configs(ctx, diag, exclude, targets);
    // V6-adapted: private SKILL.md frontmatter for .claude/skills/
    skills::validate_private_skill_frontmatter(diag, exclude);
    // V9-adapted: script ref integrity for $PWD/.claude/skills/ refs
    hygiene::validate_private_script_references(diag, exclude);
    // V10-adapted: executability for .claude/skills/*/scripts/*.sh
    hygiene::validate_private_executability(diag, exclude);
    // Skill content checks (both-mode subset: excludes S016, S017, S029, S033)
    skill_content::validate_private_skill_content_with_prompt_pass(
        diag,
        exclude,
        false,
        &[],
        &mut prompt_pass,
    );
    // V7-adapted: private agent frontmatter + field-value rules for .claude/agents/
    agents::validate_private_agents_with_prompt_pass(diag, exclude, &mut prompt_pass, false);
    claude_config::validate_private_config(ctx, diag, exclude);
    validate_optional_surfaces(diag, exclude, targets, &mut prompt_pass);
    // A030/S074: overlapping routing descriptions within simultaneously available namespaces.
    // Basic mode has no plugin manifest, so no manifest-declared agent roots apply.
    desc_overlap::validate_agent_desc_overlap(diag, exclude, false, &[], targets.cursor);
    desc_overlap::validate_discovered_skill_desc_overlap(
        ctx,
        diag,
        exclude,
        targets.agent_skills,
        targets.cursor,
    );
    prompt_content::validate_claude_md_with_prompt_pass(diag, exclude, &mut prompt_pass);
    // X002–X005: CLAUDE.md structure (when present)
    docs::validate_claudemd_structure(diag, exclude);
    // D001/D002: root CLAUDE.md canonical docs refs and size (Always)
    docs::validate_docs_references(diag, exclude);
    docs::validate_claudemd_size(diag, exclude);
    // D003: unfinished-work markers in root CLAUDE.md (Basic activation surface)
    docs::validate_claudemd_todos(diag, exclude);
    // Shared prompt/reference/script contracts for private configuration and
    // explicitly configured script or prompt-source inventories.
    contracts::validate_contracts(ctx, diag, exclude, false);
}

/// Plugin mode: run all validators plus `.claude/` checks.
fn run_plugin(
    ctx: &LintContext,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    targets: ValidationTargets,
) {
    let mut prompt_pass = prompt_content::PromptContentPass::default();
    // Repository-safe agent roots declared in plugin.json `agents`, shared by the
    // per-agent validators and the A030 overlap pool so both see one discovery set.
    let declared_agent_roots = manifest::declared_agent_roots(ctx);
    // Private .claude/ validators (also run in basic mode)
    skills::validate_private_skill_frontmatter(diag, exclude);
    // V7-adapted: private agent frontmatter + field-value rules for .claude/agents/
    agents::validate_private_agents_with_prompt_pass(diag, exclude, &mut prompt_pass, true);
    claude_config::validate_private_config(ctx, diag, exclude);
    validate_optional_surfaces(diag, exclude, targets, &mut prompt_pass);
    prompt_content::validate_claude_md_with_prompt_pass(diag, exclude, &mut prompt_pass);

    // V1: plugin.json
    diag.with_subject_path(".claude-plugin/plugin.json", |diag| {
        manifest::validate_plugin_json(ctx, diag);
    });
    // V2: marketplace.json
    diag.with_subject_path(".claude-plugin/marketplace.json", |diag| {
        manifest::validate_marketplace_json(ctx, diag);
    });
    // V3: hooks/hooks.json
    hooks::validate_hooks_json(ctx, diag);
    // V4: settings.json hook paths
    hooks::validate_settings_hooks(ctx, diag);
    // V26: hooks.json hook schema
    hooks::validate_hooks_json_schema(ctx, diag);
    // V27: settings.json hook schema
    hooks::validate_settings_schema(ctx, diag);
    // V28: settings.local.json validity + hook schema
    hooks::validate_settings_local(ctx, diag);
    mcp::validate_mcp_configs(ctx, diag, exclude, targets);
    // V5: skills layout
    skills::validate_discovered_skills_layout(ctx, diag, exclude);
    // V6: SKILL.md frontmatter (public)
    skills::validate_skill_frontmatter(diag, exclude);
    skills::validate_discovered_skill_frontmatter(ctx, diag, exclude);
    // V7: agents frontmatter (default `agents/` plus manifest-declared roots)
    agents::validate_agents_with_prompt_pass(
        diag,
        exclude,
        &declared_agent_roots,
        &mut prompt_pass,
    );
    // O001-O006: plugin-shipped output styles — the plugin-root `output-styles/`
    // directory plus every manifest-declared `outputStyles` path. Plugin mode
    // only; `.claude/output-styles/` is covered by validate_private_config above.
    claude_config::validate_plugin_output_styles(
        diag,
        exclude,
        &manifest::declared_output_style_roots(ctx),
    );
    // V8: PWD hygiene
    hygiene::validate_pwd_hygiene(diag, exclude);
    // V9: script reference integrity
    hygiene::scripts::validate_script_references_for_mode(LintMode::Plugin, diag, exclude);
    // V10: executability (generic, no hardcoded block-submodule-edit.sh)
    hygiene::scripts::validate_executability_for_mode(LintMode::Plugin, diag, exclude);
    // V11: dead-script detection
    hygiene::validate_dead_scripts(ctx, diag, exclude);
    // V12: marketplace enriched metadata
    diag.with_subject_path(".claude-plugin/marketplace.json", |diag| {
        manifest::validate_marketplace_enriched(ctx, diag);
    });
    // V13: plugin enriched metadata
    diag.with_subject_path(".claude-plugin/plugin.json", |diag| {
        manifest::validate_plugin_enriched(ctx, diag);
    });
    // V14: SECURITY.md presence
    hygiene::validate_security_md(diag);
    // V15: shared markdown reference integrity
    skills::validate_discovered_shared_md_references(ctx, diag, exclude);
    // V16/V21: opt-in larch reviewer-template convention
    agents::validate_agent_template_convention(diag, exclude);
    // V17: email format
    email::validate_email_format(ctx, diag);
    // V18/V23–V25/V33/U008: userConfig schema (top-level and channels)
    diag.with_subject_path(".claude-plugin/plugin.json", |diag| {
        user_config::validate_user_config(ctx, diag);
    });
    // D001: docs file references from CLAUDE.md Canonical sources (Always)
    docs::validate_docs_references(diag, exclude);
    // V29: component path safety and layout
    diag.with_subject_path(".claude-plugin/plugin.json", |diag| {
        manifest::validate_component_paths(ctx, diag);
    });
    // V30–V32: shared plugin-field validation on plugin and marketplace manifests.
    manifest::validate_plugin_fields(ctx, diag);
    // Original skill content checks (S009-S057, including plugin-only rules)
    skill_content::validate_discovered_skill_content_with_prompt_pass(
        ctx,
        diag,
        exclude,
        &mut prompt_pass,
    );
    // Private skill content checks (both-mode subset)
    skill_content::validate_private_skill_content_with_prompt_pass(
        diag,
        exclude,
        true,
        &declared_agent_roots,
        &mut prompt_pass,
    );
    // A030/S074: overlapping routing descriptions (Claude private∪plugin runtime union)
    desc_overlap::validate_agent_desc_overlap(
        diag,
        exclude,
        true,
        &declared_agent_roots,
        targets.cursor,
    );
    desc_overlap::validate_discovered_skill_desc_overlap(
        ctx,
        diag,
        exclude,
        targets.agent_skills,
        targets.cursor,
    );
    // D002: CLAUDE.md size (Always)
    docs::validate_claudemd_size(diag, exclude);
    // D003: TODO/FIXME in CLAUDE.md
    docs::validate_claudemd_todos(diag, exclude);
    // X002–X005: CLAUDE.md fence / XML structure
    docs::validate_claudemd_structure(diag, exclude);
    // G006: TODO/FIXME in published skills
    hygiene::validate_todo_in_skills(diag, exclude);
    // G007: TODO/FIXME in agents
    hygiene::validate_todo_in_agents(diag, exclude);
    // Prompt/reference/script contracts shared with private configuration mode.
    contracts::validate_contracts(ctx, diag, exclude, true);
}

fn validate_optional_surfaces(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    targets: ValidationTargets,
    prompt_pass: &mut prompt_content::PromptContentPass,
) {
    if targets.agents_md || targets.codex {
        instruction_files::validate_agents_files_with_prompt_pass(
            diag,
            exclude,
            targets.codex,
            prompt_pass,
        );
    }
    if targets.agent_skills {
        skills::validate_agent_skill_frontmatter_with_prompt_pass(diag, exclude, prompt_pass);
        if !targets.cursor {
            skill_content::validate_agent_skills_contract(".agents/skills", diag, exclude);
            skill_content::validate_agent_skills_content_security(".agents/skills", diag, exclude);
        }
    }
    if targets.codex {
        diag.with_subject_path(".codex/config.toml", |diag| {
            codex_config::validate_config(diag, exclude);
        });
        codex_surfaces::validate_with_prompt_pass(diag, exclude, prompt_pass);
    }
    if targets.cursor {
        skill_content::validate_cursor_runtime_skills_contract(diag, exclude);
        skill_content::validate_cursor_runtime_skills_content_security(diag, exclude);
        cursor::validate_with_prompt_pass(diag, exclude, prompt_pass);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ExcludeSet, PlatformOverrides};
    use crate::context::ManifestState;
    use crate::rules::LintRule;
    use serde_json::json;
    use std::path::Path;

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
            declared_hook_configs: vec![],
            settings_json: ManifestState::Missing,
            settings_local_json: ManifestState::Missing,
        };
        let mut diag = DiagnosticCollector::new_all_enabled();
        run_all(&ctx, &mut diag, &ExcludeSet::default());
        // Basic mode with valid .claude/ structure should pass
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn platform_overrides_gate_platform_validators() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir(".codex").unwrap();
        std::fs::write(".codex/config.toml", "not = [valid").unwrap();
        std::fs::create_dir(".cursor").unwrap();
        std::fs::write(".cursor/hooks.json", "not valid JSON").unwrap();

        let ctx = LintContext {
            base_path: tmp.path().to_path_buf(),
            mode: LintMode::Basic,
            plugin_json: ManifestState::Missing,
            marketplace_json: ManifestState::Missing,
            hooks_json: ManifestState::Missing,
            declared_hook_configs: vec![],
            settings_json: ManifestState::Missing,
            settings_local_json: ManifestState::Missing,
        };
        let mut disabled = DiagnosticCollector::new_all_enabled();
        run_all_with_targets(
            &ctx,
            &mut disabled,
            &ExcludeSet::default(),
            ValidationTargets::default(),
        );
        assert!(
            !disabled
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.rule == crate::rules::LintRule::CodexTomlInvalid)
        );
        assert!(!disabled.diagnostics().iter().any(|diagnostic| {
            diagnostic.rule == crate::rules::LintRule::CursorHooksSchemaInvalid
        }));

        let mut enabled = DiagnosticCollector::new_all_enabled();
        run_all_with_targets(
            &ctx,
            &mut enabled,
            &ExcludeSet::default(),
            ValidationTargets {
                cursor: false,
                codex: true,
                claude_md: false,
                agents_md: false,
                agent_skills: false,
            },
        );
        assert!(
            enabled
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.rule == crate::rules::LintRule::CodexTomlInvalid)
        );
        assert!(!enabled.diagnostics().iter().any(|diagnostic| {
            diagnostic.rule == crate::rules::LintRule::CursorHooksSchemaInvalid
        }));

        let mut cursor_enabled = DiagnosticCollector::new_all_enabled();
        run_all_with_targets(
            &ctx,
            &mut cursor_enabled,
            &ExcludeSet::default(),
            ValidationTargets {
                cursor: true,
                codex: false,
                claude_md: false,
                agents_md: false,
                agent_skills: false,
            },
        );
        assert!(cursor_enabled.diagnostics().iter().any(|diagnostic| {
            diagnostic.rule == crate::rules::LintRule::CursorHooksSchemaInvalid
        }));
        assert!(
            !cursor_enabled
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.rule == crate::rules::LintRule::CodexTomlInvalid)
        );
    }

    #[test]
    #[serial_test::serial]
    fn basic_mode_dispatches_explicit_script_inventory_contracts() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude/skills").unwrap();
        std::fs::create_dir("portable").unwrap();
        std::fs::write("portable/render.sh", "out=\"${out//TOKEN/$replacement}\"\n").unwrap();
        let ctx = LintContext::new(tmp.path(), LintMode::Basic);
        let config = crate::config::LintConfig {
            script_inventory: Some(vec!["portable/render.sh".into()]),
            ..crate::config::LintConfig::default()
        };
        let mut diag = DiagnosticCollector::with_config_silent(config);

        run_all_with_targets(
            &ctx,
            &mut diag,
            &ExcludeSet::default(),
            ValidationTargets::default(),
        );

        assert!(diag.diagnostics().iter().any(|diagnostic| {
            diagnostic.rule == crate::rules::LintRule::BashReplacementUnsafe
        }));
    }

    #[test]
    #[serial_test::serial]
    fn shared_agents_surface_does_not_imply_codex() {
        struct Case {
            name: &'static str,
            cursor_surface: bool,
            codex_surface: bool,
            codex_override: Option<bool>,
            expected_cursor: bool,
            expected_codex: bool,
        }

        let cases = [
            Case {
                name: "agents only",
                cursor_surface: false,
                codex_surface: false,
                codex_override: None,
                expected_cursor: false,
                expected_codex: false,
            },
            Case {
                name: "agents and cursor",
                cursor_surface: true,
                codex_surface: false,
                codex_override: None,
                expected_cursor: true,
                expected_codex: false,
            },
            Case {
                name: "agents and codex",
                cursor_surface: false,
                codex_surface: true,
                codex_override: None,
                expected_cursor: false,
                expected_codex: true,
            },
            Case {
                name: "agents with codex enabled",
                cursor_surface: false,
                codex_surface: false,
                codex_override: Some(true),
                expected_cursor: false,
                expected_codex: true,
            },
            Case {
                name: "agents with codex disabled",
                cursor_surface: false,
                codex_surface: false,
                codex_override: Some(false),
                expected_cursor: false,
                expected_codex: false,
            },
        ];

        for case in cases {
            let tmp = tempfile::tempdir().unwrap();
            let _guard = crate::test_helpers::CwdGuard::new();
            std::env::set_current_dir(tmp.path()).unwrap();
            std::fs::write(
                "AGENTS.md",
                format!(
                    "# Project instructions\ntoken = sk-12345678901234567890\n{}",
                    "x".repeat(32_769)
                ),
            )
            .unwrap();
            if case.cursor_surface {
                std::fs::create_dir(".cursor").unwrap();
                std::fs::write(".cursor/hooks.json", "not valid JSON").unwrap();
            }
            if case.codex_surface {
                std::fs::create_dir(".codex").unwrap();
                std::fs::write(".codex/config.toml", "model = 'gpt-5'\n").unwrap();
            }

            let exclude = ExcludeSet::default();
            let targets =
                crate::platforms::DetectedSurfaces::discover(&exclude).resolve(PlatformOverrides {
                    cursor: None,
                    codex: case.codex_override,
                });
            let ctx = LintContext {
                base_path: tmp.path().to_path_buf(),
                mode: LintMode::Basic,
                plugin_json: ManifestState::Missing,
                marketplace_json: ManifestState::Missing,
                hooks_json: ManifestState::Missing,
                declared_hook_configs: vec![],
                settings_json: ManifestState::Missing,
                settings_local_json: ManifestState::Missing,
            };
            let mut diag = DiagnosticCollector::new_all_enabled();
            run_all_with_targets(&ctx, &mut diag, &exclude, targets);

            assert!(targets.agents_md, "{}: shared surface missing", case.name);
            assert_eq!(targets.cursor, case.expected_cursor, "{}", case.name);
            assert_eq!(targets.codex, case.expected_codex, "{}", case.name);
            assert!(
                diag.diagnostics()
                    .iter()
                    .any(|item| item.rule == crate::rules::LintRule::InstructionFileSecret),
                "{}: shared rule did not run",
                case.name
            );
            assert_eq!(
                diag.diagnostics()
                    .iter()
                    .any(|item| item.rule == crate::rules::LintRule::CodexProjectDocBudget),
                case.expected_codex,
                "{}: Codex rule dispatch mismatch",
                case.name
            );
            assert_eq!(
                diag.diagnostics()
                    .iter()
                    .any(|item| { item.rule == crate::rules::LintRule::CursorHooksSchemaInvalid }),
                case.expected_cursor,
                "{}: Cursor rule dispatch mismatch",
                case.name
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn shared_agent_skills_do_not_imply_codex() {
        for codex_surface in [false, true] {
            let tmp = tempfile::tempdir().unwrap();
            let _guard = crate::test_helpers::CwdGuard::new();
            std::env::set_current_dir(tmp.path()).unwrap();
            std::fs::create_dir_all(".agents/skills/example").unwrap();
            std::fs::write(
                ".agents/skills/example/SKILL.md",
                "---\nname: wrong-name\ndescription: Shared example skill\ncontext: fork\n---\nBody\n",
            )
            .unwrap();
            if codex_surface {
                std::fs::create_dir(".codex").unwrap();
                std::fs::write(".codex/config.toml", "model = 'gpt-5'\n").unwrap();
            }

            let exclude = ExcludeSet::default();
            let targets = crate::platforms::DetectedSurfaces::discover(&exclude)
                .resolve(PlatformOverrides::default());
            let ctx = LintContext {
                base_path: tmp.path().to_path_buf(),
                mode: LintMode::Basic,
                plugin_json: ManifestState::Missing,
                marketplace_json: ManifestState::Missing,
                hooks_json: ManifestState::Missing,
                declared_hook_configs: vec![],
                settings_json: ManifestState::Missing,
                settings_local_json: ManifestState::Missing,
            };
            let mut diag = DiagnosticCollector::new_all_enabled();
            run_all_with_targets(&ctx, &mut diag, &exclude, targets);

            assert!(targets.agent_skills);
            assert_eq!(targets.codex, codex_surface);
            assert!(
                diag.diagnostics()
                    .iter()
                    .any(|item| item.rule == crate::rules::LintRule::FrontmatterNameMismatch)
            );
            assert_eq!(
                diag.diagnostics().iter().any(|item| {
                    item.rule == crate::rules::LintRule::CodexSkillUnsupportedFrontmatter
                }),
                codex_surface
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn name_contract_covers_each_active_skill_surface() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        for base in [
            "skills",
            ".claude/skills",
            ".agents/skills",
            ".cursor/skills",
        ] {
            std::fs::create_dir_all(format!("{base}/Invalid")).unwrap();
            std::fs::write(
                format!("{base}/Invalid/SKILL.md"),
                "---\nname: Invalid\ndescription: A valid skill description here\n---\nBody\n",
            )
            .unwrap();
        }

        let ctx = LintContext::new(tmp.path(), LintMode::Plugin);
        let mut diag = DiagnosticCollector::new_all_enabled();
        run_all_with_targets(
            &ctx,
            &mut diag,
            &ExcludeSet::default(),
            ValidationTargets {
                cursor: true,
                codex: false,
                claude_md: false,
                agents_md: false,
                agent_skills: true,
            },
        );
        let mut subjects = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == crate::rules::LintRule::NameInvalidChars)
            .filter_map(|item| item.subject_path.as_ref())
            .map(|path| path.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        subjects.sort();
        assert_eq!(
            subjects,
            vec![
                ".agents/skills/Invalid/SKILL.md",
                ".claude/skills/Invalid/SKILL.md",
                ".cursor/skills/Invalid/SKILL.md",
                "skills/Invalid/SKILL.md",
            ]
        );

        let mut disabled = DiagnosticCollector::new_all_enabled();
        run_all_with_targets(
            &ctx,
            &mut disabled,
            &ExcludeSet::default(),
            ValidationTargets::default(),
        );
        assert_eq!(
            disabled
                .diagnostics()
                .iter()
                .filter(|item| item.rule == crate::rules::LintRule::NameInvalidChars)
                .count(),
            2,
            "platform-gated surfaces must respect resolved activation"
        );
    }

    #[test]
    #[serial_test::serial]
    fn cursor_runtime_inventory_covers_nested_shared_skills_and_honors_activation() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let nested_cursor = "packages/api/.cursor/skills/group/Invalid/SKILL.md";
        let shared = "packages/api/.agents/skills/shared/SKILL.md";
        for (path, name, description) in [
            (nested_cursor, "Invalid", "x".repeat(1025)),
            (shared, "Shared", "x".repeat(19)),
        ] {
            std::fs::create_dir_all(Path::new(path).parent().unwrap()).unwrap();
            std::fs::write(
                path,
                format!(
                    "---\nname: {name}\ndescription: {description}\nuser-invocable: false\n---\nBody\n"
                ),
            )
            .unwrap();
        }
        let ctx = LintContext::new(tmp.path(), LintMode::Basic);
        let mut enabled = DiagnosticCollector::new_all_enabled();
        run_all_with_targets(
            &ctx,
            &mut enabled,
            &ExcludeSet::default(),
            ValidationTargets {
                cursor: true,
                codex: false,
                claude_md: false,
                agents_md: false,
                agent_skills: true,
            },
        );
        for path in [nested_cursor, shared] {
            assert!(enabled.diagnostics().iter().any(|item| {
                item.rule == crate::rules::LintRule::CursorSkillFieldUnsupported
                    && item.subject_path.as_deref() == Some(Path::new(path))
            }));
        }
        assert!(enabled.diagnostics().iter().any(|item| {
            item.rule == crate::rules::LintRule::NameInvalidChars
                && item.subject_path.as_deref() == Some(Path::new(nested_cursor))
        }));
        for (path, rule) in [
            (nested_cursor, crate::rules::LintRule::DescTooLong),
            (shared, crate::rules::LintRule::DescTooShort),
        ] {
            assert!(enabled.diagnostics().iter().any(|item| {
                item.rule == rule && item.subject_path.as_deref() == Some(Path::new(path))
            }));
        }

        let mut disabled = DiagnosticCollector::new_all_enabled();
        run_all_with_targets(
            &ctx,
            &mut disabled,
            &ExcludeSet::default(),
            ValidationTargets {
                cursor: false,
                codex: false,
                claude_md: false,
                agents_md: false,
                agent_skills: true,
            },
        );
        assert!(
            !disabled
                .diagnostics()
                .iter()
                .any(|item| { item.rule == crate::rules::LintRule::CursorSkillFieldUnsupported })
        );
        assert!(disabled.diagnostics().iter().any(|item| {
            item.rule == crate::rules::LintRule::NameInvalidChars
                && item.subject_path.as_deref() == Some(Path::new(shared))
        }));
        assert!(disabled.diagnostics().iter().any(|item| {
            item.rule == crate::rules::LintRule::DescTooShort
                && item.subject_path.as_deref() == Some(Path::new(shared))
        }));
        assert!(
            !disabled.diagnostics().iter().any(|item| {
                item.rule == crate::rules::LintRule::DescTooLong
                    && item.subject_path.as_deref() == Some(Path::new(nested_cursor))
            }),
            "Cursor-only skills must not leak into the Agent Skills contract when Cursor is inactive"
        );
    }

    #[test]
    #[serial_test::serial]
    fn content_security_covers_each_active_skill_surface() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        let skill_body = |name: &str, body: &str| {
            format!("---\nname: {name}\ndescription: A valid skill description here\n---\n{body}\n")
        };
        std::fs::create_dir_all("skills/leaky").unwrap();
        std::fs::write(
            "skills/leaky/SKILL.md",
            skill_body("leaky", "Set key to sk-aBcDeFgHiJkLmNoPqRsT1234"),
        )
        .unwrap();
        std::fs::create_dir_all(".claude/skills/leaky").unwrap();
        std::fs::write(
            ".claude/skills/leaky/SKILL.md",
            skill_body("leaky", "Fetch from http://api.corp/x"),
        )
        .unwrap();
        std::fs::create_dir_all(".agents/skills/leaky").unwrap();
        std::fs::write(
            ".agents/skills/leaky/SKILL.md",
            skill_body("leaky", "Set key to sk-aBcDeFgHiJkLmNoPqRsT1234"),
        )
        .unwrap();
        std::fs::create_dir_all(".cursor/skills/leaky").unwrap();
        std::fs::write(
            ".cursor/skills/leaky/SKILL.md",
            skill_body("leaky", "Fetch from http://api.corp/x"),
        )
        .unwrap();

        let ctx = LintContext::new(tmp.path(), LintMode::Plugin);
        let mut diag = DiagnosticCollector::new_all_enabled();
        run_all_with_targets(
            &ctx,
            &mut diag,
            &ExcludeSet::default(),
            ValidationTargets {
                cursor: true,
                codex: false,
                claude_md: false,
                agents_md: false,
                agent_skills: true,
            },
        );

        let mut secret_subjects = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == crate::rules::LintRule::HardcodedSecret)
            .filter_map(|item| item.subject_path.as_ref())
            .map(|path| path.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        secret_subjects.sort();
        assert_eq!(
            secret_subjects,
            vec![".agents/skills/leaky/SKILL.md", "skills/leaky/SKILL.md",]
        );

        let mut http_subjects = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == crate::rules::LintRule::NonHttpsUrl)
            .filter_map(|item| item.subject_path.as_ref())
            .map(|path| path.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        http_subjects.sort();
        assert_eq!(
            http_subjects,
            vec![
                ".claude/skills/leaky/SKILL.md",
                ".cursor/skills/leaky/SKILL.md",
            ]
        );

        let mut gated_off = DiagnosticCollector::new_all_enabled();
        run_all_with_targets(
            &ctx,
            &mut gated_off,
            &ExcludeSet::default(),
            ValidationTargets::default(),
        );
        assert!(
            !gated_off.diagnostics().iter().any(|item| {
                matches!(
                    item.rule,
                    crate::rules::LintRule::HardcodedSecret | crate::rules::LintRule::NonHttpsUrl
                ) && item.subject_path.as_ref().is_some_and(|path| {
                    let path = path.to_string_lossy();
                    path.starts_with(".agents/") || path.starts_with(".cursor/")
                })
            }),
            "platform-gated security checks must respect resolved activation"
        );
        assert!(gated_off.diagnostics().iter().any(|item| {
            item.rule == crate::rules::LintRule::HardcodedSecret
                && item
                    .subject_path
                    .as_ref()
                    .is_some_and(|path| path.to_string_lossy() == "skills/leaky/SKILL.md")
        }));
        assert!(gated_off.diagnostics().iter().any(|item| item.rule
            == crate::rules::LintRule::NonHttpsUrl
            && item.subject_path.as_ref().is_some_and(|path| {
                path.to_string_lossy() == ".claude/skills/leaky/SKILL.md"
            })));
    }

    // Integration test: Plugin mode dispatches all validators
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
            "plugins": [{"name": "p", "source": "./s", "category": "lint"}]
        });
        let hooks_val = json!({"hooks": [{"command": "echo test"}]});

        let ctx = LintContext {
            base_path: tmp.path().to_path_buf(),
            mode: LintMode::Plugin,
            plugin_json: ManifestState::parsed(plugin_val),
            marketplace_json: ManifestState::parsed(marketplace_val),
            hooks_json: ManifestState::parsed(hooks_val),
            declared_hook_configs: vec![],
            settings_json: ManifestState::Missing,
            settings_local_json: ManifestState::Missing,
        };
        let mut diag = DiagnosticCollector::new_all_enabled();
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

    // Integration test: Plugin mode also lints .claude/agents/ (A014-A027 are "Always")
    #[test]
    #[serial_test::serial]
    fn test_plugin_mode_lints_private_agents() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::create_dir_all("scripts").unwrap();
        std::fs::create_dir_all(".claude/agents").unwrap();
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
        // Private agent with an invalid model — must be caught in Plugin mode too.
        std::fs::write(
            ".claude/agents/private.md",
            format!(
                "---\nname: private\ndescription: {}\nmodel: sonet\n---\nBody\n",
                "A general-purpose code review assistant"
            ),
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
            "plugins": [{"name": "p", "source": "./s", "category": "lint"}]
        });
        let hooks_val = json!({"hooks": [{"command": "echo test"}]});

        let ctx = LintContext {
            base_path: tmp.path().to_path_buf(),
            mode: LintMode::Plugin,
            plugin_json: ManifestState::parsed(plugin_val),
            marketplace_json: ManifestState::parsed(marketplace_val),
            hooks_json: ManifestState::parsed(hooks_val),
            declared_hook_configs: vec![],
            settings_json: ManifestState::Missing,
            settings_local_json: ManifestState::Missing,
        };
        let mut diag = DiagnosticCollector::new_all_enabled();
        run_all(&ctx, &mut diag, &ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains(".claude/agents/private.md") && e.contains("model")),
            "Plugin mode should lint .claude/agents/ via validate_private_agents, got: {:?}",
            diag.errors()
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
            declared_hook_configs: vec![],
            settings_json: ManifestState::Missing,
            settings_local_json: ManifestState::Missing,
        };
        let mut diag = DiagnosticCollector::new_all_enabled();
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
            declared_hook_configs: vec![],
            settings_json: ManifestState::Missing,
            settings_local_json: ManifestState::Missing,
        };
        let mut diag = DiagnosticCollector::new_all_enabled();
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
            suppress: std::collections::HashSet::from([crate::rules::LintRule::BodyEmpty]),
            error: std::collections::HashSet::new(),
            warn: std::collections::HashSet::new(),
            exclude: vec![],
            ..crate::config::LintConfig::default()
        };

        let ctx = LintContext {
            base_path: tmp.path().to_path_buf(),
            mode: LintMode::Basic,
            plugin_json: ManifestState::Missing,
            marketplace_json: ManifestState::Missing,
            hooks_json: ManifestState::Missing,
            declared_hook_configs: vec![],
            settings_json: ManifestState::Missing,
            settings_local_json: ManifestState::Missing,
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
            "plugins": [{"name": "p", "source": "./s", "category": "lint"}]
        });
        let hooks_val = json!({"hooks": [{"command": "echo test"}]});

        let ctx = LintContext {
            base_path: tmp.path().to_path_buf(),
            mode: LintMode::Plugin,
            plugin_json: ManifestState::parsed(plugin_val),
            marketplace_json: ManifestState::parsed(marketplace_val),
            hooks_json: ManifestState::parsed(hooks_val),
            declared_hook_configs: vec![],
            settings_json: ManifestState::Missing,
            settings_local_json: ManifestState::Missing,
        };
        let mut diag = DiagnosticCollector::new_all_enabled();
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
            declared_hook_configs: vec![],
            settings_json: ManifestState::Missing,
            settings_local_json: ManifestState::Missing,
        };

        // Without exclusion: both skills produce errors
        let mut diag_all = DiagnosticCollector::new_all_enabled();
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
        let mut diag_excl = DiagnosticCollector::new_all_enabled();
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
            declared_hook_configs: vec![],
            settings_json: ManifestState::Missing,
            settings_local_json: ManifestState::Missing,
        };

        // Exclude test-* skills
        let exclude = ExcludeSet::new(&[".claude/skills/test-*/SKILL.md".to_string()]).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
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
            "plugins": [{"name": "p", "source": "./s", "category": "lint"}]
        });
        let hooks_val = json!({"hooks": [{"command": "echo test"}]});

        let ctx = LintContext {
            base_path: tmp.path().to_path_buf(),
            mode: LintMode::Plugin,
            plugin_json: ManifestState::parsed(plugin_val),
            marketplace_json: ManifestState::parsed(marketplace_val),
            hooks_json: ManifestState::parsed(hooks_val),
            declared_hook_configs: vec![],
            settings_json: ManifestState::Missing,
            settings_local_json: ManifestState::Missing,
        };

        // Exclude agents/excluded.md
        let exclude = ExcludeSet::new(&["agents/excluded.md".to_string()]).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
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
            declared_hook_configs: vec![],
            settings_json: ManifestState::Missing,
            settings_local_json: ManifestState::Missing,
        };

        // Even if we exclude everything, fixed-path validators should still work
        // (settings.json hooks validator runs in basic mode but has no effect without settings.json)
        let exclude = ExcludeSet::new(&["**/*".to_string()]).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        run_all(&ctx, &mut diag, &exclude);
        // Should run without panic — fixed-path validators are unaffected
        // No errors expected since .claude/ exists but no skills
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn d001_d002_dispatch_in_basic_and_plugin() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::write(
            "CLAUDE.md",
            format!(
                "# Project\n## Canonical sources\n- docs/missing.md\n## Other\n{}",
                "line\n".repeat(500)
            ),
        )
        .unwrap();

        let basic = LintContext {
            base_path: tmp.path().to_path_buf(),
            mode: LintMode::Basic,
            plugin_json: ManifestState::Missing,
            marketplace_json: ManifestState::Missing,
            hooks_json: ManifestState::Missing,
            declared_hook_configs: vec![],
            settings_json: ManifestState::Missing,
            settings_local_json: ManifestState::Missing,
        };
        let mut basic_diag = DiagnosticCollector::new_all_enabled();
        run_all(&basic, &mut basic_diag, &ExcludeSet::default());
        assert!(
            basic_diag
                .diagnostics()
                .iter()
                .any(|d| d.rule == LintRule::DocsRefMissing),
            "Basic mode must dispatch D001"
        );
        assert!(
            basic_diag
                .diagnostics()
                .iter()
                .any(|d| d.rule == LintRule::ClaudemdTooLarge),
            "Basic mode must dispatch D002"
        );

        std::fs::create_dir_all(".claude-plugin").unwrap();
        std::fs::write(
            ".claude-plugin/plugin.json",
            r#"{"name":"docs-dispatch","version":"1.0.0"}"#,
        )
        .unwrap();
        let plugin_val: serde_json::Value =
            serde_json::from_str(r#"{"name":"docs-dispatch","version":"1.0.0"}"#).unwrap();
        let plugin = LintContext {
            base_path: tmp.path().to_path_buf(),
            mode: LintMode::Plugin,
            plugin_json: ManifestState::parsed(plugin_val),
            marketplace_json: ManifestState::Missing,
            hooks_json: ManifestState::Missing,
            declared_hook_configs: vec![],
            settings_json: ManifestState::Missing,
            settings_local_json: ManifestState::Missing,
        };
        let mut plugin_diag = DiagnosticCollector::new_all_enabled();
        run_all(&plugin, &mut plugin_diag, &ExcludeSet::default());
        assert!(
            plugin_diag
                .diagnostics()
                .iter()
                .any(|d| d.rule == LintRule::DocsRefMissing),
            "Plugin mode must dispatch D001"
        );
        assert!(
            plugin_diag
                .diagnostics()
                .iter()
                .any(|d| d.rule == LintRule::ClaudemdTooLarge),
            "Plugin mode must dispatch D002"
        );
    }
}
