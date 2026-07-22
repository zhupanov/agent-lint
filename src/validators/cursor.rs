//! Validation for Cursor project configuration.

use std::fs;
use std::path::Path;
use std::sync::LazyLock;

use globset::GlobBuilder;
use jsonschema::error::ValidationErrorKind;
use regex::Regex;
use serde_json::Value as JsonValue;
#[cfg(test)]
use serde_json::json;

use crate::config::ExcludeSet;
use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata, SourceSpan};
use crate::frontmatter;
use crate::json_locate::{JsonScanner, Seg};
use crate::live_instructions::{InstructionSurfaceKind, LiveInstructionDocument};
use crate::markdown::MarkdownDocument;
use crate::platforms;
use crate::rules::LintRule;
use crate::sensitive::contains_sensitive_evidence;
use crate::traversal;
use crate::yaml::{Mapping, Value as YamlValue};

/// Cursor subagent identifiers: lowercase letters with single hyphens between
/// segments (`security-auditor`). Digits, underscores, and leading, trailing,
/// or consecutive hyphens are rejected.
static CURSOR_AGENT_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z]+(-[a-z]+)*$").expect("cursor agent name regex"));

const RULE_KEYS: &[&str] = &["description", "globs", "alwaysApply"];
const CURSOR_SKILL_KEYS: &[&str] = &[
    "name",
    "description",
    "paths",
    "disable-model-invocation",
    "metadata",
];
const HOOK_EVENTS: &[&str] = &[
    "sessionStart",
    "sessionEnd",
    "preToolUse",
    "postToolUse",
    "postToolUseFailure",
    "subagentStart",
    "subagentStop",
    "beforeShellExecution",
    "afterShellExecution",
    "beforeMCPExecution",
    "afterMCPExecution",
    "beforeReadFile",
    "afterFileEdit",
    "beforeSubmitPrompt",
    "preCompact",
    "stop",
    "afterAgentResponse",
    "afterAgentThought",
    "beforeTabFileRead",
    "afterTabFileEdit",
    "workspaceOpen",
];

/// Vendored Cursor cloud-environment schema. It is compiled from the checked-in
/// snapshot, so linting neither reads a local schema path nor fetches a network
/// resource. See `schemas/cursor-environment.schema.md` for provenance.
static CURSOR_ENVIRONMENT_VALIDATOR: LazyLock<jsonschema::Validator> = LazyLock::new(|| {
    let schema = serde_json::from_str(include_str!("../../schemas/cursor-environment.schema.json"))
        .expect("checked-in Cursor environment schema parses");
    jsonschema::validator_for(&schema).expect("embedded Cursor environment schema is valid")
});

/// Validate Cursor surfaces when they are present. Every validator is
/// file-gated, so Claude-only repositories receive no Cursor diagnostics.
#[cfg(test)]
pub fn validate(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let mut prompt_pass = super::prompt_content::PromptContentPass::default();
    validate_with_prompt_pass(diag, exclude, &mut prompt_pass);
}

pub(crate) fn validate_with_prompt_pass(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    prompt_pass: &mut super::prompt_content::PromptContentPass,
) {
    validate_legacy_rules(diag, exclude, prompt_pass);
    validate_project_rules(diag, exclude, prompt_pass);
    validate_hooks(diag, exclude);
    validate_agents(diag, exclude, prompt_pass);
    validate_environment(diag, exclude);
    validate_skills(diag, exclude, prompt_pass);
}

fn report(diag: &mut DiagnosticCollector, rule: LintRule, path: &str, message: &str) {
    diag.report_at(rule, path, &format!("{path}: {message}"));
}

fn report_meta(
    diag: &mut DiagnosticCollector,
    rule: LintRule,
    path: &str,
    message: &str,
    metadata: DiagnosticMetadata,
) {
    diag.report_at_with(rule, path, &format!("{path}: {message}"), metadata);
}

/// Whether `name` matches Cursor's documented lowercase-letter-and-hyphen
/// identifier format.
pub(crate) fn is_cursor_agent_identifier(name: &str) -> bool {
    CURSOR_AGENT_NAME.is_match(name)
}

/// Recursive, exclusion-filtered, sorted inventory of `.cursor/agents/**/*.md`.
///
/// CU014/CU015 and A030 share this discovery so Cursor subagent routing stays
/// on one walker (issue #303).
pub(crate) fn discover_cursor_agent_paths(exclude: &ExcludeSet) -> Vec<String> {
    let root = Path::new(".cursor/agents");
    if !root.is_dir() {
        return Vec::new();
    }
    let mut paths = Vec::new();
    for entry in traversal::recursive_files(root, Path::new("."), Some(exclude)).entries {
        if entry.path.extension().is_none_or(|ext| ext != "md") {
            continue;
        }
        paths.push(entry.display);
    }
    paths.sort();
    paths
}

fn optional_nonempty_string(
    diag: &mut DiagnosticCollector,
    path: &str,
    yaml: &Mapping,
    field: &str,
) {
    let Some(value) = yaml.get(field) else {
        return;
    };
    match value.as_str() {
        Some(text) if !text.trim().is_empty() => {}
        Some(_) => report(
            diag,
            LintRule::CursorAgentFrontmatterInvalid,
            path,
            &format!("'{field}' must be a non-empty string"),
        ),
        None => report(
            diag,
            LintRule::CursorAgentFrontmatterInvalid,
            path,
            &format!("'{field}' must be a string"),
        ),
    }
}

fn validate_agent_identifier(diag: &mut DiagnosticCollector, path: &str, yaml: &Mapping) {
    match yaml.get("name") {
        None => {
            let stem = Path::new(path)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("");
            if !is_cursor_agent_identifier(stem) {
                report(
                    diag,
                    LintRule::CursorAgentFrontmatterInvalid,
                    path,
                    &format!(
                        "derived name '{stem}' from filename must use lowercase letters and hyphens (e.g. 'code-reviewer')"
                    ),
                );
            }
        }
        Some(value) => match value.as_str() {
            Some(text) if text.trim().is_empty() => report(
                diag,
                LintRule::CursorAgentFrontmatterInvalid,
                path,
                "'name' must be a non-empty string",
            ),
            Some(text) if !is_cursor_agent_identifier(text) => report(
                diag,
                LintRule::CursorAgentFrontmatterInvalid,
                path,
                "'name' must use lowercase letters and hyphens (e.g. 'code-reviewer')",
            ),
            Some(_) => {}
            None => report(
                diag,
                LintRule::CursorAgentFrontmatterInvalid,
                path,
                "'name' must be a string",
            ),
        },
    }
}

fn yaml_parse_constraint(message: &str) -> String {
    if contains_sensitive_evidence(message) {
        "invalid syntax".to_string()
    } else {
        message.to_string()
    }
}

fn validate_legacy_rules(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    prompt_pass: &mut super::prompt_content::PromptContentPass,
) {
    const PATH: &str = ".cursorrules";
    if exclude.is_excluded(PATH) || !Path::new(PATH).is_file() {
        return;
    }
    report(
        diag,
        LintRule::CursorLegacyRules,
        PATH,
        "legacy .cursorrules file is present; migrate to .cursor/rules/*.mdc",
    );
    if let Ok(content) = fs::read_to_string(PATH) {
        if content.trim().is_empty() {
            report(
                diag,
                LintRule::CursorRuleEmpty,
                PATH,
                "rule file has no instructions",
            );
        }
        let markdown = MarkdownDocument::parse_body(&content);
        let document = LiveInstructionDocument::new(
            Path::new(PATH),
            InstructionSurfaceKind::CursorLegacyRule,
            &markdown,
        );
        prompt_pass.validate(&document, diag);
    }
}

fn validate_project_rules(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    prompt_pass: &mut super::prompt_content::PromptContentPass,
) {
    for entry in platforms::cursor_rule_candidates(exclude) {
        let extension = entry.path.extension().and_then(|ext| ext.to_str());
        let path = entry.display;
        if extension == Some("md") {
            let renamed_path = path
                .strip_suffix(".md")
                .map(|stem| format!("{stem}.mdc"))
                .expect("Cursor candidate filtering only returns .md or .mdc files");
            diag.report_at_with(
                LintRule::CursorRuleExtension,
                &path,
                &format!("{path}: Cursor project rules must use the .mdc extension"),
                DiagnosticMetadata::default().with_suggestion(format!("rename to {renamed_path}")),
            );
        } else if let Ok(content) = fs::read_to_string(&entry.path) {
            if extension == Some("mdc") {
                validate_rule_file(diag, &path, &content, prompt_pass);
            }
        }
    }
}

/// One Cursor rule type, derived from the effective values of `alwaysApply`,
/// `globs`, and `description`. Key presence is never a signal: Cursor's own
/// canonical MDC example ships an empty `globs:` line (issue #308).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CursorRuleActivation {
    Always,
    AutoAttached,
    AgentRequested,
    Manual,
}

/// Derive the single activation state in the binding precedence order:
/// `alwaysApply: true` wins; otherwise a non-empty glob selects file
/// attachment; otherwise a non-empty description selects agent-requested
/// loading; otherwise the rule is Manual. A missing or invalid `alwaysApply`
/// behaves as `false`.
fn derive_cursor_activation(
    always_apply: bool,
    has_effective_glob: bool,
    has_description: bool,
) -> CursorRuleActivation {
    if always_apply {
        CursorRuleActivation::Always
    } else if has_effective_glob {
        CursorRuleActivation::AutoAttached
    } else if has_description {
        CursorRuleActivation::AgentRequested
    } else {
        CursorRuleActivation::Manual
    }
}

/// Effective `globs` content after the CU004 field contract has been applied.
#[derive(Default)]
struct GlobsAnalysis {
    /// A well-shaped field holds at least one non-empty pattern.
    has_effective: bool,
    /// How many effective patterns are structurally valid globset syntax.
    effective_valid: usize,
}

/// Structured location for an unindented `key: value` frontmatter line, or no
/// location when the lexical helper cannot map the key exactly.
fn rule_key_metadata(lines: &[String], key: &str) -> DiagnosticMetadata {
    frontmatter::simple_top_level_key_line(lines, key)
        .map_or_else(DiagnosticMetadata::default, DiagnosticMetadata::at_line)
}

/// Whether a strict-YAML failure is the anchor/alias class that unquoted glob
/// values such as `globs: *.ts` produce.
fn is_anchor_alias_error(raw_message: &str) -> bool {
    let lowered = raw_message.to_ascii_lowercase();
    lowered.contains("anchor") || lowered.contains("alias")
}

/// Mechanical fix for an unquoted glob that strict YAML reads as an alias.
/// A trailing YAML comment is not part of what Cursor reads, so it stays
/// outside the quotes. Withheld when the raw value looks sensitive, so the
/// suggestion never echoes a possible secret.
fn quote_globs_suggestion(globs_line: &str) -> Option<String> {
    let raw = globs_line.trim_start().strip_prefix("globs:")?;
    // A YAML comment starts at a `#` that opens the value or follows
    // whitespace.
    let value = raw
        .char_indices()
        .find(|(index, character)| {
            *character == '#' && raw[..*index].chars().last().is_none_or(char::is_whitespace)
        })
        .map_or(raw, |(index, _)| &raw[..index])
        .trim();
    if value.is_empty() || contains_sensitive_evidence(value) {
        return None;
    }
    Some(format!(
        "quote the pattern so it is valid YAML, e.g. globs: \"{value}\""
    ))
}

/// Reduce a YAML parser failure to its constraint: parser-relative coordinates
/// are stripped (the diagnostic carries a structured file location instead)
/// and messages that embed a possible secret collapse to a stable constraint.
/// The wrapper-form strip is owned by `frontmatter::strip_parser_location_prefix`;
/// only the colon-less trailing form of anchor/alias errors is Cursor-local.
fn cursor_rule_yaml_constraint(raw_message: &str) -> String {
    static TRAILING_LOCATION: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\s+at line \d+, column \d+$").expect("cursor yaml trailing-location regex")
    });
    let unwrapped = frontmatter::strip_parser_location_prefix(raw_message);
    let unwrapped = TRAILING_LOCATION.replace(&unwrapped, "");
    yaml_parse_constraint(unwrapped.trim())
}

/// Report CU003 for frontmatter that is not valid YAML. The location is the
/// parser's line/column translated to file coordinates (the opening `---` is
/// file line 1). Syntax errors carry no suggestion — the parser constraint is
/// the action — except the anchor/alias class reported on a `globs:` line,
/// where quoting the unquoted pattern makes the file valid YAML without
/// changing what Cursor reads.
fn report_rule_yaml_error(
    diag: &mut DiagnosticCollector,
    path: &str,
    lines: &[String],
    raw_message: &str,
    yaml_line: Option<usize>,
    yaml_column: Option<usize>,
) {
    // Position-less failures such as duplicate keys anchor at the opening
    // delimiter, keeping every CU003 located.
    let mut metadata =
        DiagnosticMetadata::default().with_location(match (yaml_line, yaml_column) {
            (Some(line), Some(column)) => SourceSpan::point(line.saturating_add(1), column),
            (Some(line), None) => SourceSpan::line(line.saturating_add(1)),
            (None, _) => SourceSpan::line(1),
        });
    if is_anchor_alias_error(raw_message)
        && let Some(offending) = yaml_line
            .and_then(|line| line.checked_sub(1))
            .and_then(|index| lines.get(index))
        && let Some(suggestion) = quote_globs_suggestion(offending)
    {
        metadata = metadata.with_suggestion(suggestion);
    }
    report_meta(
        diag,
        LintRule::CursorRuleFrontmatterInvalid,
        path,
        &format!(
            "frontmatter is not valid YAML: {}",
            cursor_rule_yaml_constraint(raw_message)
        ),
        metadata,
    );
}

fn validate_rule_file(
    diag: &mut DiagnosticCollector,
    path: &str,
    content: &str,
    prompt_pass: &mut super::prompt_content::PromptContentPass,
) {
    // Classify recovery before prompt dispatch with the shared optional-BOM /
    // exact-delimiter boundary so CU002/CU003 and Q rules cannot disagree.
    let recovery = frontmatter::exact_leading_frontmatter(content);
    if let Some(markdown) = MarkdownDocument::parse_for_prompt_content(content) {
        let document = LiveInstructionDocument::new(
            Path::new(path),
            InstructionSurfaceKind::CursorRule,
            &markdown,
        );
        prompt_pass.validate(&document, diag);
    }
    match recovery {
        frontmatter::LeadingFrontmatterState::Absent { .. } => {
            let scanned = content.strip_prefix('\u{feff}').unwrap_or(content);
            if scanned.trim().is_empty() {
                report(
                    diag,
                    LintRule::CursorRuleEmpty,
                    path,
                    "rule file has no instructions",
                );
                return;
            }
            // Prefixes such as `----` or `---suffix` are missing openers (CU002),
            // not malformed closed frontmatter (CU003).
            report_meta(
                diag,
                LintRule::CursorRuleFrontmatterMissing,
                path,
                "missing YAML frontmatter",
                DiagnosticMetadata::at_line(1)
                    .with_suggestion("open the file with a first line containing exactly '---'"),
            );
        }
        frontmatter::LeadingFrontmatterState::Unterminated { .. } => {
            report_meta(
                diag,
                LintRule::CursorRuleFrontmatterInvalid,
                path,
                "frontmatter must have a closing '---' delimiter",
                DiagnosticMetadata::at_line(1).with_suggestion(
                    "add a line containing exactly '---' after the frontmatter fields",
                ),
            );
        }
        frontmatter::LeadingFrontmatterState::Complete(block) => {
            if block.body.trim().is_empty() {
                report(
                    diag,
                    LintRule::CursorRuleEmpty,
                    path,
                    "rule file has no instructions after frontmatter",
                );
            }
            let lines: Vec<String> = block.yaml.lines().map(str::to_owned).collect();
            // Reconstruct the document with its trailing newline so a final bare
            // `key:` line keeps the mapping shape it has in the file.
            let mut source = lines.join("\n");
            source.push('\n');
            let yaml = match crate::yaml::parse(&source) {
                Ok(YamlValue::Mapping(map)) => map,
                // Empty frontmatter has no targeting fields: a valid Manual rule.
                Ok(YamlValue::Null) => Mapping::new(),
                Ok(_) => {
                    report_meta(
                        diag,
                        LintRule::CursorRuleFrontmatterInvalid,
                        path,
                        "frontmatter must be a YAML object",
                        DiagnosticMetadata::at_line(1).with_suggestion(
                            "use `key: value` mappings for description, globs, and alwaysApply",
                        ),
                    );
                    return;
                }
                Err(error) => {
                    report_rule_yaml_error(
                        diag,
                        path,
                        &lines,
                        &error.to_string(),
                        crate::yaml::error_line(&error),
                        crate::yaml::error_column(&error),
                    );
                    return;
                }
            };
            validate_rule_frontmatter(diag, path, &yaml, &lines);
        }
    }
}

/// Validate a strictly parsed rule frontmatter mapping and derive its one
/// activation state. Only effective values drive diagnostics; key presence is
/// never a signal (issue #308).
fn validate_rule_frontmatter(
    diag: &mut DiagnosticCollector,
    path: &str,
    yaml: &Mapping,
    lines: &[String],
) {
    for key in yaml.keys() {
        if !RULE_KEYS.contains(&key.as_str()) {
            report_meta(
                diag,
                LintRule::CursorRuleFieldUnknown,
                path,
                &format!("unknown frontmatter field '{key}'"),
                rule_key_metadata(lines, key)
                    .with_evidence(key)
                    .with_suggestion(format!(
                        "remove '{key}' or use one of: description, globs, alwaysApply"
                    )),
            );
        }
    }
    // A description is textual routing metadata: null and empty strings are
    // valid unset values, every other non-string shape is structural.
    let description = yaml.get("description");
    if let Some(value) = description
        && !value.is_null()
        && value.as_str().is_none()
    {
        report_meta(
            diag,
            LintRule::CursorRuleFrontmatterInvalid,
            path,
            "'description' must be a string",
            rule_key_metadata(lines, "description")
                .with_evidence("description")
                .with_suggestion("set 'description' to a string, or remove the field"),
        );
    }
    let globs = analyze_globs(diag, path, yaml, lines);
    let always_apply = yaml.get("alwaysApply");
    if let Some(value) = always_apply
        && !value.is_bool()
    {
        report_meta(
            diag,
            LintRule::CursorAlwaysApplyInvalid,
            path,
            "'alwaysApply' must be a boolean",
            rule_key_metadata(lines, "alwaysApply")
                .with_evidence("alwaysApply")
                .with_suggestion("set 'alwaysApply' to unquoted true or false"),
        );
    }
    let has_description = description
        .and_then(YamlValue::as_str)
        .is_some_and(|text| !text.trim().is_empty());
    // A present non-boolean `alwaysApply` was reported above and recovers as
    // false for state derivation.
    let activation = derive_cursor_activation(
        always_apply.and_then(YamlValue::as_bool) == Some(true),
        globs.has_effective,
        has_description,
    );
    // CU007 warns only when an Always rule declares a real, structurally valid
    // pattern that Cursor will ignore. Unset globs and CU004 shape failures
    // stay silent.
    if activation == CursorRuleActivation::Always && globs.effective_valid > 0 {
        report_meta(
            diag,
            LintRule::CursorAlwaysApplyGlobs,
            path,
            "effective 'globs' patterns are ignored because 'alwaysApply' is true",
            rule_key_metadata(lines, "globs")
                .with_evidence("globs")
                .with_suggestion("remove 'globs' or set 'alwaysApply: false'"),
        );
    }
}

/// Apply the CU004 field contract: `globs` accepts null, a string, or a
/// sequence of strings. Null, blank strings, and sequences holding no
/// non-empty strings are unset. Any other container/scalar type, or any
/// non-string sequence member, is one field diagnostic. Effective patterns
/// then pass through conservative globset validation one by one; a quoted
/// comma-joined value is one pattern (comma splitting would corrupt brace
/// groups such as `{a,b}`).
fn analyze_globs(
    diag: &mut DiagnosticCollector,
    path: &str,
    yaml: &Mapping,
    lines: &[String],
) -> GlobsAnalysis {
    let Some(value) = yaml.get("globs") else {
        return GlobsAnalysis::default();
    };
    let patterns: Option<Vec<&str>> = match value {
        YamlValue::Null => return GlobsAnalysis::default(),
        YamlValue::String(pattern) => Some(vec![pattern.as_str()]),
        YamlValue::Sequence(items) => items.iter().map(YamlValue::as_str).collect(),
        _ => None,
    };
    let Some(patterns) = patterns else {
        report_meta(
            diag,
            LintRule::CursorRuleGlobInvalid,
            path,
            "'globs' must be a string or list of strings",
            rule_key_metadata(lines, "globs")
                .with_evidence("globs")
                .with_suggestion("set 'globs' to one glob string or a list of glob strings"),
        );
        return GlobsAnalysis::default();
    };
    let effective: Vec<&str> = patterns
        .into_iter()
        .filter(|pattern| !pattern.trim().is_empty())
        .collect();
    if effective.is_empty() {
        return GlobsAnalysis::default();
    }
    let mut effective_valid = 0;
    for pattern in effective {
        match GlobBuilder::new(pattern).build() {
            Ok(_) => effective_valid += 1,
            Err(error) => {
                // The globset error embeds the pattern; a secret-shaped
                // pattern collapses to the field name (its evidence is
                // redacted by the same shared heuristic).
                let message = if contains_sensitive_evidence(pattern) {
                    "'globs' contains an invalid pattern".to_string()
                } else {
                    format!("invalid glob '{pattern}': {error}")
                };
                report_meta(
                    diag,
                    LintRule::CursorRuleGlobInvalid,
                    path,
                    &message,
                    rule_key_metadata(lines, "globs")
                        .with_evidence(pattern)
                        .with_suggestion("write the pattern with valid globset syntax"),
                );
            }
        }
    }
    GlobsAnalysis {
        has_effective: true,
        effective_valid,
    }
}

/// Parser point for invalid JSON: serde's line/column, line-only when the
/// error carries no column.
fn json_parse_error_location(error: &serde_json::Error) -> SourceSpan {
    if error.column() > 0 {
        SourceSpan::point(error.line().max(1), error.column())
    } else {
        SourceSpan::line(error.line().max(1))
    }
}

/// Structured metadata for a hooks.json finding: bounded structural-path
/// evidence plus the source span of the named token. `key_token` anchors at
/// the key spelling (unknown event names); otherwise the value at `path` is
/// located, falling back through `fallback` (for example the owning entry when
/// a required field is absent). Instance values never enter the evidence.
fn hooks_metadata(
    content: &str,
    evidence: &str,
    key_token: bool,
    path: &[Seg<'_>],
    fallback: &[Seg<'_>],
) -> DiagnosticMetadata {
    let range = if key_token {
        JsonScanner::locate_key(content, path)
    } else {
        JsonScanner::locate(content, path).or_else(|| {
            (!fallback.is_empty())
                .then(|| JsonScanner::locate(content, fallback))
                .flatten()
        })
    };
    let mut metadata = DiagnosticMetadata::default().with_evidence(evidence);
    if let Some(span) = range.and_then(|range| SourceSpan::from_byte_range(content, range)) {
        metadata = metadata.with_location(span);
    }
    metadata
}

fn validate_hooks(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    const PATH: &str = ".cursor/hooks.json";
    if exclude.is_excluded(PATH) {
        return;
    }
    let Ok(content) = fs::read_to_string(PATH) else {
        return;
    };
    let value = match serde_json::from_str::<JsonValue>(&content) {
        Ok(value) => value,
        Err(error) => {
            report_meta(
                diag,
                LintRule::CursorHooksSchemaInvalid,
                PATH,
                &format!("invalid JSON: {error}"),
                DiagnosticMetadata::default()
                    .with_location(json_parse_error_location(&error))
                    .with_evidence("JSON syntax"),
            );
            return;
        }
    };
    let Some(root) = value.as_object() else {
        report_meta(
            diag,
            LintRule::CursorHooksSchemaInvalid,
            PATH,
            "top level must be an object",
            hooks_metadata(&content, "top level", false, &[], &[]),
        );
        return;
    };
    if !root
        .get("version")
        .is_some_and(|version| version.is_number() && version.as_f64() == Some(1.0))
    {
        report_meta(
            diag,
            LintRule::CursorHooksSchemaInvalid,
            PATH,
            "'version' must be numeric schema version 1",
            hooks_metadata(&content, "version", false, &[Seg::Key("version")], &[]),
        );
    }
    let Some(hooks) = root.get("hooks").and_then(JsonValue::as_object) else {
        report_meta(
            diag,
            LintRule::CursorHooksSchemaInvalid,
            PATH,
            "'hooks' must be an object",
            hooks_metadata(&content, "hooks", false, &[Seg::Key("hooks")], &[]),
        );
        return;
    };
    for (event, entries) in hooks {
        let event_path = [Seg::Key("hooks"), Seg::Key(event)];
        if !HOOK_EVENTS.contains(&event.as_str()) {
            report_meta(
                diag,
                LintRule::CursorHookEventUnknown,
                PATH,
                &format!("unknown hook event '{event}'"),
                hooks_metadata(&content, &format!("hooks.{event}"), true, &event_path, &[]),
            );
        }
        let Some(entries) = entries.as_array() else {
            report_meta(
                diag,
                LintRule::CursorHooksSchemaInvalid,
                PATH,
                &format!("hooks.{event} must be an array"),
                hooks_metadata(&content, &format!("hooks.{event}"), false, &event_path, &[]),
            );
            continue;
        };
        for (index, entry) in entries.iter().enumerate() {
            let label = format!("hooks.{event}[{}]", index + 1);
            let entry_path = [Seg::Key("hooks"), Seg::Key(event), Seg::Index(index)];
            let field_path = |field: &'static str| {
                [
                    Seg::Key("hooks"),
                    Seg::Key(event),
                    Seg::Index(index),
                    Seg::Key(field),
                ]
            };
            let Some(entry) = entry.as_object() else {
                report_meta(
                    diag,
                    LintRule::CursorHooksSchemaInvalid,
                    PATH,
                    &format!("{label} must be an object"),
                    hooks_metadata(&content, &label, false, &entry_path, &[]),
                );
                continue;
            };
            let type_value = entry.get("type");
            let hook_type = type_value.and_then(JsonValue::as_str);
            if (type_value.is_none() || hook_type == Some("command"))
                && entry
                    .get("command")
                    .and_then(JsonValue::as_str)
                    .is_none_or(|value| value.trim().is_empty())
            {
                report_meta(
                    diag,
                    LintRule::CursorHookCommandMissing,
                    PATH,
                    &format!("{label} is missing a non-empty 'command'"),
                    hooks_metadata(
                        &content,
                        &format!("{label}.command"),
                        false,
                        &field_path("command"),
                        &entry_path,
                    ),
                );
            }
            if let Some(kind) = entry.get("type")
                && !matches!(kind.as_str(), Some("command" | "prompt"))
            {
                report_meta(
                    diag,
                    LintRule::CursorHookTypeInvalid,
                    PATH,
                    &format!("{label}.type must be 'command' or 'prompt'"),
                    hooks_metadata(
                        &content,
                        &format!("{label}.type"),
                        false,
                        &field_path("type"),
                        &entry_path,
                    ),
                );
            }
            for (field, valid) in [
                (
                    "timeout",
                    entry.get("timeout").is_none_or(JsonValue::is_number),
                ),
                (
                    "loop_limit",
                    entry
                        .get("loop_limit")
                        .is_none_or(|value| value.is_null() || value.is_number()),
                ),
                (
                    "failClosed",
                    entry.get("failClosed").is_none_or(JsonValue::is_boolean),
                ),
                (
                    "matcher",
                    entry.get("matcher").is_none_or(JsonValue::is_string),
                ),
            ] {
                if !valid {
                    report_meta(
                        diag,
                        LintRule::CursorHookFieldTypeInvalid,
                        PATH,
                        &format!("{label}.{field} has an invalid type"),
                        hooks_metadata(
                            &content,
                            &format!("{label}.{field}"),
                            false,
                            &field_path(field),
                            &entry_path,
                        ),
                    );
                }
            }
            if hook_type == Some("prompt") {
                if entry
                    .get("prompt")
                    .and_then(JsonValue::as_str)
                    .is_none_or(|value| value.trim().is_empty())
                {
                    report_meta(
                        diag,
                        LintRule::CursorPromptHookPromptMissing,
                        PATH,
                        &format!("{label} is missing a non-empty 'prompt'"),
                        hooks_metadata(
                            &content,
                            &format!("{label}.prompt"),
                            false,
                            &field_path("prompt"),
                            &entry_path,
                        ),
                    );
                }
                if entry
                    .get("model")
                    .is_some_and(|value| value.as_str().is_none_or(|model| model.trim().is_empty()))
                {
                    report_meta(
                        diag,
                        LintRule::CursorPromptHookModelInvalid,
                        PATH,
                        &format!("{label}.model must be a non-empty string"),
                        hooks_metadata(
                            &content,
                            &format!("{label}.model"),
                            false,
                            &field_path("model"),
                            &entry_path,
                        ),
                    );
                }
            }
        }
    }
}

fn validate_agents(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    prompt_pass: &mut super::prompt_content::PromptContentPass,
) {
    for path in discover_cursor_agent_paths(exclude) {
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        validate_agent_file(diag, &path, &content, prompt_pass);
    }
}

fn validate_agent_file(
    diag: &mut DiagnosticCollector,
    path: &str,
    content: &str,
    prompt_pass: &mut super::prompt_content::PromptContentPass,
) {
    if let Some(markdown) = MarkdownDocument::parse_for_prompt_content(content) {
        let document =
            LiveInstructionDocument::new(Path::new(path), InstructionSurfaceKind::Agent, &markdown);
        prompt_pass.validate(&document, diag);
    }
    let Some(lines) = frontmatter::extract_frontmatter(content) else {
        report(
            diag,
            LintRule::CursorAgentFrontmatterInvalid,
            path,
            "missing or malformed YAML frontmatter",
        );
        // Body boundary is unknowable without valid delimiters, so CU015 does
        // not run.
        return;
    };

    match frontmatter::parse_yaml_strict(&lines) {
        Ok(YamlValue::Mapping(yaml)) => {
            validate_agent_identifier(diag, path, &yaml);
            optional_nonempty_string(diag, path, &yaml, "description");
            optional_nonempty_string(diag, path, &yaml, "model");
            for field in ["readonly", "is_background"] {
                if yaml.get(field).is_some_and(|value| !value.is_bool()) {
                    report(
                        diag,
                        LintRule::CursorAgentFrontmatterInvalid,
                        path,
                        &format!("'{field}' must be a boolean"),
                    );
                }
            }
        }
        Ok(_) => {
            report(
                diag,
                LintRule::CursorAgentFrontmatterInvalid,
                path,
                "frontmatter must be a YAML object",
            );
        }
        Err(error) => {
            report(
                diag,
                LintRule::CursorAgentFrontmatterInvalid,
                path,
                &format!(
                    "frontmatter is not valid YAML: {}",
                    yaml_parse_constraint(&error.message)
                ),
            );
        }
    }

    // Delimiters were valid, so the body boundary is known even when field
    // checks or YAML parsing failed.
    if frontmatter::extract_body(content).trim().is_empty() {
        report(
            diag,
            LintRule::CursorAgentBodyEmpty,
            path,
            "subagent body is empty",
        );
    }
}

fn validate_environment(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    const PATH: &str = ".cursor/environment.json";
    if exclude.is_excluded(PATH) {
        return;
    }
    let Ok(content) = fs::read_to_string(PATH) else {
        return;
    };
    let value = match serde_json::from_str::<JsonValue>(&content) {
        Ok(value) => value,
        Err(error) => {
            report_meta(
                diag,
                LintRule::CursorEnvironmentInvalid,
                PATH,
                &format!("invalid JSON: {error}"),
                DiagnosticMetadata::default()
                    .with_location(json_parse_error_location(&error))
                    .with_evidence("JSON syntax"),
            );
            return;
        }
    };
    let mut findings = Vec::new();
    for error in CURSOR_ENVIRONMENT_VALIDATOR.iter_errors(&value) {
        resolve_environment_error(&error, &mut findings);
    }
    for finding in suppress_unevaluated_cascades(findings) {
        let property_path = environment_property_path(&finding.segments);
        report_meta(
            diag,
            LintRule::CursorEnvironmentInvalid,
            PATH,
            // The configuration may contain commands or credentials. Retain
            // the actionable path and constraint while masking its value.
            &format!("{property_path}: {}", finding.message),
            finding.metadata(&content, &property_path),
        );
    }
}

/// One instance-path segment of an environment schema finding.
#[derive(Clone, PartialEq, Eq)]
enum EnvSeg {
    Key(String),
    Index(usize),
}

/// A schema violation resolved to its narrowest actionable property path.
struct EnvironmentFinding {
    /// Full instance path, including a `Required` error's missing property.
    segments: Vec<EnvSeg>,
    /// The final segment names a missing required property, so the source
    /// location anchors at the owning object rather than an absent token.
    missing_property: bool,
    /// `unevaluatedProperties` payload used to drop union-parent cascades.
    unevaluated: Option<Vec<String>>,
    /// Masked constraint text; instance values never appear.
    message: String,
}

impl EnvironmentFinding {
    fn from_error(error: &jsonschema::ValidationError<'_>) -> Self {
        let mut segments = pointer_segments(error.instance_path().as_str());
        let mut missing_property = false;
        let mut unevaluated = None;
        match error.kind() {
            ValidationErrorKind::Required { property } => {
                if let Some(property) = property.as_str() {
                    segments.push(EnvSeg::Key(property.to_string()));
                    missing_property = true;
                }
            }
            ValidationErrorKind::UnevaluatedProperties { unexpected } => {
                unevaluated = Some(unexpected.clone());
            }
            _ => {}
        }
        Self {
            segments,
            missing_property,
            unevaluated,
            message: error.masked().to_string(),
        }
    }

    /// Bounded structural metadata: the property path as evidence plus the
    /// narrowest recoverable source span (the value, the owning object for a
    /// missing property, or the first unexpected key token).
    fn metadata(&self, content: &str, property_path: &str) -> DiagnosticMetadata {
        let mut metadata = DiagnosticMetadata::default().with_evidence(property_path);
        let locator_path: Vec<Seg<'_>> = self
            .segments
            .iter()
            .map(|segment| match segment {
                EnvSeg::Key(key) => Seg::Key(key),
                EnvSeg::Index(index) => Seg::Index(*index),
            })
            .collect();
        let range = if self.missing_property {
            let parent = &locator_path[..locator_path.len() - 1];
            JsonScanner::locate(content, parent)
        } else if let Some(unexpected) = self
            .unevaluated
            .as_ref()
            .and_then(|unexpected| unexpected.first())
        {
            let mut key_path = locator_path.clone();
            key_path.push(Seg::Key(unexpected));
            JsonScanner::locate_key(content, &key_path)
        } else {
            JsonScanner::locate(content, &locator_path)
        };
        if let Some(span) = range.and_then(|range| SourceSpan::from_byte_range(content, range)) {
            metadata = metadata.with_location(span);
        }
        metadata
    }
}

/// Surface the narrowest actionable leaf errors for a failed `oneOf` union.
///
/// The crate reports a union failure at the union parent with every branch's
/// child errors attached. When exactly one branch failed for reasons other
/// than a fundamental type mismatch, that branch is the one the author meant,
/// so its leaf errors replace the generic parent finding; otherwise the parent
/// finding is kept unchanged.
fn resolve_environment_error(
    error: &jsonschema::ValidationError<'_>,
    findings: &mut Vec<EnvironmentFinding>,
) {
    if let ValidationErrorKind::OneOfNotValid { context } = error.kind() {
        let parent_pointer = error.instance_path().as_str().to_string();
        let mut pertinent = context.iter().filter(|branch| {
            !branch.is_empty() && !branch_is_type_mismatch(branch, &parent_pointer)
        });
        if let (Some(branch), None) = (pertinent.next(), pertinent.next()) {
            for child in branch {
                resolve_environment_error(child, findings);
            }
            return;
        }
    }
    findings.push(EnvironmentFinding::from_error(error));
}

/// A branch that failed only because the instance has the wrong fundamental
/// JSON type carries exactly one `type` error at the union parent itself.
fn branch_is_type_mismatch(
    branch: &[jsonschema::ValidationError<'_>],
    parent_pointer: &str,
) -> bool {
    matches!(branch, [only]
        if matches!(only.kind(), ValidationErrorKind::Type { .. })
            && only.instance_path().as_str() == parent_pointer)
}

/// Drop an `unevaluatedProperties` finding when every property it names is
/// already owned by a more specific finding underneath it. Failing subschemas
/// discard their annotations, so a defective known property re-surfaces as
/// "unevaluated" at its parent; the deeper finding is the actionable one.
/// Findings naming any genuinely unknown property are kept, reordered so the
/// first named property (the location anchor) is a genuinely unknown one.
fn suppress_unevaluated_cascades(findings: Vec<EnvironmentFinding>) -> Vec<EnvironmentFinding> {
    // Any finding can own a flagged property, including a deeper unevaluated
    // finding (an unknown key inside `build` also cascades to the root). An
    // owning prefix is strictly longer than the flagging finding's own path,
    // so no finding suppresses itself, and two unevaluated findings cannot
    // suppress each other cyclically.
    let all_paths: Vec<Vec<EnvSeg>> = findings
        .iter()
        .map(|finding| finding.segments.clone())
        .collect();
    findings
        .into_iter()
        .filter_map(|mut finding| {
            let Some(unexpected) = &finding.unevaluated else {
                return Some(finding);
            };
            let (genuine, cascaded): (Vec<String>, Vec<String>) =
                unexpected.iter().cloned().partition(|property| {
                    let mut prefix = finding.segments.clone();
                    prefix.push(EnvSeg::Key(property.clone()));
                    !all_paths.iter().any(|segments| {
                        segments.len() >= prefix.len() && segments[..prefix.len()] == prefix
                    })
                });
            if genuine.is_empty() {
                return None;
            }
            let mut anchored = genuine;
            anchored.extend(cascaded);
            finding.unevaluated = Some(anchored);
            Some(finding)
        })
        .collect()
}

fn pointer_segments(pointer: &str) -> Vec<EnvSeg> {
    pointer
        .trim_start_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let segment = segment.replace("~1", "/").replace("~0", "~");
            match segment.parse::<usize>() {
                Ok(index) => EnvSeg::Index(index),
                Err(_) => EnvSeg::Key(segment),
            }
        })
        .collect()
}

/// Render the property paths shown in agent-lint diagnostics. Array indices
/// are one-based, matching the validator's prior Cursor environment output.
fn environment_property_path(segments: &[EnvSeg]) -> String {
    let mut path = String::new();
    for segment in segments {
        match segment {
            EnvSeg::Index(index) => path.push_str(&format!("[{}]", index + 1)),
            EnvSeg::Key(key) => {
                if !path.is_empty() {
                    path.push('.');
                }
                path.push_str(key);
            }
        }
    }
    if path.is_empty() {
        "top level".to_string()
    } else {
        path
    }
}

fn validate_skills(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    prompt_pass: &mut super::prompt_content::PromptContentPass,
) {
    for entry in crate::platforms::cursor_runtime_skill_candidates(exclude) {
        // Shared Agent Skills receive their prompt-content pass through their
        // independent shared-surface dispatch. Only Cursor-unique paths are
        // validated here so activating both surfaces cannot duplicate Q rules.
        let Ok(content) = fs::read_to_string(&entry.path) else {
            continue;
        };
        if crate::platforms::is_cursor_skill_path(&entry.path)
            && let Some(markdown) = MarkdownDocument::parse_for_prompt_content(&content)
        {
            let document = LiveInstructionDocument::new(
                Path::new(&entry.display),
                InstructionSurfaceKind::CursorSkill,
                &markdown,
            );
            prompt_pass.validate(&document, diag);
        }
        let Some(lines) = frontmatter::extract_frontmatter(&content) else {
            continue;
        };
        let Ok(YamlValue::Mapping(yaml)) = crate::yaml::parse(&lines.join("\n")) else {
            continue;
        };
        for key in yaml.keys() {
            if !CURSOR_SKILL_KEYS.contains(&key.as_str()) {
                report(
                    diag,
                    LintRule::CursorSkillFieldUnsupported,
                    &entry.display,
                    &format!("'{key}' is not supported by Cursor skills"),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ExcludeSet;
    use crate::test_helpers::CwdGuard;

    fn codes_for(root: &Path) -> Vec<&'static str> {
        let _guard = CwdGuard::new();
        std::env::set_current_dir(root).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate(&mut diag, &ExcludeSet::default());
        diag.diagnostics()
            .iter()
            .map(|item| item.rule.code())
            .collect()
    }

    fn diagnostics_for(root: &Path) -> Vec<crate::diagnostic::Diagnostic> {
        let _guard = CwdGuard::new();
        std::env::set_current_dir(root).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate(&mut diag, &ExcludeSet::default());
        diag.diagnostics().to_vec()
    }

    /// Sorted CU codes produced by one rule file, ignoring prompt-content
    /// rules so body prose cannot perturb frontmatter assertions.
    fn rule_cu_codes(content: &str) -> Vec<&'static str> {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".cursor/rules")).unwrap();
        std::fs::write(tmp.path().join(".cursor/rules/case.mdc"), content).unwrap();
        let mut codes: Vec<_> = codes_for(tmp.path())
            .into_iter()
            .filter(|code| code.starts_with("CU"))
            .collect();
        codes.sort_unstable();
        codes
    }

    /// CU003/CU004/CU005/CU007/CU008 diagnostics for one rule file.
    fn rule_cu_diagnostics(content: &str) -> Vec<crate::diagnostic::Diagnostic> {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".cursor/rules")).unwrap();
        std::fs::write(tmp.path().join(".cursor/rules/case.mdc"), content).unwrap();
        diagnostics_for(tmp.path())
            .into_iter()
            .filter(|item| item.rule.code().starts_with("CU"))
            .collect()
    }

    fn environment_messages_for(content: &str) -> Vec<String> {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".cursor")).unwrap();
        std::fs::write(tmp.path().join(".cursor/environment.json"), content).unwrap();

        let _guard = CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate(&mut diag, &ExcludeSet::default());
        diag.diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::CursorEnvironmentInvalid)
            .map(|item| item.message.clone())
            .collect()
    }

    #[test]
    #[serial_test::serial]
    fn no_cursor_files_produce_no_cursor_diagnostics() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(codes_for(tmp.path()).is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn project_rules_cover_frontmatter_and_legacy_rules() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".cursor/rules/nested")).unwrap();
        std::fs::write(
            tmp.path().join(".cursor/rules/nested/rule.mdc"),
            "---\nalwaysApply: \"true\"\nglobs: '[unclosed'\nunknown: value\n---\n",
        )
        .unwrap();
        std::fs::write(tmp.path().join(".cursor/rules/missing.mdc"), "# Rule\n").unwrap();
        std::fs::write(
            tmp.path().join(".cursor/rules/malformed.mdc"),
            "---\nglobs: [\n---\nBody\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join(".cursor/rules/always.mdc"),
            "---\nalwaysApply: true\nglobs: '*.rs'\n---\nBody\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join(".cursor/rules/requested.mdc"),
            "---\n---\nBody\n",
        )
        .unwrap();
        std::fs::write(tmp.path().join(".cursorrules"), "").unwrap();
        let codes = codes_for(tmp.path());
        for expected in [
            "CU001", "CU002", "CU003", "CU004", "CU005", "CU006", "CU007", "CU008",
        ] {
            assert!(codes.contains(&expected), "missing {expected}: {codes:?}");
        }
        // Empty frontmatter is a valid Manual rule; CU009 is removed (#308).
        assert!(!codes.contains(&"CU009"), "CU009 must be gone: {codes:?}");
    }

    #[test]
    #[serial_test::serial]
    fn delimiter_table_pins_cu002_cu003_and_empty_rules() {
        let body = "Use the repository's established conventions.\n";
        let cases: Vec<(String, Vec<&str>, &str)> = vec![
            (
                format!("---\ndescription: Documented behavior\n---\n{body}"),
                vec![],
                "exact opener",
            ),
            (
                format!("---\r\ndescription: Documented behavior\r\n---\r\n{body}"),
                vec![],
                "CRLF delimiters",
            ),
            (
                format!("\u{feff}---\ndescription: Documented behavior\n---\n{body}"),
                vec![],
                "UTF-8 BOM before a valid opener",
            ),
            (format!("----\n{body}"), vec!["CU002"], "over-long dashes"),
            (format!("---suffix\n{body}"), vec!["CU002"], "opener suffix"),
            (format!(" ---\n{body}"), vec!["CU002"], "leading whitespace"),
            (body.to_string(), vec!["CU002"], "prose only"),
            (
                format!("\u{feff}----\n{body}"),
                vec!["CU002"],
                "BOM then near-opener",
            ),
            (
                "---\ndescription: Documented behavior\n".to_string(),
                vec!["CU003"],
                "missing closing delimiter",
            ),
            (
                format!("---\n---\n{body}"),
                vec![],
                "empty frontmatter is a Manual rule",
            ),
            ("---\n---\n".to_string(), vec!["CU001"], "empty body"),
            (String::new(), vec!["CU001"], "empty file"),
            ("\u{feff}".to_string(), vec!["CU001"], "BOM-only file"),
        ];
        for (content, expected, label) in cases {
            assert_eq!(rule_cu_codes(&content), expected, "{label}");
        }
    }

    #[test]
    #[serial_test::serial]
    fn prompt_recovery_skips_unterminated_and_bom_metadata() {
        let cases = [
            (
                "---\ndescription: malformed.\n\nRetry until success.\n",
                vec!["CU003"],
                "unterminated after punctuated metadata",
            ),
            (
                "---\ndescription: malformed\n\nRetry until success.\n",
                vec!["CU003"],
                "unterminated after blank line",
            ),
            (
                "\u{feff}---\ndescription: Metadata.\nRetry until success.: true\nalwaysApply: true\n---\nSafe body.\n",
                vec!["CU005"],
                "BOM-prefixed complete metadata must not emit Q005",
            ),
            (
                "\u{feff}---\n- not: mapping\n---\nRetry until success.\n",
                vec!["CU003", "Q005"],
                "BOM-prefixed non-object still analyzes body prose",
            ),
            (
                "---\ndescription: [unclosed\n---\nRetry until success.\n",
                vec!["CU003", "Q005"],
                "complete invalid frontmatter keeps body prose",
            ),
            (
                "Retry until success.\n",
                vec!["CU002", "Q005"],
                "missing frontmatter keeps full-file prose",
            ),
        ];
        for (content, expected, label) in cases {
            let tmp = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(tmp.path().join(".cursor/rules")).unwrap();
            std::fs::write(tmp.path().join(".cursor/rules/case.mdc"), content).unwrap();
            let mut codes: Vec<_> = codes_for(tmp.path())
                .into_iter()
                .filter(|code| matches!(*code, "CU002" | "CU003" | "CU005" | "Q005"))
                .collect();
            codes.sort_unstable();
            let mut expected = expected;
            expected.sort_unstable();
            assert_eq!(codes, expected, "{label}");
            if label.contains("BOM-prefixed complete") {
                let diagnostics = diagnostics_for(tmp.path());
                let q005 = diagnostics
                    .iter()
                    .find(|item| item.rule.code() == "Q005")
                    .map(|item| item.location.unwrap().start().line_number());
                assert_eq!(q005, None, "{label}: metadata must not emit Q005");
            }
            if label.contains("non-object") {
                let diagnostics = diagnostics_for(tmp.path());
                let q005_line = diagnostics
                    .iter()
                    .find(|item| item.rule.code() == "Q005")
                    .map(|item| item.location.unwrap().start().line_number());
                assert_eq!(q005_line, Some(4), "{label}: body keeps original lines");
            }
        }
    }

    #[test]
    fn derive_cursor_activation_follows_binding_precedence() {
        use CursorRuleActivation::{AgentRequested, Always, AutoAttached, Manual};
        let table = [
            (true, true, true, Always),
            (true, true, false, Always),
            (true, false, true, Always),
            (true, false, false, Always),
            (false, true, true, AutoAttached),
            (false, true, false, AutoAttached),
            (false, false, true, AgentRequested),
            (false, false, false, Manual),
        ];
        for (always, glob, description, expected) in table {
            assert_eq!(
                derive_cursor_activation(always, glob, description),
                expected,
                "always={always} glob={glob} description={description}"
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn activation_matrix_pins_state_and_exact_code_set() {
        use CursorRuleActivation::{AgentRequested, Always, AutoAttached, Manual};
        let always_variants = [
            (None, false, false),
            (Some("alwaysApply: false"), false, false),
            (Some("alwaysApply: true"), true, false),
            (Some("alwaysApply: \"yes\""), false, true),
        ];
        let glob_variants = [
            (None, false, false),
            (Some("globs:"), false, false),
            (Some("globs: \"\""), false, false),
            (Some("globs: \"*.rs\""), true, false),
            (Some("globs: 42"), false, true),
        ];
        let description_variants = [
            (None, false, false),
            (Some("description:"), false, false),
            (Some("description: \"\""), false, false),
            (Some("description: Applies to Rust sources"), true, false),
            (Some("description: 42"), false, true),
        ];
        for (always_line, always_effective, expect_cu008) in always_variants {
            for (glob_line, glob_effective, expect_cu004) in glob_variants {
                for (description_line, description_effective, expect_cu003) in description_variants
                {
                    let mut frontmatter = String::from("---\n");
                    for line in [description_line, glob_line, always_line]
                        .into_iter()
                        .flatten()
                    {
                        frontmatter.push_str(line);
                        frontmatter.push('\n');
                    }
                    frontmatter.push_str("---\nUse the repository's conventions.\n");

                    let mut expected = Vec::new();
                    if expect_cu003 {
                        expected.push("CU003");
                    }
                    if expect_cu004 {
                        expected.push("CU004");
                    }
                    if always_effective && glob_effective {
                        expected.push("CU007");
                    }
                    if expect_cu008 {
                        expected.push("CU008");
                    }
                    assert_eq!(
                        rule_cu_codes(&frontmatter),
                        expected,
                        "frontmatter:\n{frontmatter}"
                    );

                    let expected_state = if always_effective {
                        Always
                    } else if glob_effective {
                        AutoAttached
                    } else if description_effective {
                        AgentRequested
                    } else {
                        Manual
                    };
                    assert_eq!(
                        derive_cursor_activation(
                            always_effective,
                            glob_effective,
                            description_effective
                        ),
                        expected_state,
                        "state for:\n{frontmatter}"
                    );
                }
            }
        }
    }

    #[test]
    #[serial_test::serial]
    fn glob_field_contract_pins_unset_shapes_and_patterns() {
        let cases: Vec<(&str, Vec<&str>, &str)> = vec![
            ("globs: \"src/**/*.rs\"", vec![], "scalar pattern"),
            ("globs:", vec![], "null is unset"),
            ("globs: \"\"", vec![], "empty string is unset"),
            ("globs: \"   \"", vec![], "whitespace string is unset"),
            ("globs: []", vec![], "empty list is unset"),
            (
                "globs: [\"\", \"   \"]",
                vec![],
                "list without a real pattern is unset",
            ),
            (
                "globs: [\"src/**/*.rs\", \"*.{md,txt}\", \"[a-z].rs\"]",
                vec![],
                "recursive, brace, and class patterns are valid",
            ),
            ("globs: 42", vec!["CU004"], "number is a field error"),
            ("globs: true", vec!["CU004"], "boolean is a field error"),
            (
                "globs: {pattern: \"*.rs\"}",
                vec!["CU004"],
                "mapping is a field error",
            ),
            (
                "globs: [\"*.rs\", 42]",
                vec!["CU004"],
                "one field error for a non-string member",
            ),
            ("globs: \"[unclosed\"", vec!["CU004"], "malformed pattern"),
            (
                "globs: [\"[one\", \"[two\"]",
                vec!["CU004", "CU004"],
                "one diagnostic per malformed pattern",
            ),
            (
                "globs: \"*.ts,*.tsx\"",
                vec![],
                "comma-joined scalar validates as one pattern",
            ),
        ];
        for (line, expected, label) in cases {
            let content = format!("---\n{line}\nalwaysApply: false\n---\nUse the conventions.\n");
            assert_eq!(rule_cu_codes(&content), expected, "{label}");
        }
    }

    #[test]
    #[serial_test::serial]
    fn cu007_requires_always_state_and_effective_valid_glob() {
        let cases: Vec<(&str, Vec<&str>, &str)> = vec![
            (
                "---\nalwaysApply: true\nglobs: \"*.rs\"\n---\nBody.\n",
                vec!["CU007"],
                "always with a real pattern",
            ),
            (
                "---\nalwaysApply: true\nglobs: [\"*.rs\", \"[bad\"]\n---\nBody.\n",
                vec!["CU004", "CU007"],
                "an invalid sibling keeps the valid pattern ignored",
            ),
            (
                "---\nalwaysApply: true\nglobs:\n---\nBody.\n",
                vec![],
                "null globs are unset",
            ),
            (
                "---\nalwaysApply: true\nglobs: \"\"\n---\nBody.\n",
                vec![],
                "empty globs are unset",
            ),
            (
                "---\nalwaysApply: true\nglobs: []\n---\nBody.\n",
                vec![],
                "empty list is unset",
            ),
            (
                "---\nalwaysApply: true\nglobs: 42\n---\nBody.\n",
                vec!["CU004"],
                "no CU007 alongside a field-shape failure",
            ),
            (
                "---\nalwaysApply: true\nglobs: \"[bad\"\n---\nBody.\n",
                vec!["CU004"],
                "no CU007 without a structurally valid pattern",
            ),
            (
                "---\nalwaysApply: false\nglobs: \"*.rs\"\n---\nBody.\n",
                vec![],
                "auto-attached rules keep their globs",
            ),
            (
                "---\nalwaysApply: \"true\"\nglobs: \"*.rs\"\n---\nBody.\n",
                vec!["CU008"],
                "invalid alwaysApply recovers as false",
            ),
        ];
        for (content, expected, label) in cases {
            assert_eq!(rule_cu_codes(content), expected, "{label}");
        }
    }

    #[test]
    #[serial_test::serial]
    fn frontmatter_document_and_description_shapes_drive_cu003() {
        let cases: Vec<(&str, Vec<&str>, &str)> = vec![
            (
                "---\n- one\n- two\n---\nBody.\n",
                vec!["CU003"],
                "sequence document",
            ),
            (
                "---\njust text\n---\nBody.\n",
                vec!["CU003"],
                "scalar document",
            ),
            (
                "---\ndescription: One\ndescription: Two\n---\nBody.\n",
                vec!["CU003"],
                "duplicate keys",
            ),
            (
                "---\ndescription: 42\n---\nBody.\n",
                vec!["CU003"],
                "numeric description",
            ),
            (
                "---\ndescription: [a, b]\n---\nBody.\n",
                vec!["CU003"],
                "sequence description",
            ),
            (
                "---\ndescription: {a: b}\n---\nBody.\n",
                vec!["CU003"],
                "mapping description",
            ),
            (
                "---\ndescription: !custom text\n---\nBody.\n",
                vec!["CU003"],
                "tagged description",
            ),
            (
                "---\ndescription: true\nalwaysApply: false\n---\nBody.\n",
                vec!["CU003"],
                "boolean description",
            ),
            (
                "---\ndescription:\n---\nBody.\n",
                vec![],
                "null description",
            ),
            (
                "---\ndescription: \"\"\n---\nBody.\n",
                vec![],
                "empty description",
            ),
            (
                "---\ndescription: &d Documented behavior\nglobs: *d\n---\nBody.\n",
                vec![],
                "resolved aliases keep their scalar shape",
            ),
        ];
        for (content, expected, label) in cases {
            assert_eq!(rule_cu_codes(content), expected, "{label}");
        }
    }

    #[test]
    #[serial_test::serial]
    fn unquoted_glob_anchor_errors_carry_the_quote_suggestion() {
        for (globs, quoted) in [
            ("*.ts", "globs: \"*.ts\""),
            ("**/*.gen.ts,src/*.ts", "globs: \"**/*.gen.ts,src/*.ts\""),
        ] {
            let content =
                format!("---\ndescription:\nglobs: {globs}\nalwaysApply: false\n---\nBody.\n");
            let diagnostics = rule_cu_diagnostics(&content);
            assert_eq!(diagnostics.len(), 1, "{globs}: {diagnostics:?}");
            let diagnostic = &diagnostics[0];
            assert_eq!(diagnostic.rule, LintRule::CursorRuleFrontmatterInvalid);
            assert!(
                diagnostic.message.contains("unknown anchor"),
                "{globs}: {}",
                diagnostic.message
            );
            assert!(
                !diagnostic.message.contains("at line"),
                "parser coordinates must move to the structured location: {}",
                diagnostic.message
            );
            let suggestion = diagnostic.suggestion.as_deref().unwrap_or_default();
            assert!(
                suggestion.contains(quoted),
                "{globs}: suggestion must quote the pattern: {suggestion}"
            );
            // globs sits on file line 3: opener, description, globs.
            let location = diagnostic.location.expect("anchor errors carry a location");
            assert_eq!(location.start().line_number(), 3, "{globs}");
            assert!(location.start().column_number().is_some(), "{globs}");
        }

        // A trailing YAML comment stays outside the quoted pattern so the
        // suggestion never changes what Cursor reads.
        let content =
            "---\ndescription:\nglobs: *.ts # attach TS files\nalwaysApply: false\n---\nBody.\n";
        let diagnostics = rule_cu_diagnostics(content);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        let suggestion = diagnostics[0].suggestion.as_deref().unwrap_or_default();
        assert!(
            suggestion.contains("globs: \"*.ts\""),
            "comment must not enter the quoted pattern: {suggestion}"
        );
        assert!(
            !suggestion.contains('#'),
            "comment must stay outside the suggestion: {suggestion}"
        );

        // The same anchor class on a non-globs line keeps the generic message.
        let content = "---\ndescription: *routing\nalwaysApply: false\n---\nBody.\n";
        let diagnostics = rule_cu_diagnostics(content);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].rule, LintRule::CursorRuleFrontmatterInvalid);
        assert!(
            diagnostics[0].message.contains("unknown anchor"),
            "{}",
            diagnostics[0].message
        );
        assert_eq!(
            diagnostics[0].suggestion, None,
            "non-globs anchor errors keep the parser constraint as the action"
        );
    }

    #[test]
    #[serial_test::serial]
    fn rule_diagnostics_carry_locations_evidence_and_suggestions() {
        let content = "---\ndescription: 42\nglobs: \"[unclosed\"\nalwaysApply: \"yes\"\nunknown: value\n---\nBody.\n";
        let diagnostics = rule_cu_diagnostics(content);
        let find = |rule: LintRule| {
            diagnostics
                .iter()
                .find(|item| item.rule == rule)
                .unwrap_or_else(|| panic!("missing {rule:?}: {diagnostics:?}"))
        };

        let description = find(LintRule::CursorRuleFrontmatterInvalid);
        assert_eq!(description.location.unwrap().start().line_number(), 2);
        assert_eq!(description.evidence.as_deref(), Some("description"));
        assert!(description.suggestion.is_some());

        let glob = find(LintRule::CursorRuleGlobInvalid);
        assert_eq!(glob.location.unwrap().start().line_number(), 3);
        assert_eq!(glob.evidence.as_deref(), Some("[unclosed"));
        assert!(glob.suggestion.is_some());

        let always = find(LintRule::CursorAlwaysApplyInvalid);
        assert_eq!(always.location.unwrap().start().line_number(), 4);
        assert_eq!(always.evidence.as_deref(), Some("alwaysApply"));
        assert!(always.suggestion.is_some());

        let unknown = find(LintRule::CursorRuleFieldUnknown);
        assert_eq!(unknown.location.unwrap().start().line_number(), 5);
        assert_eq!(unknown.evidence.as_deref(), Some("unknown"));
        assert!(unknown.suggestion.is_some());

        let ignored = rule_cu_diagnostics("---\nalwaysApply: true\nglobs: \"*.rs\"\n---\nBody.\n");
        assert_eq!(ignored.len(), 1, "{ignored:?}");
        assert_eq!(ignored[0].rule, LintRule::CursorAlwaysApplyGlobs);
        assert_eq!(ignored[0].location.unwrap().start().line_number(), 3);
        assert_eq!(ignored[0].evidence.as_deref(), Some("globs"));
        assert!(ignored[0].suggestion.is_some());

        let missing = rule_cu_diagnostics("No frontmatter here.\n");
        assert_eq!(missing.len(), 1, "{missing:?}");
        assert_eq!(missing[0].rule, LintRule::CursorRuleFrontmatterMissing);
        assert_eq!(missing[0].location.unwrap().start().line_number(), 1);
        assert!(missing[0].suggestion.is_some());

        // Position-less parser failures (duplicate keys) anchor at the
        // opening delimiter instead of shipping without a location.
        let duplicate =
            rule_cu_diagnostics("---\ndescription: One\ndescription: Two\n---\nBody.\n");
        assert_eq!(duplicate.len(), 1, "{duplicate:?}");
        assert_eq!(duplicate[0].rule, LintRule::CursorRuleFrontmatterInvalid);
        assert_eq!(duplicate[0].location.unwrap().start().line_number(), 1);
    }

    #[test]
    #[serial_test::serial]
    fn rule_diagnostics_never_echo_secretlike_neighbor_values() {
        let secret = "ghp_0123456789abcdefghij0123456789abcdef";
        let with_unknown_key =
            format!("---\ndescription: Documented behavior\ntoken: {secret}\n---\nBody.\n");
        let anchored_secret =
            format!("---\ndescription:\nglobs: *{secret}\nalwaysApply: false\n---\nBody.\n");
        let malformed_secret_pattern =
            format!("---\nglobs: \"[{secret}\"\nalwaysApply: false\n---\nBody.\n");
        for content in [with_unknown_key, anchored_secret, malformed_secret_pattern] {
            let diagnostics = rule_cu_diagnostics(&content);
            assert!(!diagnostics.is_empty(), "{content}");
            for diagnostic in &diagnostics {
                for text in [
                    Some(diagnostic.message.as_str()),
                    diagnostic.evidence.as_deref(),
                    diagnostic.suggestion.as_deref(),
                ]
                .into_iter()
                .flatten()
                {
                    assert!(
                        !text.contains(secret),
                        "a diagnostic echoed the secret: {text}"
                    );
                }
            }
        }
    }

    #[test]
    #[serial_test::serial]
    fn nested_mdc_rules_are_live_and_md_rules_only_report_extension() {
        let tmp = tempfile::tempdir().unwrap();
        let mdc = tmp.path().join("packages/api/.cursor/rules/api.mdc");
        let md = tmp.path().join("packages/web/.cursor/rules/not-a-rule.md");
        std::fs::create_dir_all(mdc.parent().unwrap()).unwrap();
        std::fs::create_dir_all(md.parent().unwrap()).unwrap();
        std::fs::write(&mdc, "---\nalwaysApply: true\n---\nRetry until success.\n").unwrap();
        std::fs::write(&md, "Retry until success.\n").unwrap();

        let _guard = CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate(&mut diag, &ExcludeSet::default());

        let identities: Vec<_> = diag
            .diagnostics()
            .iter()
            .map(|item| {
                (
                    item.rule.code(),
                    item.subject_path.as_ref().unwrap().display().to_string(),
                )
            })
            .collect();
        assert_eq!(
            identities,
            vec![
                ("Q005", "packages/api/.cursor/rules/api.mdc".to_string()),
                (
                    "CU020",
                    "packages/web/.cursor/rules/not-a-rule.md".to_string()
                ),
            ]
        );
    }

    #[test]
    #[serial_test::serial]
    fn documented_hook_events_are_accepted() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".cursor")).unwrap();
        let documented_events = [
            "sessionStart",
            "sessionEnd",
            "preToolUse",
            "postToolUse",
            "postToolUseFailure",
            "subagentStart",
            "subagentStop",
            "beforeShellExecution",
            "afterShellExecution",
            "beforeMCPExecution",
            "afterMCPExecution",
            "beforeReadFile",
            "afterFileEdit",
            "beforeSubmitPrompt",
            "preCompact",
            "stop",
            "afterAgentResponse",
            "afterAgentThought",
            "beforeTabFileRead",
            "afterTabFileEdit",
            "workspaceOpen",
        ];
        assert_eq!(HOOK_EVENTS, documented_events);

        let mut hooks = serde_json::Map::new();
        for event in documented_events {
            hooks.insert(event.to_string(), json!([{"command": "echo hook"}]));
        }
        hooks.insert(
            "beforeShellExecution".to_string(),
            json!([{
                "type": "prompt",
                "prompt": "Allow read-only commands?",
                "timeout": 10,
                "futureCursorField": {"accepted": true}
            }]),
        );
        std::fs::write(
            tmp.path().join(".cursor/hooks.json"),
            serde_json::to_string(&json!({"version": 1.0, "hooks": hooks})).unwrap(),
        )
        .unwrap();
        assert!(codes_for(tmp.path()).is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn hooks_validate_schema_and_entry_shapes() {
        let cases = [
            ("null", vec!["CU010"]),
            ("{", vec!["CU010"]),
            (r#"{"hooks": {}}"#, vec!["CU010"]),
            (r#"{"version": 1}"#, vec!["CU010"]),
            (r#"{"version": 2, "hooks": {}}"#, vec!["CU010"]),
            (r#"{"version": "1", "hooks": {}}"#, vec!["CU010"]),
            (r#"{"version": 1.5, "hooks": {}}"#, vec!["CU010"]),
            (r#"{"version": 1, "hooks": {"stop": {}}}"#, vec!["CU010"]),
            (
                r#"{"version": 1, "hooks": {"stop": [null]}}"#,
                vec!["CU010"],
            ),
            (r#"{"version": 1, "hooks": {}}"#, vec![]),
            (r#"{"version": 1.0, "hooks": {}}"#, vec![]),
        ];
        for (content, expected) in cases {
            let tmp = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(tmp.path().join(".cursor")).unwrap();
            std::fs::write(tmp.path().join(".cursor/hooks.json"), content).unwrap();
            assert_eq!(codes_for(tmp.path()), expected, "content: {content}");
        }
    }

    #[test]
    #[serial_test::serial]
    fn hooks_apply_command_prompt_and_field_contracts() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".cursor")).unwrap();
        std::fs::write(
            tmp.path().join(".cursor/hooks.json"),
            r#"{
                "version": 1,
                "hooks": {
                    "unknown": [{"command": "echo hook"}],
                    "beforeShellExecution": [
                        {"type": "prompt"},
                        {"type": "prompt", "prompt": null},
                        {"type": "prompt", "prompt": 7},
                        {"type": "prompt", "prompt": "   "},
                        {"type": "prompt", "prompt": "Continue?", "model": null},
                        {"type": "prompt", "prompt": "Continue?", "model": 7},
                        {"type": "prompt", "prompt": "Continue?", "model": "  "},
                        {"command": ""},
                        {"type": "command", "command": 7},
                        {"type": "agent", "command": "echo ignored"},
                        {"type": 7},
                        {"command": "echo valid", "timeout": 1.5, "loop_limit": null, "failClosed": true, "matcher": "Bash"},
                        {"command": "echo invalid", "timeout": "fast", "loop_limit": false, "failClosed": "yes", "matcher": 7}
                    ]
                }
            }"#,
        )
        .unwrap();
        assert_eq!(
            codes_for(tmp.path()),
            vec![
                "CU018", "CU018", "CU018", "CU018", "CU019", "CU019", "CU019", "CU012", "CU012",
                "CU013", "CU013", "CU017", "CU017", "CU017", "CU017", "CU011",
            ]
        );
    }

    #[test]
    #[serial_test::serial]
    fn agents_environment_and_skills_are_validated() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".cursor/agents/nested")).unwrap();
        std::fs::create_dir_all(tmp.path().join(".cursor/skills/example")).unwrap();
        std::fs::write(
            tmp.path().join(".cursor/agents/nested/reviewer.md"),
            "---\nname: 1\ndescription: reviewer\nreadonly: \"true\"\n---\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join(".cursor/environment.json"),
            r#"{"install":1,"build":{"dockerfile":1},"terminals":[{"name":"app"}]}"#,
        )
        .unwrap();
        std::fs::write(tmp.path().join(".cursor/skills/example/SKILL.md"), "---\nname: example\ndescription: Test\nmodel: opus\ncontext: fork\nhooks: {}\n---\nBody\n").unwrap();
        let codes = codes_for(tmp.path());
        for expected in ["CU014", "CU015", "CU016", "CR-SK-001"] {
            assert!(codes.contains(&expected), "missing {expected}: {codes:?}");
        }
        assert_eq!(codes.iter().filter(|code| **code == "CR-SK-001").count(), 3);
    }

    #[test]
    #[serial_test::serial]
    fn cursor_agent_omission_and_full_frontmatter_are_clean() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".cursor/agents/review")).unwrap();
        std::fs::write(
            tmp.path().join(".cursor/agents/reviewer.md"),
            "---\nreadonly: false\nis_background: true\n---\n\nReview the requested change and return concise evidence.\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join(".cursor/agents/review/security-auditor.md"),
            "---\nname: security-auditor\ndescription: Security specialist for auth and payments.\nmodel: inherit\nreadonly: true\nis_background: false\n---\nAudit the change.\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join(".cursor/agents/review/planner.md"),
            "---\nname: planner\ndescription: Plans complex changes before implementation.\nmodel: claude-opus-4-8[effort=high]\n---\nPlan the change.\n",
        )
        .unwrap();
        let codes = codes_for(tmp.path());
        assert!(
            !codes
                .iter()
                .any(|code| *code == "CU014" || *code == "CU015"),
            "unexpected agent diagnostics: {codes:?}"
        );
    }

    #[test]
    #[serial_test::serial]
    fn cursor_agent_field_and_delimiter_contract() {
        let cases = [
            (
                "bad.md",
                "name: reviewer\ndescription: ok\n",
                true,
                false,
                "unclosed frontmatter",
            ),
            (
                "seq.md",
                "---\n- just a list\n---\nBody\n",
                true,
                false,
                "non-object YAML",
            ),
            (
                "parse.md",
                "---\nname: [unclosed\n---\nBody\n",
                true,
                false,
                "parse error",
            ),
            (
                "name-type.md",
                "---\nname: [not, a, string]\n---\nBody\n",
                true,
                false,
                "non-string name",
            ),
            (
                "empty-name.md",
                "---\nname: \"\"\ndescription: present description text\n---\nBody\n",
                true,
                false,
                "empty name",
            ),
            (
                "ws-name.md",
                "---\nname: \"   \"\ndescription: present description text\n---\nBody\n",
                true,
                false,
                "whitespace name",
            ),
            (
                "bad-name.md",
                "---\nname: Reviewer\ndescription: present description text\n---\nBody\n",
                true,
                false,
                "uppercase name",
            ),
            (
                "digit-name.md",
                "---\nname: reviewer2\ndescription: present description text\n---\nBody\n",
                true,
                false,
                "digit in name",
            ),
            (
                "empty-desc.md",
                "---\nname: reviewer\ndescription: \"\"\n---\nBody\n",
                true,
                false,
                "empty description",
            ),
            (
                "ws-desc.md",
                "---\nname: reviewer\ndescription: \"  \"\n---\nBody\n",
                true,
                false,
                "whitespace description",
            ),
            (
                "desc-type.md",
                "---\nname: reviewer\ndescription: [not, a, string]\n---\nBody\n",
                true,
                false,
                "non-string description",
            ),
            (
                "empty-model.md",
                "---\nname: reviewer\nmodel: \"\"\n---\nBody\n",
                true,
                false,
                "empty model",
            ),
            (
                "ws-model.md",
                "---\nname: reviewer\nmodel: \"   \"\n---\nBody\n",
                true,
                false,
                "whitespace model",
            ),
            (
                "model-type.md",
                "---\nname: reviewer\nmodel: 1\n---\nBody\n",
                true,
                false,
                "non-string model",
            ),
            (
                "readonly-type.md",
                "---\nname: reviewer\nreadonly: \"true\"\n---\nBody\n",
                true,
                false,
                "non-bool readonly",
            ),
            (
                "bg-type.md",
                "---\nname: reviewer\nis_background: \"yes\"\n---\nBody\n",
                true,
                false,
                "non-bool is_background",
            ),
            (
                "empty-body.md",
                "---\nname: reviewer\n---\n   \n",
                false,
                true,
                "empty body only",
            ),
            (
                "field-and-body.md",
                "---\nname: Reviewer\n---\n",
                true,
                true,
                "field error keeps CU015",
            ),
        ];
        for (file, content, expect_cu014, expect_cu015, label) in cases {
            let tmp = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(tmp.path().join(".cursor/agents")).unwrap();
            std::fs::write(tmp.path().join(".cursor/agents").join(file), content).unwrap();
            let codes = codes_for(tmp.path());
            assert_eq!(
                codes.contains(&"CU014"),
                expect_cu014,
                "{label}: CU014 mismatch in {codes:?}"
            );
            assert_eq!(
                codes.contains(&"CU015"),
                expect_cu015,
                "{label}: CU015 mismatch in {codes:?}"
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn cursor_agent_derived_name_boundaries() {
        let cases = [
            ("reviewer.md", false),
            ("code-reviewer.md", false),
            ("a.md", false),
            ("Reviewer.md", true),
            ("reviewer2.md", true),
            ("review_er.md", true),
            ("-reviewer.md", true),
            ("reviewer-.md", true),
            ("code--reviewer.md", true),
        ];
        for (file, expect_cu014) in cases {
            let tmp = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(tmp.path().join(".cursor/agents")).unwrap();
            std::fs::write(
                tmp.path().join(".cursor/agents").join(file),
                "---\nreadonly: false\n---\nBody\n",
            )
            .unwrap();
            let codes = codes_for(tmp.path());
            assert_eq!(
                codes.contains(&"CU014"),
                expect_cu014,
                "{file}: CU014 mismatch in {codes:?}"
            );
            assert!(!codes.contains(&"CU015"), "{file}: unexpected CU015");
        }
    }

    #[test]
    fn yaml_parse_constraint_masks_sensitive_evidence() {
        assert_eq!(
            yaml_parse_constraint("expected ',' or ']' in flow sequence"),
            "expected ',' or ']' in flow sequence"
        );
        assert_eq!(
            yaml_parse_constraint("token: 'this-is-a-sensitive-value'"),
            "invalid syntax"
        );
    }

    #[test]
    #[serial_test::serial]
    fn cursor_skill_allowlist_uses_the_documented_fields_and_rejects_user_invocable() {
        let tmp = tempfile::tempdir().unwrap();
        let skill = tmp.path().join(".cursor/skills/reviewer/SKILL.md");
        std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
        std::fs::write(
            skill,
            "---\nname: reviewer\ndescription: Reviews changes when verification is requested.\npaths:\n  - src/**\ndisable-model-invocation: true\nmetadata:\n  owner: platform\nuser-invocable: false\ncontext: fork\nagent: Explore\nhooks: {}\nfuture-field: true\n---\nBody\n",
        )
        .unwrap();

        let codes = codes_for(tmp.path());
        assert_eq!(
            codes.iter().filter(|code| **code == "CR-SK-001").count(),
            5,
            "only user-invocable, context, agent, hooks, and future-field are unsupported"
        );
    }

    #[test]
    #[serial_test::serial]
    fn cursor_agent_yaml_parse_error_keeps_nonsensitive_constraint() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".cursor/agents")).unwrap();
        std::fs::write(
            tmp.path().join(".cursor/agents/parse.md"),
            "---\nname: [unclosed\n---\nBody\n",
        )
        .unwrap();
        let _guard = CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate(&mut diag, &ExcludeSet::default());
        let messages: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::CursorAgentFrontmatterInvalid)
            .map(|item| item.message.as_str())
            .collect();
        assert_eq!(messages.len(), 1, "{messages:?}");
        assert!(
            messages[0].contains("not valid YAML:"),
            "missing parser constraint: {}",
            messages[0]
        );
        assert!(
            !messages[0].ends_with("not valid YAML: invalid syntax"),
            "nonsensitive parse errors must keep the real constraint: {}",
            messages[0]
        );
    }

    #[test]
    fn cursor_agent_identifier_format_matches_documented_contract() {
        assert!(is_cursor_agent_identifier("reviewer"));
        assert!(is_cursor_agent_identifier("code-reviewer"));
        assert!(!is_cursor_agent_identifier(""));
        assert!(!is_cursor_agent_identifier("Reviewer"));
        assert!(!is_cursor_agent_identifier("reviewer2"));
        assert!(!is_cursor_agent_identifier("review_er"));
        assert!(!is_cursor_agent_identifier("-reviewer"));
        assert!(!is_cursor_agent_identifier("reviewer-"));
        assert!(!is_cursor_agent_identifier("code--reviewer"));
    }

    #[test]
    #[serial_test::serial]
    fn cursor_environment_schema_is_checked_in_and_compiles() {
        let schema: JsonValue =
            serde_json::from_str(include_str!("../../schemas/cursor-environment.schema.json"))
                .expect("checked-in Cursor environment schema parses");
        assert!(schema.get("allOf").is_some());
        assert!(jsonschema::validator_for(&schema).is_ok());

        let provenance = include_str!("../../schemas/cursor-environment.schema.md");
        assert!(provenance.contains("https://www.cursor.com/schemas/environment.schema.json"));
        assert!(provenance.contains("Retrieved: 2026-07-22"));
        assert!(
            provenance.contains("62b13994164f4186198b1f002ff957605df37ba5eee803e6afe69c981af001d6")
        );
        assert!(provenance.contains("curl --fail"));
    }

    #[test]
    #[serial_test::serial]
    fn cursor_environment_schema_accepts_every_current_shape() {
        let valid = [
            ("empty", r#"{}"#),
            (
                "snapshot only",
                r#"{"snapshot":"snapshot-20260212-00000000"}"#,
            ),
            ("install only", r#"{"install":"npm ci"}"#),
            (
                "dockerfile without context",
                r#"{"build":{"dockerfile":"Dockerfile"}}"#,
            ),
            (
                "terminal object",
                r#"{"terminals":[{"command":"npm start"}]}"#,
            ),
            (
                "terminal array",
                r#"{"terminals":[[{"command":"npm start","name":"web","description":"serve"}]]}"#,
            ),
            ("lowest port", r#"{"ports":[{"port":1}]}"#),
            ("highest port", r#"{"ports":[{"port":65535,"name":"web"}]}"#),
            (
                "all common and container properties",
                r#"{"name":"development","user":"agent","install":"npm ci","start":"npm start","repositoryDependencies":["github.com/acme/api"],"ports":[{"port":3000,"name":"web"}],"terminals":[{"command":"npm start","name":"web","description":"serve"}],"build":{"dockerfile":"Dockerfile","context":"."},"snapshot":"snapshot-20260212-00000000","agentCanUpdateSnapshot":true}"#,
            ),
        ];

        for (name, content) in valid {
            let messages = environment_messages_for(content);
            assert!(messages.is_empty(), "{name}: {messages:?}");
        }
    }

    #[test]
    #[serial_test::serial]
    fn cursor_environment_schema_rejects_every_current_constraint() {
        let invalid = [
            (
                "invalid JSON",
                r#"{"install":"unterminated"#,
                "invalid JSON:",
            ),
            ("root type", r#"[]"#, "top level:"),
            (
                "obsolete update",
                r#"{"update":"npm update"}"#,
                "top level:",
            ),
            ("unknown top level", r#"{"futureField":true}"#, "top level:"),
            ("name type", r#"{"name":false}"#, "name:"),
            ("user type", r#"{"user":false}"#, "user:"),
            ("install type", r#"{"install":false}"#, "install:"),
            ("start type", r#"{"start":false}"#, "start:"),
            (
                "repository dependency type",
                r#"{"repositoryDependencies":[false]}"#,
                "repositoryDependencies[1]:",
            ),
            (
                "repository dependencies type",
                r#"{"repositoryDependencies":false}"#,
                "repositoryDependencies:",
            ),
            ("ports type", r#"{"ports":false}"#, "ports:"),
            ("port entry type", r#"{"ports":[false]}"#, "ports[1]:"),
            ("port required", r#"{"ports":[{}]}"#, "ports[1].port:"),
            (
                "port name type",
                r#"{"ports":[{"port":1,"name":false}]}"#,
                "ports[1].name:",
            ),
            ("port zero", r#"{"ports":[{"port":0}]}"#, "ports[1].port:"),
            (
                "port too high",
                r#"{"ports":[{"port":65536}]}"#,
                "ports[1].port:",
            ),
            ("terminal type", r#"{"terminals":false}"#, "terminals:"),
            // Union failures surface the narrowest leaf property, not the
            // oneOf parent.
            (
                "terminal command required",
                r#"{"terminals":[{}]}"#,
                "terminals[1].command:",
            ),
            (
                "terminal command type",
                r#"{"terminals":[{"command":false}]}"#,
                "terminals[1].command:",
            ),
            (
                "terminal name type",
                r#"{"terminals":[{"command":"run","name":false}]}"#,
                "terminals[1].name:",
            ),
            (
                "terminal description type",
                r#"{"terminals":[{"command":"run","description":false}]}"#,
                "terminals[1].description:",
            ),
            (
                "nested terminal command required",
                r#"{"terminals":[[{}]]}"#,
                "terminals[1][1].command:",
            ),
            (
                "nested terminal field type",
                r#"{"terminals":[[{"command":"run","name":false,"description":false}]]}"#,
                "terminals[1][1].name:",
            ),
            ("build type", r#"{"build":false}"#, "build:"),
            (
                "dockerfile required",
                r#"{"build":{}}"#,
                "build.dockerfile:",
            ),
            (
                "dockerfile type",
                r#"{"build":{"dockerfile":false}}"#,
                "build.dockerfile:",
            ),
            (
                "context type",
                r#"{"build":{"dockerfile":"Dockerfile","context":false}}"#,
                "build.context:",
            ),
            (
                "unknown build property",
                r#"{"build":{"dockerfile":"Dockerfile","image":"base"}}"#,
                "build:",
            ),
            (
                "agent can update snapshot type",
                r#"{"agentCanUpdateSnapshot":"yes"}"#,
                "agentCanUpdateSnapshot:",
            ),
            ("snapshot type", r#"{"snapshot":false}"#, "snapshot:"),
        ];

        for (name, content, path) in invalid {
            let messages = environment_messages_for(content);
            assert!(!messages.is_empty(), "{name} produced no CU016 diagnostic");
            assert!(
                messages.iter().any(|message| message.contains(path)),
                "{name} did not report {path}: {messages:?}"
            );
            assert!(
                messages
                    .iter()
                    .all(|message| message.starts_with(".cursor/environment.json: ")),
                "{name} has an unstable subject: {messages:?}"
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn cursor_environment_unknown_nested_key_reports_once_without_cascade() {
        // The unknown key inside `build` is owned by the build-level finding;
        // the root-level unevaluated cascade naming `build` is dropped even
        // though the owning finding is itself an unevaluated finding.
        let messages = environment_messages_for(r#"{"build":{"dockerfile":"D","extra":1}}"#);
        assert_eq!(messages.len(), 1, "{messages:?}");
        assert!(
            messages[0].starts_with(".cursor/environment.json: build: "),
            "{messages:?}"
        );
    }

    #[test]
    #[serial_test::serial]
    fn cursor_environment_schema_never_echoes_invalid_instance_values() {
        let invalid = [
            (r#"{"install":"unclosed-secret"#, "unclosed-secret"),
            (r#"{"install":["type-secret"]}"#, "type-secret"),
            (
                r#"{"build":{"context":"required-secret"}}"#,
                "required-secret",
            ),
            (
                r#"{"ports":[{"port":0,"name":"range-secret"}]}"#,
                "range-secret",
            ),
            (
                r#"{"terminals":[{"command":["one-of-secret"]}]}"#,
                "one-of-secret",
            ),
            (r#"{"build":"object-secret"}"#, "object-secret"),
            (r#"{"unknown":"unevaluated-secret"}"#, "unevaluated-secret"),
        ];

        for (content, secret) in invalid {
            let messages = environment_messages_for(content);
            assert!(
                !messages.is_empty(),
                "{secret} produced no CU016 diagnostic"
            );
            assert!(
                messages.iter().all(|message| !message.contains(secret)),
                "a diagnostic exposed {secret}: {messages:?}"
            );
        }
    }
}
