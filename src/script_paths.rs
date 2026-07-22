//! Shared, repository-safe extraction of script references.
//!
//! This is deliberately lexical: a reference may not escape the repository,
//! but symlink targets are left to the filesystem operation that consumes it.

use regex::Regex;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

static ASSIGNMENT_WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*=").unwrap());

/// The file kinds shared by script discovery and the hygiene and portability
/// rules.  Match full filename suffixes because `Path::extension` cannot
/// distinguish `.inc.bash` from an ordinary `.bash` file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScriptKind {
    Shell,
    Awk,
    Other,
}

pub(crate) fn script_kind(path: &Path) -> Option<ScriptKind> {
    let value = path.to_string_lossy();
    if value.ends_with(".sh") || value.ends_with(".bash") || value.ends_with(".inc.bash") {
        Some(ScriptKind::Shell)
    } else if value.ends_with(".awk") {
        Some(ScriptKind::Awk)
    } else if value.ends_with(".py")
        || value.ends_with(".js")
        || value.ends_with(".mjs")
        || path.extension().is_none()
    {
        Some(ScriptKind::Other)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Invocation {
    Direct,
    Interpreter,
    Sourced,
    Mention,
}

/// The lexical base used to resolve a script reference.  Consumers that know
/// a more specific local base (such as a SKILL.md directory) may give only
/// relative references an additional lookup location.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScriptReferenceBase {
    RepositoryRoot,
    Relative,
    Absolute,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScriptReference {
    pub(crate) reference: String,
    pub(crate) path: PathBuf,
    pub(crate) base: ScriptReferenceBase,
    pub(crate) invocation: Invocation,
    pub(crate) line: usize,
}

impl ScriptReference {
    /// True for the shipped-script kinds whose flag signatures S059 can
    /// inspect. Keeping this classification here prevents consumers from
    /// drifting on path spellings or file kinds.
    pub(crate) fn is_flag_signature_script(&self) -> bool {
        matches!(
            self.path
                .extension()
                .and_then(|extension| extension.to_str()),
            Some("sh") | Some("py")
        )
    }
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
                base: ScriptReferenceBase::RepositoryRoot,
                invocation: invocation_for(command, start),
                line,
            });
            continue;
        };
        references.push(ScriptReference {
            reference,
            path,
            base: ScriptReferenceBase::RepositoryRoot,
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
    let mut start = 0;
    while start < command.len() {
        let end = command[start..]
            .find(is_shell_path_delimiter)
            .map(|index| start + index)
            .unwrap_or(command.len());
        let reference =
            command[start..end].trim_matches(|character| matches!(character, '\'' | '"'));
        let relative = reference.strip_prefix("./").unwrap_or(reference);
        if next_reference(reference, 0).is_none()
            && (relative.starts_with("scripts/") || relative.contains("/scripts/"))
            && let Some(path) = normalize_repository_path(relative)
        {
            references.push(ScriptReference {
                reference: reference.to_string(),
                path,
                base: ScriptReferenceBase::Relative,
                invocation: invocation_for(command, start),
                line,
            });
        }
        start = end.saturating_add(1);
    }
    references
}

/// Extract the normalized shipped-script reference represented by one shell
/// token. This is the shared lexical boundary for script consumers; it
/// recognizes documented root placeholders, relative `scripts/` spellings,
/// and the historically supported absolute form.
pub(crate) fn extract_script_token_references(token: &str, line: usize) -> Vec<ScriptReference> {
    let mut references = extract_command_references(token, line);
    references.extend(extract_bare_script_references(token, line));
    if references.is_empty() {
        let path = Path::new(token);
        if path.is_absolute() {
            references.push(ScriptReference {
                reference: token.to_string(),
                path: path.to_path_buf(),
                base: ScriptReferenceBase::Absolute,
                invocation: Invocation::Direct,
                line,
            });
        }
    }
    references.retain(ScriptReference::is_flag_signature_script);
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
        "'${CLAUDE_PLUGIN_ROOT}'/",
        "'${CLAUDE_PROJECT_DIR}'/",
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
    let path_quote = prefix
        .chars()
        .next()
        .filter(|quote| matches!(quote, '\'' | '"'));
    let whole_path_is_quoted =
        path_quote.is_some() && !prefix.contains("}\"/") && !prefix.contains("}'/");
    let path_end = if whole_path_is_quoted {
        let quote = path_quote.expect("whole quoted path has an opening quote");
        command[path_start..]
            .find(quote)
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
    let raw_preceding = &command[..reference_start];
    let preceding = raw_preceding.trim_end();
    let separated_from_previous_word = raw_preceding.len() != preceding.len();
    let mut words = preceding
        .trim_matches('"')
        .split_whitespace()
        .collect::<Vec<_>>();
    if words
        .last()
        .is_some_and(|word| ASSIGNMENT_WORD.is_match(word))
        && !separated_from_previous_word
    {
        return Invocation::Mention;
    }
    while words
        .first()
        .is_some_and(|word| ASSIGNMENT_WORD.is_match(word))
    {
        words.remove(0);
    }
    let context = words
        .iter()
        .rev()
        .copied()
        .find(|word| !ASSIGNMENT_WORD.is_match(word))
        .unwrap_or("");
    if matches!(context, "source" | ".") {
        Invocation::Sourced
    } else if matches!(
        context,
        "bash" | "sh" | "zsh" | "fish" | "python" | "python3" | "node" | "ruby" | "perl"
    ) {
        Invocation::Interpreter
    } else if context.starts_with('-') || matches!(context, "[[" | "[" | "test" | "if" | "while") {
        Invocation::Mention
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
            "\"${CLAUDE_PLUGIN_ROOT}\"/scripts/check.py '${CLAUDE_PLUGIN_ROOT}/scripts/single.sh' $CLAUDE_PROJECT_DIR/bin/tool ${CLAUDE_PLUGIN_ROOT}/../outside",
            7,
        );
        assert_eq!(refs[0].path, PathBuf::from("scripts/check.py"));
        assert_eq!(refs[1].path, PathBuf::from("scripts/single.sh"));
        assert_eq!(refs[2].path, PathBuf::from("bin/tool"));
        assert!(refs[3].path.as_os_str().is_empty());
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

    #[test]
    fn classifies_env_prefixed_direct_calls() {
        assert_eq!(
            extract_command_references("FOO=1 ${CLAUDE_PLUGIN_ROOT}/scripts/a", 1)[0].invocation,
            Invocation::Direct
        );
    }

    #[test]
    fn does_not_treat_assignment_values_or_conditionals_as_direct_calls() {
        assert_eq!(
            extract_command_references("FOO=${CLAUDE_PLUGIN_ROOT}/scripts/a", 1)[0].invocation,
            Invocation::Mention
        );
        assert_eq!(
            extract_command_references("if FOO=1 ${CLAUDE_PLUGIN_ROOT}/scripts/a", 1)[0].invocation,
            Invocation::Mention
        );
    }

    #[test]
    fn classifies_full_name_script_suffixes() {
        assert_eq!(
            script_kind(Path::new("scripts/a.bash")),
            Some(ScriptKind::Shell)
        );
        assert_eq!(
            script_kind(Path::new("scripts/a.inc.bash")),
            Some(ScriptKind::Shell)
        );
        assert_eq!(
            script_kind(Path::new("scripts/a.awk")),
            Some(ScriptKind::Awk)
        );
        assert_eq!(script_kind(Path::new("scripts/a")), Some(ScriptKind::Other));
        assert_eq!(script_kind(Path::new("scripts/a.txt")), None);
    }
}
