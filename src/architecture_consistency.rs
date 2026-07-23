//! Test-only checks that keep the architecture contracts tied to their code.

use crate::fence::{CodeFenceTracker, LineClass};
use crate::markdown::MarkdownDocument;
use crate::markdown_refs::percent_decode_once;
use crate::repo_path::{PathProbe, ResolutionBase, resolve_repo_path};
use crate::test_helpers::CwdGuard;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

const ARCHITECTURE_DOCUMENTS: &[&str] =
    &["ARCHITECTURAL_GUIDELINES.md", "ARCHITECTURAL_INVARIANTS.md"];
const EXTERNAL_OR_LANGUAGE_REFERENCES: &[&str] = &["Result", "expect", "serial_test", "unwrap"];
const CONCEPTUAL_RUST_PATHS: &[&str] = &["validators/**"];

struct SymbolReference {
    rendered: &'static str,
    source: &'static str,
    declaration_needle: &'static str,
}

// This intentionally textual index makes an architecture-document reference a
// conscious, reviewable dependency on the named declaration.
const ARCHITECTURE_SYMBOLS: &[SymbolReference] = &[
    SymbolReference {
        rendered: "ALL_RULES",
        source: "src/rules.rs",
        declaration_needle: "pub const ALL_RULES",
    },
    SymbolReference {
        rendered: "LintRule",
        source: "src/rules.rs",
        declaration_needle: "pub enum LintRule",
    },
    SymbolReference {
        rendered: "LintRule::is_autofixable",
        source: "src/rules.rs",
        declaration_needle: "pub const fn is_autofixable",
    },
    SymbolReference {
        rendered: "DiagnosticCollector",
        source: "src/diagnostic.rs",
        declaration_needle: "pub struct DiagnosticCollector",
    },
    SymbolReference {
        rendered: "DiagnosticCollector::report",
        source: "src/diagnostic.rs",
        declaration_needle: "pub fn report(",
    },
    SymbolReference {
        rendered: "DiagnosticCollector::report_at",
        source: "src/diagnostic.rs",
        declaration_needle: "pub fn report_at(",
    },
    SymbolReference {
        rendered: "Diagnostic::subject_path",
        source: "src/diagnostic.rs",
        declaration_needle: "pub subject_path:",
    },
    SymbolReference {
        rendered: "DiagnosticMetadata",
        source: "src/diagnostic.rs",
        declaration_needle: "pub struct DiagnosticMetadata",
    },
    SymbolReference {
        rendered: "Severity",
        source: "src/diagnostic.rs",
        declaration_needle: "pub enum Severity",
    },
    SymbolReference {
        rendered: "LintConfig",
        source: "src/config.rs",
        declaration_needle: "pub struct LintConfig",
    },
    SymbolReference {
        rendered: "LintConfig::load",
        source: "src/config.rs",
        declaration_needle: "pub fn load(",
    },
    SymbolReference {
        rendered: "LintConfig::apply_cli_mode",
        source: "src/config.rs",
        declaration_needle: "pub fn apply_cli_mode(",
    },
    SymbolReference {
        rendered: "ExcludeSet",
        source: "src/config.rs",
        declaration_needle: "pub struct ExcludeSet",
    },
    SymbolReference {
        rendered: "ExcludeSet::is_excluded",
        source: "src/config.rs",
        declaration_needle: "pub fn is_excluded(",
    },
    SymbolReference {
        rendered: "DetectedSurfaces",
        source: "src/platforms.rs",
        declaration_needle: "pub struct DetectedSurfaces",
    },
    SymbolReference {
        rendered: "DetectedSurfaces::discover",
        source: "src/platforms.rs",
        declaration_needle: "pub fn discover(",
    },
    SymbolReference {
        rendered: "DetectedSurfaces::resolve",
        source: "src/platforms.rs",
        declaration_needle: "pub fn resolve(",
    },
    SymbolReference {
        rendered: "PlatformOverrides",
        source: "src/config.rs",
        declaration_needle: "pub struct PlatformOverrides",
    },
    SymbolReference {
        rendered: "ValidationTargets",
        source: "src/platforms.rs",
        declaration_needle: "pub struct ValidationTargets",
    },
    SymbolReference {
        rendered: "run_all_with_targets",
        source: "src/validators/mod.rs",
        declaration_needle: "pub fn run_all_with_targets(",
    },
    SymbolReference {
        rendered: "RunPolicy",
        source: "src/config.rs",
        declaration_needle: "pub struct RunPolicy",
    },
    SymbolReference {
        rendered: "CliMode",
        source: "src/config.rs",
        declaration_needle: "pub enum CliMode",
    },
    SymbolReference {
        rendered: "LintContext",
        source: "src/context.rs",
        declaration_needle: "pub struct LintContext",
    },
    SymbolReference {
        rendered: "LintMode",
        source: "src/context.rs",
        declaration_needle: "pub enum LintMode",
    },
    SymbolReference {
        rendered: "LintContext::new",
        source: "src/context.rs",
        declaration_needle: "pub fn new(base_path:",
    },
    SymbolReference {
        rendered: "ManifestState",
        source: "src/context.rs",
        declaration_needle: "pub enum ManifestState",
    },
    SymbolReference {
        rendered: "MarkdownDocument",
        source: "src/markdown.rs",
        declaration_needle: "pub struct MarkdownDocument",
    },
    SymbolReference {
        rendered: "markdown_commands",
        source: "src/main.rs",
        declaration_needle: "mod markdown_commands;",
    },
    SymbolReference {
        rendered: "script_paths",
        source: "src/main.rs",
        declaration_needle: "mod script_paths;",
    },
    SymbolReference {
        rendered: "autofix::apply_fix",
        source: "src/autofix.rs",
        declaration_needle: "pub fn apply_fix(",
    },
    SymbolReference {
        rendered: "run_autofix",
        source: "src/main.rs",
        declaration_needle: "fn run_autofix(",
    },
    SymbolReference {
        rendered: "resolve_repo_root",
        source: "src/main.rs",
        declaration_needle: "fn resolve_repo_root(",
    },
    SymbolReference {
        rendered: "CwdGuard",
        source: "src/test_helpers.rs",
        declaration_needle: "pub struct CwdGuard",
    },
    SymbolReference {
        rendered: "set_current_dir",
        source: "src/main.rs",
        declaration_needle: "set_current_dir",
    },
];

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
struct Violation {
    document: String,
    line: usize,
    kind: &'static str,
    token: String,
    message: String,
}

impl Violation {
    fn new(
        document: &str,
        line: usize,
        kind: &'static str,
        token: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            document: document.into(),
            line,
            kind,
            token: token.into(),
            message: message.into(),
        }
    }
}

static CONTRACT_HEADING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^### (G|I)-([A-Za-z][A-Za-z0-9]*)-([1-9][0-9]*): (\S.*)$").unwrap()
});
static SYMBOL_SHAPE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:[A-Z][A-Za-z0-9]*::[a-z_][A-Za-z0-9_]*|[a-z_][A-Za-z0-9_]*::[a-z_][A-Za-z0-9_]*|[A-Z][A-Za-z0-9]*|[a-z_][A-Za-z0-9_]*)$").unwrap()
});

fn architecture_documents() -> Vec<(&'static str, String)> {
    ARCHITECTURE_DOCUMENTS
        .iter()
        .map(|path| {
            (
                *path,
                fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(path))
                    .unwrap_or_else(|error| panic!("cannot read {path}: {error}")),
            )
        })
        .collect()
}

fn raw_level_three_headings(content: &str) -> Vec<(usize, String)> {
    let mut tracker = CodeFenceTracker::new();
    content
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let class = tracker.process_line(line);
            (class == LineClass::Outside && line.starts_with("### "))
                .then(|| (index + 1, line.to_owned()))
        })
        .collect()
}

fn validate_contract_ids(documents: &[(&str, String)], violations: &mut Vec<Violation>) {
    let mut identifiers = HashMap::new();
    for (path, content) in documents {
        let expected = if *path == "ARCHITECTURAL_GUIDELINES.md" {
            "G"
        } else {
            "I"
        };
        for (line, heading) in raw_level_three_headings(content) {
            let Some(captures) = CONTRACT_HEADING.captures(&heading) else {
                violations.push(Violation::new(
                    path,
                    line,
                    "contract-id",
                    &heading,
                    format!("malformed contract heading: {heading}"),
                ));
                continue;
            };
            let prefix = captures.get(1).unwrap().as_str();
            let id = heading[4..].split_once(':').unwrap().0;
            if prefix != expected {
                violations.push(Violation::new(
                    path,
                    line,
                    "contract-id",
                    id,
                    format!("{path}:{line}: contract ID {id} must begin with {expected}-"),
                ));
            }
            if let Some((previous_path, previous_line)) =
                identifiers.insert(id.to_owned(), ((*path).to_owned(), line))
            {
                violations.push(Violation::new(path, line, "contract-id", id, format!("duplicate contract ID {id}; first appears at {previous_path}:{previous_line}")));
            }
        }
    }
}

fn validate_mechanical_backing(path: &str, content: &str, violations: &mut Vec<Violation>) {
    if path != "ARCHITECTURAL_INVARIANTS.md" {
        return;
    }
    let headings = raw_level_three_headings(content);
    let lines: Vec<_> = content.lines().collect();
    for (position, (line, heading)) in headings.iter().enumerate() {
        let Some(captures) = CONTRACT_HEADING.captures(heading) else {
            continue;
        };
        if captures.get(1).unwrap().as_str() != "I" {
            continue;
        }
        let id = heading[4..].split_once(':').unwrap().0;
        let end = headings
            .get(position + 1)
            .map_or(lines.len(), |(next, _)| next - 1);
        let clauses: Vec<_> = ((*line)..end)
            .filter(|index| lines[*index].starts_with("Mechanical backing:"))
            .collect();
        if clauses.len() != 1 {
            let detail = if clauses.is_empty() {
                "is missing"
            } else {
                "appears more than once"
            };
            violations.push(Violation::new(
                path,
                *line,
                "mechanical-backing",
                id,
                format!("invariant {id} {detail} its Mechanical backing clause"),
            ));
            continue;
        }
        let clause_line = clauses[0];
        let own_text = lines[clause_line]["Mechanical backing:".len()..].trim();
        let continuation = lines
            .get(clause_line + 1)
            .is_some_and(|line| !line.trim().is_empty());
        if own_text.is_empty() && !continuation {
            violations.push(Violation::new(
                path,
                clause_line + 1,
                "mechanical-backing",
                id,
                format!("invariant {id} has an empty Mechanical backing clause"),
            ));
        }
    }
}

fn github_slug(text: &str) -> String {
    let mut slug = String::new();
    let mut hyphen = false;
    for character in text.chars().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() || character == '-' {
            slug.push(character);
            hyphen = false;
        } else if character.is_whitespace() && !slug.is_empty() && !hyphen {
            slug.push('-');
            hyphen = true;
        }
    }
    slug.trim_matches('-').to_owned()
}

fn validate_local_links(path: &str, content: &str, violations: &mut Vec<Violation>) {
    let document = MarkdownDocument::parse(content);
    for link in document.links() {
        let authored = &link.raw_destination;
        if authored.starts_with("http:")
            || authored.starts_with("https:")
            || authored.starts_with("mailto:")
            || authored.starts_with('#')
        {
            continue;
        }
        let decoded = percent_decode_once(&link.destination);
        let (target, fragment) = decoded
            .split_once('#')
            .map_or((decoded.as_str(), None), |(target, fragment)| {
                (target, Some(fragment))
            });
        let source = Path::new(path);
        match resolve_repo_path(source, target, ResolutionBase::SourceRelative) {
            PathProbe::File(normalized) => {
                if let Some(fragment) = fragment {
                    if normalized
                        .extension()
                        .is_some_and(|extension| extension == "md")
                    {
                        let target_content = fs::read_to_string(&normalized).unwrap_or_default();
                        let target_document = MarkdownDocument::parse(target_content);
                        let expected = github_slug(fragment);
                        if !target_document
                            .headings()
                            .iter()
                            .any(|heading| github_slug(&heading.text) == expected)
                        {
                            violations.push(Violation::new(
                                path,
                                link.line,
                                "local-link",
                                authored,
                                format!(
                                    "{path}:{}: fragment #{fragment} is missing from {}",
                                    link.line,
                                    normalized.display()
                                ),
                            ));
                        }
                    }
                }
            }
            PathProbe::Missing(normalized) => violations.push(Violation::new(
                path,
                link.line,
                "local-link",
                authored,
                format!(
                    "{path}:{}: {authored} resolves to missing {}",
                    link.line,
                    normalized.display()
                ),
            )),
            PathProbe::Directory(normalized) => violations.push(Violation::new(
                path,
                link.line,
                "local-link",
                authored,
                format!(
                    "{path}:{}: {authored} resolves to non-file {}",
                    link.line,
                    normalized.display()
                ),
            )),
            PathProbe::Rejected => violations.push(Violation::new(
                path,
                link.line,
                "local-link",
                authored,
                format!("{path}:{}: unsafe local link {authored}", link.line),
            )),
        }
    }
}

fn is_rust_path(token: &str) -> Option<String> {
    if CONCEPTUAL_RUST_PATHS.contains(&token) || token.contains('*') {
        return None;
    }
    let trimmed = token.trim_start_matches("./");
    if !trimmed.ends_with(".rs") || trimmed.contains(' ') || Path::new(trimmed).is_absolute() {
        return None;
    }
    Some(if trimmed.starts_with("src/") {
        trimmed.into()
    } else {
        format!("src/{trimmed}")
    })
}

fn validate_rust_paths(path: &str, content: &str, violations: &mut Vec<Violation>) {
    for code in MarkdownDocument::parse(content).inline_code() {
        let Some(normalized) = is_rust_path(&code.literal) else {
            continue;
        };
        match crate::repo_path::probe_repo_relative(Path::new(&normalized)) {
            PathProbe::File(_) => {}
            _ => violations.push(Violation::new(
                path,
                code.start_line,
                "rust-path",
                &code.literal,
                format!(
                    "{path}:{}: Rust path `{}` resolves to missing or unsafe {normalized}",
                    code.start_line, code.literal
                ),
            )),
        }
    }
}

fn documented_symbols(documents: &[(&str, String)]) -> HashSet<String> {
    documents
        .iter()
        .flat_map(|(_, content)| {
            MarkdownDocument::parse(content)
                .inline_code()
                .iter()
                .map(|code| code.literal.clone())
                .collect::<Vec<_>>()
        })
        .collect()
}

fn validate_symbols(documents: &[(&str, String)], violations: &mut Vec<Violation>) {
    let documented = documented_symbols(documents);
    let mut indexed = HashSet::new();
    for reference in ARCHITECTURE_SYMBOLS {
        if !indexed.insert(reference.rendered) {
            violations.push(Violation::new(
                "ARCHITECTURE_SYMBOLS",
                0,
                "symbol-index",
                reference.rendered,
                format!("duplicate rendered symbol {}", reference.rendered),
            ));
        }
        if !documented.contains(reference.rendered) {
            violations.push(Violation::new(
                "ARCHITECTURE_SYMBOLS",
                0,
                "symbol-index",
                reference.rendered,
                format!(
                    "indexed symbol {} is absent from architecture documents",
                    reference.rendered
                ),
            ));
        }
        let source = Path::new(reference.source);
        match crate::repo_path::probe_repo_relative(source) {
            PathProbe::File(_)
                if source
                    .extension()
                    .is_some_and(|extension| extension == "rs") =>
            {
                match fs::read_to_string(source) {
                    Ok(contents) if contents.contains(reference.declaration_needle) => {}
                    _ => violations.push(Violation::new(
                        "ARCHITECTURE_SYMBOLS",
                        0,
                        "symbol-index",
                        reference.rendered,
                        format!(
                            "{} no longer contains declaration needle {:?}",
                            reference.source, reference.declaration_needle
                        ),
                    )),
                }
            }
            _ => violations.push(Violation::new(
                "ARCHITECTURE_SYMBOLS",
                0,
                "symbol-index",
                reference.rendered,
                format!("indexed source {} is missing or unsafe", reference.source),
            )),
        }
    }
    for (path, content) in documents {
        for code in MarkdownDocument::parse(content).inline_code() {
            let token = &code.literal;
            if !SYMBOL_SHAPE.is_match(token)
                || EXTERNAL_OR_LANGUAGE_REFERENCES.contains(&token.as_str())
                || is_rust_path(token).is_some()
                || token == "Missing"
            {
                continue;
            }
            if !indexed.contains(token.as_str()) {
                violations.push(Violation::new(
                    path,
                    code.start_line,
                    "symbol-index",
                    token,
                    format!(
                        "{path}:{}: in-repository-looking symbol `{token}` is not indexed",
                        code.start_line
                    ),
                ));
            }
        }
    }
}

#[test]
#[serial_test::serial]
fn architectural_documents_reference_current_contracts() {
    let _guard = CwdGuard::new();
    std::env::set_current_dir(env!("CARGO_MANIFEST_DIR")).expect("manifest directory is readable");
    let documents = architecture_documents();
    let mut violations = Vec::new();
    validate_contract_ids(&documents, &mut violations);
    for (path, content) in &documents {
        validate_mechanical_backing(path, content, &mut violations);
        validate_local_links(path, content, &mut violations);
        validate_rust_paths(path, content, &mut violations);
    }
    validate_symbols(&documents, &mut violations);
    violations.sort();
    assert!(
        violations.is_empty(),
        "architecture-document contract drift:\n{}",
        violations
            .into_iter()
            .map(|violation| violation.message)
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(path: &str, content: &str) -> (String, String) {
        (path.into(), content.into())
    }

    #[test]
    fn rejects_malformed_wrong_and_duplicate_contract_ids_but_ignores_fences() {
        let documents = [
            doc(
                "ARCHITECTURAL_GUIDELINES.md",
                "### I-Wrong-1: wrong\n\n```md\n### G-Fenced-1: ignored\n```\n### malformed\n",
            ),
            doc(
                "ARCHITECTURAL_INVARIANTS.md",
                "### I-Wrong-1: duplicate\nMechanical backing: test\n",
            ),
        ];
        let documents: Vec<_> = documents
            .iter()
            .map(|(path, content)| (path.as_str(), content.clone()))
            .collect();
        let mut violations = Vec::new();
        validate_contract_ids(&documents, &mut violations);
        let messages: Vec<_> = violations
            .iter()
            .map(|violation| violation.message.as_str())
            .collect();
        assert!(
            messages
                .iter()
                .any(|message| message.contains("must begin with G-"))
        );
        assert!(
            messages
                .iter()
                .any(|message| message.contains("malformed contract heading"))
        );
        assert!(
            messages
                .iter()
                .any(|message| message.contains("duplicate contract ID"))
        );
        assert!(!messages.iter().any(|message| message.contains("Fenced")));
    }

    #[test]
    fn checks_mechanical_backing_clauses() {
        let mut violations = Vec::new();
        validate_mechanical_backing(
            "ARCHITECTURAL_INVARIANTS.md",
            "### I-One-1: missing\ntext\n### I-Two-1: empty\nMechanical backing:\n\n### I-Three-1: duplicate\nMechanical backing: one\nMechanical backing: two\n### I-Four-1: valid\nMechanical backing:\ncontinued\n",
            &mut violations,
        );
        assert_eq!(violations.len(), 3);
        assert!(
            violations
                .iter()
                .any(|violation| violation.message.contains("missing"))
        );
        assert!(
            violations
                .iter()
                .any(|violation| violation.message.contains("empty"))
        );
        assert!(
            violations
                .iter()
                .any(|violation| violation.message.contains("more than once"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn validates_local_markdown_targets_and_fragments() {
        let temporary = tempfile::tempdir().unwrap();
        let _guard = CwdGuard::new();
        std::env::set_current_dir(temporary.path()).unwrap();
        fs::write("target.md", "# Existing heading\n").unwrap();
        let mut violations = Vec::new();
        validate_local_links(
            "ARCHITECTURAL_GUIDELINES.md",
            "[ok](target.md) [anchor](target.md#existing-heading) [remote](https://example.com) [missing](missing.md) [escape](../outside.md)",
            &mut violations,
        );
        assert_eq!(violations.len(), 2);
        assert!(
            violations
                .iter()
                .any(|violation| violation.message.contains("missing.md"))
        );
        assert!(
            violations
                .iter()
                .any(|violation| violation.message.contains("unsafe local link"))
        );
    }

    #[test]
    fn stale_symbol_spellings_are_not_accepted_by_the_index() {
        let documents = [doc(
            "ARCHITECTURAL_INVARIANTS.md",
            "`PlatformDetection` `run_all_with_platforms`\n",
        )];
        let documents: Vec<_> = documents
            .iter()
            .map(|(path, content)| (path.as_str(), content.clone()))
            .collect();
        let mut violations = Vec::new();
        validate_symbols(&documents, &mut violations);
        assert!(
            violations
                .iter()
                .any(|violation| violation.token == "PlatformDetection")
        );
        assert!(
            violations
                .iter()
                .any(|violation| violation.token == "run_all_with_platforms")
        );
    }
}
