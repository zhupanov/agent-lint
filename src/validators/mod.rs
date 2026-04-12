mod agents;
mod docs;
mod email;
mod hooks;
mod hygiene;
mod manifest;
mod skills;
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
}

/// Plugin mode: run all 25 validators plus .claude/ checks.
fn run_plugin(ctx: &LintContext, diag: &mut DiagnosticCollector) {
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
}
