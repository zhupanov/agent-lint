//! Shared extraction of repository-resolvable paths from Claude hook commands.
//!
//! Both H004/H005 validation and H005 autofix consume this module so they
//! agree about which hook commands refer to a repository file.

use serde_json::Value;
use std::path::PathBuf;

use crate::script_paths::{Invocation, extract_command_references};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HookCommandPath {
    pub(crate) reference: String,
    pub(crate) path: PathBuf,
    pub(crate) invocation: Invocation,
}

/// Extract repository-relative script paths from command positions in the
/// supported hook layouts. This deliberately does not inspect descriptions,
/// matcher strings, arguments, or other arbitrary JSON values.
pub(crate) fn extract_hook_command_paths(value: &Value) -> Vec<HookCommandPath> {
    let mut commands = Vec::new();
    let Some(hooks) = value.get("hooks") else {
        return commands;
    };

    match hooks {
        // Legacy hook configuration: {"hooks": [{"command": "..."}]}.
        Value::Array(entries) => {
            for entry in entries {
                collect_hook_object_paths(entry, &mut commands);
            }
        }
        // Canonical hook configuration: event -> matcher group -> hooks.
        Value::Object(events) => {
            for groups in events.values() {
                let Some(groups) = groups.as_array() else {
                    continue;
                };
                for group in groups {
                    let Some(hooks) = group.get("hooks").and_then(Value::as_array) else {
                        continue;
                    };
                    for hook in hooks {
                        collect_hook_object_paths(hook, &mut commands);
                    }
                }
            }
        }
        _ => {}
    }
    commands
}

fn collect_hook_object_paths(value: &Value, paths: &mut Vec<HookCommandPath>) {
    let Some(hook) = value.as_object() else {
        return;
    };
    let Some(command) = hook.get("command") else {
        return;
    };
    collect_command_value_paths(command, paths);
}

/// Accept the regular string form and the common exec form, where the command
/// is nested with its `args`, or where the executable is the first array item.
fn collect_command_value_paths(command: &Value, paths: &mut Vec<HookCommandPath>) {
    match command {
        Value::String(command) => extract_paths_from_command(command, paths),
        Value::Object(exec) => {
            if let Some(command) = exec.get("command").and_then(Value::as_str) {
                extract_paths_from_command(command, paths);
            }
        }
        Value::Array(exec) => {
            if let Some(command) = exec.first().and_then(Value::as_str) {
                extract_paths_from_command(command, paths);
            }
        }
        _ => {}
    }
}

fn extract_paths_from_command(command: &str, paths: &mut Vec<HookCommandPath>) {
    for candidate in extract_command_references(command, 0) {
        paths.push(HookCommandPath {
            reference: candidate.reference,
            path: candidate.path,
            invocation: candidate.invocation,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::Path;

    #[test]
    fn extracts_documented_forms_only_from_command_positions() {
        let value = json!({
            "hooks": {
                "PreToolUse": [{"hooks": [
                    {"command": "${CLAUDE_PLUGIN_ROOT}/scripts/check.py arg"},
                    {"command": "\"${CLAUDE_PROJECT_DIR}\"/.claude/hooks/check-style"},
                    {"command": "\"${CLAUDE_PROJECT_DIR}/.claude/hooks/check style.sh\""},
                    {"command": "$PWD/bin/check"},
                    {"command": "$CLAUDE_PLUGIN_ROOT/scripts/brace-less"},
                    {"command": {"command": "$CLAUDE_PROJECT_DIR/bin/exec", "args": ["ignored"]}},
                    {"command": ["$PWD/bin/array-exec", "ignored"]}
                ]}]
            },
            "description": "${CLAUDE_PLUGIN_ROOT}/scripts/not-a-command.sh",
            "args": ["$PWD/not-a-command"]
        });

        let paths = extract_hook_command_paths(&value);
        assert_eq!(
            paths
                .iter()
                .map(|path| path.path.as_path())
                .collect::<Vec<_>>(),
            vec![
                Path::new("scripts/check.py"),
                Path::new(".claude/hooks/check-style"),
                Path::new(".claude/hooks/check style.sh"),
                Path::new("bin/check"),
                Path::new("scripts/brace-less"),
                Path::new("bin/exec"),
                Path::new("bin/array-exec"),
            ]
        );
    }

    #[test]
    fn preserves_unsafe_paths_and_invocation_classification() {
        let value = json!({"hooks": [{"command": "${CLAUDE_PLUGIN_DATA}/state ${CLAUDE_PLUGIN_ROOT}/../outside $PWD/ok"}]});
        let paths = extract_hook_command_paths(&value);
        assert_eq!(paths.len(), 2);
        assert!(paths[0].path.as_os_str().is_empty());
        assert_eq!(paths[0].invocation, Invocation::Mention);
        assert_eq!(paths[1].path, PathBuf::from("ok"));
        assert_eq!(paths[1].invocation, Invocation::Mention);
    }

    #[test]
    fn retains_invocation_state_for_hook_consumers() {
        let value = json!({"hooks": [{"command": "${CLAUDE_PLUGIN_ROOT}/scripts/direct; python3 ${CLAUDE_PLUGIN_ROOT}/scripts/interpreted.py; source ${CLAUDE_PLUGIN_ROOT}/scripts/library.sh; echo ${CLAUDE_PLUGIN_ROOT}/output.json"}]});
        assert_eq!(
            extract_hook_command_paths(&value)
                .into_iter()
                .map(|path| path.invocation)
                .collect::<Vec<_>>(),
            vec![
                Invocation::Direct,
                Invocation::Interpreter,
                Invocation::Sourced,
                Invocation::Mention,
            ]
        );
    }
}
