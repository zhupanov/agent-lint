use crate::diagnostic::DiagnosticCollector;
use crate::frontmatter;
use crate::rules::LintRule;
use crate::validators::common::{RE_NAME_INVALID, is_valid_model_value};
use crate::validators::skills::SkillInfo;
use crate::yaml::Mapping;
use std::path::Path;

use super::KNOWN_SKILL_FRONTMATTER_FIELDS;

/// Built-in subagent types for `context: fork` (Claude Code docs).
const BUILTIN_AGENTS: &[&str] = &["Explore", "Plan", "general-purpose"];

/// Hyphen-delimited name segments that imply side effects (CC-SK-006 / S066).
const SIDE_EFFECT_SEGMENTS: &[&str] = &[
    "deploy", "ship", "publish", "delete", "drop", "destroy", "remove", "revoke", "purge",
];

pub(super) fn check_frontmatter_fields(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    // These rules read canonical YAML values so comments, YAML-1.2 boolean
    // spellings, and quoting cannot leak into the compared value. Invalid or
    // non-mapping frontmatter is owned by X001/S004/S005, so they skip it.
    if let Some(map) = info.frontmatter_mapping() {
        check_bool_fields(info, map, diag);
        check_context_field(info, map, diag);
        check_effort_field(info, map, diag);
        check_shell_field(info, map, diag);
        check_unreachable(info, map, diag);
        check_model_field(info, map, diag);
        check_agent_context_pairing(info, map, diag);
        check_side_effect_auto(info, map, diag);
        check_unknown_fields(info, map, diag);
        check_paths_empty(info, map, diag);
    }
    // S065 (agent-unknown) stays line-oriented and is owned separately (#344);
    // it continues to run regardless of YAML validity.
    check_agent_value(info, diag);
}

/// S023: `user-invocable` / `disable-model-invocation` must be a YAML boolean
/// (any casing) or the accepted quoted strings `"true"`/`"false"`.
fn check_bool_fields(info: &SkillInfo, map: &Mapping, diag: &mut DiagnosticCollector) {
    for field_name in ["user-invocable", "disable-model-invocation"] {
        if let Some(value) = map.get(field_name) {
            if frontmatter::canonical_bool_value(value).is_none() {
                diag.report(
                    LintRule::BoolFieldInvalid,
                    &format!(
                        "{}: '{}' must be true or false, got '{}'",
                        info.path, field_name, value
                    ),
                );
            }
        }
    }
}

/// S024: `context` must be the string `fork`.
fn check_context_field(info: &SkillInfo, map: &Mapping, diag: &mut DiagnosticCollector) {
    if let Some(value) = map.get("context") {
        if value.as_str() != Some("fork") {
            diag.report(
                LintRule::ContextFieldInvalid,
                &format!("{}: 'context' must be 'fork', got '{}'", info.path, value),
            );
        }
    }
}

/// S025: `effort` must be one of low/medium/high/xhigh/max.
fn check_effort_field(info: &SkillInfo, map: &Mapping, diag: &mut DiagnosticCollector) {
    if let Some(value) = map.get("effort") {
        let valid = value
            .as_str()
            .is_some_and(|effort| ["low", "medium", "high", "xhigh", "max"].contains(&effort));
        if !valid {
            diag.report(
                LintRule::EffortFieldInvalid,
                &format!(
                    "{}: 'effort' must be low/medium/high/xhigh/max, got '{}'",
                    info.path, value
                ),
            );
        }
    }
}

/// S026: `shell` must be one of the recognized shells.
fn check_shell_field(info: &SkillInfo, map: &Mapping, diag: &mut DiagnosticCollector) {
    if let Some(value) = map.get("shell") {
        let valid = value
            .as_str()
            .is_some_and(|shell| crate::validators::common::VALID_SHELLS.contains(&shell));
        if !valid {
            diag.report(
                LintRule::ShellFieldInvalid,
                &format!(
                    "{}: 'shell' must be bash/powershell, got '{}'",
                    info.path, value
                ),
            );
        }
    }
}

/// S027: a skill reachable by neither model nor user is dead configuration.
fn check_unreachable(info: &SkillInfo, map: &Mapping, diag: &mut DiagnosticCollector) {
    let dmi = map
        .get("disable-model-invocation")
        .and_then(frontmatter::canonical_bool_value);
    let ui = map
        .get("user-invocable")
        .and_then(frontmatter::canonical_bool_value);
    if dmi == Some(true) && ui == Some(false) {
        diag.report(
            LintRule::SkillUnreachable,
            &format!(
                "{}: skill is unreachable (disable-model-invocation: true and user-invocable: false)",
                info.path
            ),
        );
    }
}

/// S063: `model` must be a recognized alias or `claude-…` id.
fn check_model_field(info: &SkillInfo, map: &Mapping, diag: &mut DiagnosticCollector) {
    if let Some(value) = map.get("model") {
        if !value.as_str().is_some_and(is_valid_model_value) {
            diag.report(
                LintRule::ModelInvalid,
                &format!(
                    "{}: 'model' must be a recognized alias (sonnet/opus/haiku/inherit/…) or claude-… ID, got '{}'",
                    info.path, value
                ),
            );
        }
    }
}

/// S064: `agent` only takes effect in a forked subagent. Runs only for a usable
/// agent declaration (a non-empty string scalar); empty/null/non-string agent
/// shapes are owned by S065 (#344).
fn check_agent_context_pairing(info: &SkillInfo, map: &Mapping, diag: &mut DiagnosticCollector) {
    let has_usable_agent = map
        .get("agent")
        .and_then(|value| value.as_str())
        .is_some_and(|agent| !agent.is_empty());
    if !has_usable_agent {
        return;
    }
    let context_ok = map.get("context").and_then(|value| value.as_str()) == Some("fork");
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

/// S066: a side-effect-named skill should opt out of automatic invocation.
fn check_side_effect_auto(info: &SkillInfo, map: &Mapping, diag: &mut DiagnosticCollector) {
    if !name_has_side_effect_segment(&info.dir_name) {
        return;
    }
    if map
        .get("disable-model-invocation")
        .and_then(frontmatter::canonical_bool_value)
        == Some(true)
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

/// S070: catch typo'd top-level frontmatter keys. Iterating the canonical
/// mapping keys means quoted (`"name"`) and spaced (`name :`) spellings are
/// read as their real key, not as unknown fields.
fn check_unknown_fields(info: &SkillInfo, map: &Mapping, diag: &mut DiagnosticCollector) {
    for key in map.keys() {
        if !KNOWN_SKILL_FRONTMATTER_FIELDS.contains(&key.as_str()) {
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

/// S071: `paths` present but with no usable glob. A non-empty string or a
/// sequence with at least one non-empty string passes; null, an empty string,
/// an empty sequence, or any other shape (mapping, sequence of non-strings)
/// fires.
fn check_paths_empty(info: &SkillInfo, map: &Mapping, diag: &mut DiagnosticCollector) {
    let Some(value) = map.get("paths") else {
        return;
    };
    let has_glob = if let Some(text) = value.as_str() {
        !text.is_empty()
    } else if let Some(items) = value.as_sequence() {
        items
            .iter()
            .any(|item| item.as_str().is_some_and(|text| !text.is_empty()))
    } else {
        false
    };
    if !has_glob {
        diag.report(
            LintRule::PathsEmpty,
            &format!(
                "{}: 'paths' is present but empty (provide glob patterns or remove the field)",
                info.path
            ),
        );
    }
}
