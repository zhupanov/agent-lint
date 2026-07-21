//! Shared, repository-safe extraction of script references.
//!
//! This is deliberately lexical: a reference may not escape the repository,
//! but symlink targets are left to the filesystem operation that consumes it.

use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Invocation {
    Direct,
    Interpreter,
    Sourced,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScriptReference {
    pub(crate) reference: String,
    pub(crate) path: PathBuf,
    pub(crate) invocation: Invocation,
    pub(crate) line: usize,
}

/// Extract root-qualified script paths from a command-like fragment.  The
/// caller decides which repository surfaces are command-like; this function
/// never scans arbitrary prose on its own.
pub(crate) fn extract_command_references(command: &str, line: usize) -> Vec<ScriptReference> {
    let mut references = Vec::new();
    let mut offset = 0;
    while let Some((reference, path, start, end)) = next_reference(command, offset) {
        offset = end;
        let Some(path) = normalize_repository_path(&path) else {
            // Keep an escaping root-qualified reference so G002 can explain
            // that it is unresolvable, without ever probing outside the root.
            references.push(ScriptReference {
                reference,
                path: PathBuf::new(),
                invocation: invocation_for(command, start),
                line,
            });
            continue;
        };
        references.push(ScriptReference {
            reference,
            path,
            invocation: invocation_for(command, start),
            line,
        });
    }
    references
}

/// Extract a relative `scripts/...` token used by Makefiles, workflows, and
/// shell-to-shell calls. This is kept beside root extraction so every script
/// rule gets the same lexical safety boundary.
pub(crate) fn extract_bare_script_references(command: &str, line: usize) -> Vec<ScriptReference> {
    let mut references = Vec::new();
    let mut offset = 0;
    while let Some(found) = command[offset..].find("scripts/") {
        let start = offset + found;
        if start > 0
            && (command.as_bytes()[start - 1].is_ascii_alphanumeric()
                || matches!(command.as_bytes()[start - 1], b'/' | b'.'))
        {
            offset = start + "scripts/".len();
            continue;
        }
        let end = command[start..]
            .find(is_shell_path_delimiter)
            .map(|index| start + index)
            .unwrap_or(command.len());
        let reference = &command[start..end];
        if let Some(path) = normalize_repository_path(reference) {
            references.push(ScriptReference {
                reference: reference.to_string(),
                path,
                invocation: invocation_for(command, start),
                line,
            });
        }
        offset = end.max(start + "scripts/".len());
    }
    references
}

/// Normalize a repository-relative path lexically.  It intentionally does
/// not canonicalize, so a symlink is checked with normal `is_file`/metadata
/// semantics while `..` escapes are always rejected before filesystem access.
pub(crate) fn normalize_repository_path(raw: &str) -> Option<PathBuf> {
    let path = Path::new(raw);
    if path.as_os_str().is_empty() || path.is_absolute() {
        return None;
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!normalized.as_os_str().is_empty()).then_some(normalized)
}

fn next_reference(command: &str, start: usize) -> Option<(String, String, usize, usize)> {
    const PREFIXES: &[&str] = &[
        "\"${CLAUDE_PLUGIN_ROOT}\"/",
        "\"${CLAUDE_PROJECT_DIR}\"/",
        "\"$CLAUDE_PLUGIN_ROOT/",
        "\"$CLAUDE_PROJECT_DIR/",
        "\"$PWD/",
        "\"${CLAUDE_PLUGIN_ROOT}/",
        "\"${CLAUDE_PROJECT_DIR}/",
        "${CLAUDE_PLUGIN_ROOT_PLACEHOLDER:-$PWD}/",
        "${CLAUDE_PLUGIN_ROOT}/",
        "${CLAUDE_PROJECT_DIR}/",
        "$CLAUDE_PLUGIN_ROOT/",
        "$CLAUDE_PROJECT_DIR/",
        "$PWD/",
    ];
    let tail = &command[start..];
    let (relative_start, prefix) = PREFIXES
        .iter()
        .filter_map(|prefix| tail.find(prefix).map(|index| (index, *prefix)))
        .min_by_key(|(index, _)| *index)?;
    let match_start = start + relative_start;
    let path_start = match_start + prefix.len();
    let whole_path_is_quoted = prefix.starts_with('"') && !prefix.contains("}\"/");
    let path_end = if whole_path_is_quoted {
        command[path_start..]
            .find('"')
            .map(|i| path_start + i)
            .unwrap_or(command.len())
    } else {
        command[path_start..]
            .find(is_shell_path_delimiter)
            .map(|i| path_start + i)
            .unwrap_or(command.len())
    };
    if path_start == path_end {
        return next_reference(command, path_end);
    }
    let end = if whole_path_is_quoted && path_end < command.len() {
        path_end + 1
    } else {
        path_end
    };
    Some((
        command[match_start..end].to_string(),
        command[path_start..path_end].to_string(),
        match_start,
        end,
    ))
}

fn is_shell_path_delimiter(character: char) -> bool {
    character.is_whitespace()
        || matches!(
            character,
            '"' | '\'' | '`' | ';' | '|' | '&' | '<' | '>' | '(' | ')'
        )
}

fn invocation_for(command: &str, reference_start: usize) -> Invocation {
    let preceding = command[..reference_start].trim_end();
    let first = preceding
        .trim_matches('"')
        .split_whitespace()
        .last()
        .unwrap_or("");
    if matches!(first, "source" | ".") {
        Invocation::Sourced
    } else if matches!(
        first,
        "bash" | "sh" | "zsh" | "fish" | "python" | "python3" | "node" | "ruby" | "perl"
    ) {
        Invocation::Interpreter
    } else {
        Invocation::Direct
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_documented_roots_and_rejects_escapes() {
        let refs = extract_command_references(
            "\"${CLAUDE_PLUGIN_ROOT}\"/scripts/check.py $CLAUDE_PROJECT_DIR/bin/tool ${CLAUDE_PLUGIN_ROOT}/../outside",
            7,
        );
        assert_eq!(refs[0].path, PathBuf::from("scripts/check.py"));
        assert_eq!(refs[1].path, PathBuf::from("bin/tool"));
        assert!(refs[2].path.as_os_str().is_empty());
        assert_eq!(refs[0].line, 7);
    }

    #[test]
    fn classifies_interpreter_and_source_calls() {
        assert_eq!(
            extract_command_references("bash ${CLAUDE_PLUGIN_ROOT}/scripts/a", 1)[0].invocation,
            Invocation::Interpreter
        );
        assert_eq!(
            extract_command_references("source ${CLAUDE_PLUGIN_ROOT}/scripts/a", 1)[0].invocation,
            Invocation::Sourced
        );
    }
}
