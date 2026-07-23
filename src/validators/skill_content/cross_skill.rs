use crate::config::ExcludeSet;
use crate::diagnostic::DiagnosticCollector;
use crate::markdown::MarkdownDocument;
use crate::rules::LintRule;
use crate::script_paths;
use crate::traversal;
use crate::validators::shared_md_refs::{contains_shared_md_ref, find_shared_md_refs};
use crate::validators::skills::SkillInfo;
use globset::Glob;
use regex::Regex;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// S048: denylist for non-descriptive reference file names in skill directories.
/// Matches generic stems (doc, file, ref, data, info, tmp, test) with optional
/// digits, single letters (case-insensitive), and pure numeric names — all with .md extension.
static RE_GENERIC_REF_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i:^(?:(?:doc|file|ref|data|info|tmp|test)\d*|[a-z]|\d+)\.md$)").unwrap()
});

/// Bounded Python test forms that can enumerate a fixture directory. The
/// directory itself must be assembled from literal path components, so this
/// deliberately does not attempt to evaluate Python expressions.
static PYTHON_GLOB_CALL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?m)\b(?P<directory>[A-Za-z_][A-Za-z0-9_]*)\.glob\(\s*[\"'](?P<pattern>[^\"']+)[\"']\s*\)"#)
        .unwrap()
});
static PYTHON_PATH_CHILD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\b(?P<directory>[A-Za-z_][A-Za-z0-9_]*)\s*/\s*[\"'](?P<child>[^\"']+)[\"']"#)
        .unwrap()
});
static PYTHON_STEM_SUFFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\.stem\.removesuffix\(\s*[\"'](?P<from>[^\"']+)[\"']\s*\)\s*\+\s*[\"'](?P<to>[^\"']+)[\"']"#)
        .unwrap()
});

const REF_NO_TOC_THRESHOLD: usize = 100;

/// S029: Check for deeply nested shared markdown references.
/// Matches `$CLAUDE_PLUGIN_ROOT/<base_dir>/shared/*.md` references.
pub(super) fn validate_nested_references(
    base_dir: &str,
    skills: &[SkillInfo],
    diag: &mut DiagnosticCollector,
) {
    let shared_dir = Path::new(base_dir).join("shared");
    if !shared_dir.is_dir() {
        return;
    }

    // Cache: which shared .md files are nested (avoids re-reading files from disk)
    let mut checked: HashSet<String> = HashSet::new();
    let mut nested: HashSet<String> = HashSet::new();

    for info in skills {
        // Find shared-md references in this skill's body
        for shared_ref in find_shared_md_refs(&info.body, base_dir) {
            let rel = &shared_ref.relative_path;
            let rel_path = Path::new(rel);

            if !rel_path.is_file() {
                continue; // S008 handles missing refs
            }

            // Check the file once for nesting, cache result
            if !checked.contains(rel) {
                checked.insert(rel.clone());
                if let Ok(content) = fs::read_to_string(rel_path) {
                    if contains_shared_md_ref(&content, base_dir) {
                        nested.insert(rel.clone());
                    }
                }
            }

            // Report for every referencing skill (not just the first)
            if nested.contains(rel) {
                diag.report_at(
                    LintRule::NestedRefDeep,
                    &info.path,
                    &format!(
                        "{}: references {} which itself references other shared .md files (keep references one level deep)",
                        info.path, shared_ref.reference
                    ),
                );
            }
        }
    }
}

/// S030: Detect orphaned files in skill scripts/ subdirectories.
///
/// Ownership is Markdown-first, then transitive through supported skill-local
/// harness scripts, plus bounded references from repository Python tests to
/// fixture assets. Repository-ignored paths and conventional cache artifacts
/// are excluded from discovery so local build noise cannot affect output.
pub(super) fn validate_orphaned_skill_files(
    base_dir: &str,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
) {
    let dir = Path::new(base_dir);
    if !dir.is_dir() {
        return;
    }

    let noise = traversal::SkillScriptNoiseFilter::discover();
    let python_test_sources = repository_python_test_sources(exclude);

    for entry in traversal::shallow_directories(dir, Path::new("."), None).entries {
        let path = entry.path;
        let dir_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if dir_name == "shared" {
            continue;
        }

        let skill_path = format!("{base_dir}/{dir_name}/SKILL.md");
        if exclude.is_excluded(&skill_path) {
            continue;
        }

        let scripts_dir = path.join("scripts");
        if !scripts_dir.is_dir() {
            continue;
        }

        let docs = read_skill_markdown_docs(&path);

        let candidates: Vec<ScriptAsset> = traversal::recursive_files_with_pruning(
            &scripts_dir,
            Path::new("."),
            None,
            traversal::should_descend_except_git_and_cache,
        )
        .entries
        .into_iter()
        .filter(|script| !exclude.is_excluded(&script.display))
        .filter(|script| !noise.is_noise(&script.path, &script.display))
        .map(|script| {
            let relative = traversal::display_path(&path, &script.path);
            let basename = script
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            ScriptAsset {
                path: script.path,
                display: script.display,
                relative,
                basename,
            }
        })
        .collect();

        let basename_counts = candidates
            .iter()
            .fold(HashMap::new(), |mut counts, script| {
                *counts.entry(script.basename.clone()).or_insert(0usize) += 1;
                counts
            });

        let mut live = reachable_script_assets(&candidates, &basename_counts, &docs);
        live.extend(python_test_owned_fixture_assets(
            &candidates,
            &python_test_sources,
        ));

        for (index, script) in candidates.iter().enumerate() {
            if live.contains(&index) {
                continue;
            }
            diag.report_at(
                LintRule::OrphanedSkillFiles,
                &script.display,
                &format!(
                    "{}: not referenced from skill-local Markdown or reachable harness scripts under {base_dir}/{dir_name}",
                    script.display
                ),
            );
        }
    }
}

struct ScriptAsset {
    path: PathBuf,
    display: String,
    relative: String,
    basename: String,
}

/// Mark assets reachable from skill-local Markdown, then transitively from
/// supported harness scripts that are already live.
fn reachable_script_assets(
    candidates: &[ScriptAsset],
    basename_counts: &HashMap<String, usize>,
    docs: &[String],
) -> HashSet<usize> {
    let mut live = HashSet::new();
    let mut queue = VecDeque::new();

    for (index, script) in candidates.iter().enumerate() {
        let unique_basename = basename_counts.get(&script.basename) == Some(&1);
        if docs
            .iter()
            .any(|doc| script_referenced(doc, &script.relative, &script.basename, unique_basename))
        {
            live.insert(index);
            queue.push_back(index);
        }
    }

    let contents: Vec<Option<String>> = candidates
        .iter()
        .map(|script| {
            if script_paths::script_kind(&script.path).is_some() {
                fs::read_to_string(&script.path).ok()
            } else {
                None
            }
        })
        .collect();

    while let Some(index) = queue.pop_front() {
        let Some(content) = contents[index].as_deref() else {
            continue;
        };
        let referrer = &candidates[index];
        for (target_index, target) in candidates.iter().enumerate() {
            if target_index == index || live.contains(&target_index) {
                continue;
            }
            let unique_basename = basename_counts.get(&target.basename) == Some(&1);
            if harness_references_asset(content, referrer, target, unique_basename) {
                live.insert(target_index);
                queue.push_back(target_index);
            }
        }
    }

    live
}

/// Return fixture assets owned by literal paths or deterministic `Path.glob`
/// enumeration in repository Python tests. This accepts only candidates below
/// `scripts/fixtures/`; an arbitrary test string can therefore never keep a
/// runtime harness alive.
fn python_test_owned_fixture_assets(
    candidates: &[ScriptAsset],
    test_sources: &[String],
) -> HashSet<usize> {
    let mut live = HashSet::new();

    for source in test_sources {
        let literals = python_string_literals(source);
        let directories = python_fixture_directories(source);
        let paired_suffixes: Vec<_> = PYTHON_STEM_SUFFIX
            .captures_iter(source)
            .map(|capture| (capture["from"].to_string(), capture["to"].to_string()))
            .collect();

        for (index, candidate) in candidates.iter().enumerate() {
            if !is_fixture_asset(candidate) {
                continue;
            }
            if literals.iter().any(|literal| literal == &candidate.display)
                || python_child_reference_matches(source, &directories, &candidate.display)
                || python_glob_reference_matches(
                    source,
                    &directories,
                    &paired_suffixes,
                    &candidate.display,
                    candidates,
                )
            {
                live.insert(index);
            }
        }
    }

    live
}

fn repository_python_test_sources(exclude: &ExcludeSet) -> Vec<String> {
    traversal::recursive_files(Path::new("."), Path::new("."), Some(exclude))
        .entries
        .into_iter()
        .filter(|entry| is_repository_python_test(&entry.path))
        .filter_map(|entry| fs::read_to_string(entry.path).ok())
        .map(|source| python_source_without_comments(&source))
        .collect()
}

fn is_repository_python_test(path: &Path) -> bool {
    path.extension().and_then(|extension| extension.to_str()) == Some("py")
        && path
            .components()
            .any(|component| component.as_os_str() == "tests")
        && !path
            .components()
            .any(|component| matches!(component.as_os_str().to_str(), Some("skills" | ".claude")))
}

fn is_fixture_asset(candidate: &ScriptAsset) -> bool {
    candidate.relative.starts_with("scripts/fixtures/")
}

/// Extract `NAME = root / "literal" / ...` fixture directory declarations.
/// We retain only repository-relative skill fixture paths; dynamic prefixes
/// such as `Path(__file__).parents[...]` are intentionally ignored.
fn python_fixture_directories(source: &str) -> HashMap<String, String> {
    let mut directories = HashMap::new();
    for line in source.lines() {
        let Some((name, expression)) = line.split_once('=') else {
            continue;
        };
        let name = name.trim();
        if !is_python_identifier(name) || !expression.contains('/') {
            continue;
        }
        let segments = python_string_literals(expression);
        for start in 0..segments.len() {
            let path = normalize_python_path(&segments[start..].join("/"));
            if is_skill_fixture_path(&path) {
                directories.insert(name.to_string(), path);
                break;
            }
        }
    }
    directories
}

fn python_child_reference_matches(
    source: &str,
    directories: &HashMap<String, String>,
    candidate: &str,
) -> bool {
    PYTHON_PATH_CHILD.captures_iter(source).any(|capture| {
        directories
            .get(&capture["directory"])
            .is_some_and(|directory| {
                normalize_python_path(&format!("{directory}/{}", &capture["child"])) == candidate
            })
    })
}

fn python_glob_reference_matches(
    source: &str,
    directories: &HashMap<String, String>,
    paired_suffixes: &[(String, String)],
    candidate: &str,
    candidates: &[ScriptAsset],
) -> bool {
    PYTHON_GLOB_CALL.captures_iter(source).any(|capture| {
        let Some(directory) = directories.get(&capture["directory"]) else {
            return false;
        };
        let Some(relative) = candidate.strip_prefix(&format!("{directory}/")) else {
            return false;
        };
        let Ok(glob) = Glob::new(&capture["pattern"]) else {
            return false;
        };
        let matcher = glob.compile_matcher();
        if matcher.is_match(relative) {
            return true;
        }
        paired_suffixes.iter().any(|(from, to)| {
            candidates
                .iter()
                .filter(|source| is_fixture_asset(source))
                .any(|source| {
                    let Some(source_relative) =
                        source.display.strip_prefix(&format!("{directory}/"))
                    else {
                        return false;
                    };
                    matcher.is_match(source_relative)
                        && paired_fixture_path(directory, source_relative, from, to).as_deref()
                            == Some(candidate)
                })
        })
    })
}

fn paired_fixture_path(directory: &str, relative: &str, from: &str, to: &str) -> Option<String> {
    let path = Path::new(relative);
    let stem = path.file_stem()?.to_str()?;
    let paired_stem = stem.strip_suffix(from)?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    Some(match parent {
        Some(parent) => format!("{directory}/{}/{paired_stem}{to}", parent.display()),
        None => format!("{directory}/{paired_stem}{to}"),
    })
}

fn is_python_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic() || character == '_')
        && chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn is_skill_fixture_path(path: &str) -> bool {
    let components: Vec<_> = Path::new(path)
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect();
    components
        .windows(2)
        .any(|window| window == ["scripts", "fixtures"])
        && (components.starts_with(&["skills"]) || components.starts_with(&[".claude", "skills"]))
}

fn normalize_python_path(value: &str) -> String {
    value
        .replace('\\', "/")
        .trim_start_matches("./")
        .trim_matches('/')
        .to_string()
}

/// A small lexical extractor sufficient for the supported literal forms. It
/// skips comments and never evaluates interpolation, imports, or calls.
fn python_string_literals(source: &str) -> Vec<String> {
    let mut literals = Vec::new();
    let mut characters = source.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '#' {
            while characters.next().is_some_and(|next| next != '\n') {}
        } else if matches!(character, '\'' | '\"') {
            let quote = character;
            let mut literal = String::new();
            let mut escaped = false;
            for next in characters.by_ref() {
                if escaped {
                    literal.push(next);
                    escaped = false;
                } else if next == '\\' {
                    escaped = true;
                } else if next == quote {
                    break;
                } else {
                    literal.push(next);
                }
            }
            literals.push(normalize_python_path(&literal));
        }
    }
    literals
}

fn python_source_without_comments(source: &str) -> String {
    let mut result = String::new();
    let mut quote: Option<char> = None;
    let mut comment = false;
    let mut escaped = false;
    for character in source.chars() {
        if comment {
            if character == '\n' {
                result.push(character);
                comment = false;
            }
        } else if let Some(active) = quote {
            result.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == active {
                quote = None;
            }
        } else if matches!(character, '\'' | '\"') {
            result.push(character);
            quote = Some(character);
        } else if character == '#' {
            comment = true;
        } else {
            result.push(character);
        }
    }
    result
}

/// Read every `*.md` under a skill directory in deterministic sorted order.
fn read_skill_markdown_docs(skill_dir: &Path) -> Vec<String> {
    traversal::recursive_files_with_pruning(
        skill_dir,
        Path::new("."),
        None,
        traversal::should_descend_except_git_and_cache,
    )
    .entries
    .into_iter()
    .filter(|entry| {
        entry
            .path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
    })
    .filter_map(|entry| fs::read_to_string(&entry.path).ok())
    .collect()
}

fn script_referenced(
    content: &str,
    relative_path: &str,
    basename: &str,
    unique_basename: bool,
) -> bool {
    path_token_referenced(content, relative_path)
        || (unique_basename && token_referenced(content, basename, true))
}

/// Harness ownership accepts skill-relative paths, `scripts/`-stripped paths,
/// paths relative to the referring harness directory, and unique basenames.
fn harness_references_asset(
    content: &str,
    referrer: &ScriptAsset,
    target: &ScriptAsset,
    unique_basename: bool,
) -> bool {
    if script_referenced(content, &target.relative, &target.basename, unique_basename) {
        return true;
    }
    if let Some(stripped) = target.relative.strip_prefix("scripts/") {
        if path_token_referenced(content, stripped) {
            return true;
        }
    }
    let Some(referrer_dir) = referrer.path.parent() else {
        return false;
    };
    let Ok(peer_relative) = target.path.strip_prefix(referrer_dir) else {
        return false;
    };
    let peer = peer_relative.to_string_lossy().replace('\\', "/");
    !peer.is_empty() && path_token_referenced(content, &peer)
}

/// Match a repository-style path token, including a single `./` prefix form.
fn path_token_referenced(content: &str, path: &str) -> bool {
    token_referenced(content, path, true)
        || (!path.is_empty()
            && !path.starts_with("./")
            && token_referenced(content, &format!("./{path}"), true))
}

/// True when `token` appears in `content` with exact token boundaries. When
/// requested, reject a `/` continuation so a path to a child is not an
/// ownership reference to its parent file.
fn token_referenced(content: &str, token: &str, reject_slash_suffix: bool) -> bool {
    let mut start = 0;
    while let Some(offset) = content[start..].find(token) {
        let abs = start + offset;
        let leading_boundary_ok = abs == 0
            || content[..abs]
                .chars()
                .next_back()
                .is_some_and(|prev| !is_name_boundary_char(prev));
        let end = abs + token.len();
        let trailing_boundary_ok = trailing_boundary_ok(&content[end..], reject_slash_suffix);
        if leading_boundary_ok && trailing_boundary_ok {
            return true;
        }
        start = end;
    }
    false
}

/// A trailing period is a sentence boundary only when it does not continue a
/// filename or path segment, such as in `helper.sh.bak` or `helper.sh./child`.
fn trailing_boundary_ok(suffix: &str, reject_slash_suffix: bool) -> bool {
    let Some(next) = suffix.chars().next() else {
        return true;
    };
    if !is_token_continuation(next, reject_slash_suffix) {
        return true;
    }
    if next != '.' {
        return false;
    }
    let after_period = &suffix[next.len_utf8()..];
    !after_period
        .chars()
        .next()
        .is_some_and(|after| is_token_continuation(after, reject_slash_suffix))
}

fn is_token_continuation(c: char, reject_slash_suffix: bool) -> bool {
    is_name_boundary_char(c) || (reject_slash_suffix && c == '/')
}

fn is_name_boundary_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')
}

/// S036: Check that referenced shared .md files > 100 lines have headings (TOC).
/// Only runs in plugin mode (called from validate_skill_content).
pub(super) fn validate_ref_no_toc(
    base_dir: &str,
    skills: &[SkillInfo],
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
) {
    let shared_dir = Path::new(base_dir).join("shared");
    if !shared_dir.is_dir() {
        return;
    }

    let mut checked: HashSet<String> = HashSet::new();

    for info in skills {
        for shared_ref in find_shared_md_refs(&info.body, base_dir) {
            let rel = &shared_ref.relative_path;

            if !checked.insert(rel.clone()) {
                continue;
            }

            if exclude.is_excluded(rel) {
                continue;
            }

            let rel_path = Path::new(rel);
            if !rel_path.is_file() {
                continue;
            }

            if let Ok(content) = fs::read_to_string(rel_path) {
                let line_count = content.lines().count();
                if line_count > REF_NO_TOC_THRESHOLD {
                    let document = MarkdownDocument::parse(&content);
                    let has_headings = !document.headings().is_empty();
                    if !has_headings {
                        diag.report_at(
                            LintRule::RefNoToc,
                            rel,
                            &format!(
                                "{}: references {} ({} lines) which has no headings for navigation",
                                info.path, shared_ref.reference, line_count
                            ),
                        );
                    }
                }
            }
        }
    }
}

/// S048: Detect non-descriptive reference file names in skill directories.
/// Recursively walks skill-local Markdown outside `scripts/`, excluding
/// `SKILL.md`, and flags names matching the generic denylist.
pub(super) fn validate_generic_ref_names(
    base_dir: &str,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
) {
    let dir = Path::new(base_dir);
    if !dir.is_dir() {
        return;
    }

    for entry in traversal::shallow_directories(dir, Path::new("."), None).entries {
        let path = entry.path;
        let dir_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if dir_name == "shared" {
            continue;
        }

        let skill_path = format!("{base_dir}/{dir_name}/SKILL.md");
        if exclude.is_excluded(&skill_path) {
            continue;
        }

        for file_entry in traversal::recursive_files_with_pruning(
            &path,
            Path::new("."),
            None,
            should_descend_skill_reference_directory,
        )
        .entries
        {
            let file_path = file_entry.path;
            let file_name = match file_path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };

            if file_name == "SKILL.md" {
                continue;
            }

            let display_path = file_entry.display;
            if exclude.is_excluded(&display_path) {
                continue;
            }

            if RE_GENERIC_REF_NAME.is_match(&file_name) {
                diag.report_at(
                    LintRule::RefNameGeneric,
                    &display_path,
                    &format!(
                        "{}: non-descriptive reference file name (use a descriptive name like 'form-validation-rules.md')",
                        display_path
                    ),
                );
            }
        }
    }
}

fn should_descend_skill_reference_directory(entry: traversal::DirectoryEntry<'_>) -> bool {
    traversal::should_descend_except_git(entry)
        && !(entry.depth() == 1 && entry.file_name() == "scripts")
}
