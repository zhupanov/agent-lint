use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde::{Deserialize, Deserializer, Serialize};
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
    "m010-m011-enrichment",
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
    #[serde(default)]
    unchanged_after_fix: Vec<String>,
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
    #[serde(default)]
    evidence: Option<String>,
    #[serde(default, rename = "location")]
    expected_location: Option<ExpectedLocation>,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct ExpectedLocation {
    line: usize,
    #[serde(default)]
    column: Option<usize>,
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
    evidence: Option<String>,
    location: Option<OutputLocation>,
}

#[derive(Debug, Deserialize)]
struct OutputLocation {
    start: OutputPosition,
}

#[derive(Debug, Deserialize)]
struct OutputPosition {
    line: usize,
    #[serde(default)]
    column: Option<usize>,
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
            evidence: diagnostic.evidence.clone(),
            expected_location: None,
        }
    }
}

struct LoadedCase {
    name: String,
    manifest: CaseManifest,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequiredContracts {
    requirements: Vec<ContractRequirement>,
    baseline: Vec<ContractBaseline>,
    admission_baseline: Vec<AdmissionBaseline>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AdmissionBaseline {
    rule: String,
    reason: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LiveContractCatalog {
    rules: Vec<LiveRuleContract>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LiveRuleContract {
    code: String,
    surfaces: Vec<LiveContractSurface>,
    platform: ContractPlatform,
    autofix: bool,
    default_severity: ContractSeverity,
    pedantic_exempt: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LiveContractSurface {
    id: ContractSurface,
    modes: Vec<ContractMode>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContractRequirement {
    rule: String,
    surface: ContractSurface,
    classes: Vec<CaseClass>,
    axes: Vec<ContractAxis>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContractBaseline {
    rule: String,
    surface: ContractSurface,
    class: CaseClass,
    axis: ContractAxis,
    reason: String,
}

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
struct ContractSurface(String);

impl ContractSurface {
    fn new(value: String) -> Result<Self, String> {
        let valid = !value.is_empty()
            && value.split('-').all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            });
        if valid {
            Ok(Self(value))
        } else {
            Err(format!(
                "surface '{value}' is not non-empty kebab-case [a-z0-9]+(?:-[a-z0-9]+)*"
            ))
        }
    }

    fn name(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ContractSurface {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for ContractSurface {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "lowercase")]
enum ContractMode {
    Basic,
    Plugin,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "lowercase")]
enum ContractPlatform {
    Claude,
    Cursor,
    Codex,
    Shared,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "lowercase")]
enum ContractSeverity {
    Error,
    Warning,
    Suppressed,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "kebab-case")]
enum ContractAxis {
    JsonOutput,
    FocusedSelection,
    BasicMode,
    PluginMode,
    NormalPolicy,
    PedanticPolicy,
    AllPolicy,
    ClaudeActive,
    CursorActive,
    CodexActive,
    PlatformDisabled,
    PlatformForced,
    GlobalSuppression,
    PerFileSuppression,
    Exclusion,
    Diagnostic,
    Subject,
    Location,
    Suggestion,
    Autofix,
    AutofixIdempotent,
    AutofixScoped,
    TextJsonParity,
    DeterministicOrder,
    NoAutofix,
}

impl ContractAxis {
    fn name(self) -> &'static str {
        match self {
            Self::JsonOutput => "json-output",
            Self::FocusedSelection => "focused-selection",
            Self::BasicMode => "basic-mode",
            Self::PluginMode => "plugin-mode",
            Self::NormalPolicy => "normal-policy",
            Self::PedanticPolicy => "pedantic-policy",
            Self::AllPolicy => "all-policy",
            Self::ClaudeActive => "claude-active",
            Self::CursorActive => "cursor-active",
            Self::CodexActive => "codex-active",
            Self::PlatformDisabled => "platform-disabled",
            Self::PlatformForced => "platform-forced",
            Self::GlobalSuppression => "global-suppression",
            Self::PerFileSuppression => "per-file-suppression",
            Self::Exclusion => "exclusion",
            Self::Diagnostic => "diagnostic",
            Self::Subject => "subject",
            Self::Location => "location",
            Self::Suggestion => "suggestion",
            Self::Autofix => "autofix",
            Self::AutofixIdempotent => "autofix-idempotent",
            Self::AutofixScoped => "autofix-scoped",
            Self::TextJsonParity => "text-json-parity",
            Self::DeterministicOrder => "deterministic-order",
            Self::NoAutofix => "no-autofix",
        }
    }
}

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
struct ContractTuple {
    rule: String,
    surface: ContractSurface,
    class: CaseClass,
    axis: ContractAxis,
}

impl ContractTuple {
    fn display(&self) -> String {
        format!(
            "rule={}, surface={}, class={:?}, axis={}",
            self.rule,
            self.surface,
            self.class,
            self.axis.name()
        )
    }
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

#[test]
fn test_required_contract_matrix_covers_rule_surface_policy_axes() {
    let corpus = corpus_root();
    let cases = load_cases(&corpus);
    let contracts = load_json::<RequiredContracts>(&corpus.join("required-contracts.json"));
    let catalog = load_json::<LiveContractCatalog>(&corpus.join("live-rule-contracts.json"));
    validate_catalog(&catalog, &known_rule_codes()).unwrap_or_else(|errors| panic!("{errors}"));
    validate_contract_matrix(&catalog, &contracts, &cases)
        .unwrap_or_else(|errors| panic!("{errors}"));
}

fn load_json<T: for<'de> Deserialize<'de>>(path: &Path) -> T {
    let bytes =
        fs::read(path).unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("invalid JSON {}: {error}", path.display()))
}

fn known_rule_codes() -> Vec<String> {
    include_str!("../src/rules.rs")
        .lines()
        .filter_map(|line| line.split_once("code = \"").map(|(_, rest)| rest))
        .filter_map(|rest| rest.split_once('"').map(|(code, _)| code.to_owned()))
        .collect()
}

fn validate_catalog(catalog: &LiveContractCatalog, live_codes: &[String]) -> Result<(), String> {
    let mut errors = Vec::new();
    let mut seen = BTreeSet::new();
    for rule in &catalog.rules {
        if !seen.insert(rule.code.as_str()) {
            errors.push(format!("duplicate live catalog row '{}'", rule.code));
        }
        if rule.surfaces.is_empty() {
            errors.push(format!("{} has no contract surfaces", rule.code));
        }
        let ids: Vec<_> = rule
            .surfaces
            .iter()
            .map(|surface| surface.id.name())
            .collect();
        let mut sorted_ids = ids.clone();
        sorted_ids.sort_unstable();
        sorted_ids.dedup();
        if ids != sorted_ids {
            errors.push(format!(
                "{} surface IDs must be sorted and deduplicated: {ids:?}",
                rule.code
            ));
        }
        for surface in &rule.surfaces {
            if surface.modes.is_empty() {
                errors.push(format!("{} / {} has no modes", rule.code, surface.id));
            }
            let mut modes = surface.modes.clone();
            modes.sort();
            modes.dedup();
            if surface.modes != modes {
                errors.push(format!(
                    "{} / {} modes must be [basic, plugin] order and deduplicated",
                    rule.code, surface.id
                ));
            }
        }
    }

    let actual: Vec<_> = catalog.rules.iter().map(|rule| rule.code.clone()).collect();
    for code in actual.iter().filter(|code| !live_codes.contains(code)) {
        errors.push(format!("unknown catalog code '{code}'"));
    }
    for code in live_codes.iter().filter(|code| !actual.contains(code)) {
        errors.push(format!("missing live catalog row '{code}'"));
    }
    if actual.len() == live_codes.len()
        && actual.iter().all(|code| live_codes.contains(code))
        && actual != live_codes
    {
        errors.push("live catalog rows are not in registry order".to_owned());
    }
    if errors.is_empty() {
        Ok(())
    } else {
        errors.sort();
        Err(errors.join("\n"))
    }
}

fn validate_contract_matrix(
    catalog: &LiveContractCatalog,
    contracts: &RequiredContracts,
    cases: &[LoadedCase],
) -> Result<(), String> {
    let by_code: BTreeMap<_, _> = catalog
        .rules
        .iter()
        .map(|rule| (rule.code.as_str(), rule))
        .collect();
    let mut required = BTreeSet::new();
    let mut required_rules = BTreeSet::new();
    let mut errors = Vec::new();
    for requirement in &contracts.requirements {
        let Some(rule) = by_code.get(requirement.rule.as_str()) else {
            errors.push(format!("unknown matrix rule '{}'", requirement.rule));
            continue;
        };
        required_rules.insert(requirement.rule.as_str());
        if !rule
            .surfaces
            .iter()
            .any(|surface| surface.id == requirement.surface)
        {
            errors.push(format!(
                "matrix surface '{}' is not owned by {}",
                requirement.surface, requirement.rule
            ));
        }
        if requirement.classes.is_empty() {
            errors.push(format!(
                "{} / {} has no required classes",
                requirement.rule, requirement.surface
            ));
        }
        if requirement.axes.is_empty() {
            errors.push(format!(
                "{} / {} has no required axes",
                requirement.rule, requirement.surface
            ));
        }
        for class in &requirement.classes {
            for axis in &requirement.axes {
                let tuple = ContractTuple {
                    rule: requirement.rule.clone(),
                    surface: requirement.surface.clone(),
                    class: *class,
                    axis: *axis,
                };
                if !required.insert(tuple.clone()) {
                    errors.push(format!("duplicate contract tuple: {}", tuple.display()));
                }
            }
        }
    }
    if required.is_empty() {
        errors.push("contract matrix has no requirements".to_owned());
    }

    let mut baseline = BTreeMap::new();
    for gap in &contracts.baseline {
        if !by_code.contains_key(gap.rule.as_str()) {
            errors.push(format!("unknown baseline rule '{}'", gap.rule));
        }
        if gap.reason.trim().is_empty() {
            errors.push(format!(
                "baseline reason is empty for rule={}, surface={}, class={:?}, axis={}",
                gap.rule,
                gap.surface,
                gap.class,
                gap.axis.name()
            ));
        }
        let tuple = ContractTuple {
            rule: gap.rule.clone(),
            surface: gap.surface.clone(),
            class: gap.class,
            axis: gap.axis,
        };
        if !required.contains(&tuple) {
            errors.push(format!(
                "baseline tuple is not required: {}",
                tuple.display()
            ));
        }
        if baseline
            .insert(tuple.clone(), gap.reason.as_str())
            .is_some()
        {
            errors.push(format!("duplicate baseline tuple: {}", tuple.display()));
        }
    }

    let uncovered: BTreeSet<_> = required
        .iter()
        .filter(|tuple| {
            by_code.get(tuple.rule.as_str()).is_none_or(|rule| {
                !cases
                    .iter()
                    .any(|case| case_covers_contract(case, tuple, rule))
            })
        })
        .cloned()
        .collect();
    let baseline_tuples: BTreeSet<_> = baseline.keys().cloned().collect();
    let mut all_missing: Vec<_> = uncovered.difference(&baseline_tuples).cloned().collect();
    for tuple in baseline.keys() {
        if !uncovered.contains(tuple) {
            errors.push(format!(
                "covered baseline tuple must be removed: {}",
                tuple.display()
            ));
        }
    }

    let mut admission = BTreeMap::new();
    let ranks: BTreeMap<_, _> = catalog
        .rules
        .iter()
        .enumerate()
        .map(|(rank, rule)| (rule.code.as_str(), rank))
        .collect();
    let mut last_rank = None;
    for row in &contracts.admission_baseline {
        let Some(rule) = by_code.get(row.rule.as_str()) else {
            errors.push(format!("unknown admission baseline rule '{}'", row.rule));
            continue;
        };
        if row.reason.trim().is_empty() {
            errors.push(format!("blank admission baseline reason for {}", row.rule));
        }
        if admission
            .insert(row.rule.as_str(), row.reason.as_str())
            .is_some()
        {
            errors.push(format!("duplicate admission baseline rule '{}'", row.rule));
        }
        if !is_matrix_required(rule) {
            errors.push(format!(
                "admission baseline rule {} is no longer matrix-required",
                row.rule
            ));
        }
        let rank = ranks[row.rule.as_str()];
        if last_rank.is_some_and(|previous| rank <= previous) {
            errors.push("admission baseline rows are not in live-registry order".to_owned());
        }
        last_rank = Some(rank);
    }

    let mut derived_missing = Vec::new();
    for rule in catalog.rules.iter().filter(|rule| is_matrix_required(rule)) {
        let obligations = derived_obligations(rule);
        let missing: Vec<_> = obligations
            .into_iter()
            .filter(|tuple| {
                !cases
                    .iter()
                    .any(|case| case_covers_contract(case, tuple, rule))
            })
            .collect();
        if admission.contains_key(rule.code.as_str()) {
            if missing.is_empty() {
                errors.push(format!(
                    "stale admission baseline row: {} is fully covered",
                    rule.code
                ));
            }
        } else if !required_rules.contains(rule.code.as_str()) {
            errors.push(format!(
                "newly matrix-required rule {} is absent from requirements and admission_baseline; baseline growth after adoption is prohibited",
                rule.code
            ));
        } else {
            derived_missing.extend(missing);
        }
    }
    all_missing.append(&mut derived_missing);
    all_missing.sort_by_key(|tuple| {
        (
            ranks[tuple.rule.as_str()],
            tuple.surface.clone(),
            tuple.class,
            tuple.axis,
        )
    });
    all_missing.dedup();
    for tuple in &all_missing {
        errors.push(format!("missing contract tuple: {}", tuple.display()));
    }
    if !all_missing.is_empty() {
        errors.push(format_missing_requirements(&all_missing));
    }

    for requirement in &contracts.requirements {
        let surface_is_observable = cases.iter().any(|case| {
            case.manifest
                .covers
                .iter()
                .any(|tag| tag == requirement.surface.name())
        }) || contracts
            .baseline
            .iter()
            .any(|gap| gap.surface == requirement.surface);
        if !surface_is_observable {
            errors.push(format!(
                "matrix surface '{}' has no exact case covers tag or uncovered baseline tuple",
                requirement.surface
            ));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

fn is_matrix_required(rule: &LiveRuleContract) -> bool {
    let modes: BTreeSet<_> = rule
        .surfaces
        .iter()
        .flat_map(|surface| surface.modes.iter().copied())
        .collect();
    rule.surfaces.len() > 1
        || modes.len() > 1
        || matches!(
            rule.platform,
            ContractPlatform::Cursor | ContractPlatform::Codex
        )
        || rule.autofix
}

fn tuple(
    rule: &LiveRuleContract,
    surface: &ContractSurface,
    class: CaseClass,
    axis: ContractAxis,
) -> ContractTuple {
    ContractTuple {
        rule: rule.code.clone(),
        surface: surface.clone(),
        class,
        axis,
    }
}

fn derived_obligations(rule: &LiveRuleContract) -> Vec<ContractTuple> {
    let mut obligations = Vec::new();
    for surface in &rule.surfaces {
        for axis in [ContractAxis::JsonOutput, ContractAxis::FocusedSelection] {
            obligations.push(tuple(rule, &surface.id, CaseClass::Clean, axis));
            obligations.push(tuple(rule, &surface.id, CaseClass::HardNegative, axis));
        }
        for axis in [
            ContractAxis::JsonOutput,
            ContractAxis::FocusedSelection,
            ContractAxis::Diagnostic,
            ContractAxis::Subject,
        ] {
            obligations.push(tuple(rule, &surface.id, CaseClass::Broken, axis));
        }
        for mode in &surface.modes {
            obligations.push(tuple(
                rule,
                &surface.id,
                CaseClass::Broken,
                match mode {
                    ContractMode::Basic => ContractAxis::BasicMode,
                    ContractMode::Plugin => ContractAxis::PluginMode,
                },
            ));
        }
        if rule.autofix {
            for axis in [
                ContractAxis::Autofix,
                ContractAxis::AutofixIdempotent,
                ContractAxis::AutofixScoped,
            ] {
                obligations.push(tuple(rule, &surface.id, CaseClass::Broken, axis));
            }
            for axis in [
                ContractAxis::GlobalSuppression,
                ContractAxis::PerFileSuppression,
            ] {
                obligations.push(tuple(rule, &surface.id, CaseClass::HardNegative, axis));
            }
        } else {
            obligations.push(tuple(
                rule,
                &surface.id,
                CaseClass::Broken,
                ContractAxis::NoAutofix,
            ));
        }
        match rule.default_severity {
            ContractSeverity::Warning => {
                for axis in [
                    ContractAxis::NormalPolicy,
                    ContractAxis::PedanticPolicy,
                    ContractAxis::AllPolicy,
                ] {
                    obligations.push(tuple(rule, &surface.id, CaseClass::Broken, axis));
                }
            }
            ContractSeverity::Error => {
                for axis in [ContractAxis::NormalPolicy, ContractAxis::AllPolicy] {
                    obligations.push(tuple(rule, &surface.id, CaseClass::Broken, axis));
                }
            }
            ContractSeverity::Suppressed => {
                obligations.push(tuple(
                    rule,
                    &surface.id,
                    CaseClass::Clean,
                    ContractAxis::NormalPolicy,
                ));
                obligations.push(tuple(
                    rule,
                    &surface.id,
                    CaseClass::Broken,
                    ContractAxis::AllPolicy,
                ));
            }
        }
    }
    if let Some(surface) = rule.surfaces.first() {
        let active_axis = match rule.platform {
            ContractPlatform::Cursor => Some(ContractAxis::CursorActive),
            ContractPlatform::Codex => Some(ContractAxis::CodexActive),
            ContractPlatform::Claude | ContractPlatform::Shared => None,
        };
        if let Some(axis) = active_axis {
            obligations.push(tuple(rule, &surface.id, CaseClass::Broken, axis));
            for axis in [
                ContractAxis::PlatformDisabled,
                ContractAxis::PlatformForced,
                ContractAxis::Exclusion,
            ] {
                obligations.push(tuple(rule, &surface.id, CaseClass::HardNegative, axis));
            }
        }
    }
    obligations.sort();
    obligations.dedup();
    obligations
}

fn format_missing_requirements(tuples: &[ContractTuple]) -> String {
    #[derive(Serialize)]
    struct PasteRequirement {
        rule: String,
        surface: String,
        classes: Vec<&'static str>,
        axes: Vec<&'static str>,
    }
    let mut grouped: BTreeMap<_, BTreeSet<_>> = BTreeMap::new();
    for tuple in tuples {
        grouped
            .entry((tuple.rule.clone(), tuple.surface.clone(), tuple.class))
            .or_default()
            .insert(tuple.axis);
    }
    let rows: Vec<_> = grouped
        .into_iter()
        .map(|((rule, surface, class), axes)| PasteRequirement {
            rule,
            surface: surface.0,
            classes: vec![match class {
                CaseClass::Clean => "clean",
                CaseClass::Broken => "broken",
                CaseClass::HardNegative => "hard-negative",
            }],
            axes: axes.into_iter().map(ContractAxis::name).collect(),
        })
        .collect();
    format!(
        "ready-to-paste missing requirements:\n{}",
        serde_json::to_string_pretty(&rows).expect("missing requirements serialize")
    )
}

fn case_covers_contract(case: &LoadedCase, tuple: &ContractTuple, rule: &LiveRuleContract) -> bool {
    let class_matches = case.manifest.class == tuple.class
        || (tuple.axis == ContractAxis::PlatformForced
            && matches!(case.manifest.class, CaseClass::Clean | CaseClass::Broken));
    if !class_matches
        || !case
            .manifest
            .covers
            .iter()
            .any(|tag| tag == tuple.surface.name())
    {
        return false;
    }

    let arguments = &case.manifest.invocation.arguments;
    let matching_diagnostics = || {
        case.manifest
            .expected_diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == tuple.rule)
    };
    let focuses_rule = || {
        arguments.windows(2).any(|pair| {
            pair[0] == "--only" && pair[1].split(',').any(|selected| selected == tuple.rule)
        })
    };
    match tuple.axis {
        ContractAxis::JsonOutput => arguments
            .windows(2)
            .any(|pair| pair == ["--format", "json"]),
        ContractAxis::FocusedSelection => focuses_rule(),
        ContractAxis::BasicMode => case.manifest.invocation.mode == "basic",
        ContractAxis::PluginMode => case.manifest.invocation.mode == "plugin",
        ContractAxis::NormalPolicy => {
            let normal = !arguments
                .iter()
                .any(|argument| argument == "--pedantic" || argument == "--all");
            normal
                && if tuple.class != CaseClass::Broken
                    && rule.default_severity != ContractSeverity::Suppressed
                {
                    true
                } else {
                    match rule.default_severity {
                        ContractSeverity::Suppressed => {
                            matching_diagnostics().next().is_none()
                                && focuses_rule()
                                && case.manifest.expected_suppressed > 0
                        }
                        ContractSeverity::Error => {
                            matching_diagnostics().any(|diagnostic| diagnostic.severity == "error")
                        }
                        ContractSeverity::Warning => matching_diagnostics()
                            .any(|diagnostic| diagnostic.severity == "warning"),
                    }
                }
        }
        ContractAxis::PedanticPolicy => {
            arguments.iter().any(|argument| argument == "--pedantic")
                && matching_diagnostics().any(|diagnostic| {
                    diagnostic.severity
                        == if rule.pedantic_exempt {
                            "warning"
                        } else {
                            "error"
                        }
                })
        }
        ContractAxis::AllPolicy => {
            arguments.iter().any(|argument| argument == "--all")
                && matching_diagnostics().any(|diagnostic| diagnostic.severity == "error")
        }
        ContractAxis::ClaudeActive => case
            .manifest
            .expected_active_platforms
            .iter()
            .any(|platform| platform == "claude"),
        ContractAxis::CursorActive => case
            .manifest
            .expected_active_platforms
            .iter()
            .any(|platform| platform == "cursor"),
        ContractAxis::CodexActive => case
            .manifest
            .expected_active_platforms
            .iter()
            .any(|platform| platform == "codex"),
        ContractAxis::PlatformDisabled => case
            .manifest
            .covers
            .iter()
            .any(|tag| tag == "platform-disabled"),
        ContractAxis::PlatformForced => case
            .manifest
            .covers
            .iter()
            .any(|tag| tag == "platform-forced"),
        ContractAxis::GlobalSuppression => {
            case.manifest
                .covers
                .iter()
                .any(|tag| tag == "global-suppression")
                && focuses_rule()
                && case.manifest.expected_suppressed > 0
                && (!rule.autofix
                    || (arguments.iter().any(|argument| argument == "--autofix")
                        && !case.manifest.unchanged_after_fix.is_empty()))
        }
        ContractAxis::PerFileSuppression => {
            case.manifest
                .covers
                .iter()
                .any(|tag| tag == "per-file-suppression")
                && focuses_rule()
                && case.manifest.expected_suppressed > 0
                && (!rule.autofix
                    || (arguments.iter().any(|argument| argument == "--autofix")
                        && !case.manifest.unchanged_after_fix.is_empty()))
        }
        ContractAxis::Exclusion => case.manifest.covers.iter().any(|tag| tag == "exclusion"),
        ContractAxis::Diagnostic => matching_diagnostics().next().is_some(),
        ContractAxis::Subject => matching_diagnostics().any(|diagnostic| {
            diagnostic.subject_path.is_some() || !diagnostic.related_subjects.is_empty()
        }),
        ContractAxis::Location => {
            matching_diagnostics().any(|diagnostic| diagnostic.expected_location.is_some())
        }
        ContractAxis::Suggestion => {
            matching_diagnostics().any(|diagnostic| diagnostic.suggestion.is_some())
        }
        ContractAxis::Autofix => {
            arguments.iter().any(|argument| argument == "--autofix")
                && !case.manifest.post_fix.is_empty()
        }
        ContractAxis::AutofixIdempotent => {
            arguments.iter().any(|argument| argument == "--autofix")
                && !case.manifest.post_fix.is_empty()
        }
        ContractAxis::AutofixScoped => {
            arguments.iter().any(|argument| argument == "--autofix")
                && !case.manifest.post_fix.is_empty()
                && !case.manifest.unchanged_after_fix.is_empty()
        }
        ContractAxis::TextJsonParity => case
            .manifest
            .covers
            .iter()
            .any(|tag| tag == "text-json-parity"),
        ContractAxis::DeterministicOrder => case
            .manifest
            .covers
            .iter()
            .any(|tag| tag == "deterministic-order"),
        ContractAxis::NoAutofix => {
            !arguments.iter().any(|argument| argument == "--autofix")
                && case.manifest.post_fix.is_empty()
        }
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
        validate_unchanged_paths(
            &repositories.join(&case.manifest.repository),
            &case.name,
            &case.manifest.post_fix,
            &case.manifest.unchanged_after_fix,
        )
        .unwrap_or_else(|error| panic!("{error}"));
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

    let unchanged_before: Vec<_> = case
        .manifest
        .unchanged_after_fix
        .iter()
        .map(|relative| {
            (
                relative,
                fs::read(temp.path().join(relative)).expect("unchanged file is readable"),
            )
        })
        .collect();

    let first = invoke(temp.path(), &case.manifest.invocation.arguments);
    let first_report = assert_run(case, &first, schema);
    assert_report(case, &first_report);
    assert_unchanged_files(temp.path(), &case.name, &unchanged_before);

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
        assert_unchanged_files(temp.path(), &case.name, &unchanged_before);
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

fn assert_unchanged_files(repository: &Path, case: &str, before: &[(&String, Vec<u8>)]) {
    for (relative, bytes) in before {
        assert_eq!(
            fs::read(repository.join(relative)).expect("unchanged file remains readable"),
            *bytes,
            "{case}: autofix changed explicitly protected file {relative}"
        );
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

    assert_expected_metadata(case, report);

    let diagnostics: Vec<_> = report
        .diagnostics
        .iter()
        .map(DiagnosticIdentity::from)
        .map(|mut diagnostic| {
            diagnostic.suggestion = None;
            diagnostic.evidence = None;
            diagnostic.expected_location = None;
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
            diagnostic.evidence = None;
            diagnostic.expected_location = None;
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

/// Bind every metadata assertion (suggestion, evidence, location) of one
/// expected diagnostic to the same output diagnostic: the n-th expected entry
/// with a given identity tuple asserts against the n-th output diagnostic
/// with that identity, matching the ordered identity comparison. A single
/// binding cannot silently validate different diagnostics per field.
fn assert_expected_metadata(case: &LoadedCase, report: &OutputReport) {
    let matches = |actual: &OutputDiagnostic, expected: &DiagnosticIdentity| {
        actual.code == expected.code
            && actual.name == expected.name
            && actual.severity == expected.severity
            && actual.subject_path == expected.subject_path
            && actual.related_subjects == expected.related_subjects
    };
    for (position, expected) in case.manifest.expected_diagnostics.iter().enumerate() {
        if expected.suggestion.is_none()
            && expected.evidence.is_none()
            && expected.expected_location.is_none()
        {
            continue;
        }
        let rank = case.manifest.expected_diagnostics[..position]
            .iter()
            .filter(|other| {
                other.code == expected.code
                    && other.name == expected.name
                    && other.severity == expected.severity
                    && other.subject_path == expected.subject_path
                    && other.related_subjects == expected.related_subjects
            })
            .count();
        let actual = report
            .diagnostics
            .iter()
            .filter(|actual| matches(actual, expected))
            .nth(rank)
            .unwrap_or_else(|| {
                panic!(
                    "{}: cannot find diagnostic for expected metadata {:?}",
                    case.name, expected
                )
            });
        if expected.suggestion.is_some() {
            assert_eq!(
                actual.suggestion, expected.suggestion,
                "{}: diagnostic suggestion changed",
                case.name
            );
        }
        if expected.evidence.is_some() {
            assert_eq!(
                actual.evidence, expected.evidence,
                "{}: diagnostic evidence changed",
                case.name
            );
        }
        if let Some(location) = &expected.expected_location {
            assert_eq!(
                actual
                    .location
                    .as_ref()
                    .map(|actual_location| actual_location.start.line),
                Some(location.line),
                "{}: diagnostic source line changed",
                case.name
            );
            if let Some(column) = location.column {
                assert_eq!(
                    actual
                        .location
                        .as_ref()
                        .and_then(|actual_location| actual_location.start.column),
                    Some(column),
                    "{}: diagnostic source column changed",
                    case.name
                );
            }
        }
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

fn validate_unchanged_paths(
    repository: &Path,
    case: &str,
    post_fix: &[ExpectedFile],
    unchanged: &[String],
) -> Result<(), String> {
    let post_paths: BTreeSet<_> = post_fix.iter().map(|file| file.path.as_str()).collect();
    if post_paths.len() != post_fix.len() {
        return Err(format!("{case}: duplicate post_fix path"));
    }
    let mut seen = BTreeSet::new();
    let canonical_repository = repository
        .canonicalize()
        .map_err(|error| format!("{case}: cannot canonicalize fixture root: {error}"))?;
    for relative in unchanged {
        let path = Path::new(relative);
        if path.is_absolute()
            || !path
                .components()
                .all(|component| matches!(component, std::path::Component::Normal(_)))
        {
            return Err(format!(
                "{case}: unsafe unchanged_after_fix path {relative}"
            ));
        }
        if !seen.insert(relative.as_str()) {
            return Err(format!(
                "{case}: duplicate unchanged_after_fix path {relative}"
            ));
        }
        if post_paths.contains(relative.as_str()) {
            return Err(format!(
                "{case}: unchanged_after_fix path {relative} duplicates post_fix.path"
            ));
        }
        let authored = repository.join(relative);
        let canonical = authored.canonicalize().map_err(|error| {
            format!("{case}: missing unchanged_after_fix path {relative}: {error}")
        })?;
        if !canonical.starts_with(&canonical_repository) {
            return Err(format!(
                "{case}: unchanged_after_fix path {relative} escapes through a symlink"
            ));
        }
        if !canonical.is_file() {
            return Err(format!(
                "{case}: unchanged_after_fix path {relative} is not a file"
            ));
        }
    }
    Ok(())
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

fn fixture_rule(
    code: &str,
    surfaces: &[(&str, &[ContractMode])],
    autofix: bool,
) -> LiveRuleContract {
    LiveRuleContract {
        code: code.to_owned(),
        surfaces: surfaces
            .iter()
            .map(|(id, modes)| LiveContractSurface {
                id: ContractSurface::new((*id).to_owned()).unwrap(),
                modes: modes.to_vec(),
            })
            .collect(),
        platform: ContractPlatform::Shared,
        autofix,
        default_severity: ContractSeverity::Error,
        pedantic_exempt: false,
    }
}

fn fixture_case(class: CaseClass, surface: &str, axis: ContractAxis) -> LoadedCase {
    let mut covers = vec![surface.to_owned()];
    let mut arguments = vec![
        "--format".to_owned(),
        "json".to_owned(),
        "--only".to_owned(),
        "R001".to_owned(),
    ];
    match axis {
        ContractAxis::PedanticPolicy => arguments.push("--pedantic".to_owned()),
        ContractAxis::AllPolicy => arguments.push("--all".to_owned()),
        ContractAxis::Autofix | ContractAxis::AutofixIdempotent | ContractAxis::AutofixScoped => {
            arguments.push("--autofix".to_owned())
        }
        ContractAxis::GlobalSuppression => {
            covers.push("global-suppression".to_owned());
            arguments.push("--autofix".to_owned());
        }
        ContractAxis::PerFileSuppression => {
            covers.push("per-file-suppression".to_owned());
            arguments.push("--autofix".to_owned());
        }
        ContractAxis::TextJsonParity => covers.push("text-json-parity".to_owned()),
        ContractAxis::DeterministicOrder => covers.push("deterministic-order".to_owned()),
        _ => {}
    }
    arguments.push(".".to_owned());
    let post_fix = matches!(
        axis,
        ContractAxis::Autofix | ContractAxis::AutofixIdempotent | ContractAxis::AutofixScoped
    )
    .then(|| ExpectedFile {
        path: "fixed.txt".to_owned(),
        contents: "fixed".to_owned(),
    })
    .into_iter()
    .collect();
    let unchanged_after_fix = matches!(
        axis,
        ContractAxis::AutofixScoped
            | ContractAxis::GlobalSuppression
            | ContractAxis::PerFileSuppression
    )
    .then(|| "unchanged.txt".to_owned())
    .into_iter()
    .collect();
    let expected_diagnostics = (class == CaseClass::Broken)
        .then(|| DiagnosticIdentity {
            code: "R001".to_owned(),
            name: "fixture-rule".to_owned(),
            severity: "error".to_owned(),
            subject_path: Some("fixture.txt".to_owned()),
            related_subjects: Vec::new(),
            suggestion: None,
            evidence: None,
            expected_location: None,
        })
        .into_iter()
        .collect();
    LoadedCase {
        name: format!("fixture-{}", axis.name()),
        manifest: CaseManifest {
            class,
            repository: "fixture".to_owned(),
            covers,
            invocation: Invocation {
                mode: "plugin".to_owned(),
                arguments,
            },
            expected_exit_code: 1,
            expected_status: "errors".to_owned(),
            expected_active_platforms: Vec::new(),
            expected_suppressed: 1,
            expected_diagnostics,
            allowed_additional_diagnostics: Vec::new(),
            post_fix,
            unchanged_after_fix,
        },
    }
}

#[test]
fn catalog_validation_rejects_unknown_missing_duplicate_and_unsorted_rows() {
    let basic = [ContractMode::Basic];
    let valid = LiveContractCatalog {
        rules: vec![fixture_rule("R001", &[("alpha", &basic)], false)],
    };
    assert!(validate_catalog(&valid, &["R001".to_owned()]).is_ok());

    let unknown = validate_catalog(&valid, &["R002".to_owned()]).unwrap_err();
    assert!(unknown.contains("unknown catalog code 'R001'"), "{unknown}");
    assert!(
        unknown.contains("missing live catalog row 'R002'"),
        "{unknown}"
    );

    let missing = LiveContractCatalog { rules: Vec::new() };
    assert!(
        validate_catalog(&missing, &["R001".to_owned()])
            .unwrap_err()
            .contains("missing live catalog row 'R001'")
    );

    let duplicate = LiveContractCatalog {
        rules: vec![
            fixture_rule("R001", &[("alpha", &basic)], false),
            fixture_rule("R001", &[("alpha", &basic)], false),
        ],
    };
    assert!(
        validate_catalog(&duplicate, &["R001".to_owned()])
            .unwrap_err()
            .contains("duplicate live catalog row 'R001'")
    );

    for surfaces in [
        vec![("zeta", basic.as_slice()), ("alpha", basic.as_slice())],
        vec![("alpha", basic.as_slice()), ("alpha", basic.as_slice())],
    ] {
        let invalid = LiveContractCatalog {
            rules: vec![fixture_rule("R001", &surfaces, false)],
        };
        assert!(
            validate_catalog(&invalid, &["R001".to_owned()])
                .unwrap_err()
                .contains("surface IDs must be sorted and deduplicated")
        );
    }

    let malformed = r#"{"rules":[{"code":"R001","surfaces":[{"id":"Not-Kebab","modes":["basic"]}],"platform":"shared","autofix":false,"default_severity":"error","pedantic_exempt":false}]}"#;
    assert!(
        serde_json::from_str::<LiveContractCatalog>(malformed)
            .unwrap_err()
            .to_string()
            .contains("not non-empty kebab-case")
    );
}

fn one_requirement(rule: &str, surface: &str) -> RequiredContracts {
    RequiredContracts {
        requirements: vec![ContractRequirement {
            rule: rule.to_owned(),
            surface: ContractSurface::new(surface.to_owned()).unwrap(),
            classes: vec![CaseClass::Broken],
            axes: vec![ContractAxis::NoAutofix],
        }],
        baseline: Vec::new(),
        admission_baseline: Vec::new(),
    }
}

#[test]
fn matrix_validation_rejects_unknown_unowned_and_unadmitted_rules() {
    let plugin = [ContractMode::Plugin];
    let catalog = LiveContractCatalog {
        rules: vec![fixture_rule("R001", &[("alpha", &plugin)], false)],
    };
    let case = fixture_case(CaseClass::Broken, "alpha", ContractAxis::NoAutofix);

    let unknown = one_requirement("R999", "alpha");
    assert!(
        validate_contract_matrix(&catalog, &unknown, std::slice::from_ref(&case))
            .unwrap_err()
            .contains("unknown matrix rule 'R999'")
    );

    let unowned = one_requirement("R001", "beta");
    assert!(
        validate_contract_matrix(&catalog, &unowned, std::slice::from_ref(&case))
            .unwrap_err()
            .contains("matrix surface 'beta' is not owned by R001")
    );

    let both = [ContractMode::Basic, ContractMode::Plugin];
    let high_risk = LiveContractCatalog {
        rules: vec![fixture_rule("R001", &[("alpha", &both)], false)],
    };
    let empty = RequiredContracts {
        requirements: Vec::new(),
        baseline: Vec::new(),
        admission_baseline: Vec::new(),
    };
    let error = validate_contract_matrix(&high_risk, &empty, &[]).unwrap_err();
    assert!(
        error.contains("newly matrix-required rule R001 is absent"),
        "{error}"
    );
}

#[test]
fn admission_baseline_rejects_duplicate_blank_and_stale_rows() {
    let plugin = [ContractMode::Plugin];
    let rule = fixture_rule("R001", &[("alpha", &plugin)], true);
    let catalog = LiveContractCatalog { rules: vec![rule] };
    let duplicate = RequiredContracts {
        requirements: Vec::new(),
        baseline: Vec::new(),
        admission_baseline: vec![
            AdmissionBaseline {
                rule: "R001".to_owned(),
                reason: String::new(),
            },
            AdmissionBaseline {
                rule: "R001".to_owned(),
                reason: "duplicate".to_owned(),
            },
        ],
    };
    let error = validate_contract_matrix(&catalog, &duplicate, &[]).unwrap_err();
    assert!(error.contains("blank admission baseline reason"), "{error}");
    assert!(
        error.contains("duplicate admission baseline rule"),
        "{error}"
    );

    let obligations = derived_obligations(&catalog.rules[0]);
    let cases: Vec<_> = obligations
        .iter()
        .map(|obligation| fixture_case(obligation.class, "alpha", obligation.axis))
        .collect();
    let stale = RequiredContracts {
        requirements: Vec::new(),
        baseline: Vec::new(),
        admission_baseline: vec![AdmissionBaseline {
            rule: "R001".to_owned(),
            reason: "pre-admission gap".to_owned(),
        }],
    };
    let error = validate_contract_matrix(&catalog, &stale, &cases).unwrap_err();
    assert!(
        error.contains("stale admission baseline row: R001"),
        "{error}"
    );
}

#[test]
fn new_contract_axes_require_explicit_evidence() {
    let plugin = [ContractMode::Plugin];
    let rule = fixture_rule("R001", &[("alpha", &plugin)], true);
    for axis in [
        ContractAxis::AutofixIdempotent,
        ContractAxis::AutofixScoped,
        ContractAxis::TextJsonParity,
        ContractAxis::DeterministicOrder,
    ] {
        let mut case = fixture_case(CaseClass::Broken, "alpha", axis);
        let contract = tuple(&rule, &rule.surfaces[0].id, CaseClass::Broken, axis);
        assert!(case_covers_contract(&case, &contract, &rule), "{axis:?}");
        match axis {
            ContractAxis::AutofixIdempotent => case.manifest.post_fix.clear(),
            ContractAxis::AutofixScoped => case.manifest.unchanged_after_fix.clear(),
            ContractAxis::TextJsonParity => {
                case.manifest.covers.retain(|tag| tag != "text-json-parity")
            }
            ContractAxis::DeterministicOrder => case
                .manifest
                .covers
                .retain(|tag| tag != "deterministic-order"),
            _ => unreachable!(),
        }
        assert!(!case_covers_contract(&case, &contract, &rule), "{axis:?}");
    }
}

#[test]
fn unchanged_after_fix_rejects_duplicate_unsafe_missing_and_escaping_paths() {
    let repository = tempfile::tempdir().unwrap();
    fs::write(repository.path().join("same.txt"), "same").unwrap();
    let post = vec![ExpectedFile {
        path: "same.txt".to_owned(),
        contents: "fixed".to_owned(),
    }];
    assert!(
        validate_unchanged_paths(
            repository.path(),
            "duplicate",
            &post,
            &["same.txt".to_owned()]
        )
        .unwrap_err()
        .contains("duplicates post_fix.path")
    );
    assert!(
        validate_unchanged_paths(repository.path(), "unsafe", &[], &["../outside".to_owned()])
            .unwrap_err()
            .contains("unsafe unchanged_after_fix path")
    );
    assert!(
        validate_unchanged_paths(
            repository.path(),
            "missing",
            &[],
            &["missing.txt".to_owned()]
        )
        .unwrap_err()
        .contains("missing unchanged_after_fix path")
    );

    #[cfg(unix)]
    {
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("outside.txt"), "outside").unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("outside.txt"),
            repository.path().join("escape.txt"),
        )
        .unwrap();
        assert!(
            validate_unchanged_paths(
                repository.path(),
                "symlink",
                &[],
                &["escape.txt".to_owned()]
            )
            .unwrap_err()
            .contains("escapes through a symlink")
        );
    }
}
