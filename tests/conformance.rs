use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde::Deserialize;
use serde_json::Value;

const REQUIRED_CLASS_COVERAGE: &[&str] = &[
    "all",
    "autofix",
    "basic-mode",
    "claude-agent-prompt",
    "claude-skill-prompt",
    "codex",
    "codex-plugin",
    "cursor-legacy",
    "cursor-mdc",
    "cursor-hooks-basic",
    "cursor-hooks-plugin",
    "global-suppression",
    "hook-schema",
    "json",
    "mcp",
    "m024-whitespace",
    "nested-agents-md",
    "normal",
    "pedantic",
    "per-file-suppression",
    "plugin-mode",
    "security-policy",
    "q005-unbounded-retry",
    "q006",
    "agent-stop",
    "desc-overlap",
    "cursor-agents",
    "userconfig",
];

const REQUIRED_SMOKE_COVERAGE: &[&str] = &[
    "i001-empty",
    "i002-instruction-secret",
    "i003-bare-extension",
    "i004-generic",
    "q002-quoted-example",
    "q002-safety-negative",
    "q006-multi-format-clean",
    "q006-output-conflict",
    "skill-structural",
    "s055-script-errhand",
];

#[derive(Debug, Clone, Copy, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "kebab-case")]
enum CaseClass {
    Clean,
    Broken,
    HardNegative,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseManifest {
    class: CaseClass,
    repository: String,
    covers: Vec<String>,
    invocation: Invocation,
    expected_exit_code: i32,
    expected_status: String,
    expected_active_platforms: Vec<String>,
    expected_suppressed: u64,
    expected_diagnostics: Vec<DiagnosticIdentity>,
    allowed_additional_diagnostics: Vec<AllowedDiagnostic>,
    #[serde(default)]
    post_fix: Vec<ExpectedFile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Invocation {
    mode: String,
    arguments: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct DiagnosticIdentity {
    code: String,
    name: String,
    severity: String,
    subject_path: Option<String>,
    #[serde(default)]
    related_subjects: Vec<String>,
    #[serde(default)]
    suggestion: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AllowedDiagnostic {
    diagnostic: DiagnosticIdentity,
    justification: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedFile {
    path: String,
    contents: String,
}

#[derive(Debug, Deserialize)]
struct OutputReport {
    mode: Option<String>,
    strictness: String,
    active_platforms: Vec<String>,
    status: String,
    counts: OutputCounts,
    diagnostics: Vec<OutputDiagnostic>,
}

#[derive(Debug, Deserialize)]
struct OutputCounts {
    suppressed: u64,
}

#[derive(Debug, Deserialize)]
struct OutputDiagnostic {
    code: String,
    name: String,
    severity: String,
    subject_path: Option<String>,
    #[serde(default)]
    related_subjects: Vec<String>,
    suggestion: Option<String>,
}

impl From<&OutputDiagnostic> for DiagnosticIdentity {
    fn from(diagnostic: &OutputDiagnostic) -> Self {
        Self {
            code: diagnostic.code.clone(),
            name: diagnostic.name.clone(),
            severity: diagnostic.severity.clone(),
            subject_path: diagnostic.subject_path.clone(),
            related_subjects: diagnostic.related_subjects.clone(),
            suggestion: diagnostic.suggestion.clone(),
        }
    }
}

struct LoadedCase {
    name: String,
    manifest: CaseManifest,
}

#[test]
fn checked_in_conformance_corpus_matches_released_cli_contract() {
    let corpus = corpus_root();
    let cases = load_cases(&corpus);
    assert_fixture_convention(&corpus, &cases);
    assert_required_coverage(&cases);

    let schema: Value =
        serde_json::from_str(include_str!("../schemas/diagnostic-output-v1.schema.json"))
            .expect("checked-in diagnostic schema parses");
    let schema_validator =
        jsonschema::validator_for(&schema).expect("checked-in diagnostic schema compiles");

    for case in cases {
        run_case(&corpus, &case, &schema_validator);
    }
}

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/conformance")
}

fn load_cases(corpus: &Path) -> Vec<LoadedCase> {
    let manifests = corpus.join("manifests");
    let mut paths: Vec<_> = fs::read_dir(&manifests)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", manifests.display()))
        .map(|entry| entry.expect("manifest directory entry is readable").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "conformance corpus has no manifests");

    paths
        .into_iter()
        .map(|path| {
            let name = path
                .file_stem()
                .and_then(|name| name.to_str())
                .expect("manifest has a UTF-8 filename")
                .to_string();
            let bytes = fs::read(&path)
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
            let manifest = serde_json::from_slice(&bytes)
                .unwrap_or_else(|error| panic!("invalid manifest {}: {error}", path.display()));
            LoadedCase { name, manifest }
        })
        .collect()
}

fn assert_fixture_convention(corpus: &Path, cases: &[LoadedCase]) {
    let repositories = corpus.join("repositories");
    let mut repository_names: BTreeSet<String> = fs::read_dir(&repositories)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", repositories.display()))
        .map(|entry| {
            let entry = entry.expect("repository directory entry is readable");
            assert!(
                entry
                    .file_type()
                    .expect("fixture file type is readable")
                    .is_dir(),
                "{} must contain only repository directories",
                repositories.display()
            );
            entry
                .file_name()
                .into_string()
                .expect("repository fixture name is UTF-8")
        })
        .collect();

    for case in cases {
        assert_eq!(
            case.name, case.manifest.repository,
            "{} must name its same-named repository fixture",
            case.name
        );
        assert!(
            repository_names.remove(&case.manifest.repository),
            "{} references a missing or duplicate repository fixture",
            case.name
        );
        assert!(
            !case.manifest.covers.is_empty(),
            "{} must declare coverage tags",
            case.name
        );
        assert!(
            case.manifest
                .invocation
                .arguments
                .windows(2)
                .any(|arguments| { arguments == ["--format".to_string(), "json".to_string()] }),
            "{} must exercise structured JSON output",
            case.name
        );
        assert_eq!(
            case.manifest
                .invocation
                .arguments
                .last()
                .map(String::as_str),
            Some("."),
            "{} must lint the copied repository root",
            case.name
        );
        for diagnostic in &case.manifest.expected_diagnostics {
            match diagnostic.subject_path.as_deref() {
                Some(path) => assert_safe_relative_path(&case.name, path),
                None => {
                    assert!(
                        !diagnostic.related_subjects.is_empty(),
                        "{} pathless expected diagnostics must include related_subjects",
                        case.name
                    );
                    for path in &diagnostic.related_subjects {
                        assert_safe_relative_path(&case.name, path);
                    }
                }
            }
        }
        for allowed in &case.manifest.allowed_additional_diagnostics {
            assert!(
                !allowed.justification.trim().is_empty(),
                "{} has an allowed diagnostic without a justification",
                case.name
            );
            match allowed.diagnostic.subject_path.as_deref() {
                Some(path) => assert_safe_relative_path(&case.name, path),
                None => {
                    assert!(
                        !allowed.diagnostic.related_subjects.is_empty(),
                        "{} pathless allowed diagnostics must include related_subjects",
                        case.name
                    );
                    for path in &allowed.diagnostic.related_subjects {
                        assert_safe_relative_path(&case.name, path);
                    }
                }
            }
            assert!(
                !case
                    .manifest
                    .expected_diagnostics
                    .contains(&allowed.diagnostic),
                "{} lists the same diagnostic as expected and additional",
                case.name
            );
        }
        for expected in &case.manifest.post_fix {
            assert_safe_relative_path(&case.name, &expected.path);
        }
        if case.manifest.covers.iter().any(|tag| tag == "autofix") {
            assert!(
                !case.manifest.post_fix.is_empty(),
                "{}: autofix cases must declare post-fix files",
                case.name
            );
            let changed_files = case
                .manifest
                .post_fix
                .iter()
                .filter(|expected| {
                    fs::read_to_string(
                        repositories
                            .join(&case.manifest.repository)
                            .join(&expected.path),
                    )
                    .is_ok_and(|original| original != expected.contents)
                })
                .count();
            match case.manifest.class {
                CaseClass::Broken => assert!(
                    changed_files > 0,
                    "{}: a broken autofix fixture must begin unfixed",
                    case.name
                ),
                CaseClass::Clean | CaseClass::HardNegative => assert_eq!(
                    changed_files, 0,
                    "{}: a clean or hard-negative autofix fixture must not be mutated",
                    case.name
                ),
            }
        }
    }

    assert!(
        repository_names.is_empty(),
        "repository fixtures without manifests: {repository_names:?}"
    );
}

fn assert_required_coverage(cases: &[LoadedCase]) {
    let mut coverage: BTreeMap<&str, BTreeSet<CaseClass>> = BTreeMap::new();
    for case in cases {
        for tag in &case.manifest.covers {
            coverage.entry(tag).or_default().insert(case.manifest.class);
        }
    }
    let all_classes =
        BTreeSet::from([CaseClass::Clean, CaseClass::Broken, CaseClass::HardNegative]);
    for required in REQUIRED_CLASS_COVERAGE {
        assert_eq!(
            coverage.get(required).cloned().unwrap_or_default(),
            all_classes,
            "coverage tag '{required}' must have clean, broken, and hard-negative cases"
        );
    }
    for required in REQUIRED_SMOKE_COVERAGE {
        assert!(
            coverage.contains_key(required),
            "missing required smoke coverage tag '{required}'"
        );
    }
}

fn run_case(corpus: &Path, case: &LoadedCase, schema: &jsonschema::Validator) {
    let temp = tempfile::tempdir().expect("temporary repository is created");
    copy_tree(
        &corpus.join("repositories").join(&case.manifest.repository),
        temp.path(),
    );
    let git = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(temp.path())
        .output()
        .expect("git init runs for conformance fixture");
    assert!(
        git.status.success(),
        "{}: git init failed: {}",
        case.name,
        String::from_utf8_lossy(&git.stderr)
    );

    let first = invoke(temp.path(), &case.manifest.invocation.arguments);
    let first_report = assert_run(case, &first, schema);
    assert_report(case, &first_report);

    if !case.manifest.post_fix.is_empty() {
        for expected in &case.manifest.post_fix {
            let path = temp.path().join(&expected.path);
            assert_eq!(
                fs::read_to_string(&path).unwrap_or_else(|error| panic!(
                    "{}: cannot read {}: {error}",
                    case.name,
                    path.display()
                )),
                expected.contents,
                "{}: unexpected post-fix contents for {}",
                case.name,
                expected.path
            );
        }

        let before_second_run: Vec<_> = case
            .manifest
            .post_fix
            .iter()
            .map(|expected| {
                let path = temp.path().join(&expected.path);
                (
                    expected.path.clone(),
                    fs::read(path).expect("post-fix file is readable"),
                )
            })
            .collect();
        let second = invoke(temp.path(), &case.manifest.invocation.arguments);
        let second_report = assert_run(case, &second, schema);
        assert_report(case, &second_report);
        for (relative, before) in before_second_run {
            assert_eq!(
                fs::read(temp.path().join(&relative)).expect("post-fix file remains readable"),
                before,
                "{}: autofix is not idempotent for {relative}",
                case.name
            );
        }
    }
}

fn invoke(repository: &Path, arguments: &[String]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_agent-lint"))
        .args(arguments)
        .current_dir(repository)
        .output()
        .expect("agent-lint binary runs")
}

fn assert_run(case: &LoadedCase, output: &Output, schema: &jsonschema::Validator) -> OutputReport {
    assert_eq!(
        output.status.code(),
        Some(case.manifest.expected_exit_code),
        "{}: wrong exit status; stderr:\n{}",
        case.name,
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{}: stdout is not one JSON document: {error}\nstdout:\n{}\nstderr:\n{}",
            case.name,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    if let Err(error) = schema.validate(&value) {
        panic!(
            "{}: JSON output failed schema validation: {error}\n{value:#}",
            case.name
        );
    }
    serde_json::from_value(value)
        .unwrap_or_else(|error| panic!("{}: cannot read JSON report fields: {error}", case.name))
}

fn assert_report(case: &LoadedCase, report: &OutputReport) {
    assert_eq!(
        report.mode.as_deref(),
        Some(case.manifest.invocation.mode.as_str()),
        "{}: wrong detected invocation mode",
        case.name
    );
    assert_eq!(
        report.strictness,
        expected_strictness(&case.manifest.invocation.arguments),
        "{}: wrong resolved strictness",
        case.name
    );
    assert_eq!(
        report.active_platforms, case.manifest.expected_active_platforms,
        "{}: wrong platform activation",
        case.name
    );
    assert_eq!(
        report.status, case.manifest.expected_status,
        "{}: wrong report status",
        case.name
    );
    assert_eq!(
        report.counts.suppressed, case.manifest.expected_suppressed,
        "{}: wrong suppressed diagnostic count",
        case.name
    );

    assert_expected_suggestions(case, report);

    let diagnostics: Vec<_> = report
        .diagnostics
        .iter()
        .map(DiagnosticIdentity::from)
        .map(|mut diagnostic| {
            diagnostic.suggestion = None;
            diagnostic
        })
        .collect();
    let expected_diagnostics: Vec<_> = case
        .manifest
        .expected_diagnostics
        .iter()
        .cloned()
        .map(|mut diagnostic| {
            diagnostic.suggestion = None;
            diagnostic
        })
        .collect();

    if case.manifest.allowed_additional_diagnostics.is_empty() {
        assert_eq!(
            diagnostics, expected_diagnostics,
            "{}: diagnostic identities, severities, paths, or public order changed",
            case.name
        );
        return;
    }

    let expected_in_output_order: Vec<_> = diagnostics
        .iter()
        .filter(|diagnostic| expected_diagnostics.contains(diagnostic))
        .cloned()
        .collect();
    assert_eq!(
        expected_in_output_order, expected_diagnostics,
        "{}: expected diagnostics changed or were duplicated",
        case.name
    );
    let mut remaining_allowances: Vec<_> = case
        .manifest
        .allowed_additional_diagnostics
        .iter()
        .map(|allowed| &allowed.diagnostic)
        .collect();
    for diagnostic in diagnostics
        .iter()
        .filter(|diagnostic| !expected_diagnostics.contains(diagnostic))
    {
        let position = remaining_allowances
            .iter()
            .position(|allowed| **allowed == *diagnostic)
            .unwrap_or_else(|| panic!("{}: unexpected diagnostic {diagnostic:?}", case.name));
        remaining_allowances.remove(position);
    }
}

fn assert_expected_suggestions(case: &LoadedCase, report: &OutputReport) {
    let mut matched_diagnostics = vec![false; report.diagnostics.len()];
    for expected in case
        .manifest
        .expected_diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.suggestion.is_some())
    {
        let (index, actual) = report
            .diagnostics
            .iter()
            .enumerate()
            .find(|actual| {
                !matched_diagnostics[actual.0]
                    && actual.1.code == expected.code
                    && actual.1.name == expected.name
                    && actual.1.severity == expected.severity
                    && actual.1.subject_path == expected.subject_path
                    && actual.1.related_subjects == expected.related_subjects
            })
            .unwrap_or_else(|| {
                panic!(
                    "{}: cannot find diagnostic for expected suggestion {:?}",
                    case.name, expected
                )
            });
        matched_diagnostics[index] = true;
        assert_eq!(
            actual.suggestion, expected.suggestion,
            "{}: diagnostic suggestion changed",
            case.name
        );
    }
}

fn expected_strictness(arguments: &[String]) -> &'static str {
    if arguments.iter().any(|argument| argument == "--all") {
        "all"
    } else if arguments.iter().any(|argument| argument == "--pedantic") {
        "pedantic"
    } else {
        "normal"
    }
}

fn assert_safe_relative_path(case: &str, path: &str) {
    let path = Path::new(path);
    assert!(
        !path.is_absolute()
            && path
                .components()
                .all(|component| matches!(component, std::path::Component::Normal(_))),
        "{case}: unsafe post-fix path {}",
        path.display()
    );
}

/// Recursively copy a fixture repository.
///
/// Files named `*.alint-bytes` are size markers: the file body is a decimal
/// byte count, and the destination is a zero-filled file with that stem name
/// (so oversized S072 fixtures need not be checked into git).
fn copy_tree(source: &Path, destination: &Path) {
    for entry in fs::read_dir(source)
        .unwrap_or_else(|error| panic!("cannot read fixture {}: {error}", source.display()))
    {
        let entry = entry.expect("fixture directory entry is readable");
        let file_type = entry.file_type().expect("fixture file type is readable");
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        if let Some(stem) = name.strip_suffix(".alint-bytes") {
            assert!(
                file_type.is_file(),
                "size-marker fixtures must be files: {}",
                entry.path().display()
            );
            let raw = fs::read_to_string(entry.path()).unwrap_or_else(|error| {
                panic!(
                    "cannot read size marker {}: {error}",
                    entry.path().display()
                )
            });
            let bytes: usize = raw.trim().parse().unwrap_or_else(|error| {
                panic!("invalid size marker {}: {error}", entry.path().display())
            });
            let target = destination.join(stem);
            fs::write(&target, vec![0u8; bytes])
                .unwrap_or_else(|error| panic!("cannot materialize {}: {error}", target.display()));
            continue;
        }
        let target = destination.join(&file_name);
        if file_type.is_dir() {
            fs::create_dir(&target).expect("fixture directory is copied");
            copy_tree(&entry.path(), &target);
        } else {
            assert!(file_type.is_file(), "fixture symlinks are not supported");
            fs::copy(entry.path(), target).expect("fixture file is copied");
        }
    }
}
