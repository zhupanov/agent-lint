//! Shared lexical handling for Claude plugin component declarations.
//!
//! This module deliberately classifies untrusted manifest strings without
//! touching the filesystem. Validators use the classification for M012/M013;
//! discovery consumers use [`safe_component_path`] so they cannot probe a
//! declaration that the safety layer rejects.

use crate::validators::json_locate::Seg;
use regex::Regex;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::LazyLock;

const PLUGIN_DIR: &str = ".claude-plugin";

static RE_WIN_DRIVE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z]:[\\/]").unwrap());

#[derive(Debug, Clone, Copy)]
pub(crate) struct ComponentPathField {
    pub label: &'static str,
    pub keys: &'static [&'static str],
}

/// Component fields accepted by both plugin manifests and marketplace entries.
pub(crate) const COMPONENT_PATH_FIELDS: &[ComponentPathField] = &[
    ComponentPathField {
        label: "commands",
        keys: &["commands"],
    },
    ComponentPathField {
        label: "agents",
        keys: &["agents"],
    },
    ComponentPathField {
        label: "skills",
        keys: &["skills"],
    },
    ComponentPathField {
        label: "hooks",
        keys: &["hooks"],
    },
    ComponentPathField {
        label: "mcpServers",
        keys: &["mcpServers"],
    },
    ComponentPathField {
        label: "outputStyles",
        keys: &["outputStyles"],
    },
    ComponentPathField {
        label: "lspServers",
        keys: &["lspServers"],
    },
    ComponentPathField {
        label: "experimental.themes",
        keys: &["experimental", "themes"],
    },
    ComponentPathField {
        label: "experimental.monitors",
        keys: &["experimental", "monitors"],
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeclaredComponentPath<'a> {
    pub label: String,
    pub raw: &'a str,
    /// Structural JSON path of the declared string inside its owning manifest
    /// value, so span recovery follows the owning field rather than searching
    /// the document for an equal value.
    pub path: Vec<Seg<'a>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ComponentPathSafety {
    Safe,
    Absolute,
    Traversal,
    MissingPrefix,
    NestedPluginDir,
}

/// Extract all documented component path forms in deterministic field/index
/// order. `commands` additionally supports an object whose entries declare a
/// path-bearing `source` member.
pub(crate) fn declared_component_paths(value: &Value) -> Vec<DeclaredComponentPath<'_>> {
    let mut paths = Vec::new();
    for field in COMPONENT_PATH_FIELDS {
        let field_path = || {
            field
                .keys
                .iter()
                .map(|key| Seg::Key(key))
                .collect::<Vec<_>>()
        };
        let value = field
            .keys
            .iter()
            .try_fold(value, |value, key| value.get(*key));
        match value {
            Some(Value::String(raw)) => paths.push(DeclaredComponentPath {
                label: field.label.to_string(),
                raw,
                path: field_path(),
            }),
            Some(Value::Array(items)) => {
                for (index, value) in items.iter().enumerate() {
                    if let Some(raw) = value.as_str() {
                        let mut path = field_path();
                        path.push(Seg::Index(index));
                        paths.push(DeclaredComponentPath {
                            label: format!("{}[{index}]", field.label),
                            raw,
                            path,
                        });
                    }
                }
            }
            Some(Value::Object(commands)) if field.label == "commands" => {
                for (name, command) in commands {
                    if let Some(raw) = command.get("source").and_then(Value::as_str) {
                        let mut path = field_path();
                        path.push(Seg::Key(name));
                        path.push(Seg::Key("source"));
                        paths.push(DeclaredComponentPath {
                            label: format!("commands.{name}.source"),
                            raw,
                            path,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    paths
}

pub(crate) fn path_segments(path: &str) -> impl Iterator<Item = &str> {
    path.split(['/', '\\'])
        .filter(|segment| !segment.is_empty() && *segment != ".")
}

/// Whether a declaration retains a usable component after normalization.
/// Dot-only forms normalize to no path and therefore cannot name a component.
pub(crate) fn has_normalized_path_segment(path: &str) -> bool {
    path_segments(path).next().is_some()
}

pub(crate) fn is_absolute_path(path: &str) -> bool {
    path.starts_with('/') || path.starts_with('\\') || RE_WIN_DRIVE.is_match(path)
}

/// Classify one component declaration. The match order is itself the M013
/// precedence contract.
pub(crate) fn classify_component_path(path: &str) -> ComponentPathSafety {
    if is_absolute_path(path) {
        ComponentPathSafety::Absolute
    } else if path_segments(path).any(|segment| segment == "..") {
        ComponentPathSafety::Traversal
    } else if !path.starts_with("./") {
        ComponentPathSafety::MissingPrefix
    } else if path_segments(path).next() == Some(PLUGIN_DIR) {
        ComponentPathSafety::NestedPluginDir
    } else {
        ComponentPathSafety::Safe
    }
}

/// Return a normalized, safe repository-relative probe path. Unsafe values
/// are rejected before any filesystem operation and separator styles agree.
pub(crate) fn safe_component_path(raw: &str) -> Option<PathBuf> {
    if classify_component_path(raw) != ComponentPathSafety::Safe {
        return None;
    }
    has_normalized_path_segment(raw).then(|| path_segments(raw).collect())
}

/// Classify `metadata.pluginRoot`; unlike a component path it may name the
/// manifest directory, but must still be a non-empty `./` relative path with
/// no parent traversal.
pub(crate) fn plugin_root_is_safe(root: &str) -> bool {
    !root.trim().is_empty()
        && root.starts_with("./")
        && !is_absolute_path(root)
        && !path_segments(root).any(|segment| segment == "..")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_string_array_and_command_object_forms() {
        let value = json!({
            "commands": {"z": {"source": "./z.md"}, "a": {"source": "./a.md"}},
            "skills": ["./one", "./two"],
        });
        let paths = declared_component_paths(&value);
        assert_eq!(
            paths.into_iter().map(|path| path.label).collect::<Vec<_>>(),
            vec![
                "commands.a.source",
                "commands.z.source",
                "skills[0]",
                "skills[1]"
            ]
        );
    }

    #[test]
    fn path_classification_has_documented_precedence() {
        assert_eq!(
            classify_component_path("/x/../y"),
            ComponentPathSafety::Absolute
        );
        assert_eq!(
            classify_component_path("x/../y"),
            ComponentPathSafety::Traversal
        );
        assert_eq!(
            classify_component_path("skills"),
            ComponentPathSafety::MissingPrefix
        );
        assert_eq!(
            classify_component_path("./.claude-plugin/x"),
            ComponentPathSafety::NestedPluginDir
        );
        assert_eq!(
            classify_component_path("./skills"),
            ComponentPathSafety::Safe
        );
    }
}
