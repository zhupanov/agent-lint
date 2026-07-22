//! Validators for optional Claude Code configuration surfaces.
//!
//! These surfaces are intentionally optional: their validators are silent when
//! the corresponding directory or settings file is absent.

use crate::config::ExcludeSet;
use crate::context::{LintContext, ManifestState, ParsedManifest};
use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata, SourceSpan};
use crate::frontmatter::{self, LeadingFrontmatterState};
use crate::rules::LintRule;
use crate::traversal;
use crate::yaml::{Mapping, Value as YamlValue};
use serde_json::Value as JsonValue;
use std::collections::HashSet;
use std::fs;
use std::ops::Range;
use std::path::Path;
use std::sync::LazyLock;

const RULES_FIELDS: &[&str] = &["paths"];
const R001_SUGGESTION: &str = "use a valid Claude gitignore-style paths pattern";
const R002_SUGGESTION: &str = "remove the unsupported key";
const R003_SUGGESTION: &str = "repair or remove the attempted frontmatter";
const OUTPUT_STYLE_FIELDS: &[&str] = &["name", "description", "keep-coding-instructions"];
const OUTPUT_STYLE_FORCE_FOR_PLUGIN: &str = "force-for-plugin";
const OUTPUT_STYLE_DESCRIPTION_SUGGESTION: &str =
    "add a non-empty string description frontmatter field";
const OUTPUT_STYLE_INSTRUCTIONS_SUGGESTION: &str =
    "set keep-coding-instructions to true, false, \"true\", or \"false\"";
const OUTPUT_STYLE_FIELD_SUGGESTION: &str =
    "remove the unsupported field or relocate it to a plugin-bundled output style";
const OUTPUT_STYLE_BODY_SUGGESTION: &str = "add output-style instructions to the body";
const OUTPUT_STYLE_FRONTMATTER_SUGGESTION: &str =
    "repair the attempted frontmatter or remove it to use a body-only style";
const PR_URL_TEMPLATE_PLACEHOLDERS: &[&str] = &["{host}", "{owner}", "{repo}", "{number}", "{url}"];
const CHANNELS_ENABLED_SUGGESTION: &str =
    "remove channelsEnabled here and configure it through managed organization policy";

static RE_BRACE_TOKEN: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"\{[A-Za-z][A-Za-z0-9_-]*\}").expect("valid placeholder-token regex")
});

/// Validate every optional private Claude configuration surface.
pub fn validate_private_config(
    ctx: &LintContext,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
) {
    validate_rules(diag, exclude);
    validate_output_styles(diag, exclude);
    validate_typed_settings(ctx, diag);
}

fn validate_rules(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    validate_markdown_directory(".claude/rules", diag, exclude, |path, content, diag| {
        validate_rule_file(diag, path, content);
    });
}

fn validate_rule_file(diag: &mut DiagnosticCollector, path: &str, content: &str) {
    let (frontmatter, yaml_range) = match frontmatter::leading_frontmatter(content) {
        LeadingFrontmatterState::Absent { .. } => return,
        LeadingFrontmatterState::Unterminated { delimiter_range } => {
            report_rules_frontmatter_invalid(
                diag,
                path,
                content,
                delimiter_range,
                "frontmatter: missing closer",
            );
            return;
        }
        LeadingFrontmatterState::Complete(block) => {
            let yaml = match crate::yaml::parse(block.yaml) {
                Ok(yaml) if yaml.is_null() => Mapping::new(),
                Ok(yaml) => match yaml.as_mapping() {
                    Some(mapping) => mapping.clone(),
                    None => {
                        report_rules_frontmatter_invalid(
                            diag,
                            path,
                            content,
                            block.yaml_range,
                            "frontmatter: non-mapping",
                        );
                        return;
                    }
                },
                Err(error) => {
                    let range = crate::yaml::error_line(&error)
                        .and_then(|line| yaml_line_range(content, block.yaml_range.clone(), line))
                        .unwrap_or(block.yaml_range);
                    report_rules_frontmatter_invalid(
                        diag,
                        path,
                        content,
                        range,
                        "frontmatter: invalid yaml",
                    );
                    return;
                }
            };
            (yaml, Some(block.yaml_range))
        }
    };

    report_rules_unknown_fields(diag, path, content, yaml_range.clone(), &frontmatter);
    validate_rule_paths(diag, path, content, yaml_range, &frontmatter);
}

/// Which output-style loading surface a file belongs to. The per-file O rules
/// are identical across surfaces with one documented asymmetry: `force-for-plugin`
/// is honored only for plugin-bundled styles, so it is a recognized field under
/// [`OutputStyleSurface::Plugin`] and an ignored-placement O003 under
/// [`OutputStyleSurface::Private`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutputStyleSurface {
    /// Private `.claude/output-styles/` styles (Basic and Plugin mode).
    Private,
    /// Plugin-bundled styles: the plugin-root `output-styles/` directory and
    /// manifest-declared `outputStyles` paths (Plugin mode only).
    Plugin,
}

fn validate_output_styles(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    validate_markdown_directory(
        ".claude/output-styles",
        diag,
        exclude,
        |path, content, diag| {
            validate_output_style(diag, path, content, OutputStyleSurface::Private);
        },
    );
}

/// O001-O006 on plugin-shipped output styles (Plugin mode only).
///
/// Discovers the union of the plugin-root `output-styles/` directory and every
/// manifest-declared `outputStyles` path (`declared_roots`, already
/// repository-safe, normalized, and deduplicated), then runs the same per-file
/// checks as `.claude/output-styles/` through [`validate_output_style`] with the
/// [`OutputStyleSurface::Plugin`] asymmetry. Discovery reuses the shared
/// recursive-file collector, so it is symlink-safe, honors `[lint].exclude` and
/// the pruned-directory set, and deduplicates across roots — a file reachable
/// from both the default directory and a declared path is linted once. A declared
/// path may name a directory or a single `.md` file; declared paths that do not
/// exist are silently absent, since their existence and safety remain M-rule
/// (manifest) territory. `.claude/output-styles/` is not scanned here: it is a
/// distinct surface covered by [`validate_output_styles`] in both modes.
pub(crate) fn validate_plugin_output_styles(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    declared_roots: &[String],
) {
    // The implicit default plugin-root `output-styles/` first, then each declared
    // root; the shared collector deduplicates overlapping roots and files.
    let mut roots: Vec<&str> = vec!["output-styles"];
    roots.extend(declared_roots.iter().map(String::as_str));
    for path in &super::agent_discovery::collect(&roots, exclude).lint_files {
        // The `.claude-plugin/` tree is manifest-owned (M012), so its contents are
        // never linted as component content. Declared paths pointing *at*
        // `.claude-plugin/` are already rejected by the M012/M013 classifier; this
        // also drops a nested `.claude-plugin/` reached incidentally while walking
        // a declared directory root.
        if path.split('/').any(|segment| segment == ".claude-plugin") {
            continue;
        }
        let Ok(content) = fs::read_to_string(path) else {
            continue;
        };
        diag.with_subject_path(path, |diag| {
            validate_output_style(diag, path, &content, OutputStyleSurface::Plugin);
        });
    }
}

fn validate_output_style(
    diag: &mut DiagnosticCollector,
    path: &str,
    content: &str,
    surface: OutputStyleSurface,
) {
    let (frontmatter, body, yaml_range) = match frontmatter::leading_frontmatter(content) {
        LeadingFrontmatterState::Absent { body } => (Mapping::new(), body, None),
        LeadingFrontmatterState::Unterminated { delimiter_range } => {
            report_output_style_frontmatter_invalid(
                diag,
                path,
                content,
                delimiter_range,
                "frontmatter opening delimiter has no matching closer",
            );
            return;
        }
        LeadingFrontmatterState::Complete(block) => {
            let yaml = match crate::yaml::parse(block.yaml) {
                Ok(yaml) if yaml.is_null() => Mapping::new(),
                Ok(yaml) => match yaml.as_mapping() {
                    Some(mapping) => mapping.clone(),
                    None => {
                        report_output_style_frontmatter_invalid(
                            diag,
                            path,
                            content,
                            block.yaml_range,
                            "frontmatter must be a YAML mapping",
                        );
                        return;
                    }
                },
                Err(error) => {
                    let range = crate::yaml::error_line(&error)
                        .and_then(|line| yaml_line_range(content, block.yaml_range.clone(), line))
                        .unwrap_or(block.yaml_range);
                    report_output_style_frontmatter_invalid(
                        diag,
                        path,
                        content,
                        range,
                        "frontmatter is not valid YAML",
                    );
                    return;
                }
            };
            (yaml, block.body, Some(block.yaml_range))
        }
    };

    report_output_style_unknown_fields(
        diag,
        path,
        content,
        yaml_range.clone(),
        &frontmatter,
        surface,
    );
    validate_output_style_fields(diag, path, content, yaml_range, &frontmatter);
    if body.trim().is_empty() {
        let body_range = body_range(content, body).unwrap_or(0..content.len());
        diag.report_with(
            LintRule::OutputStyleBodyEmpty,
            &format!("{path}: output-style body is empty or whitespace-only"),
            metadata_for_range(
                content,
                body_range,
                "body: whitespace-only",
                OUTPUT_STYLE_BODY_SUGGESTION,
            ),
        );
    }
}

fn validate_markdown_directory<F>(
    directory: &str,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    mut validate: F,
) where
    F: FnMut(&str, &str, &mut DiagnosticCollector),
{
    let dir = Path::new(directory);
    if !dir.is_dir() {
        return;
    }

    for entry in traversal::recursive_files(dir, Path::new("."), Some(exclude)).entries {
        let path = entry.path;
        if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }
        let display = entry.display;
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        diag.with_subject_path(&display, |diag| {
            validate(&display, &content, diag);
        });
    }
}

fn report_rules_frontmatter_invalid(
    diag: &mut DiagnosticCollector,
    path: &str,
    content: &str,
    range: Range<usize>,
    evidence: &str,
) {
    diag.report_at_with(
        LintRule::RulesFrontmatterInvalid,
        path,
        &format!("{path}: rule frontmatter is missing or invalid"),
        metadata_for_range(content, range, evidence, R003_SUGGESTION),
    );
}

fn report_rules_unknown_fields(
    diag: &mut DiagnosticCollector,
    path: &str,
    content: &str,
    yaml_range: Option<Range<usize>>,
    frontmatter: &Mapping,
) {
    for key in frontmatter.keys() {
        if RULES_FIELDS.contains(&key.as_str()) {
            continue;
        }
        diag.report_at_with(
            LintRule::RulesFieldUnknown,
            path,
            &format!("{path}: unknown frontmatter field"),
            metadata_for_field(
                content,
                yaml_range.clone(),
                key,
                &format!("unknown field: {key}"),
                R002_SUGGESTION,
            ),
        );
    }
}

fn validate_rule_paths(
    diag: &mut DiagnosticCollector,
    path: &str,
    content: &str,
    yaml_range: Option<Range<usize>>,
    frontmatter: &Mapping,
) {
    let Some(value) = frontmatter.get("paths") else {
        return;
    };

    let field_metadata = metadata_for_field(
        content,
        yaml_range.clone(),
        "paths",
        "paths: empty",
        R001_SUGGESTION,
    );

    let top_level_ok = matches!(value, YamlValue::String(_) | YamlValue::Sequence(_));
    let has_non_string_leaf = paths_has_non_string_leaf(value);
    let source_strings = collect_paths_source_strings(value);
    let mut effective_by_source: Vec<(&str, Vec<String>)> = Vec::new();
    let mut all_effective = Vec::new();
    for source in &source_strings {
        let expanded = normalize_claude_paths_string(source);
        all_effective.extend(expanded.iter().cloned());
        effective_by_source.push((source, expanded));
    }

    let empty_effective = all_effective.is_empty();
    if !top_level_ok || has_non_string_leaf || empty_effective {
        let evidence = if !top_level_ok {
            "paths: wrong type"
        } else if has_non_string_leaf {
            "paths: non-string leaf"
        } else {
            "paths: empty"
        };
        let metadata = metadata_for_field(
            content,
            yaml_range.clone(),
            "paths",
            evidence,
            R001_SUGGESTION,
        );
        diag.report_at_with(
            LintRule::RulesGlobInvalid,
            path,
            &format!("{path}: paths metadata is not a usable Claude paths value"),
            metadata,
        );
    }

    let mut seen_invalid = HashSet::new();
    for (source, patterns) in effective_by_source {
        for pattern in patterns {
            if node_ignore_pattern_is_valid(&pattern) || !seen_invalid.insert(pattern.clone()) {
                continue;
            }
            let metadata = locate_paths_scalar_metadata(content, yaml_range.clone(), source)
                .unwrap_or_else(|| field_metadata.clone());
            let metadata = metadata
                .with_evidence(pattern_evidence_fragment(&pattern))
                .with_suggestion(R001_SUGGESTION);
            diag.report_at_with(
                LintRule::RulesGlobInvalid,
                path,
                &format!("{path}: paths entry is not a valid Claude gitignore-style paths pattern"),
                metadata,
            );
        }
    }
}

fn paths_has_non_string_leaf(value: &YamlValue) -> bool {
    match value {
        YamlValue::String(_) => false,
        YamlValue::Sequence(items) => items.iter().any(paths_has_non_string_leaf),
        _ => true,
    }
}

fn collect_paths_source_strings(value: &YamlValue) -> Vec<&str> {
    match value {
        YamlValue::String(text) => vec![text.as_str()],
        YamlValue::Sequence(items) => items
            .iter()
            .flat_map(collect_paths_source_strings)
            .collect(),
        _ => Vec::new(),
    }
}

/// Claude's `Z2r`/`UHc` normalization plus terminal `/**` stripping.
fn normalize_claude_paths_string(input: &str) -> Vec<String> {
    split_top_level_commas(input)
        .into_iter()
        .flat_map(|part| expand_braces(&part))
        .map(|entry| {
            if let Some(stripped) = entry.trim().strip_suffix("/**") {
                stripped.to_string()
            } else {
                entry.trim().to_string()
            }
        })
        .filter(|entry| !entry.is_empty())
        .collect()
}

fn split_top_level_commas(input: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0isize;
    for ch in input.chars() {
        match ch {
            '{' => {
                depth += 1;
                current.push(ch);
            }
            '}' => {
                depth -= 1;
                current.push(ch);
            }
            ',' if depth == 0 => {
                let trimmed = current.trim();
                if !trimmed.is_empty() {
                    parts.push(trimmed.to_string());
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        parts.push(trimmed.to_string());
    }
    parts
}

fn expand_braces(input: &str) -> Vec<String> {
    let Some((prefix, body, suffix)) = split_first_brace_group(input) else {
        return vec![input.to_string()];
    };
    let mut expanded = Vec::new();
    for alternative in body.split(',') {
        let candidate = format!("{prefix}{}{suffix}", alternative.trim());
        expanded.extend(expand_braces(&candidate));
    }
    expanded
}

fn split_first_brace_group(input: &str) -> Option<(&str, &str, &str)> {
    let start = input.find('{')?;
    let end = input[start + 1..].find('}')? + start + 1;
    let prefix = &input[..start];
    let body = &input[start + 1..end];
    if body.is_empty() {
        return None;
    }
    let suffix = &input[end + 1..];
    Some((prefix, body, suffix))
}

/// Claude's node-ignore compile probe never rejects a UTF-8 string pattern.
fn node_ignore_pattern_is_valid(_pattern: &str) -> bool {
    true
}

fn pattern_evidence_fragment(pattern: &str) -> String {
    const MAX_CHARS: usize = 32;
    let mut fragment: String = pattern.chars().take(MAX_CHARS).collect();
    if pattern.chars().count() > MAX_CHARS {
        fragment.push('…');
    }
    format!("paths pattern: {fragment}")
}

fn locate_paths_scalar_metadata(
    content: &str,
    yaml_range: Option<Range<usize>>,
    value: &str,
) -> Option<DiagnosticMetadata> {
    let yaml_range = yaml_range?;
    let yaml = &content[yaml_range.clone()];
    let candidates = [
        format!("'{value}'"),
        format!("\"{value}\""),
        value.to_string(),
    ];
    for candidate in candidates {
        if let Some(local) = yaml.find(&candidate) {
            let start = yaml_range.start + local;
            let end = start + candidate.len();
            return Some(metadata_for_range(
                content,
                start..end,
                "paths pattern: invalid",
                R001_SUGGESTION,
            ));
        }
    }
    None
}

fn report_output_style_frontmatter_invalid(
    diag: &mut DiagnosticCollector,
    path: &str,
    content: &str,
    range: Range<usize>,
    message: &str,
) {
    diag.report_with(
        LintRule::OutputStyleFrontmatterInvalid,
        &format!("{path}: {message}"),
        metadata_for_range(
            content,
            range,
            "frontmatter: invalid",
            OUTPUT_STYLE_FRONTMATTER_SUGGESTION,
        ),
    );
}

fn report_output_style_unknown_fields(
    diag: &mut DiagnosticCollector,
    path: &str,
    content: &str,
    yaml_range: Option<Range<usize>>,
    frontmatter: &Mapping,
    surface: OutputStyleSurface,
) {
    for key in frontmatter.keys() {
        if OUTPUT_STYLE_FIELDS.contains(&key.as_str()) {
            continue;
        }
        let (message, evidence) = if key == OUTPUT_STYLE_FORCE_FOR_PLUGIN {
            match surface {
                // Plugin-bundled styles honor `force-for-plugin`, so it is a
                // recognized field and emits no O003. No type validation is added
                // because no recorded schema type exists for it.
                OutputStyleSurface::Plugin => continue,
                // Private `.claude/output-styles/` styles ignore the field; report
                // the specific placement message rather than a generic unknown one.
                OutputStyleSurface::Private => (
                    "force-for-plugin is honored only for plugin-bundled output styles and is ignored under .claude/output-styles",
                    "force-for-plugin: private-style placement",
                ),
            }
        } else {
            (
                "output-style frontmatter contains an unsupported field",
                "frontmatter: unsupported field",
            )
        };
        diag.report_with(
            LintRule::OutputStyleFieldUnknown,
            &format!("{path}: {message}"),
            metadata_for_field(
                content,
                yaml_range.clone(),
                key,
                evidence,
                OUTPUT_STYLE_FIELD_SUGGESTION,
            ),
        );
    }
}

fn validate_output_style_fields(
    diag: &mut DiagnosticCollector,
    path: &str,
    content: &str,
    yaml_range: Option<Range<usize>>,
    frontmatter: &Mapping,
) {
    let description_issue = match frontmatter.get("description") {
        None => Some(("missing", "description: missing")),
        Some(YamlValue::String(value)) if value.trim().is_empty() => {
            Some(("blank", "description: blank"))
        }
        Some(YamlValue::String(_)) => None,
        Some(_) => Some(("must be a string", "description: non-string")),
    };
    if let Some((category, evidence)) = description_issue {
        diag.report_with(
            LintRule::OutputStyleDescriptionMissing,
            &format!("{path}: 'description' {category}"),
            metadata_for_field(
                content,
                yaml_range.clone(),
                "description",
                evidence,
                OUTPUT_STYLE_DESCRIPTION_SUGGESTION,
            ),
        );
    }

    if let Some(value) = frontmatter.get("keep-coding-instructions")
        && !output_style_instructions_value_is_valid(value)
    {
        diag.report_with(
            LintRule::OutputStyleKeepCodingInstructionsInvalid,
            &format!(
                "{path}: 'keep-coding-instructions' must be true, false, \"true\", or \"false\""
            ),
            metadata_for_field(
                content,
                yaml_range,
                "keep-coding-instructions",
                "keep-coding-instructions: unsupported value",
                OUTPUT_STYLE_INSTRUCTIONS_SUGGESTION,
            ),
        );
    }
}

fn output_style_instructions_value_is_valid(value: &YamlValue) -> bool {
    value.is_bool() || matches!(value.as_str(), Some("true" | "false"))
}

fn metadata_for_field(
    content: &str,
    yaml_range: Option<Range<usize>>,
    key: &str,
    evidence: &str,
    suggestion: &str,
) -> DiagnosticMetadata {
    let range = yaml_range
        .and_then(|range| yaml_field_range(content, range, key))
        .unwrap_or(0..0);
    metadata_for_range(content, range, evidence, suggestion)
}

fn metadata_for_range(
    content: &str,
    range: Range<usize>,
    evidence: &str,
    suggestion: &str,
) -> DiagnosticMetadata {
    let mut metadata = DiagnosticMetadata::default()
        .with_evidence(evidence)
        .with_suggestion(suggestion);
    if let Some(span) = SourceSpan::from_byte_range(content, range) {
        metadata = metadata.with_location(span);
    }
    metadata
}

fn yaml_line_range(content: &str, yaml_range: Range<usize>, line: usize) -> Option<Range<usize>> {
    let yaml = &content[yaml_range.clone()];
    let start = yaml
        .split_inclusive('\n')
        .take(line.saturating_sub(1))
        .map(str::len)
        .sum::<usize>();
    let end = yaml[start..]
        .find('\n')
        .map_or(yaml.len(), |offset| start + offset);
    (start <= end).then_some((yaml_range.start + start)..(yaml_range.start + end))
}

fn yaml_field_range(content: &str, yaml_range: Range<usize>, key: &str) -> Option<Range<usize>> {
    let yaml = &content[yaml_range.clone()];
    let mut offset = 0;
    for line in yaml.split_inclusive('\n') {
        let text = line.trim_end_matches(['\r', '\n']);
        if yaml_top_level_key_matches(text, key) {
            return Some(yaml_range.start + offset..yaml_range.start + offset + text.len());
        }
        offset += line.len();
    }
    None
}

fn yaml_top_level_key_matches(line: &str, key: &str) -> bool {
    if line.starts_with([' ', '\t']) {
        return false;
    }
    let Some(after_key) = line.strip_prefix(key) else {
        return [format!("\"{key}\""), format!("'{key}'")]
            .into_iter()
            .any(|quoted| {
                line.strip_prefix(&quoted)
                    .is_some_and(|rest| rest.starts_with(':'))
            });
    };
    after_key.starts_with(':')
}

fn body_range(content: &str, body: &str) -> Option<Range<usize>> {
    let start = body.as_ptr() as usize - content.as_ptr() as usize;
    (start <= content.len()).then_some(start..content.len())
}

fn validate_typed_settings(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    for (path, state) in [
        (".claude/settings.json", &ctx.settings_json),
        (".claude/settings.local.json", &ctx.settings_local_json),
    ] {
        if let ManifestState::Parsed(settings) = state {
            validate_typed_settings_file(diag, path, settings);
        }
    }
}

/// Validate one parsed settings file. New typed fields belong here.
fn validate_typed_settings_file(
    diag: &mut DiagnosticCollector,
    path: &str,
    settings: &ParsedManifest,
) {
    if let Some(value) = settings.get("prUrlTemplate") {
        if let Some(category) = pr_url_template_category(value) {
            report_typed_setting(
                diag,
                LintRule::SettingsPrUrlTemplateInvalid,
                path,
                settings,
                TypedSettingIssue {
                    key: "prUrlTemplate",
                    evidence: category.evidence(),
                    suggestion: "replace prUrlTemplate with a documented-placeholder http(s) URL template",
                    message: "prUrlTemplate is not a usable PR URL template",
                },
            );
        }
    }

    if settings.get("channelsEnabled").is_some() {
        report_typed_setting(
            diag,
            LintRule::SettingsChannelsEnabledInvalid,
            path,
            settings,
            TypedSettingIssue {
                key: "channelsEnabled",
                evidence: "channelsEnabled: unsupported scope",
                suggestion: CHANNELS_ENABLED_SUGGESTION,
                message: "channelsEnabled is managed-policy-only and ignored in project/local settings",
            },
        );
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PrUrlTemplateCategory {
    WrongType,
    BlankOrSurroundingWhitespace,
    NoDocumentedPlaceholder,
    UnknownPlaceholder,
    InvalidRenderedUrl,
}

impl PrUrlTemplateCategory {
    fn evidence(self) -> &'static str {
        match self {
            Self::WrongType => "prUrlTemplate: wrong type",
            Self::BlankOrSurroundingWhitespace => "prUrlTemplate: blank or surrounding whitespace",
            Self::NoDocumentedPlaceholder => "prUrlTemplate: no documented placeholder",
            Self::UnknownPlaceholder => "prUrlTemplate: unknown placeholder",
            Self::InvalidRenderedUrl => "prUrlTemplate: invalid rendered URL",
        }
    }
}

fn pr_url_template_category(value: &JsonValue) -> Option<PrUrlTemplateCategory> {
    let Some(template) = value.as_str() else {
        return Some(PrUrlTemplateCategory::WrongType);
    };
    if template.is_empty() || template.trim() != template {
        return Some(PrUrlTemplateCategory::BlankOrSurroundingWhitespace);
    }
    if !PR_URL_TEMPLATE_PLACEHOLDERS
        .iter()
        .any(|placeholder| template.contains(placeholder))
    {
        return Some(PrUrlTemplateCategory::NoDocumentedPlaceholder);
    }
    if RE_BRACE_TOKEN
        .find_iter(template)
        .any(|token| !PR_URL_TEMPLATE_PLACEHOLDERS.contains(&token.as_str()))
    {
        return Some(PrUrlTemplateCategory::UnknownPlaceholder);
    }

    let rendered = render_pr_url_template(template);
    let is_http_url = url::Url::parse(&rendered)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host().is_some());
    (rendered.chars().any(char::is_whitespace) || !is_http_url)
        .then_some(PrUrlTemplateCategory::InvalidRenderedUrl)
}

fn render_pr_url_template(template: &str) -> String {
    template
        .replace("{host}", "github.com")
        .replace("{owner}", "owner")
        .replace("{repo}", "repo")
        .replace("{number}", "1")
        .replace("{url}", "https://github.com/owner/repo/pull/1")
}

struct TypedSettingIssue {
    key: &'static str,
    evidence: &'static str,
    suggestion: &'static str,
    message: &'static str,
}

fn report_typed_setting(
    diag: &mut DiagnosticCollector,
    rule: LintRule,
    path: &str,
    settings: &ParsedManifest,
    issue: TypedSettingIssue,
) {
    let metadata = typed_setting_metadata(settings, issue.key, issue.evidence, issue.suggestion);
    diag.report_at_with(rule, path, &format!("{path}: {}", issue.message), metadata);
}

fn typed_setting_metadata(
    settings: &ParsedManifest,
    key: &str,
    evidence: &str,
    suggestion: &str,
) -> DiagnosticMetadata {
    let mut metadata = DiagnosticMetadata::default()
        .with_evidence(evidence)
        .with_suggestion(suggestion);
    if let Some(span) = settings
        .source()
        .and_then(|source| json_top_level_member_range(source, key))
        .and_then(|range| {
            settings
                .source()
                .and_then(|source| SourceSpan::from_byte_range(source, range))
        })
    {
        metadata = metadata.with_location(span);
    }
    metadata
}

/// Locate the final effective top-level JSON member from its key through its
/// value. The manifest was already parsed, so this only maps source offsets.
fn json_top_level_member_range(source: &str, wanted_key: &str) -> Option<Range<usize>> {
    let mut scanner = TopLevelMemberScanner::new(source);
    scanner.scan_root(wanted_key);
    scanner.found
}

struct TopLevelMemberScanner<'a> {
    input: &'a [u8],
    position: usize,
    found: Option<Range<usize>>,
}

impl<'a> TopLevelMemberScanner<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            input: source.as_bytes(),
            position: 0,
            found: None,
        }
    }

    fn scan_root(&mut self, wanted_key: &str) {
        self.skip_whitespace();
        if self.consume(b'{') {
            loop {
                self.skip_whitespace();
                if self.consume(b'}') {
                    return;
                }
                let member_start = self.position;
                let key = self.scan_string();
                self.skip_whitespace();
                self.consume(b':');
                self.skip_whitespace();
                self.scan_value();
                if key == wanted_key {
                    self.found = Some(member_start..self.position);
                }
                self.skip_whitespace();
                self.consume(b',');
            }
        }
    }

    fn scan_value(&mut self) {
        self.skip_whitespace();
        match self.input.get(self.position) {
            Some(b'{') => self.scan_object(),
            Some(b'[') => self.scan_array(),
            Some(b'"') => {
                self.scan_string();
            }
            Some(_) => self.scan_scalar(),
            None => {}
        }
    }

    fn scan_object(&mut self) {
        self.position += 1;
        loop {
            self.skip_whitespace();
            if self.consume(b'}') {
                return;
            }
            self.scan_string();
            self.skip_whitespace();
            self.consume(b':');
            self.scan_value();
            self.skip_whitespace();
            self.consume(b',');
        }
    }

    fn scan_array(&mut self) {
        self.position += 1;
        loop {
            self.skip_whitespace();
            if self.consume(b']') {
                return;
            }
            self.scan_value();
            self.skip_whitespace();
            self.consume(b',');
        }
    }

    fn scan_string(&mut self) -> String {
        let start = self.position;
        self.position += 1;
        while self.position < self.input.len() {
            match self.input[self.position] {
                b'\\' => self.position += 2,
                b'"' => {
                    self.position += 1;
                    break;
                }
                _ => self.position += 1,
            }
        }
        serde_json::from_slice(&self.input[start..self.position])
            .expect("scanner only runs after successful JSON parsing")
    }

    fn scan_scalar(&mut self) {
        while self.position < self.input.len()
            && !matches!(
                self.input[self.position],
                b',' | b']' | b'}' | b' ' | b'\n' | b'\r' | b'\t'
            )
        {
            self.position += 1;
        }
    }

    fn skip_whitespace(&mut self) {
        while self.position < self.input.len() && self.input[self.position].is_ascii_whitespace() {
            self.position += 1;
        }
    }

    fn consume(&mut self, byte: u8) -> bool {
        if self.input.get(self.position) == Some(&byte) {
            self.position += 1;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_temp_dir(test: impl FnOnce()) {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        test();
    }

    fn validate() -> DiagnosticCollector {
        let ctx = LintContext::new(
            &std::env::current_dir().expect("current directory is readable"),
            crate::context::LintMode::Basic,
        );
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_private_config(&ctx, &mut diag, &ExcludeSet::default());
        diag
    }

    #[test]
    #[serial_test::serial]
    fn rules_empty_paths_reports_r001() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude/rules").unwrap();
            fs::write(".claude/rules/rule.md", "---\npaths: []\n---\nRule body\n").unwrap();
            let diag = validate();
            let finding = diag
                .diagnostics()
                .iter()
                .find(|d| d.rule == LintRule::RulesGlobInvalid)
                .expect("R001");
            assert_eq!(finding.suggestion.as_deref(), Some(R001_SUGGESTION));
            assert_eq!(finding.evidence.as_deref(), Some("paths: empty"));
            assert!(finding.location.is_some());
        });
    }

    #[test]
    #[serial_test::serial]
    fn rules_accept_node_ignore_patterns_rejected_by_globset() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude/rules").unwrap();
            fs::write(".claude/rules/rule.md", "---\npaths: '['\n---\nRule body\n").unwrap();
            let diag = validate();
            assert!(
                !diag
                    .diagnostics()
                    .iter()
                    .any(|d| d.rule == LintRule::RulesGlobInvalid)
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn rules_missing_closer_reports_r003() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude/rules").unwrap();
            fs::write(
                ".claude/rules/rule.md",
                "---\npaths: ['src/**']\nRule body\n",
            )
            .unwrap();
            let diag = validate();
            assert!(diag.diagnostics().iter().any(|d| {
                d.rule == LintRule::RulesFrontmatterInvalid
                    && d.evidence.as_deref() == Some("frontmatter: missing closer")
            }));
        });
    }

    #[test]
    #[serial_test::serial]
    fn rules_no_frontmatter_is_clean() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude/rules").unwrap();
            fs::write(".claude/rules/rule.md", "Unconditional rule body\n").unwrap();
            let diag = validate();
            assert_eq!(diag.error_count(), 0);
            assert_eq!(diag.warning_count(), 0);
        });
    }

    #[test]
    fn claude_paths_normalization_matches_runtime_order() {
        assert_eq!(
            normalize_claude_paths_string("src/{a,b}/**, lib/**/*.rs"),
            vec!["src/a", "src/b", "lib/**/*.rs"]
        );
        assert_eq!(normalize_claude_paths_string("**"), vec!["**"]);
        assert_eq!(normalize_claude_paths_string("src/**"), vec!["src"]);
    }

    #[test]
    #[serial_test::serial]
    fn rules_unknown_field_reports_r002() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude/rules").unwrap();
            fs::write(
                ".claude/rules/rule.md",
                "---\nunknown: value\n---\nRule body\n",
            )
            .unwrap();
            let diag = validate();
            assert!(
                diag.diagnostics()
                    .iter()
                    .any(|d| d.rule == LintRule::RulesFieldUnknown)
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn output_style_invalid_frontmatter_reports_o006() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude/output-styles").unwrap();
            fs::write(
                ".claude/output-styles/style.md",
                "---\ndescription: [\n---\nBody\n",
            )
            .unwrap();
            let diag = validate();
            assert!(
                diag.diagnostics()
                    .iter()
                    .any(|d| d.rule == LintRule::OutputStyleFrontmatterInvalid)
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn output_style_description_reports_o001() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude/output-styles").unwrap();
            fs::write(
                ".claude/output-styles/style.md",
                "---\ndescription: '   '\n---\nBody\n",
            )
            .unwrap();
            let diag = validate();
            assert!(
                diag.diagnostics()
                    .iter()
                    .any(|d| d.rule == LintRule::OutputStyleDescriptionMissing)
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn output_style_keep_coding_type_reports_o002() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude/output-styles").unwrap();
            fs::write(
                ".claude/output-styles/style.md",
                "---\ndescription: Good\nkeep-coding-instructions: 'TRUE'\n---\nBody\n",
            )
            .unwrap();
            let diag = validate();
            assert!(
                diag.diagnostics()
                    .iter()
                    .any(|d| d.rule == LintRule::OutputStyleKeepCodingInstructionsInvalid)
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn output_style_unknown_field_reports_o003() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude/output-styles").unwrap();
            fs::write(
                ".claude/output-styles/style.md",
                "---\ndescription: Good\nextra: value\n---\nBody\n",
            )
            .unwrap();
            let diag = validate();
            assert!(
                diag.diagnostics()
                    .iter()
                    .any(|d| d.rule == LintRule::OutputStyleFieldUnknown)
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn output_style_empty_body_reports_o004() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude/output-styles").unwrap();
            fs::write(
                ".claude/output-styles/style.md",
                "---\ndescription: Good\n---\n \n",
            )
            .unwrap();
            let diag = validate();
            assert!(
                diag.diagnostics()
                    .iter()
                    .any(|d| d.rule == LintRule::OutputStyleBodyEmpty)
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn output_style_long_name_is_clean_and_o005_is_inert() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude/output-styles").unwrap();
            fs::write(
                ".claude/output-styles/style.md",
                format!(
                    "---\nname: {}\ndescription: Good\n---\nBody\n",
                    "a".repeat(65)
                ),
            )
            .unwrap();
            let diag = validate();
            assert!(
                !diag
                    .diagnostics()
                    .iter()
                    .any(|d| d.rule == LintRule::OutputStyleNameTooLong)
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn output_styles_accept_body_only_and_quoted_lowercase_instruction_booleans() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude/output-styles/nested").unwrap();
            fs::write(
                ".claude/output-styles/nested/style.md",
                "Use terse, actionable answers.\n",
            )
            .unwrap();
            fs::write(
                ".claude/output-styles/quoted.md",
                "--- \t\r\ndescription: Good\r\nkeep-coding-instructions: \"true\"\r\n---  \r\nBody\r\n",
            )
            .unwrap();
            let diag = validate();
            assert!(diag.diagnostics().iter().all(|diagnostic| {
                diagnostic.rule != LintRule::OutputStyleFrontmatterInvalid
                    && diagnostic.rule != LintRule::OutputStyleKeepCodingInstructionsInvalid
            }));
            assert!(diag.diagnostics().iter().any(|diagnostic| {
                diagnostic.rule == LintRule::OutputStyleDescriptionMissing
                    && diagnostic.subject_path.as_deref()
                        == Some(std::path::Path::new(
                            ".claude/output-styles/nested/style.md",
                        ))
            }));
        });
    }

    #[test]
    #[serial_test::serial]
    fn output_style_o006_suppresses_field_cascades_and_has_safe_metadata() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude/output-styles").unwrap();
            fs::write(
                ".claude/output-styles/style.md",
                "---\ndescription: [\nkeep-coding-instructions: nope\nunknown-secret-key: sk_this-must-not-leak\n---\n",
            )
            .unwrap();
            let diag = validate();
            assert_eq!(diag.diagnostics().len(), 1);
            let diagnostic = &diag.diagnostics()[0];
            assert_eq!(diagnostic.rule, LintRule::OutputStyleFrontmatterInvalid);
            assert!(diagnostic.location.is_some());
            assert_eq!(diagnostic.evidence.as_deref(), Some("frontmatter: invalid"));
            assert_eq!(
                diagnostic.suggestion.as_deref(),
                Some(OUTPUT_STYLE_FRONTMATTER_SUGGESTION)
            );
            assert!(!diagnostic.message.contains("sk_this-must-not-leak"));
        });
    }

    #[test]
    #[serial_test::serial]
    fn output_style_reports_category_specific_description_and_private_field_metadata() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude/output-styles").unwrap();
            fs::write(
                ".claude/output-styles/style.md",
                "---\ndescription: 7\nforce-for-plugin: false\nvery-long-unknown-key-that-must-not-appear-in-output: secret\n---\nBody\n",
            )
            .unwrap();
            let diag = validate();
            let description = diag
                .diagnostics()
                .iter()
                .find(|diagnostic| diagnostic.rule == LintRule::OutputStyleDescriptionMissing)
                .unwrap();
            assert!(description.message.contains("must be a string"));
            assert_eq!(
                description.evidence.as_deref(),
                Some("description: non-string")
            );
            assert!(description.location.is_some());
            let force = diag
                .diagnostics()
                .iter()
                .find(|diagnostic| diagnostic.message.contains("plugin-bundled"))
                .unwrap();
            assert_eq!(
                force.evidence.as_deref(),
                Some("force-for-plugin: private-style placement")
            );
            assert!(diag.diagnostics().iter().all(|diagnostic| {
                !diagnostic
                    .message
                    .contains("very-long-unknown-key-that-must-not-appear-in-output")
            }));
        });
    }

    // ── #392: plugin-shipped output styles ──────────────────────────

    /// Discover and lint plugin-shipped output styles using the current
    /// directory as a Plugin-mode repository, exactly as `run_plugin` does:
    /// declared `outputStyles` roots come from the on-disk plugin.json.
    fn plugin_output_style_diag_with(exclude: &ExcludeSet) -> DiagnosticCollector {
        let ctx = LintContext::new(
            &std::env::current_dir().expect("current directory is readable"),
            crate::context::LintMode::Plugin,
        );
        let roots = crate::validators::manifest::declared_output_style_roots(&ctx);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_plugin_output_styles(&mut diag, exclude, &roots);
        diag
    }

    fn plugin_output_style_diag() -> DiagnosticCollector {
        plugin_output_style_diag_with(&ExcludeSet::default())
    }

    fn diagnostics_for<'a>(
        diag: &'a DiagnosticCollector,
        subject: &str,
    ) -> Vec<&'a crate::diagnostic::Diagnostic> {
        diag.diagnostics()
            .iter()
            .filter(|d| d.subject_path.as_deref() == Some(Path::new(subject)))
            .collect()
    }

    #[test]
    #[serial_test::serial]
    fn plugin_root_broken_style_matches_private_twin_identity() {
        with_temp_dir(|| {
            fs::create_dir_all("output-styles").unwrap();
            fs::create_dir_all(".claude/output-styles").unwrap();
            let broken = "---\ndescription: [\n---\nBody\n";
            fs::write("output-styles/broken.md", broken).unwrap();
            fs::write(".claude/output-styles/broken.md", broken).unwrap();

            let private = validate();
            let plugin = plugin_output_style_diag();
            let private_twin = diagnostics_for(&private, ".claude/output-styles/broken.md");
            let plugin_twin = diagnostics_for(&plugin, "output-styles/broken.md");

            assert_eq!(private_twin.len(), 1, "private twin emits one diagnostic");
            assert_eq!(plugin_twin.len(), 1, "plugin twin emits one diagnostic");
            assert_eq!(
                private_twin[0].rule,
                LintRule::OutputStyleFrontmatterInvalid
            );
            // Same rule identity, severity, evidence, and suggestion across surfaces.
            assert_eq!(plugin_twin[0].rule, private_twin[0].rule);
            assert_eq!(plugin_twin[0].severity, private_twin[0].severity);
            assert_eq!(plugin_twin[0].evidence, private_twin[0].evidence);
            assert_eq!(plugin_twin[0].suggestion, private_twin[0].suggestion);
        });
    }

    #[test]
    #[serial_test::serial]
    fn plugin_root_output_styles_apply_every_active_o_rule_including_nested() {
        with_temp_dir(|| {
            fs::create_dir_all("output-styles/nested").unwrap();
            fs::write("output-styles/o001.md", "---\ndescription: 7\n---\nBody\n").unwrap();
            fs::write(
                "output-styles/o002.md",
                "---\ndescription: Good\nkeep-coding-instructions: nope\n---\nBody\n",
            )
            .unwrap();
            fs::write(
                "output-styles/nested/o003.md",
                "---\ndescription: Good\nextra: value\n---\nBody\n",
            )
            .unwrap();
            fs::write("output-styles/o004.md", "---\ndescription: Good\n---\n \n").unwrap();
            fs::write("output-styles/o006.md", "---\ndescription: [\n---\nBody\n").unwrap();

            let diag = plugin_output_style_diag();
            let has = |subject: &str, rule: LintRule| {
                diagnostics_for(&diag, subject)
                    .iter()
                    .any(|d| d.rule == rule)
            };
            assert!(has(
                "output-styles/o001.md",
                LintRule::OutputStyleDescriptionMissing
            ));
            assert!(has(
                "output-styles/o002.md",
                LintRule::OutputStyleKeepCodingInstructionsInvalid
            ));
            assert!(has(
                "output-styles/nested/o003.md",
                LintRule::OutputStyleFieldUnknown
            ));
            assert!(has("output-styles/o004.md", LintRule::OutputStyleBodyEmpty));
            assert!(has(
                "output-styles/o006.md",
                LintRule::OutputStyleFrontmatterInvalid
            ));
        });
    }

    #[test]
    #[serial_test::serial]
    fn force_for_plugin_is_recognized_on_plugin_styles_and_flagged_on_private() {
        with_temp_dir(|| {
            fs::create_dir_all("output-styles").unwrap();
            fs::create_dir_all(".claude/output-styles").unwrap();
            let style = "---\ndescription: Good\nforce-for-plugin: true\n---\nBody\n";
            fs::write("output-styles/s.md", style).unwrap();
            fs::write(".claude/output-styles/s.md", style).unwrap();

            // Plugin-shipped: force-for-plugin is recognized, so no O003 at all.
            let plugin = plugin_output_style_diag();
            assert!(
                plugin
                    .diagnostics()
                    .iter()
                    .all(|d| d.rule != LintRule::OutputStyleFieldUnknown),
                "force-for-plugin must not warn on plugin-shipped styles: {:?}",
                plugin.diagnostics()
            );

            // Private: keep #348's specific ignored-placement O003 message.
            let private = validate();
            let o003 = diagnostics_for(&private, ".claude/output-styles/s.md");
            assert_eq!(o003.len(), 1);
            assert_eq!(o003[0].rule, LintRule::OutputStyleFieldUnknown);
            assert!(o003[0].message.contains("plugin-bundled"));
            assert_eq!(
                o003[0].evidence.as_deref(),
                Some("force-for-plugin: private-style placement")
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn plugin_output_styles_discover_declared_forms_and_lint_default_once() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude-plugin").unwrap();
            fs::create_dir_all("output-styles").unwrap();
            fs::create_dir_all("declared-dir/deep").unwrap();
            fs::create_dir_all("single").unwrap();
            // Declares a directory, a single file, and a duplicate of the default
            // plugin-root directory. The duplicate must not double-lint its files.
            fs::write(
                ".claude-plugin/plugin.json",
                r#"{"name":"p","version":"1.0.0","outputStyles":["./declared-dir","./single/one.md","./output-styles"]}"#,
            )
            .unwrap();
            fs::write(
                "output-styles/def.md",
                "---\ndescription: Good\nextra: 1\n---\nBody\n",
            )
            .unwrap();
            fs::write("declared-dir/deep/d.md", "Body only, no frontmatter.\n").unwrap();
            fs::write(
                "single/one.md",
                "---\ndescription: Good\nweird: 1\n---\nBody\n",
            )
            .unwrap();

            let diag = plugin_output_style_diag();
            // Default directory file linted exactly once despite the `./output-styles`
            // duplicate declaration.
            assert_eq!(
                diagnostics_for(&diag, "output-styles/def.md")
                    .iter()
                    .filter(|d| d.rule == LintRule::OutputStyleFieldUnknown)
                    .count(),
                1,
            );
            // Declared directory is scanned recursively (body-only → O001).
            assert!(
                diagnostics_for(&diag, "declared-dir/deep/d.md")
                    .iter()
                    .any(|d| d.rule == LintRule::OutputStyleDescriptionMissing)
            );
            // Declared single `.md` file contributes that file.
            assert!(
                diagnostics_for(&diag, "single/one.md")
                    .iter()
                    .any(|d| d.rule == LintRule::OutputStyleFieldUnknown)
            );
        });
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn plugin_output_styles_never_escape_via_symlinked_declared_ancestor() {
        use std::os::unix::fs::symlink;
        with_temp_dir(|| {
            let outside = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(outside.path().join("styles")).unwrap();
            std::fs::write(
                outside.path().join("styles/leak.md"),
                "---\ndescription: [\n---\n",
            )
            .unwrap();
            fs::create_dir_all(".claude-plugin").unwrap();
            fs::write(
                ".claude-plugin/plugin.json",
                r#"{"name":"p","version":"1.0.0","outputStyles":["./via/styles"]}"#,
            )
            .unwrap();
            // `via` is an intermediate symlink pointing outside the repository.
            symlink(outside.path(), "via").unwrap();

            let diag = plugin_output_style_diag();
            assert!(
                diag.diagnostics().is_empty(),
                "a declared root behind a symlinked ancestor must not read outside the repository: {:?}",
                diag.diagnostics()
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn plugin_output_styles_report_o006_for_non_mapping_and_crlf_frontmatter() {
        with_temp_dir(|| {
            fs::create_dir_all("output-styles").unwrap();
            // A syntactically valid YAML sequence is not a mapping: O006, no cascade.
            fs::write("output-styles/seq.md", "---\n- a\n- b\n---\nBody\n").unwrap();
            // CRLF delimiters with a genuinely malformed YAML body: O006.
            fs::write(
                "output-styles/crlf.md",
                "---\r\ndescription: [\r\n---\r\nBody\r\n",
            )
            .unwrap();
            let diag = plugin_output_style_diag();
            for subject in ["output-styles/seq.md", "output-styles/crlf.md"] {
                let found = diagnostics_for(&diag, subject);
                assert_eq!(found.len(), 1, "{subject} emits exactly one diagnostic");
                assert_eq!(found[0].rule, LintRule::OutputStyleFrontmatterInvalid);
            }
        });
    }

    #[test]
    #[serial_test::serial]
    fn output_style_diagnostics_are_deterministically_ordered_across_surfaces() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude/output-styles").unwrap();
            fs::create_dir_all("output-styles").unwrap();
            fs::create_dir_all(".claude-plugin").unwrap();
            fs::write(
                ".claude-plugin/plugin.json",
                r#"{"name":"p","version":"1.0.0"}"#,
            )
            .unwrap();
            let invalid = "---\ndescription: [\n---\n";
            fs::write(".claude/output-styles/priv.md", invalid).unwrap();
            fs::write("output-styles/plug.md", invalid).unwrap();

            // Run the full plugin pipeline: the private surface is validated
            // before the plugin surface, and the collector orders by registry
            // rank then emission order, so the private O006 precedes the plugin
            // O006 deterministically.
            let ctx = LintContext::new(
                &std::env::current_dir().unwrap(),
                crate::context::LintMode::Plugin,
            );
            let mut diag = DiagnosticCollector::new_all_enabled();
            super::super::run_all(&ctx, &mut diag, &ExcludeSet::default());
            let o006: Vec<_> = diag
                .diagnostics()
                .iter()
                .filter(|d| d.rule == LintRule::OutputStyleFrontmatterInvalid)
                .filter_map(|d| d.subject_path.as_deref().map(Path::to_path_buf))
                .collect();
            assert_eq!(
                o006,
                vec![
                    Path::new(".claude/output-styles/priv.md").to_path_buf(),
                    Path::new("output-styles/plug.md").to_path_buf(),
                ],
                "private-surface O006 must precede plugin-surface O006 deterministically"
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn plugin_output_styles_never_read_unsafe_or_nonexistent_declared_paths() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude-plugin/output-styles").unwrap();
            fs::create_dir_all("bundle/.claude-plugin/output-styles").unwrap();
            fs::write(
                ".claude-plugin/plugin.json",
                r#"{"name":"p","version":"1.0.0","outputStyles":["/abs/x.md","../up/y.md","./.claude-plugin/output-styles","./bundle","./missing"]}"#,
            )
            .unwrap();
            // Files at rejected locations that must never be read or reported: the
            // repo-root manifest tree, and a `.claude-plugin/` nested under an
            // otherwise-safe declared directory root.
            fs::write(
                ".claude-plugin/output-styles/nested.md",
                "---\ndescription: [\n---\n",
            )
            .unwrap();
            fs::write(
                "bundle/.claude-plugin/output-styles/leak.md",
                "---\ndescription: [\n---\n",
            )
            .unwrap();

            let diag = plugin_output_style_diag();
            assert!(
                diag.diagnostics().is_empty(),
                "absolute, traversal, manifest-tree, and missing declared paths must not be read: {:?}",
                diag.diagnostics()
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn basic_mode_never_scans_plugin_shipped_output_styles() {
        with_temp_dir(|| {
            fs::create_dir_all("output-styles").unwrap();
            fs::write("output-styles/plugin-only.md", "---\ndescription: [\n---\n").unwrap();
            let ctx = LintContext::new(
                &std::env::current_dir().unwrap(),
                crate::context::LintMode::Basic,
            );
            let mut diag = DiagnosticCollector::new_all_enabled();
            super::super::run_all(&ctx, &mut diag, &ExcludeSet::default());
            assert!(
                diagnostics_for(&diag, "output-styles/plugin-only.md").is_empty(),
                "Basic mode must never scan the plugin-root output-styles/ surface"
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn plugin_output_styles_honor_exclusions() {
        with_temp_dir(|| {
            fs::create_dir_all("output-styles/skip").unwrap();
            fs::write("output-styles/keep.md", "---\ndescription: [\n---\n").unwrap();
            fs::write("output-styles/skip/drop.md", "---\ndescription: [\n---\n").unwrap();
            let exclude = ExcludeSet::new(&["output-styles/skip/**".to_string()]).unwrap();

            let diag = plugin_output_style_diag_with(&exclude);
            assert!(!diagnostics_for(&diag, "output-styles/keep.md").is_empty());
            assert!(
                diagnostics_for(&diag, "output-styles/skip/drop.md").is_empty(),
                "excluded plugin styles must not be read"
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn plugin_output_styles_honor_per_file_suppression() {
        with_temp_dir(|| {
            fs::create_dir_all("output-styles").unwrap();
            fs::write("output-styles/reported.md", "---\ndescription: [\n---\n").unwrap();
            fs::write("output-styles/muted.md", "---\ndescription: [\n---\n").unwrap();
            fs::write(
                "agent-lint.toml",
                "[lint]\n[[lint.overrides]]\nfiles = [\"output-styles/muted.md\"]\nsuppress = [\"O006\"]\n",
            )
            .unwrap();
            let config = crate::config::LintConfig::load(".").expect("config loads");
            let ctx = LintContext::new(
                &std::env::current_dir().unwrap(),
                crate::context::LintMode::Plugin,
            );
            let roots = crate::validators::manifest::declared_output_style_roots(&ctx);
            let mut diag = DiagnosticCollector::with_config(config);
            validate_plugin_output_styles(&mut diag, &ExcludeSet::default(), &roots);

            // The plugin-style subject path flows into per-file override matching:
            // the unsuppressed twin still reports O006; the matched twin is silenced.
            assert!(
                diagnostics_for(&diag, "output-styles/reported.md")
                    .iter()
                    .any(|d| d.rule == LintRule::OutputStyleFrontmatterInvalid)
            );
            assert!(diagnostics_for(&diag, "output-styles/muted.md").is_empty());
            assert_eq!(diag.suppressed_count(), 1);
        });
    }

    #[test]
    #[serial_test::serial]
    fn invalid_pr_url_template_reports_t001_for_local_settings() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude").unwrap();
            fs::write(
                ".claude/settings.local.json",
                r#"{"prUrlTemplate":"https://example.com"}"#,
            )
            .unwrap();
            let diag = validate();
            assert!(
                diag.diagnostics()
                    .iter()
                    .any(|d| d.rule == LintRule::SettingsPrUrlTemplateInvalid)
            );
        });
    }

    #[test]
    fn pr_url_template_validation_has_deterministic_categories_and_accepts_documented_shapes() {
        let invalid = [
            (serde_json::json!(null), PrUrlTemplateCategory::WrongType),
            (serde_json::json!(true), PrUrlTemplateCategory::WrongType),
            (serde_json::json!([]), PrUrlTemplateCategory::WrongType),
            (
                serde_json::json!(" "),
                PrUrlTemplateCategory::BlankOrSurroundingWhitespace,
            ),
            (
                serde_json::json!("https://example.test/static"),
                PrUrlTemplateCategory::NoDocumentedPlaceholder,
            ),
            (
                serde_json::json!("https://example.test/{number}/{ticket}"),
                PrUrlTemplateCategory::UnknownPlaceholder,
            ),
            (
                serde_json::json!("mailto:{number}"),
                PrUrlTemplateCategory::InvalidRenderedUrl,
            ),
            (
                serde_json::json!("https://example.test/{number} {ticket}"),
                PrUrlTemplateCategory::UnknownPlaceholder,
            ),
        ];
        for (value, expected) in invalid {
            assert_eq!(pr_url_template_category(&value), Some(expected), "{value}");
        }

        for template in [
            "{url}",
            "https://{host}:8443/{owner}/{repo}?pr={number}#review",
            "http://localhost/{number}",
            "https://127.0.0.1/{number}",
            "https://例え.テスト/{repo}/%7Bkept%7D",
            "https://reviews.example/?target={url}&again={url}",
        ] {
            assert_eq!(
                pr_url_template_category(&serde_json::json!(template)),
                None,
                "{template} should be valid"
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn invalid_channels_enabled_reports_t002() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude").unwrap();
            fs::write(".claude/settings.json", r#"{"channelsEnabled":"true"}"#).unwrap();
            let diag = validate();
            assert!(
                diag.diagnostics()
                    .iter()
                    .any(|d| d.rule == LintRule::SettingsChannelsEnabledInvalid)
            );
        });
    }

    #[test]
    fn channels_enabled_reports_once_for_every_json_type_on_each_settings_surface() {
        for path in [".claude/settings.json", ".claude/settings.local.json"] {
            for value in [
                serde_json::json!(null),
                serde_json::json!(true),
                serde_json::json!(1),
                serde_json::json!("true"),
                serde_json::json!([]),
                serde_json::json!({}),
            ] {
                let state = ManifestState::parsed(serde_json::json!({"channelsEnabled": value}));
                let ManifestState::Parsed(settings) = state else {
                    unreachable!("test state is parsed");
                };
                let mut diag = DiagnosticCollector::new_all_enabled();
                validate_typed_settings_file(&mut diag, path, &settings);
                assert_eq!(diag.diagnostics().len(), 1, "{path}: {value}");
                assert_eq!(
                    diag.diagnostics()[0].rule,
                    LintRule::SettingsChannelsEnabledInvalid,
                    "{path}: {value}"
                );
            }
        }
    }

    #[test]
    #[serial_test::serial]
    fn typed_settings_report_exact_member_metadata_without_exposing_values() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude").unwrap();
            let secret_like = "not-a-url/{number}?token=sk_this-value-must-not-appear";
            fs::write(
                ".claude/settings.json",
                format!(
                    "{{\n  \"prUrlTemplate\": \"{secret_like}\",\n  \"channelsEnabled\": true\n}}"
                ),
            )
            .unwrap();
            let diag = validate();
            let template = diag
                .diagnostics()
                .iter()
                .find(|diagnostic| diagnostic.rule == LintRule::SettingsPrUrlTemplateInvalid)
                .expect("T001 diagnostic");
            assert_eq!(
                template.evidence.as_deref(),
                Some("prUrlTemplate: invalid rendered URL")
            );
            assert_eq!(
                template.suggestion.as_deref(),
                Some("replace prUrlTemplate with a documented-placeholder http(s) URL template")
            );
            let location = template.location.expect("T001 has a member span");
            assert_eq!(location.start().line_number(), 2);
            assert_eq!(location.start().column_number(), Some(3));

            let channels = diag
                .diagnostics()
                .iter()
                .find(|diagnostic| diagnostic.rule == LintRule::SettingsChannelsEnabledInvalid)
                .expect("T002 diagnostic");
            assert_eq!(
                channels.evidence.as_deref(),
                Some("channelsEnabled: unsupported scope")
            );
            assert_eq!(
                channels
                    .location
                    .expect("T002 has a member span")
                    .start()
                    .line_number(),
                3
            );
            assert!(!template.message.contains(secret_like));
        });
    }

    #[test]
    #[serial_test::serial]
    fn typed_settings_use_the_context_snapshot_and_ignore_invalid_states() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude").unwrap();
            fs::write(
                ".claude/settings.json",
                r#"{"prUrlTemplate":"https://example.test/{number}/{ticket}"}"#,
            )
            .unwrap();
            let ctx = LintContext::new(
                &std::env::current_dir().unwrap(),
                crate::context::LintMode::Basic,
            );
            fs::write(".claude/settings.json", "{").unwrap();
            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_private_config(&ctx, &mut diag, &ExcludeSet::default());
            assert!(
                diag.diagnostics()
                    .iter()
                    .any(|diagnostic| diagnostic.rule == LintRule::SettingsPrUrlTemplateInvalid)
            );

            let invalid_ctx = LintContext::new(
                &std::env::current_dir().unwrap(),
                crate::context::LintMode::Basic,
            );
            let mut invalid_diag = DiagnosticCollector::new_all_enabled();
            validate_private_config(&invalid_ctx, &mut invalid_diag, &ExcludeSet::default());
            assert!(invalid_diag.diagnostics().is_empty());
        });
    }

    #[test]
    #[serial_test::serial]
    fn valid_optional_configuration_is_silent() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude/rules").unwrap();
            fs::create_dir_all(".claude/output-styles").unwrap();
            fs::write(
                ".claude/rules/rule.md",
                "---\npaths: ['src/**/*.rs']\n---\nRule body\n",
            )
            .unwrap();
            fs::write(".claude/output-styles/style.md", "---\nname: concise\ndescription: Keep answers concise\nkeep-coding-instructions: true\n---\nBody\n").unwrap();
            fs::write(
                ".claude/settings.json",
                r#"{"prUrlTemplate":"https://{host}/{owner}/{repo}/pull/{number}"}"#,
            )
            .unwrap();
            let diag = validate();
            assert_eq!(diag.error_count(), 0);
            assert_eq!(diag.warning_count(), 0);
        });
    }

    #[test]
    #[serial_test::serial]
    fn optional_surfaces_absent_are_silent() {
        with_temp_dir(|| {
            let diag = validate();
            assert_eq!(diag.error_count(), 0);
            assert_eq!(diag.warning_count(), 0);
        });
    }

    #[test]
    #[serial_test::serial]
    fn rules_are_dispatched_in_basic_and_plugin_modes() {
        with_temp_dir(|| {
            fs::create_dir_all(".claude/rules").unwrap();
            fs::write(".claude/rules/rule.md", "---\npaths: []\n---\nRule body\n").unwrap();

            for mode in [
                crate::context::LintMode::Basic,
                crate::context::LintMode::Plugin,
            ] {
                let ctx = crate::context::LintContext {
                    base_path: std::env::current_dir().unwrap(),
                    mode,
                    plugin_json: crate::context::ManifestState::Missing,
                    marketplace_json: crate::context::ManifestState::Missing,
                    hooks_json: crate::context::ManifestState::Missing,
                    declared_hook_configs: vec![],
                    settings_json: crate::context::ManifestState::Missing,
                    settings_local_json: crate::context::ManifestState::Missing,
                };
                let mut diag = DiagnosticCollector::new_all_enabled();
                super::super::run_all(&ctx, &mut diag, &ExcludeSet::default());
                assert!(
                    diag.diagnostics()
                        .iter()
                        .any(|d| d.rule == LintRule::RulesGlobInvalid),
                    "R001 was not dispatched in {mode:?} mode"
                );
            }
        });
    }
}
