//! Shared extraction of repository-resolvable paths from Claude hook commands.
//!
//! Both H004/H005 validation and H005 autofix consume this module so they
//! agree about which hook commands refer to a repository file.

use serde_json::Value;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HookCommandPath {
    pub(crate) reference: String,
    pub(crate) path: PathBuf,
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
    let mut offset = 0;
    while let Some(candidate) = next_reference(command, offset) {
        offset = candidate.end;
        let rel = Path::new(&candidate.path);
        if is_repository_relative(rel) {
            paths.push(HookCommandPath {
                reference: candidate.reference,
                path: rel.to_path_buf(),
            });
        }
    }
}

struct ReferenceCandidate {
    reference: String,
    path: String,
    end: usize,
}

fn next_reference(command: &str, start: usize) -> Option<ReferenceCandidate> {
    const PREFIXES: &[(&str, &str)] = &[
        ("\"${CLAUDE_PLUGIN_ROOT}\"/", "\"${CLAUDE_PLUGIN_ROOT}\"/"),
        ("\"${CLAUDE_PROJECT_DIR}\"/", "\"${CLAUDE_PROJECT_DIR}\"/"),
        ("\"$CLAUDE_PLUGIN_ROOT/", "\"$CLAUDE_PLUGIN_ROOT/"),
        ("\"$CLAUDE_PROJECT_DIR/", "\"$CLAUDE_PROJECT_DIR/"),
        ("\"$PWD/", "\"$PWD/"),
        ("\"${CLAUDE_PLUGIN_ROOT}/", "\"${CLAUDE_PLUGIN_ROOT}/"),
        ("\"${CLAUDE_PROJECT_DIR}/", "\"${CLAUDE_PROJECT_DIR}/"),
        ("${CLAUDE_PLUGIN_ROOT}/", "${CLAUDE_PLUGIN_ROOT}/"),
        ("${CLAUDE_PROJECT_DIR}/", "${CLAUDE_PROJECT_DIR}/"),
        ("$CLAUDE_PLUGIN_ROOT/", "$CLAUDE_PLUGIN_ROOT/"),
        ("$CLAUDE_PROJECT_DIR/", "$CLAUDE_PROJECT_DIR/"),
        ("$PWD/", "$PWD/"),
    ];

    let tail = &command[start..];
    let (relative_start, prefix) = PREFIXES
        .iter()
        .filter_map(|(needle, prefix)| tail.find(needle).map(|index| (index, *prefix)))
        .min_by_key(|(index, _)| *index)?;
    let match_start = start + relative_start;
    let path_start = match_start + prefix.len();
    let whole_path_is_quoted = prefix.starts_with('"') && !prefix.contains("}\"/");
    let path_end = if whole_path_is_quoted {
        command[path_start..]
            .find('"')
            .map(|index| path_start + index)
            .unwrap_or(command.len())
    } else {
        command[path_start..]
            .find(is_shell_path_delimiter)
            .map(|index| path_start + index)
            .unwrap_or(command.len())
    };
    let path = &command[path_start..path_end];
    if path.is_empty() {
        return next_reference(command, path_start);
    }
    let end = if whole_path_is_quoted && path_end < command.len() {
        path_end + '"'.len_utf8()
    } else {
        path_end
    };
    Some(ReferenceCandidate {
        reference: command[match_start..end].to_string(),
        path: path.to_string(),
        end,
    })
}

fn is_shell_path_delimiter(character: char) -> bool {
    character.is_whitespace()
        || matches!(
            character,
            '"' | '\'' | '`' | ';' | '|' | '&' | '<' | '>' | '(' | ')'
        )
}

fn is_repository_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path.components().all(|component| {
            !matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
    fn ignores_runtime_roots_and_unsafe_paths() {
        let value = json!({"hooks": [{"command": "${CLAUDE_PLUGIN_DATA}/state ${CLAUDE_PLUGIN_ROOT}/../outside $PWD/ok"}]});
        let paths = extract_hook_command_paths(&value);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].path, PathBuf::from("ok"));
    }
}
