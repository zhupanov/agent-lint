//! Shared extraction of repository-resolvable paths from Claude hook commands.
//!
//! Both H004/H005 validation and H005 autofix consume this module so they
//! agree about which hook commands refer to a repository file.

use serde_json::Value;
use std::path::PathBuf;

use crate::script_paths::{Invocation, extract_command_references_with_ranges};
use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HookCommandPath {
    pub(crate) reference: String,
    pub(crate) path: PathBuf,
    pub(crate) invocation: Invocation,
    /// The JSON string value that supplied this command reference, when the
    /// parsed manifest retained its source text.
    pub(crate) source_range: Option<Range<usize>>,
}

/// Extract repository-relative script paths from command positions in the
/// supported hook layouts. This deliberately does not inspect descriptions,
/// matcher strings, arguments, or other arbitrary JSON values.
pub(crate) fn extract_hook_command_paths(
    value: &Value,
    source: Option<&str>,
) -> Vec<HookCommandPath> {
    let mut commands = Vec::new();
    let mut locator = SourceLocator::new(source);
    let Some(hooks) = value.get("hooks") else {
        return commands;
    };

    match hooks {
        // Legacy hook configuration: {"hooks": [{"command": "..."}]}.
        Value::Array(entries) => {
            for entry in entries {
                collect_hook_object_paths(entry, &mut commands, &mut locator);
            }
        }
        // Canonical hook configuration: event -> matcher group -> hooks.
        Value::Object(events) => {
            let mut events = events.iter().collect::<Vec<_>>();
            if let Some(source) = source {
                // serde_json's default map order is key-sorted, while source
                // locations must follow document order. Only event traversal
                // uses a map; matcher groups and handlers are arrays.
                events.sort_by_key(|(event, _)| {
                    let token = serde_json::to_string(event)
                        .expect("object keys always serialize as JSON strings");
                    source.find(&token).unwrap_or(usize::MAX)
                });
            }
            for (_, groups) in events {
                let Some(groups) = groups.as_array() else {
                    continue;
                };
                for group in groups {
                    let Some(hooks) = group.get("hooks").and_then(Value::as_array) else {
                        continue;
                    };
                    for hook in hooks {
                        collect_hook_object_paths(hook, &mut commands, &mut locator);
                    }
                }
            }
        }
        _ => {}
    }
    commands
}

fn collect_hook_object_paths(
    value: &Value,
    paths: &mut Vec<HookCommandPath>,
    locator: &mut SourceLocator<'_>,
) {
    let Some(hook) = value.as_object() else {
        return;
    };
    let Some(command) = hook.get("command") else {
        return;
    };
    let args = hook.get("args");
    collect_command_value_paths(command, args, paths, locator);
}

/// Accept the regular string form and the common exec form, where the command
/// is nested with its `args`, or where the executable is the first array item.
fn collect_command_value_paths(
    command: &Value,
    sibling_args: Option<&Value>,
    paths: &mut Vec<HookCommandPath>,
    locator: &mut SourceLocator<'_>,
) {
    match command {
        Value::String(command) => {
            collect_fragments(command, sibling_args, paths, locator);
        }
        Value::Object(exec) => {
            if let Some(command) = exec.get("command").and_then(Value::as_str) {
                collect_fragments(command, exec.get("args"), paths, locator);
            }
        }
        Value::Array(exec) => {
            if let Some(command) = exec.first().and_then(Value::as_str) {
                let args = Value::Array(exec.iter().skip(1).cloned().collect());
                collect_fragments(command, Some(&args), paths, locator);
            }
        }
        _ => {}
    }
}

fn collect_fragments(
    command: &str,
    args: Option<&Value>,
    paths: &mut Vec<HookCommandPath>,
    locator: &mut SourceLocator<'_>,
) {
    let mut fragments = vec![command];
    if let Some(args) = args.and_then(Value::as_array) {
        fragments.extend(args.iter().filter_map(Value::as_str));
    }
    let ranges = fragments
        .iter()
        .map(|fragment| locator.locate(fragment))
        .collect::<Vec<_>>();
    let mut command = String::new();
    let mut fragment_offsets = Vec::new();
    for fragment in fragments {
        if !command.is_empty() {
            command.push(' ');
        }
        let start = command.len();
        command.push_str(fragment);
        fragment_offsets.push(start..command.len());
    }
    for (candidate, candidate_range) in extract_command_references_with_ranges(&command, 0) {
        let source_range = fragment_offsets
            .iter()
            .position(|range| {
                range.start <= candidate_range.start && candidate_range.start < range.end
            })
            .and_then(|index| ranges[index].clone());
        paths.push(HookCommandPath {
            reference: candidate.reference,
            path: candidate.path,
            invocation: candidate.invocation,
            source_range,
        });
    }
}

/// Finds JSON string tokens in document order. We use serialized tokens so
/// escaping is respected and equal command/argument strings consume distinct
/// source occurrences rather than all pointing at the first one.
struct SourceLocator<'a> {
    source: Option<&'a str>,
    search_from: usize,
}

impl<'a> SourceLocator<'a> {
    fn new(source: Option<&'a str>) -> Self {
        Self {
            source,
            search_from: 0,
        }
    }

    fn locate(&mut self, value: &str) -> Option<Range<usize>> {
        let source = self.source?;
        let token = serde_json::to_string(value).expect("strings always serialize as JSON");
        let start = source[self.search_from..]
            .find(&token)
            .map(|offset| self.search_from + offset)?;
        let end = start + token.len();
        self.search_from = end;
        Some(start..end)
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

        let paths = extract_hook_command_paths(&value, None);
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
        let paths = extract_hook_command_paths(&value, None);
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
            extract_hook_command_paths(&value, None)
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

    #[test]
    fn combines_exec_form_arguments_without_treating_data_as_scripts() {
        let value = json!({"hooks": {"PreToolUse": [{"hooks": [
            {"command": "python3", "args": ["${CLAUDE_PLUGIN_ROOT}/scripts/interpreted.py"]},
            {"command": {"command": "source", "args": ["${CLAUDE_PLUGIN_ROOT}/scripts/library.sh"]}},
            {"command": ["${CLAUDE_PLUGIN_ROOT}/scripts/direct", "--checked"]},
            {"command": "echo", "args": ["${CLAUDE_PLUGIN_ROOT}/generated/data.json"]}
        ]}]}});

        assert_eq!(
            extract_hook_command_paths(&value, None)
                .into_iter()
                .map(|path| (path.path, path.invocation))
                .collect::<Vec<_>>(),
            vec![
                (
                    PathBuf::from("scripts/interpreted.py"),
                    Invocation::Interpreter
                ),
                (PathBuf::from("scripts/library.sh"), Invocation::Sourced),
                (PathBuf::from("scripts/direct"), Invocation::Direct),
                (PathBuf::from("generated/data.json"), Invocation::Mention),
            ]
        );
    }
}
