use crate::diagnostic::DiagnosticCollector;
use crate::frontmatter;
use crate::rules::LintRule;
use crate::validators::common::{RE_NAME_INVALID, is_valid_model_value};
use crate::validators::skills::SkillInfo;
use std::path::Path;

use super::KNOWN_SKILL_FRONTMATTER_FIELDS;

/// Built-in subagent types for `context: fork` (Claude Code docs).
const BUILTIN_AGENTS: &[&str] = &["Explore", "Plan", "general-purpose"];

/// Hyphen-delimited name segments that imply side effects (CC-SK-006 / S066).
const SIDE_EFFECT_SEGMENTS: &[&str] = &[
    "deploy", "ship", "publish", "delete", "drop", "destroy", "remove", "revoke", "purge",
];

pub(super) fn check_frontmatter_fields(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    // S023: boolean fields
    for field_name in &["user-invocable", "disable-model-invocation"] {
        match frontmatter::get_field_state(&info.fm_lines, field_name) {
            frontmatter::FieldState::Value(val) => {
                if val != "true" && val != "false" {
                    diag.report(
                        LintRule::BoolFieldInvalid,
                        &format!(
                            "{}: '{}' must be true or false, got '{}'",
                            info.path, field_name, val
                        ),
                    );
                }
            }
            frontmatter::FieldState::Empty => {
                diag.report(
                    LintRule::BoolFieldInvalid,
                    &format!(
                        "{}: '{}' is present but empty (must be true or false)",
                        info.path, field_name
                    ),
                );
            }
            frontmatter::FieldState::Missing => {} // Not required
        }
    }

    // S024: context field
    match frontmatter::get_field_state(&info.fm_lines, "context") {
        frontmatter::FieldState::Value(val) => {
            if val != "fork" {
                diag.report(
                    LintRule::ContextFieldInvalid,
                    &format!("{}: 'context' must be 'fork', got '{}'", info.path, val),
                );
            }
        }
        frontmatter::FieldState::Empty => {
            diag.report(
                LintRule::ContextFieldInvalid,
                &format!(
                    "{}: 'context' is present but empty (must be 'fork')",
                    info.path
                ),
            );
        }
        frontmatter::FieldState::Missing => {}
    }

    // S025: effort field (Claude Code docs: low/medium/high/xhigh/max)
    match frontmatter::get_field_state(&info.fm_lines, "effort") {
        frontmatter::FieldState::Value(val) => {
            if !["low", "medium", "high", "xhigh", "max"].contains(&val.as_str()) {
                diag.report(
                    LintRule::EffortFieldInvalid,
                    &format!(
                        "{}: 'effort' must be low/medium/high/xhigh/max, got '{}'",
                        info.path, val
                    ),
                );
            }
        }
        frontmatter::FieldState::Empty => {
            diag.report(
                LintRule::EffortFieldInvalid,
                &format!("{}: 'effort' is present but empty", info.path),
            );
        }
        frontmatter::FieldState::Missing => {}
    }

    // S026: shell field
    match frontmatter::get_field_state(&info.fm_lines, "shell") {
        frontmatter::FieldState::Value(val) => {
            if !["bash", "powershell"].contains(&val.as_str()) {
                diag.report(
                    LintRule::ShellFieldInvalid,
                    &format!(
                        "{}: 'shell' must be bash/powershell, got '{}'",
                        info.path, val
                    ),
                );
            }
        }
        frontmatter::FieldState::Empty => {
            diag.report(
                LintRule::ShellFieldInvalid,
                &format!("{}: 'shell' is present but empty", info.path),
            );
        }
        frontmatter::FieldState::Missing => {}
    }

    // S027: unreachable skill
    let dmi = frontmatter::get_field(&info.fm_lines, "disable-model-invocation");
    let ui = frontmatter::get_field(&info.fm_lines, "user-invocable");
    if dmi.as_deref() == Some("true") && ui.as_deref() == Some("false") {
        diag.report(
            LintRule::SkillUnreachable,
            &format!(
                "{}: skill is unreachable (disable-model-invocation: true and user-invocable: false)",
                info.path
            ),
        );
    }

    check_model_field(info, diag);
    check_agent_context_pairing(info, diag);
    check_agent_value(info, diag);
    check_side_effect_auto(info, diag);
    check_unknown_fields(info, diag);
    check_paths_empty(info, diag);
}

fn check_model_field(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    match frontmatter::get_field_state(&info.fm_lines, "model") {
        frontmatter::FieldState::Value(val) => {
            if !is_valid_model_value(&val) {
                diag.report(
                    LintRule::ModelInvalid,
                    &format!(
                        "{}: 'model' must be a recognized alias (sonnet/opus/haiku/inherit/…) or claude-… ID, got '{}'",
                        info.path, val
                    ),
                );
            }
        }
        frontmatter::FieldState::Empty => {
            diag.report(
                LintRule::ModelInvalid,
                &format!("{}: 'model' is present but empty", info.path),
            );
        }
        frontmatter::FieldState::Missing => {}
    }
}

fn check_agent_context_pairing(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    // S064: agent without context: fork (dead field). CC-SK-003 (fork without agent)
    // is intentionally not implemented — docs default agent to general-purpose.
    let has_agent = frontmatter::field_exists(&info.fm_lines, "agent");
    if !has_agent {
        return;
    }
    let context_ok = frontmatter::get_field(&info.fm_lines, "context").as_deref() == Some("fork");
    if !context_ok {
        diag.report(
            LintRule::AgentNoFork,
            &format!(
                "{}: 'agent' is set without 'context: fork' (agent only applies in a forked subagent)",
                info.path
            ),
        );
    }
}

fn agents_dir_for_skill(skill_path: &str) -> &str {
    if skill_path.starts_with(".claude/skills/") {
        ".claude/agents"
    } else {
        "agents"
    }
}

fn check_agent_value(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    let agent = match frontmatter::get_field_state(&info.fm_lines, "agent") {
        frontmatter::FieldState::Value(v) => {
            let trimmed = v.trim();
            if trimmed.is_empty() {
                diag.report(
                    LintRule::AgentUnknown,
                    &format!("{}: 'agent' is present but empty", info.path),
                );
                return;
            }
            trimmed.to_string()
        }
        frontmatter::FieldState::Empty => {
            diag.report(
                LintRule::AgentUnknown,
                &format!("{}: 'agent' is present but empty", info.path),
            );
            return;
        }
        frontmatter::FieldState::Missing => return,
    };

    if BUILTIN_AGENTS.contains(&agent.as_str()) {
        return;
    }

    // Custom agents must be kebab-case and exist on disk.
    if RE_NAME_INVALID.is_match(&agent) || agent.starts_with('-') || agent.ends_with('-') {
        diag.report(
            LintRule::AgentUnknown,
            &format!(
                "{}: 'agent' must be a built-in (Explore/Plan/general-purpose) or kebab-case custom name, got '{}'",
                info.path, agent
            ),
        );
        return;
    }

    let agents_dir = agents_dir_for_skill(&info.path);
    let agent_path = Path::new(agents_dir).join(format!("{agent}.md"));
    if !agent_path.is_file() {
        diag.report(
            LintRule::AgentUnknown,
            &format!(
                "{}: custom agent '{}' not found in {}/ (expected {}/{}.md)",
                info.path, agent, agents_dir, agents_dir, agent
            ),
        );
    }
}

fn name_has_side_effect_segment(name: &str) -> bool {
    name.split('-')
        .any(|seg| SIDE_EFFECT_SEGMENTS.contains(&seg))
}

fn check_side_effect_auto(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    if !name_has_side_effect_segment(&info.dir_name) {
        return;
    }
    if frontmatter::get_field(&info.fm_lines, "disable-model-invocation").as_deref() == Some("true")
    {
        return;
    }
    diag.report(
        LintRule::SideEffectAuto,
        &format!(
            "{}: side-effect-named skill should set disable-model-invocation: true to prevent auto-invocation",
            info.path
        ),
    );
}

fn check_unknown_fields(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    for line in &info.fm_lines {
        if line.is_empty()
            || line.starts_with(' ')
            || line.starts_with('\t')
            || line.starts_with('#')
        {
            continue;
        }
        let Some(colon) = line.find(':') else {
            continue;
        };
        let key = &line[..colon];
        if key.is_empty() {
            continue;
        }
        if !KNOWN_SKILL_FRONTMATTER_FIELDS.contains(&key) {
            diag.report(
                LintRule::UnknownFmField,
                &format!(
                    "{}: unknown skill frontmatter field '{}' (possible typo; known fields include name, description, model, agent, …)",
                    info.path, key
                ),
            );
        }
    }
}

/// True when indented YAML list items follow `key:` (possibly after blanks).
fn has_yaml_list_items(fm_lines: &[String], key: &str) -> bool {
    let prefix = format!("{key}:");
    fm_lines
        .iter()
        .position(|l| l.starts_with(&prefix))
        .is_some_and(|i| {
            fm_lines[i + 1..]
                .iter()
                .take_while(|l| {
                    l.is_empty() || l.starts_with(' ') || l.starts_with('\t') || l.starts_with("- ")
                })
                .any(|l| {
                    let trimmed = l.trim_start();
                    trimmed.starts_with("- ") && trimmed.len() > 2
                })
        })
}

fn check_paths_empty(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    if !frontmatter::field_exists(&info.fm_lines, "paths") {
        return;
    }
    if frontmatter::get_field(&info.fm_lines, "paths").is_some() {
        return;
    }
    if has_yaml_list_items(&info.fm_lines, "paths") {
        return;
    }
    diag.report(
        LintRule::PathsEmpty,
        &format!(
            "{}: 'paths' is present but empty (provide glob patterns or remove the field)",
            info.path
        ),
    );
}
