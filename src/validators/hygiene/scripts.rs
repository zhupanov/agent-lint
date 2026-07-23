use crate::config::ExcludeSet;
use crate::context::{LintContext, LintMode};
use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata, SourceSpan};
use crate::rules::LintRule;
use crate::script_paths::{
    Invocation, ScriptReference, ScriptReferenceBase, extract_bare_script_references,
    extract_instruction_command_references, extract_prose_bare_script_references,
    extract_prose_command_references, script_kind,
};
use crate::traversal;
use regex::Regex;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// Directory patterns used by `--list-scripts` and conventional script
/// discovery.
pub const PLUGIN_SCRIPT_DIRS: &[&str] =
    &["scripts", "skills/*/scripts", ".claude/skills/*/scripts"];
pub const BASIC_SCRIPT_DIRS: &[&str] = &[".claude/skills/*/scripts"];

static RE_FULL_HASH_COMMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[[:space:]]*#").unwrap());
static RE_YAML_KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[[:space:]]*(?:-[[:space:]]+)?([A-Za-z_-]+):").unwrap());
static RE_PYTHON_ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[[:space:]]*([A-Za-z_][A-Za-z0-9_]*)[[:space:]]*=[[:space:]]*(.+)$").unwrap()
});
static RE_PYTHON_PATH_BASE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:Path[[:space:]]*\([[:space:]]*([A-Za-z_][A-Za-z0-9_]*)[[:space:]]*\)|([A-Za-z_][A-Za-z0-9_]*))[[:space:]]*")
        .unwrap()
});
static RE_PYTHON_PATH_IMPORT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^[[:space:]]*from[[:space:]]+pathlib[[:space:]]+import[^\n#]*\bPath\b")
        .unwrap()
});
static RE_PYTHON_SUBPROCESS_RUN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?s)\bsubprocess[[:space:]]*\.[[:space:]]*run[[:space:]]*\([[:space:]]*\[[[:space:]]*(?:str[[:space:]]*\([[:space:]]*([A-Za-z_][A-Za-z0-9_]*)[[:space:]]*\)|([A-Za-z_][A-Za-z0-9_]*))[[:space:]]*(?:,|\])",
    )
    .unwrap()
});
static RE_PYTHON_SUBPROCESS_LITERAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?s)\bsubprocess[[:space:]]*\.[[:space:]]*(?:run|call|check_call|check_output|Popen)[[:space:]]*\([[:space:]]*(?:args[[:space:]]*=[[:space:]]*)?\[",
    )
    .unwrap()
});

pub(super) fn strip_yaml_comments(content: &str) -> String {
    content
        .lines()
        .map(|line| {
            if RE_FULL_HASH_COMMENT.is_match(line) {
                String::new()
            } else {
                strip_trailing_yaml_comment(line)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn strip_trailing_yaml_comment(line: &str) -> String {
    let mut quote = None;
    let mut previous_ws = false;
    for (index, character) in line.char_indices() {
        match quote {
            Some(q) if character == q => quote = None,
            Some(_) => {}
            None if matches!(character, '\'' | '"') => quote = Some(character),
            None if character == '#' && previous_ws => return line[..index].trim_end().to_string(),
            None => {}
        }
        previous_ws = character.is_whitespace();
    }
    line.to_string()
}

pub(super) fn collect_makefile_contents(exclude: &ExcludeSet) -> Vec<(String, String)> {
    let mut candidates = vec![PathBuf::from("Makefile")];
    for entry in traversal::shallow_files(Path::new("."), Path::new("."), None).entries {
        if entry
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".mk"))
        {
            candidates.push(entry.path);
        }
    }
    candidates
        .into_iter()
        .filter_map(|path| {
            let display = path.display().to_string();
            (!exclude.is_excluded(&display))
                .then(|| {
                    fs::read_to_string(path)
                        .ok()
                        .map(|content| (display, strip_yaml_comments(&content)))
                })
                .flatten()
        })
        .collect()
}

/// The lexical position a command fragment was found in. Fence bodies,
/// workflow `run` values, and script/Makefile lines are executable command
/// positions; inline code in prose illustrates a command without invoking it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FragmentContext {
    Command,
    Prose,
}

/// A source-owned reference. The source path remains the G002 diagnostic
/// subject, while the normalized target is consumed by G003/G004.
///
/// This is the shared candidate pipeline for the script rules: sources are
/// limited to command-bearing file kinds, fragments keep their
/// [`FragmentContext`], and every reference is resolved once here (skill-local
/// base, quoted command arguments, placeholder actionability) with its parsed
/// [`Invocation`] preserved.
pub(crate) fn collect_references(
    ctx: &LintContext,
    exclude: &ExcludeSet,
) -> Vec<(String, ScriptReference)> {
    let sources = crate::validators::skill_discovery::SkillDiscovery::from_context(ctx, exclude)
        .hygiene_source_files;
    let mut references = Vec::new();
    for path in sources {
        // Script fixture trees ship deliberately incomplete example content;
        // references inside them are test data, never production invocations.
        if is_script_fixture_path(&path) {
            continue;
        }
        let source = path.to_string_lossy().replace('\\', "/");
        let is_markdown =
            source.ends_with(".md") || source.ends_with(".markdown") || source.ends_with(".mdx");
        let is_yaml = source.ends_with(".yml") || source.ends_with(".yaml");
        let is_python = path.extension().is_some_and(|extension| extension == "py");
        // Only command-bearing kinds are scanned line-wise: supported script
        // files and make include files. Data files (TSV, JSONL, JSON, plain
        // text) are not command surfaces.
        let is_line_command_source = script_kind(&path).is_some()
            || path
                .file_name()
                .is_some_and(|name| name == "Makefile" || name.to_string_lossy().ends_with(".mk"));
        if !is_markdown && !is_yaml && !is_line_command_source {
            continue;
        }
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let skill_dir = owning_skill_dir(&path);
        if path.file_name().is_some_and(|name| name == "SKILL.md") {
            references.extend(
                frontmatter_hook_references(&content)
                    .into_iter()
                    .filter_map(|reference| resolve_reference(skill_dir.as_deref(), reference))
                    .map(|reference| (source.clone(), reference)),
            );
        }
        let fragments = if is_python {
            // Python has structured execution syntax. Feeding arbitrary
            // source lines to the shell extractor turns strings, regular
            // expressions, and fixture data into fictitious commands.
            Vec::new()
        } else if is_markdown {
            markdown_command_fragments(&content)
        } else if is_yaml {
            with_context(
                yaml_command_line_fragments(&strip_yaml_comments(&content)),
                FragmentContext::Command,
            )
        } else {
            with_context(
                command_line_fragments(&strip_yaml_comments(&content)),
                FragmentContext::Command,
            )
        };
        for (line, fragment, context) in fragments {
            if !is_executable_fragment(&fragment) {
                continue;
            }
            references.extend(extract_fragment_references(
                &source,
                skill_dir.as_deref(),
                &fragment,
                line,
                context,
            ));
        }
        // Python's `Path` composition has no contiguous path token for the
        // lexical extractor above.  Its bounded recognizer still emits the
        // same shared reference type, so G002/G003/G004 agree on the exact
        // executable candidate rather than giving G004 a private exception.
        if is_python {
            references.extend(
                python_execution_references(&content)
                    .into_iter()
                    .map(|reference| (source.clone(), reference)),
            );
        }
    }
    if ctx.mode == LintMode::Plugin {
        for (source, content) in collect_makefile_contents(exclude) {
            for (line, fragment) in command_line_fragments(&content) {
                if is_executable_fragment(&fragment) {
                    references.extend(extract_fragment_references(
                        &source,
                        None,
                        &fragment,
                        line,
                        FragmentContext::Command,
                    ));
                }
            }
        }
    }
    references
}

/// Extract script references from Python only where bounded execution syntax
/// proves that the value is an argv element zero. Python source text is not a
/// shell command surface: strings in tests, regular expressions, assertions,
/// and ordinary function calls must not reach G002/G003/G004.
fn python_execution_references(content: &str) -> Vec<ScriptReference> {
    let code = mask_python_non_code(content);
    let mut references = python_literal_subprocess_references(content, &code);
    references.extend(python_composed_subprocess_references_with_code(
        content, &code,
    ));
    references
}

/// Accept a literal repository script only as the first element of a list
/// supplied to a supported subprocess API. The code-only view proves the
/// call shape; the original source supplies the literal value at the same
/// byte offset after strings and comments have been ruled out.
fn python_literal_subprocess_references(content: &str, code: &str) -> Vec<ScriptReference> {
    RE_PYTHON_SUBPROCESS_LITERAL
        .find_iter(code)
        .filter_map(|matched| {
            let start = matched.end();
            let (literal, _) = parse_python_string_literal(content[start..].trim_start())?;
            // A bare command name such as `python3` names PATH state, not a
            // repository file. A filename suffix or a path separator proves
            // this is a repository-local script candidate.
            if !literal.contains('/') && Path::new(&literal).extension().is_none() {
                return None;
            }
            let path = crate::script_paths::normalize_repository_path(&literal)?;
            script_kind(&path)?;
            let line = content[..matched.start()]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count()
                + 1;
            Some(ScriptReference {
                reference: literal,
                path,
                base: ScriptReferenceBase::RepositoryRoot,
                invocation: Invocation::Direct,
                line,
            })
        })
        .collect()
}

/// Extract a repository-local Python `Path` composition only when a variable
/// holding it is passed as argv element zero to `subprocess.run`. This is not
/// general dataflow: each literal composition is a one-line assignment rooted
/// at an explicitly root-named anchor, and no assignment-to-assignment flow,
/// dynamic segment, or alternate argv position is accepted.
fn python_composed_subprocess_references_with_code(
    content: &str,
    code: &str,
) -> Vec<ScriptReference> {
    // `Path` is only accepted as the standard library type, not an arbitrary
    // project class with an overloaded `/` operator.
    if !RE_PYTHON_PATH_IMPORT.is_match(code) {
        return Vec::new();
    }
    let line_scopes = python_line_scopes(code);
    let mut assignments = HashMap::new();
    for (line_index, (line, masked_line)) in content.lines().zip(code.lines()).enumerate() {
        // The masked line must itself contain the assignment syntax. This
        // excludes comments and triple-quoted documentation while the
        // original line retains the literal segments for path parsing.
        let Some(captures) = RE_PYTHON_ASSIGNMENT.captures(masked_line) else {
            continue;
        };
        let Some(name) = captures.get(1).map(|capture| capture.as_str()) else {
            continue;
        };
        let path = RE_PYTHON_ASSIGNMENT
            .captures(line)
            .and_then(|captures| captures.get(2))
            .and_then(|expression| parse_python_path_composition(expression.as_str()));
        let key = (line_scopes[line_index], name.to_string());
        // This recognizer intentionally has no assignment dataflow. A second
        // assignment in the same Python scope can change the value before the
        // call, so it makes the candidate unprovable rather than selecting
        // either assignment. Local names in distinct functions are separate
        // bindings and therefore must not invalidate each other.
        if assignments.insert(key.clone(), path).is_some() {
            assignments.insert(key, None);
        }
    }
    let candidates: HashMap<_, _> = assignments
        .into_iter()
        .filter_map(|(key, path)| path.map(|path| (key, path)))
        .collect();
    if candidates.is_empty() {
        return Vec::new();
    }

    // Calls may span lines. Mask strings and comments first so illustrative
    // source text cannot manufacture an executable invocation.
    RE_PYTHON_SUBPROCESS_RUN
        .captures_iter(code)
        .filter_map(|captures| {
            let variable = captures.get(1).or_else(|| captures.get(2))?.as_str();
            let line = content[..captures.get(0)?.start()]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count()
                + 1;
            let path = candidates
                .get(&(line_scopes[line - 1], variable.to_string()))?
                .clone();
            Some(ScriptReference {
                reference: format!("{variable} (Python Path composition)"),
                path,
                base: ScriptReferenceBase::RepositoryRoot,
                invocation: Invocation::Direct,
                line,
            })
        })
        .collect()
}

/// Return the lexical function/class scope for each source line. Python uses
/// indentation to delimit these scopes, while ordinary blocks (`if`, `for`,
/// `with`, and `try`) leave local bindings in the enclosing function scope.
/// This deliberately small recognizer only supplies the bound needed by the
/// subprocess analysis; it does not attempt to parse Python expressions.
fn python_line_scopes(code: &str) -> Vec<usize> {
    let mut scopes = vec![(0, 0)];
    let mut next_scope = 1;
    let mut line_scopes = Vec::new();
    for line in code.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            line_scopes.push(scopes.last().expect("module scope exists").1);
            continue;
        }
        let indent = line.len() - trimmed.len();
        while scopes.len() > 1 && indent <= scopes.last().expect("module scope exists").0 {
            scopes.pop();
        }
        line_scopes.push(scopes.last().expect("module scope exists").1);
        if is_python_scope_declaration(trimmed) {
            scopes.push((indent, next_scope));
            next_scope += 1;
        }
    }
    line_scopes
}

fn is_python_scope_declaration(line: &str) -> bool {
    line.contains(':')
        && (line.starts_with("def ")
            || line.starts_with("async def ")
            || line.starts_with("class "))
}

fn parse_python_path_composition(expression: &str) -> Option<PathBuf> {
    let captures = RE_PYTHON_PATH_BASE.captures(expression)?;
    let root = captures.get(1).or_else(|| captures.get(2))?.as_str();
    if !is_python_root_anchor(root) {
        return None;
    }
    let mut rest = expression[captures.get(0)?.end()..].trim_start();
    let mut segments = Vec::new();
    while let Some(after_slash) = rest.strip_prefix('/') {
        let (segment, after_segment) = parse_python_string_literal(after_slash.trim_start())?;
        segments.push(segment);
        rest = after_segment.trim_start();
    }
    if segments.len() < 2 || (!rest.is_empty() && !rest.starts_with('#')) {
        return None;
    }
    // The shared script contract is intentionally only about repository
    // scripts. Requiring this canonical first segment also prevents a
    // root-looking variable from proving arbitrary files reachable.
    (segments.first().is_some_and(|segment| segment == "scripts"))
        .then(|| crate::script_paths::normalize_repository_path(&segments.join("/")))?
}

fn is_python_root_anchor(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    value == "root" || value.ends_with("_root") || value.ends_with("_root_dir")
}

/// Parse one unescaped, single- or double-quoted Python literal. Escapes are
/// deliberately rejected: accepting them would require Python's full string
/// semantics before we could prove the resulting repository path.
fn parse_python_string_literal(value: &str) -> Option<(String, &str)> {
    let quote = value.chars().next()?;
    if !matches!(quote, '\'' | '"') {
        return None;
    }
    let body = &value[quote.len_utf8()..];
    let end = body.find(quote)?;
    let segment = &body[..end];
    (!segment.is_empty() && !segment.contains(['\\', '$', '<', '>']))
        .then(|| (segment.to_string(), &body[end + quote.len_utf8()..]))
}

/// Replace Python strings and comments with spaces while retaining newlines
/// and byte positions. The subprocess grammar runs on this code-only view,
/// which keeps comments and documentation strings from counting as commands.
fn mask_python_non_code(content: &str) -> String {
    #[derive(Clone, Copy)]
    enum Quote {
        Single,
        Double,
        TripleSingle,
        TripleDouble,
    }

    let bytes = content.as_bytes();
    let mut masked = bytes.to_vec();
    let mut index = 0;
    let mut quote = None;
    while index < bytes.len() {
        match quote {
            Some(Quote::Single) | Some(Quote::Double) => {
                let delimiter = match quote {
                    Some(Quote::Single) => b'\'',
                    Some(Quote::Double) => b'"',
                    _ => unreachable!(),
                };
                masked[index] = if bytes[index] == b'\n' { b'\n' } else { b' ' };
                if bytes[index] == delimiter && (index == 0 || bytes[index - 1] != b'\\') {
                    quote = None;
                }
                index += 1;
            }
            Some(Quote::TripleSingle) | Some(Quote::TripleDouble) => {
                let delimiter = match quote {
                    Some(Quote::TripleSingle) => b'\'',
                    Some(Quote::TripleDouble) => b'"',
                    _ => unreachable!(),
                };
                let closes = index + 2 < bytes.len()
                    && bytes[index] == delimiter
                    && bytes[index + 1] == delimiter
                    && bytes[index + 2] == delimiter;
                masked[index] = if bytes[index] == b'\n' { b'\n' } else { b' ' };
                if closes {
                    masked[index + 1] = b' ';
                    masked[index + 2] = b' ';
                    quote = None;
                    index += 3;
                } else {
                    index += 1;
                }
            }
            None if bytes[index] == b'#' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    masked[index] = b' ';
                    index += 1;
                }
            }
            None if bytes[index] == b'\'' || bytes[index] == b'"' => {
                let delimiter = bytes[index];
                let triple = index + 2 < bytes.len()
                    && bytes[index + 1] == delimiter
                    && bytes[index + 2] == delimiter;
                masked[index] = b' ';
                if triple {
                    masked[index + 1] = b' ';
                    masked[index + 2] = b' ';
                    quote = Some(if delimiter == b'\'' {
                        Quote::TripleSingle
                    } else {
                        Quote::TripleDouble
                    });
                    index += 3;
                } else {
                    quote = Some(if delimiter == b'\'' {
                        Quote::Single
                    } else {
                        Quote::Double
                    });
                    index += 1;
                }
            }
            None => index += 1,
        }
    }
    String::from_utf8(masked).expect("masking preserves UTF-8 bytes")
}

fn extract_fragment_references(
    source: &str,
    skill_dir: Option<&Path>,
    fragment: &str,
    line: usize,
    context: FragmentContext,
) -> Vec<(String, ScriptReference)> {
    let mut references = match context {
        FragmentContext::Command => extract_instruction_command_references(fragment, line),
        FragmentContext::Prose => extract_prose_command_references(fragment, line),
    };
    references.extend(match context {
        FragmentContext::Command => extract_bare_script_references(fragment, line),
        FragmentContext::Prose => extract_prose_bare_script_references(fragment, line),
    });
    references
        .into_iter()
        .filter(|reference| !contains_unsupported_glob(reference))
        .flat_map(expand_glob_reference)
        .filter_map(|reference| resolve_reference(skill_dir, reference))
        .map(|reference| (source.to_string(), reference))
        .collect()
}

/// Resolve one extracted reference into the shared candidate contract. The
/// invocation classified at parse time is never modified here; only the path
/// candidate is refined, and unprovable references are dropped.
fn resolve_reference(
    skill_dir: Option<&Path>,
    mut reference: ScriptReference,
) -> Option<ScriptReference> {
    // A skill file addresses its bundled resources relative to the skill
    // directory; prefer that documented base when it resolves on disk.
    if reference.base == ScriptReferenceBase::Relative
        && !reference.path.as_os_str().is_empty()
        && let Some(skill_dir) = skill_dir
    {
        let local = skill_dir.join(&reference.path);
        if local.exists() {
            reference.path = local;
        }
    }
    // A whole-quoted value may document arguments after the script path
    // (`"${CLAUDE_PLUGIN_ROOT}/python/cli.py redact secrets"`). The exact
    // quoted path wins when it exists (spaces are legal in filenames);
    // otherwise a provable leading command word resolves the reference.
    if !reference.path.as_os_str().is_empty() && !reference.path.exists() {
        let text = reference.path.to_string_lossy().into_owned();
        if let Some((head, _)) = text.split_once(char::is_whitespace)
            && !head.is_empty()
            && Path::new(head).is_file()
        {
            reference.path = PathBuf::from(head);
        }
    }
    // Unresolved variables and placeholders prove no exact repository path,
    // and an un-invoked $PWD path names runtime state, not a shipped file.
    if reference.has_unresolved_placeholder() {
        return None;
    }
    if reference.is_pwd_rooted() && reference.invocation == Invocation::Mention {
        return None;
    }
    Some(reference)
}

/// Skill-scoped hooks declared in SKILL.md frontmatter are a documented
/// runtime invocation surface: their command values reference scripts exactly
/// like hooks.json commands do, so they feed G002 existence, G003
/// executability, and G004 reachability through the shared hook extractor.
fn frontmatter_hook_references(content: &str) -> Vec<ScriptReference> {
    let Some(lines) = crate::frontmatter::extract_frontmatter(content) else {
        return Vec::new();
    };
    let Ok(value) = crate::frontmatter::parse_yaml_strict(&lines) else {
        return Vec::new();
    };
    let Some(hooks) = value
        .get("hooks")
        .and_then(crate::frontmatter::yaml_to_json)
    else {
        return Vec::new();
    };
    let wrapper =
        serde_json::Value::Object(serde_json::Map::from_iter([("hooks".to_string(), hooks)]));
    // Frontmatter starts at line 2 (after the opening delimiter); locate the
    // hooks mapping key so diagnostics land on the declaring line.
    let line = crate::frontmatter::simple_top_level_key_line(&lines, "hooks").unwrap_or(1);
    crate::hook_commands::extract_hook_command_paths(&wrapper, None)
        .into_iter()
        .map(|command| ScriptReference {
            reference: command.reference,
            path: command.path,
            base: ScriptReferenceBase::RepositoryRoot,
            invocation: command.invocation,
            line,
        })
        .collect()
}

/// The skill directory owning `source`, when the source is bundled skill
/// content: the nearest ancestor directory that holds a SKILL.md.
fn owning_skill_dir(source: &Path) -> Option<PathBuf> {
    source
        .ancestors()
        .skip(1)
        .filter(|ancestor| !ancestor.as_os_str().is_empty())
        .find(|ancestor| ancestor.join("SKILL.md").is_file())
        .map(Path::to_path_buf)
}

/// A `fixtures` directory directly under a `scripts` directory holds negative
/// test data whose paths may deliberately not exist (`scripts/fixtures/`,
/// `skills/x/scripts/fixtures/`). Only that documented script-bundle layout
/// is excluded — as a reference source and as a G004 candidate — so a skill
/// or directory merely named `fixtures` keeps normal validation.
pub(super) fn is_script_fixture_path(path: &Path) -> bool {
    let components: Vec<_> = path
        .components()
        .map(|component| component.as_os_str())
        .collect();
    components
        .windows(2)
        .any(|pair| pair[0] == "scripts" && pair[1] == "fixtures")
}

fn with_context(
    fragments: Vec<(usize, String)>,
    context: FragmentContext,
) -> Vec<(usize, String, FragmentContext)> {
    fragments
        .into_iter()
        .map(|(line, fragment)| (line, fragment, context))
        .collect()
}

fn command_line_fragments(content: &str) -> Vec<(usize, String)> {
    content
        .lines()
        .enumerate()
        .map(|(index, line)| (index + 1, line.to_string()))
        .collect()
}

/// Workflows are command-like only in `run` values.  This small lexical
/// boundary deliberately leaves block scalar continuation lines executable
/// while preventing value keys such as `path:` from becoming invocations.
fn yaml_command_line_fragments(content: &str) -> Vec<(usize, String)> {
    content
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let captures = RE_YAML_KEY.captures(line)?;
            (captures
                .get(1)
                .is_some_and(|capture| capture.as_str() == "run"))
            .then(|| (index + 1, line.to_string()))
        })
        .chain(
            content
                .lines()
                .enumerate()
                .filter(|(_, line)| !RE_YAML_KEY.is_match(line))
                .map(|(index, line)| (index + 1, line.to_string())),
        )
        .collect()
}

/// Markdown fragments keep their lexical role: bodies of shell fences are
/// executable command positions, while inline code and prose `Run`/`Execute`
/// instructions outside fences are extracted with prose semantics (an
/// explicit marker still proves an invocation). Fences in data or non-shell
/// languages illustrate content and are not command surfaces.
fn markdown_command_fragments(content: &str) -> Vec<(usize, String, FragmentContext)> {
    let mut fragments = Vec::new();
    let mut fence_lines = HashSet::new();
    for fence in crate::fence::markdown_fences(content) {
        for line in fence.start_line..=fence.end_line {
            fence_lines.insert(line);
        }
        if fence_is_command_surface(&fence.info) {
            for (line, text) in fence.body {
                fragments.push((line, text, FragmentContext::Command));
            }
        }
    }
    for (index, line) in content.lines().enumerate() {
        if fence_lines.contains(&(index + 1)) {
            continue;
        }
        if line.trim_start().starts_with("Run ") || line.trim_start().starts_with("Execute ") {
            fragments.push((
                index + 1,
                line.trim_start().to_string(),
                FragmentContext::Prose,
            ));
        }
        let mut rest = line;
        while let Some(start) = rest.find('`') {
            let after = &rest[start + 1..];
            let Some(end) = after.find('`') else {
                break;
            };
            fragments.push((index + 1, after[..end].to_string(), FragmentContext::Prose));
            rest = &after[end + 1..];
        }
    }
    fragments
}

/// Positive grammar for fence info strings whose bodies are executable shell
/// command lines. Everything else (json, yaml, python, text, output, ...)
/// illustrates data or foreign-language content instead of invoking it.
fn fence_is_command_surface(info: &str) -> bool {
    let language = info
        .split([' ', '\t', ','])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        language.as_str(),
        "" | "bash" | "sh" | "shell" | "zsh" | "fish" | "console" | "terminal" | "shellsession"
    )
}

fn is_executable_fragment(fragment: &str) -> bool {
    let first = fragment
        .trim_start()
        .trim_matches('"')
        .split_whitespace()
        .next()
        .unwrap_or("");
    !matches!(
        first,
        "" | "#" | "echo" | "printf" | "cat" | "grep" | "sed" | "awk"
    )
}

fn contains_unsupported_glob(reference: &ScriptReference) -> bool {
    reference.reference.contains(['?', '['])
        || reference
            .path
            .components()
            .any(|component| component.as_os_str().to_string_lossy().contains(['?', '[']))
}

fn expand_glob_reference(reference: ScriptReference) -> Vec<ScriptReference> {
    if !reference.path.to_string_lossy().contains('*') {
        return vec![reference];
    }
    let matches = expand_path_glob(&reference.path);
    if matches.is_empty() {
        vec![reference]
    } else {
        matches
            .into_iter()
            .map(|path| ScriptReference {
                path,
                ..reference.clone()
            })
            .collect()
    }
}

/// Expand `*` component-wise below the repository root. Intermediate matches
/// must be directories; a final component may identify either a file or a
/// directory so G002 can apply its normal directory semantics.
fn expand_path_glob(pattern: &Path) -> Vec<PathBuf> {
    let components: Vec<_> = pattern.components().collect();
    let mut candidates = vec![PathBuf::new()];
    for (index, component) in components.iter().enumerate() {
        let segment = component.as_os_str().to_string_lossy();
        let final_component = index + 1 == components.len();
        let mut next = Vec::new();
        for base in &candidates {
            let directory = if base.as_os_str().is_empty() {
                Path::new(".")
            } else {
                base.as_path()
            };
            if segment.contains('*') {
                for entry in traversal::shallow_directories(directory, Path::new("."), None).entries
                {
                    let name = entry.path.file_name().unwrap_or_default();
                    if wildcard_matches(&segment, &name.to_string_lossy()) {
                        next.push(base.join(name));
                    }
                }
                if final_component {
                    for entry in traversal::shallow_files(directory, Path::new("."), None).entries {
                        let name = entry.path.file_name().unwrap_or_default();
                        if wildcard_matches(&segment, &name.to_string_lossy()) {
                            next.push(base.join(name));
                        }
                    }
                }
            } else {
                let child = base.join(segment.as_ref());
                if child.is_dir() || (final_component && child.exists()) {
                    next.push(child);
                }
            }
        }
        candidates = next;
    }
    candidates
}

fn wildcard_matches(pattern: &str, value: &str) -> bool {
    let mut remaining = value;
    let mut parts = pattern.split('*').peekable();
    let anchored_start = !pattern.starts_with('*');
    if let Some(first) = parts.next()
        && anchored_start
    {
        let Some(after) = remaining.strip_prefix(first) else {
            return false;
        };
        remaining = after;
    }
    while let Some(part) = parts.next() {
        if parts.peek().is_none() && !pattern.ends_with('*') {
            return remaining.ends_with(part);
        }
        let Some(index) = remaining.find(part) else {
            return false;
        };
        remaining = &remaining[index + part.len()..];
    }
    true
}

/// G002: missing or unsafe script references. Dedupe includes the source path
/// so collector policy is applied independently for every source file.
#[cfg(test)]
pub fn validate_script_references(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let ctx = LintContext::new(Path::new("."), LintMode::Plugin);
    validate_script_references_for_context(&ctx, diag, exclude);
}

pub fn validate_script_references_for_context(
    ctx: &LintContext,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
) {
    let mut seen = HashSet::new();
    for (source, reference) in collect_references(ctx, exclude) {
        let dedupe_key = if reference.path.as_os_str().is_empty() {
            reference.reference.clone()
        } else {
            reference.path.display().to_string()
        };
        if !seen.insert((source.clone(), dedupe_key)) {
            continue;
        }
        if reference.path.as_os_str().is_empty() || !reference.path.exists() {
            let expected = reference.path.display().to_string();
            diag.report_at_with(
                LintRule::ScriptRefMissing,
                &source,
                &format!(
                    "script reference missing on disk or unsafe at line {}: {} (expected {})",
                    reference.line,
                    reference.reference,
                    if expected.is_empty() {
                        "an in-repository path"
                    } else {
                        &expected
                    }
                ),
                DiagnosticMetadata::default()
                    .with_location(SourceSpan::line(reference.line))
                    .with_evidence(reference.reference)
                    .with_suggestion("use a normalized path within the repository"),
            );
        }
    }
}

#[cfg(test)]
pub fn validate_private_script_references(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let ctx = LintContext::new(Path::new("."), LintMode::Basic);
    validate_script_references_for_context(&ctx, diag, exclude);
}

/// Direct invocation of a supported script kind determines G003 scope.
/// Interpreter-launched and sourced files need no execute bit, and
/// non-script files (documentation, data) are outside the rule's contract
/// even when a command line names them.
pub(crate) fn direct_script_paths(ctx: &LintContext, exclude: &ExcludeSet) -> Vec<PathBuf> {
    let mut paths = BTreeSet::new();
    for (_, reference) in collect_references(ctx, exclude) {
        if reference.invocation == Invocation::Direct
            && !reference.path.as_os_str().is_empty()
            && script_kind(&reference.path).is_some()
            && reference.path.is_file()
        {
            paths.insert(reference.path);
        }
    }
    paths.into_iter().collect()
}

#[cfg(unix)]
#[cfg(test)]
pub fn validate_executability(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let ctx = LintContext::new(Path::new("."), LintMode::Plugin);
    validate_executability_for_context(&ctx, diag, exclude);
}

#[cfg(unix)]
pub fn validate_executability_for_context(
    ctx: &LintContext,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
) {
    use std::os::unix::fs::PermissionsExt;
    for path in direct_script_paths(ctx, exclude) {
        if let Ok(meta) = path.metadata()
            && meta.permissions().mode() & 0o111 == 0
        {
            diag.report_at_with(
                LintRule::ScriptNotExecutable,
                &path,
                &format!(
                    "directly executed script is not executable: {}",
                    path.display()
                ),
                DiagnosticMetadata::default()
                    .with_evidence(path.display().to_string())
                    .with_suggestion("run chmod +x on this file"),
            );
        }
    }
}

#[cfg(not(unix))]
pub fn validate_executability_for_context(
    _ctx: &LintContext,
    _diag: &mut DiagnosticCollector,
    _exclude: &ExcludeSet,
) {
}

#[cfg(test)]
pub fn validate_private_executability(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let ctx = LintContext::new(Path::new("."), LintMode::Basic);
    validate_executability_for_context(&ctx, diag, exclude);
}

pub fn expand_script_dirs(patterns: &[&str]) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for pattern in patterns {
        let mut candidates = vec![PathBuf::new()];
        for segment in pattern.split('/') {
            let mut next = Vec::new();
            for base in &candidates {
                if segment == "*" {
                    let directory = if base.as_os_str().is_empty() {
                        Path::new(".")
                    } else {
                        base.as_path()
                    };
                    for entry in
                        traversal::shallow_directories(directory, Path::new("."), None).entries
                    {
                        next.push(if base.as_os_str().is_empty() {
                            PathBuf::from(entry.path.file_name().unwrap_or_default())
                        } else {
                            base.join(entry.path.file_name().unwrap_or_default())
                        });
                    }
                } else {
                    let child = if base.as_os_str().is_empty() {
                        PathBuf::from(segment)
                    } else {
                        base.join(segment)
                    };
                    if child.is_dir() {
                        next.push(child);
                    }
                }
            }
            candidates = next;
        }
        dirs.extend(candidates);
    }
    dirs
}

pub fn collect_script_paths(mode: LintMode, exclude: &ExcludeSet) -> Vec<String> {
    let patterns = match mode {
        LintMode::Plugin => PLUGIN_SCRIPT_DIRS,
        LintMode::Basic => BASIC_SCRIPT_DIRS,
    };
    let mut paths = BTreeSet::new();
    for dir in expand_script_dirs(patterns) {
        for entry in traversal::shallow_files(&dir, Path::new("."), None).entries {
            let display = entry.path.display().to_string();
            if is_supported_script_file(&entry.path) && !exclude.is_excluded(&display) {
                paths.insert(display);
            }
        }
    }
    paths.into_iter().collect()
}

fn is_supported_script_file(path: &Path) -> bool {
    script_kind(path).is_some()
}
