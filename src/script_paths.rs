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
    ///
    /// Deliberately narrower than `script_kind`'s shell matrix: S059's
    /// documented contract covers `.sh` and `.py` flag signatures only, so
    /// admitting `.bash` here would broaden a rule contract rather than fix a
    /// dispatch gap (see #551, which scoped `.bash` parity to G008-G011).
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
    extract_references(command, line, false)
}

/// As [`extract_command_references`], retaining the byte span of each
/// reference in the command fragment. Hook JSON uses this to associate a
/// finding with the command or argument string that supplied it.
pub(crate) fn extract_command_references_with_ranges(
    command: &str,
    line: usize,
) -> Vec<(ScriptReference, std::ops::Range<usize>)> {
    let references = extract_command_references(command, line);
    let mut search_from = 0;
    references
        .into_iter()
        .filter_map(|reference| {
            let start = command[search_from..]
                .find(&reference.reference)
                .map(|offset| search_from + offset)?;
            let end = start + reference.reference.len();
            search_from = end;
            Some((reference, start..end))
        })
        .collect()
}

/// Extract root-qualified script paths from instruction and workflow command
/// surfaces. These also support the explicit prose `Run`/`Execute` and YAML
/// `run:` command markers that are not valid shell command words.
pub(crate) fn extract_instruction_command_references(
    command: &str,
    line: usize,
) -> Vec<ScriptReference> {
    extract_references(command, line, true)
}

fn extract_references(
    command: &str,
    line: usize,
    supports_instruction_markers: bool,
) -> Vec<ScriptReference> {
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
                invocation: invocation_for(command, start, supports_instruction_markers),
                line,
            });
            continue;
        };
        references.push(ScriptReference {
            reference,
            path,
            base: ScriptReferenceBase::RepositoryRoot,
            invocation: invocation_for(command, start, supports_instruction_markers),
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
        // `is_shell_path_delimiter` matches multibyte Unicode whitespace, so
        // advance past the delimiter by its full width (issue #600).
        let delimiter = command[start..]
            .char_indices()
            .find(|&(_, character)| is_shell_path_delimiter(character));
        let end = delimiter.map_or(command.len(), |(index, _)| start + index);
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
                invocation: invocation_for(command, start, false),
                line,
            });
        }
        start = delimiter.map_or(command.len(), |(index, character)| {
            start + index + character.len_utf8()
        });
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
    // Instruction prose commonly terminates an unquoted invocation with a
    // sentence mark (for example `Run ${CLAUDE_PLUGIN_ROOT}/scripts/check.sh.`).
    // Keep quoted shell tokens byte-exact, but exclude only terminal prose
    // punctuation from the lexical path before repository normalization.
    let normalized_path_end = if whole_path_is_quoted {
        path_end
    } else {
        let raw = &command[path_start..path_end];
        path_start + raw.trim_end_matches(['.', ',', '!', '?', ':']).len()
    };
    Some((
        command[match_start..normalized_path_end].to_string(),
        command[path_start..normalized_path_end].to_string(),
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

fn invocation_for(
    command: &str,
    reference_start: usize,
    supports_instruction_markers: bool,
) -> Invocation {
    let segment_start = command[..reference_start]
        .rfind([';', '|', '&', '\n'])
        .map_or(0, |index| index + 1);
    let segment = &command[segment_start..reference_start];
    // The trailing whitespace scalar can be multibyte, so step past it by its
    // full width rather than one byte (issue #600).
    let word_start = segment
        .char_indices()
        .rev()
        .find(|&(_, character)| character.is_whitespace())
        .map_or(0, |(index, character)| index + character.len_utf8());
    let word_prefix = &segment[word_start..];

    // A root placeholder embedded in an assignment, argument, comparison, or
    // other token cannot be proven to be an invoked script.
    if !word_prefix
        .trim_matches(|character| matches!(character, '\'' | '"'))
        .is_empty()
    {
        return Invocation::Mention;
    }

    let mut words = segment[..word_start]
        .split_whitespace()
        .map(|word| word.trim_matches(|character| matches!(character, '\'' | '"')))
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    while words
        .first()
        .is_some_and(|word| ASSIGNMENT_WORD.is_match(word))
    {
        words.remove(0);
    }
    // Markdown command instructions and GitHub Actions `run:` values are
    // command surfaces for G002/G003. Their lexical marker is not a shell
    // executable, so discard it before classifying the actual command word.
    if supports_instruction_markers
        && (matches!(words.first(), Some(&"Run" | &"Execute" | &"run:"))
            || matches!(words.as_slice(), ["-", "run:", ..]))
    {
        words.remove(0);
        if words.first() == Some(&"run:") {
            words.remove(0);
        }
        while words
            .first()
            .is_some_and(|word| ASSIGNMENT_WORD.is_match(word))
        {
            words.remove(0);
        }
    }
    let Some(command_word) = words.first().copied() else {
        return Invocation::Direct;
    };

    if command_word == "env" {
        words.remove(0);
        while let Some(word) = words.first().copied() {
            if matches!(word, "-u" | "--unset") {
                words.remove(0);
                if words.is_empty() {
                    return Invocation::Mention;
                }
                words.remove(0);
            } else if ASSIGNMENT_WORD.is_match(word) || word.starts_with('-') {
                words.remove(0);
            } else {
                break;
            }
        }
        return if words.is_empty() {
            Invocation::Direct
        } else {
            Invocation::Mention
        };
    }

    if matches!(command_word, "source" | ".") {
        return if words[1..].iter().all(|word| word.starts_with('-')) {
            Invocation::Sourced
        } else {
            Invocation::Mention
        };
    }

    if matches!(
        command_word,
        "bash" | "sh" | "zsh" | "fish" | "python" | "python3" | "node" | "ruby" | "perl"
    ) {
        // These interpreter options consume their following token as source
        // text or a module name, never as a script filename. Treat a
        // root-qualified value there as data rather than guessing.
        if words[1..].last().is_some_and(|word| {
            matches!(
                *word,
                "-c" | "--command" | "-e" | "--eval" | "-m" | "--module" | "-r"
            )
        }) {
            return Invocation::Mention;
        }
        return if words[1..].iter().all(|word| word.starts_with('-')) {
            Invocation::Interpreter
        } else {
            Invocation::Mention
        };
    }

    Invocation::Mention
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
    fn classifies_simple_command_segments_conservatively() {
        let cases = [
            ("${CLAUDE_PLUGIN_ROOT}/scripts/direct", Invocation::Direct),
            (
                "FOO=bar ${CLAUDE_PLUGIN_ROOT}/scripts/direct",
                Invocation::Direct,
            ),
            (
                "env FOO=bar ${CLAUDE_PLUGIN_ROOT}/scripts/direct",
                Invocation::Direct,
            ),
            (
                "env -u FOO ${CLAUDE_PLUGIN_ROOT}/scripts/direct",
                Invocation::Direct,
            ),
            (
                "python3 -u ${CLAUDE_PLUGIN_ROOT}/scripts/interpreted.py",
                Invocation::Interpreter,
            ),
            (
                "python3 -c ${CLAUDE_PLUGIN_ROOT}/generated/code.py",
                Invocation::Mention,
            ),
            (
                "source ${CLAUDE_PLUGIN_ROOT}/scripts/library.sh",
                Invocation::Sourced,
            ),
            (
                "echo ${CLAUDE_PLUGIN_ROOT}/generated/output.json",
                Invocation::Mention,
            ),
            (
                "tool ${CLAUDE_PLUGIN_ROOT}/generated/output.json",
                Invocation::Mention,
            ),
            (
                "INPUT=${CLAUDE_PLUGIN_ROOT}/generated/output.json echo ok",
                Invocation::Mention,
            ),
            (
                "test -f ${CLAUDE_PLUGIN_ROOT}/generated/output.json",
                Invocation::Mention,
            ),
            (
                "echo ok; ${CLAUDE_PLUGIN_ROOT}/scripts/semicolon",
                Invocation::Direct,
            ),
            (
                "echo ok && ${CLAUDE_PLUGIN_ROOT}/scripts/and",
                Invocation::Direct,
            ),
            (
                "echo ok || ${CLAUDE_PLUGIN_ROOT}/scripts/or",
                Invocation::Direct,
            ),
            (
                "echo ok | ${CLAUDE_PLUGIN_ROOT}/scripts/pipe",
                Invocation::Direct,
            ),
            (
                "echo ok\n${CLAUDE_PLUGIN_ROOT}/scripts/newline",
                Invocation::Direct,
            ),
        ];
        for (command, expected) in cases {
            assert_eq!(
                extract_command_references(command, 1)[0].invocation,
                expected,
                "{command}"
            );
        }
    }

    #[test]
    fn instruction_markers_are_not_hook_command_words() {
        assert_eq!(
            extract_command_references("run: ${CLAUDE_PLUGIN_ROOT}/scripts/workflow", 1)[0]
                .invocation,
            Invocation::Mention
        );
        assert_eq!(
            extract_instruction_command_references(
                "Run ${CLAUDE_PLUGIN_ROOT}/scripts/documented",
                1
            )[0]
            .invocation,
            Invocation::Direct
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

    #[test]
    fn bare_references_tolerate_multibyte_shell_whitespace() {
        // U+00A0 (2 bytes) and U+3000 (3 bytes) match `is_whitespace`, so the
        // scan must step past them by their full width (issue #600).
        let refs = extract_bare_script_references("./tool x\u{a0}y scripts/check.sh", 3);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].path, PathBuf::from("scripts/check.sh"));
        assert_eq!(refs[0].line, 3);
        let refs = extract_bare_script_references("run\u{3000}scripts/wide.sh", 1);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].path, PathBuf::from("scripts/wide.sh"));
    }

    #[test]
    fn invocations_tolerate_multibyte_whitespace_before_the_reference() {
        // The scalar before the reference is multibyte whitespace; stepping
        // one byte past it used to split the scalar and panic (issue #600).
        let instruction = extract_instruction_command_references(
            "Run\u{a0}${CLAUDE_PLUGIN_ROOT}/scripts/x.sh",
            1,
        );
        assert_eq!(instruction.len(), 1);
        assert_eq!(instruction[0].path, PathBuf::from("scripts/x.sh"));
        assert_eq!(instruction[0].invocation, Invocation::Direct);
        let command = extract_command_references("ok;\u{a0}${CLAUDE_PLUGIN_ROOT}/scripts/a.sh", 1);
        assert_eq!(command.len(), 1);
        assert_eq!(command[0].invocation, Invocation::Direct);
    }
}
