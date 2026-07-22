//! Validation for Codex override files, plugin manifests, and skills.
//!
//! Codex plugin manifest linting is rebuilt around Codex's real manifest
//! discovery and schema. Classification of checks:
//!
//! - Runtime compatibility (parse/shape/path/prompt): CX047, CX050–CX055.
//! - Public authoring policy: CX048 (name present), CX049 (kebab-case name).
//! - Publishing / install quality: CX056 (interface URLs), CX057 (asset paths),
//!   CX059 (description), CX063 (ignored prompt-key migration aid).
//! - Soft-retired compatibility identifiers: CX046 (mislocated manifest — a
//!   recognized manifest always establishes its own plugin root) and CX058
//!   (unsupported hooks — Codex loads plugin-bundled hooks). Neither is emitted;
//!   both remain parseable for existing configuration.
//!
//! Verified against openai/codex commit
//! `7442f5f9323d116755dfe630e22c931a8aeaa5c7`
//! (`codex-rs/core-plugins/src/manifest.rs` and
//! `codex-rs/exec-server-protocol/src/protocol.rs`) and the public authoring
//! documentation at <https://developers.openai.com/codex/plugins/build> on
//! 2026-07-21.

use crate::config::ExcludeSet;
use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata, SourceSpan};
use crate::frontmatter;
use crate::live_instructions::{InstructionSurfaceKind, LiveInstructionDocument};
use crate::markdown::MarkdownDocument;
use crate::rules::LintRule;
use crate::traversal;
use regex::Regex;
use serde_json::{Map, Value};
use std::ops::Range;
use std::path::Path;
use std::sync::LazyLock;
use url::Url;

// The three-prompt count cap and 128 Unicode-scalar length cap are the current
// Codex runtime limits from the pinned `manifest.rs` above.
const MAX_DEFAULT_PROMPT_COUNT: usize = 3;
const MAX_DEFAULT_PROMPT_LEN: usize = 128;
/// Interface URLs are install/publishing metadata; Codex accepts long strings,
/// but agent-lint bounds them for install-surface sanity.
const MAX_INTERFACE_URL_LEN: usize = 1024;
/// Evidence values are truncated to this many Unicode scalars before the shared
/// secret classifier and the collector's own byte cap run.
const MAX_EVIDENCE_SCALARS: usize = 80;
/// Codex plugin manifests are metadata and small in practice (a few KB). A
/// larger input is treated as unvalidatable rather than parsed, bounding
/// worst-case work on a hostile manifest (G-Input-1: bound content-heavy work).
const MAX_MANIFEST_BYTES: usize = 64 * 1024;
/// Per-array element bound. Real manifests hold a handful of entries per field;
/// this caps diagnostic volume — and the collector's O(n) ordering insert per
/// diagnostic — on a hostile array without changing any legitimate outcome.
const MAX_VALIDATED_ARRAY_ELEMENTS: usize = 256;
const CODEX_SKILL_UNSUPPORTED_FIELDS: &[&str] = &["context", "agent", "hooks"];

/// Codex plugin manifest name identifier contract (kebab-case).
static PLUGIN_NAME_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[a-z0-9]+(?:-[a-z0-9]+)*$").expect("plugin name regex is valid")
});

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
    validate_override_tracking(diag, exclude, prompt_pass);
    validate_plugin_manifests(diag, exclude);
    validate_codex_skill_frontmatter(diag, exclude);
}

fn validate_override_tracking(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    prompt_pass: &mut super::prompt_content::PromptContentPass,
) {
    let path = "AGENTS.override.md";
    if exclude.is_excluded(path) || !Path::new(path).is_file() {
        return;
    }
    if let Ok(content) = std::fs::read_to_string(path) {
        let markdown = MarkdownDocument::parse_body(content);
        let document = LiveInstructionDocument::new(
            Path::new(path),
            InstructionSurfaceKind::CodexAgentsOverride,
            &markdown,
        );
        prompt_pass.validate(&document, diag);
    }
    if !is_git_tracked(path) {
        return;
    }
    diag.report_at(LintRule::CodexAgentsOverrideTracked, path, "AGENTS.override.md is tracked by Git; add it to .gitignore because it holds user-specific overrides");
}

fn is_git_tracked(path: &str) -> bool {
    std::process::Command::new("git")
        .args(["ls-files", "--error-unmatch", "--", path])
        .output()
        .is_ok_and(|output| output.status.success())
}

// ── Plugin manifest discovery and validation ─────────────────────────────

/// Validate every selected Codex plugin manifest.
///
/// The one deterministic discovery layer lives in [`crate::platforms`]: it
/// classifies candidates by exact parent-directory component and selects one
/// manifest per plugin root in Codex precedence order. Each selected manifest
/// is parsed once here and its physical subject path plus field/index location
/// data carried through every rule. An unreadable active manifest, invalid
/// JSON, or a non-object root is CX047 alone; no downstream rule cascades.
///
/// In plugin mode a `.claude-plugin/plugin.json` is also parsed by `LintContext`
/// for the Claude M/U rules; re-reading it here is intentional — Codex applies a
/// different contract to the same physical file.
fn validate_plugin_manifests(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    for manifest in crate::platforms::codex_plugin_manifests(exclude) {
        let display = manifest.display;
        let content = match std::fs::read_to_string(&manifest.path) {
            Ok(content) => content,
            Err(error) => {
                diag.report_at(
                    LintRule::CodexPluginManifestInvalid,
                    &display,
                    &format!("{display} could not be read as UTF-8 text: {error}"),
                );
                continue;
            }
        };
        if content.len() > MAX_MANIFEST_BYTES {
            diag.report_at_with(
                LintRule::CodexPluginManifestInvalid,
                &display,
                &format!(
                    "{display} exceeds the {MAX_MANIFEST_BYTES}-byte Codex plugin manifest limit and was not validated"
                ),
                DiagnosticMetadata::default().with_suggestion(
                    "keep the plugin manifest small; Codex manifests hold metadata, not content",
                ),
            );
            continue;
        }
        let value: Value = match serde_json::from_str(&content) {
            Ok(value) => value,
            Err(error) => {
                diag.report_at_with(
                    LintRule::CodexPluginManifestInvalid,
                    &display,
                    &format!("{display} is not valid JSON: {error}"),
                    json_parse_error_metadata(&error),
                );
                continue;
            }
        };
        diag.with_subject_path(&display, |diag| {
            validate_plugin_manifest_value(diag, &display, &content, &value);
        });
    }
}

fn validate_plugin_manifest_value(
    diag: &mut DiagnosticCollector,
    display: &str,
    source: &str,
    value: &Value,
) {
    let Some(root) = value.as_object() else {
        // CX047 owns a non-object root; do not cascade downstream rules.
        let metadata = field_metadata(
            source,
            &[],
            "root",
            &render_value(value),
            "make the manifest a JSON object with a \"name\" field",
        );
        diag.report_with(
            LintRule::CodexPluginManifestInvalid,
            &format!(
                "{display}: plugin manifest root must be a JSON object; found {}",
                type_name(value)
            ),
            metadata,
        );
        return;
    };
    validate_name(diag, display, source, root);
    validate_component_fields(diag, display, source, root);
    validate_description(diag, display, source, root);
    validate_interface(diag, display, source, root);
}

// ── CX048 / CX049: name ──────────────────────────────────────────────────

fn validate_name(
    diag: &mut DiagnosticCollector,
    display: &str,
    source: &str,
    root: &Map<String, Value>,
) {
    match root.get("name") {
        None => diag.report_with(
            LintRule::CodexPluginNameMissing,
            &format!("{display}: plugin manifest is missing the required `name` field"),
            DiagnosticMetadata::default()
                .with_suggestion("add a kebab-case \"name\", for example \"my-plugin\""),
        ),
        Some(Value::String(name)) if name.trim().is_empty() => diag.report_with(
            LintRule::CodexPluginNameMissing,
            &format!("{display}: plugin manifest `name` must not be blank"),
            field_metadata(
                source,
                &[Seg::Key("name")],
                "name",
                name,
                "set \"name\" to a kebab-case identifier such as \"my-plugin\"",
            ),
        ),
        Some(Value::String(name)) => {
            if !PLUGIN_NAME_PATTERN.is_match(name) {
                diag.report_with(
                    LintRule::CodexPluginNameInvalid,
                    &format!(
                        "{display}: plugin manifest `name` must be kebab-case (lowercase letters, digits, and single hyphens)"
                    ),
                    field_metadata(
                        source,
                        &[Seg::Key("name")],
                        "name", name,
                        "rename to kebab-case, for example \"my-plugin\"",
                    ),
                );
            }
        }
        Some(other) => report_type_error(
            diag,
            display,
            source,
            &[Seg::Key("name")],
            "name",
            other,
            "a string",
        ),
    }
}

// ── CX050 / CX051 / CX052 / CX047: component path fields ─────────────────

fn validate_component_fields(
    diag: &mut DiagnosticCollector,
    display: &str,
    source: &str,
    root: &Map<String, Value>,
) {
    // Path-bearing runtime fields with their exact accepted shapes. CX047 owns
    // every other present shape. Inline MCP and inline hook objects are valid
    // and path-checked only in their string forms; their semantics are owned by
    // #280 (MCP) and Codex itself (hook contents).
    validate_string_or_string_array(diag, display, source, root, "skills");
    validate_string_or_object(diag, display, source, root, "mcpServers");
    validate_string_only(diag, display, source, root, "apps");
    validate_string_or_string_array(diag, display, source, root, "commands");
    validate_hooks(diag, display, source, root);
}

fn validate_string_or_string_array(
    diag: &mut DiagnosticCollector,
    display: &str,
    source: &str,
    root: &Map<String, Value>,
    field: &str,
) {
    match root.get(field) {
        None => {}
        Some(Value::String(path)) => {
            check_component_path(diag, display, source, &[Seg::Key(field)], field, path);
        }
        Some(Value::Array(items)) => {
            check_string_array_items(diag, display, source, field, items);
        }
        Some(other) => report_type_error(
            diag,
            display,
            source,
            &[Seg::Key(field)],
            field,
            other,
            "a string or array of strings",
        ),
    }
}

fn validate_string_only(
    diag: &mut DiagnosticCollector,
    display: &str,
    source: &str,
    root: &Map<String, Value>,
    field: &str,
) {
    match root.get(field) {
        None => {}
        Some(Value::String(path)) => {
            check_component_path(diag, display, source, &[Seg::Key(field)], field, path);
        }
        Some(other) => report_type_error(
            diag,
            display,
            source,
            &[Seg::Key(field)],
            field,
            other,
            "a string",
        ),
    }
}

fn validate_string_or_object(
    diag: &mut DiagnosticCollector,
    display: &str,
    source: &str,
    root: &Map<String, Value>,
    field: &str,
) {
    match root.get(field) {
        None => {}
        Some(Value::String(path)) => {
            check_component_path(diag, display, source, &[Seg::Key(field)], field, path);
        }
        // An inline object is a valid form; its contents are validated elsewhere.
        Some(Value::Object(_)) => {}
        Some(other) => report_type_error(
            diag,
            display,
            source,
            &[Seg::Key(field)],
            field,
            other,
            "a string path or an inline object",
        ),
    }
}

fn validate_hooks(
    diag: &mut DiagnosticCollector,
    display: &str,
    source: &str,
    root: &Map<String, Value>,
) {
    match root.get("hooks") {
        None => {}
        Some(Value::String(path)) => {
            check_component_path(diag, display, source, &[Seg::Key("hooks")], "hooks", path);
        }
        // Inline hook object: valid; Codex owns its runtime schema.
        Some(Value::Object(_)) => {}
        Some(Value::Array(items)) => {
            for (index, item) in items.iter().take(MAX_VALIDATED_ARRAY_ELEMENTS).enumerate() {
                match item {
                    Value::String(path) => {
                        let label = format!("hooks[{index}]");
                        check_component_path(
                            diag,
                            display,
                            source,
                            &[Seg::Key("hooks"), Seg::Index(index)],
                            &label,
                            path,
                        );
                    }
                    // Inline hook object array entry: valid.
                    Value::Object(_) => {}
                    other => report_type_error(
                        diag,
                        display,
                        source,
                        &[Seg::Key("hooks"), Seg::Index(index)],
                        &format!("hooks[{index}]"),
                        other,
                        "a string path or an inline hook object",
                    ),
                }
            }
        }
        Some(other) => report_type_error(
            diag,
            display,
            source,
            &[Seg::Key("hooks")],
            "hooks",
            other,
            "a string, array, inline object, or array of inline objects",
        ),
    }
}

fn check_string_array_items(
    diag: &mut DiagnosticCollector,
    display: &str,
    source: &str,
    field: &str,
    items: &[Value],
) {
    for (index, item) in items.iter().take(MAX_VALIDATED_ARRAY_ELEMENTS).enumerate() {
        match item {
            Value::String(path) => {
                let label = format!("{field}[{index}]");
                check_component_path(
                    diag,
                    display,
                    source,
                    &[Seg::Key(field), Seg::Index(index)],
                    &label,
                    path,
                );
            }
            other => report_type_error(
                diag,
                display,
                source,
                &[Seg::Key(field), Seg::Index(index)],
                &format!("{field}[{index}]"),
                other,
                "a string path",
            ),
        }
    }
}

/// Classify a raw component path string and emit CX050/CX051/CX052 as needed.
fn check_component_path(
    diag: &mut DiagnosticCollector,
    display: &str,
    source: &str,
    path: &[Seg],
    label: &str,
    raw: &str,
) {
    let Some(defect) = classify_path(raw) else {
        return;
    };
    let (rule, message, suggestion) = match defect {
        PathDefect::MissingPrefix => (
            LintRule::CodexPluginPathPrefix,
            format!("{display}: {label} path must start with `./`"),
            "prefix the path with `./`, for example `./skills`",
        ),
        PathDefect::Bare => (
            LintRule::CodexPluginPathBare,
            format!("{display}: {label} path must reference a file or directory, not bare `./`"),
            "point the path at a specific file or directory inside the plugin",
        ),
        PathDefect::Traversal => (
            LintRule::CodexPluginPathTraversal,
            format!(
                "{display}: {label} path must stay inside the plugin root (no `..`, absolute, or rooted paths)"
            ),
            "use a `./`-relative path contained within the plugin root",
        ),
    };
    diag.report_with(
        rule,
        &message,
        field_metadata(source, path, label, raw, suggestion),
    );
}

// ── CX059: description ───────────────────────────────────────────────────

fn validate_description(
    diag: &mut DiagnosticCollector,
    display: &str,
    source: &str,
    root: &Map<String, Value>,
) {
    // Description is optional at runtime. A missing, blank, or non-string
    // description is a single install-surface quality warning (an agent-lint
    // recommendation, not a Codex load requirement).
    let (detail, metadata) = match root.get("description") {
        Some(Value::String(text)) if !text.trim().is_empty() => return,
        None => (
            "is missing".to_string(),
            DiagnosticMetadata::default()
                .with_suggestion("add a non-empty \"description\" for install-surface quality"),
        ),
        Some(Value::String(text)) => (
            "is blank".to_string(),
            field_metadata(
                source,
                &[Seg::Key("description")],
                "description",
                text,
                "add a non-empty \"description\" for install-surface quality",
            ),
        ),
        Some(other) => (
            format!("is {} rather than a string", type_name(other)),
            field_metadata(
                source,
                &[Seg::Key("description")],
                "description",
                &render_value(other),
                "set \"description\" to a non-empty string",
            ),
        ),
    };
    diag.report_with(
        LintRule::CodexPluginDescriptionMissing,
        &format!(
            "{display}: plugin manifest `description` {detail} (agent-lint install-surface recommendation)"
        ),
        metadata,
    );
}

// ── interface object ─────────────────────────────────────────────────────

fn validate_interface(
    diag: &mut DiagnosticCollector,
    display: &str,
    source: &str,
    root: &Map<String, Value>,
) {
    let interface = match root.get("interface") {
        None => return,
        Some(Value::Object(interface)) => interface,
        Some(other) => {
            return report_type_error(
                diag,
                display,
                source,
                &[Seg::Key("interface")],
                "interface",
                other,
                "a JSON object",
            );
        }
    };
    validate_default_prompt(diag, display, source, interface);
    validate_prompt_field_aliases(diag, display, source, interface);
    validate_interface_urls(diag, display, source, interface);
    validate_interface_assets(diag, display, source, interface);
}

// ── CX053 / CX054 / CX055 / CX047: default prompts ───────────────────────

fn validate_default_prompt(
    diag: &mut DiagnosticCollector,
    display: &str,
    source: &str,
    interface: &Map<String, Value>,
) {
    // Only the canonical `interface.defaultPrompt` is validated for prompt shape.
    let prompts: Vec<(Vec<Seg>, &str)> = match interface.get("defaultPrompt") {
        None => return,
        Some(Value::String(prompt)) => {
            vec![(
                vec![Seg::Key("interface"), Seg::Key("defaultPrompt")],
                prompt.as_str(),
            )]
        }
        Some(Value::Array(items)) => {
            let mut accepted = Vec::new();
            for (index, item) in items.iter().take(MAX_VALIDATED_ARRAY_ELEMENTS).enumerate() {
                match item {
                    Value::String(prompt) => accepted.push((
                        vec![
                            Seg::Key("interface"),
                            Seg::Key("defaultPrompt"),
                            Seg::Index(index),
                        ],
                        prompt.as_str(),
                    )),
                    other => report_type_error(
                        diag,
                        display,
                        source,
                        &[
                            Seg::Key("interface"),
                            Seg::Key("defaultPrompt"),
                            Seg::Index(index),
                        ],
                        &format!("interface.defaultPrompt[{index}]"),
                        other,
                        "a string",
                    ),
                }
            }
            accepted
        }
        Some(other) => {
            return report_type_error(
                diag,
                display,
                source,
                &[Seg::Key("interface"), Seg::Key("defaultPrompt")],
                "interface.defaultPrompt",
                other,
                "a string or array of strings",
            );
        }
    };

    if prompts.len() > MAX_DEFAULT_PROMPT_COUNT {
        diag.report_with(
            LintRule::CodexPluginDefaultPromptCount,
            &format!(
                "{display}: interface.defaultPrompt has {} entries; Codex accepts at most {MAX_DEFAULT_PROMPT_COUNT}",
                prompts.len()
            ),
            field_metadata(
                source,
                &[Seg::Key("interface"), Seg::Key("defaultPrompt")],
                "interface.defaultPrompt count",
                &prompts.len().to_string(),
                "keep at most three default prompts",
            ),
        );
    }

    for (path, prompt) in &prompts {
        let label = prompt_label(path);
        let normalized = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
        if normalized.is_empty() {
            diag.report_with(
                LintRule::CodexPluginDefaultPromptEmpty,
                &format!("{display}: {label} must not be empty after whitespace normalization"),
                field_metadata(
                    source,
                    path,
                    &label,
                    prompt,
                    "remove the empty prompt entry",
                ),
            );
        } else if normalized.chars().count() > MAX_DEFAULT_PROMPT_LEN {
            diag.report_with(
                LintRule::CodexPluginDefaultPromptLength,
                &format!(
                    "{display}: {label} exceeds Codex's {MAX_DEFAULT_PROMPT_LEN} Unicode-scalar limit"
                ),
                field_metadata(
                    source,
                    path,
                    &label, prompt,
                    "shorten this prompt to 128 characters or fewer",
                ),
            );
        }
    }
}

/// CX063: `interface.default_prompt` / `interface.default_prompts` are read by
/// no Codex runtime; each present ignored key is one migration warning.
fn validate_prompt_field_aliases(
    diag: &mut DiagnosticCollector,
    display: &str,
    source: &str,
    interface: &Map<String, Value>,
) {
    for key in ["default_prompt", "default_prompts"] {
        let Some(value) = interface.get(key) else {
            continue;
        };
        diag.report_with(
            LintRule::CodexPluginPromptField,
            &format!(
                "{display}: interface.{key} is ignored by Codex; the runtime field is interface.defaultPrompt"
            ),
            field_metadata(
                source,
                &[Seg::Key("interface"), Seg::Key(key)],
                &format!("interface.{key}"),
                &render_value(value),
                "rename the key to interface.defaultPrompt",
            ),
        );
    }
}

// ── CX056 / CX047: interface URLs ────────────────────────────────────────

fn validate_interface_urls(
    diag: &mut DiagnosticCollector,
    display: &str,
    source: &str,
    interface: &Map<String, Value>,
) {
    // Each logical URL is accepted under both runtime spellings. The public
    // `...URL` spelling is preferred in suggestions. A wrong type is CX047; an
    // unusable URL string is a CX056 publishing-metadata warning.
    for (canonical, alias) in [
        ("websiteURL", "websiteUrl"),
        ("privacyPolicyURL", "privacyPolicyUrl"),
        ("termsOfServiceURL", "termsOfServiceUrl"),
    ] {
        for spelling in [canonical, alias] {
            let Some(value) = interface.get(spelling) else {
                continue;
            };
            let path = [Seg::Key("interface"), Seg::Key(spelling)];
            match value {
                Value::String(url) if is_valid_publish_url(url) => {}
                Value::String(url) => {
                    let label = format!("interface.{spelling}");
                    let base = DiagnosticMetadata::default().with_suggestion(format!(
                        "set interface.{canonical} to an absolute https:// URL"
                    ));
                    // Embedded credentials are secret-like but not a recognized
                    // token format, so redact rather than echo them in evidence.
                    let has_credentials = Url::parse(url).is_ok_and(|parsed| {
                        !parsed.username().is_empty() || parsed.password().is_some()
                    });
                    let metadata = if has_credentials {
                        base.with_redacted_evidence()
                    } else {
                        attach_evidence(base, &label, url)
                    }
                    .maybe_location(source, &path);
                    diag.report_with(
                        LintRule::CodexPluginInterfaceUrl,
                        &format!(
                            "{display}: interface.{spelling} must be an absolute https:// URL with a host, no embedded credentials, and at most {MAX_INTERFACE_URL_LEN} characters"
                        ),
                        metadata,
                    );
                }
                other => report_type_error(
                    diag,
                    display,
                    source,
                    &path,
                    &format!("interface.{spelling}"),
                    other,
                    "a string",
                ),
            }
        }
    }
}

// ── CX057 / CX047: interface asset paths ─────────────────────────────────

fn validate_interface_assets(
    diag: &mut DiagnosticCollector,
    display: &str,
    source: &str,
    interface: &Map<String, Value>,
) {
    for field in ["composerIcon", "logo", "logoDark"] {
        match interface.get(field) {
            None => {}
            Some(Value::String(path)) => check_asset_path(
                diag,
                display,
                source,
                &[Seg::Key("interface"), Seg::Key(field)],
                &format!("interface.{field}"),
                path,
            ),
            Some(other) => report_type_error(
                diag,
                display,
                source,
                &[Seg::Key("interface"), Seg::Key(field)],
                &format!("interface.{field}"),
                other,
                "a string",
            ),
        }
    }
    match interface.get("screenshots") {
        None => {}
        Some(Value::Array(items)) => {
            for (index, item) in items.iter().take(MAX_VALIDATED_ARRAY_ELEMENTS).enumerate() {
                match item {
                    Value::String(path) => check_asset_path(
                        diag,
                        display,
                        source,
                        &[
                            Seg::Key("interface"),
                            Seg::Key("screenshots"),
                            Seg::Index(index),
                        ],
                        &format!("interface.screenshots[{index}]"),
                        path,
                    ),
                    other => report_type_error(
                        diag,
                        display,
                        source,
                        &[
                            Seg::Key("interface"),
                            Seg::Key("screenshots"),
                            Seg::Index(index),
                        ],
                        &format!("interface.screenshots[{index}]"),
                        other,
                        "a string",
                    ),
                }
            }
        }
        Some(other) => report_type_error(
            diag,
            display,
            source,
            &[Seg::Key("interface"), Seg::Key("screenshots")],
            "interface.screenshots",
            other,
            "an array of strings",
        ),
    }
}

/// Asset paths use the same containment contract as component paths, but every
/// defect maps to the single CX057 identity.
fn check_asset_path(
    diag: &mut DiagnosticCollector,
    display: &str,
    source: &str,
    path: &[Seg],
    label: &str,
    raw: &str,
) {
    if classify_path(raw).is_none() {
        return;
    }
    diag.report_with(
        LintRule::CodexPluginInterfaceAssetPath,
        &format!(
            "{display}: {label} must be a `./`-relative path inside the plugin root (not bare `./`, absolute, or traversing)"
        ),
        field_metadata(
            source,
            path,
            label, raw,
            "use a `./`-relative asset path such as `./assets/logo.svg`",
        ),
    );
}

// ── path containment ─────────────────────────────────────────────────────

enum PathDefect {
    MissingPrefix,
    Bare,
    Traversal,
}

/// Classify a raw path string exactly as written (no trimming). A containment
/// escape is reported ahead of a missing prefix so the most severe defect wins.
fn classify_path(raw: &str) -> Option<PathDefect> {
    if raw == "./" {
        return Some(PathDefect::Bare);
    }
    let stripped = raw.strip_prefix("./").unwrap_or(raw);
    if escapes_containment(stripped) {
        return Some(PathDefect::Traversal);
    }
    if !raw.starts_with("./") {
        return Some(PathDefect::MissingPrefix);
    }
    None
}

/// Whether a path (with at most one leading `./` already stripped) is rooted or
/// escapes its containing directory under POSIX or Windows separators.
fn escapes_containment(path: &str) -> bool {
    // POSIX root or leading backslash / UNC root.
    if path.starts_with('/') || path.starts_with('\\') {
        return true;
    }
    // Windows drive-letter form, e.g. `C:` or `C:\` or `C:/`.
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return true;
    }
    // Any parent-directory segment under either separator.
    path.split(['/', '\\']).any(|segment| segment == "..")
}

/// Absolute HTTPS URL with a host, no embedded credentials, bounded length.
fn is_valid_publish_url(value: &str) -> bool {
    value.chars().count() <= MAX_INTERFACE_URL_LEN
        && Url::parse(value).is_ok_and(|url| {
            url.scheme() == "https"
                && url.host_str().is_some_and(|host| !host.is_empty())
                && url.username().is_empty()
                && url.password().is_none()
        })
}

// ── diagnostic metadata helpers ──────────────────────────────────────────

fn report_type_error(
    diag: &mut DiagnosticCollector,
    display: &str,
    source: &str,
    path: &[Seg],
    label: &str,
    value: &Value,
    expected: &str,
) {
    diag.report_with(
        LintRule::CodexPluginManifestInvalid,
        &format!(
            "{display}: {label} must be {expected}; found {}",
            type_name(value)
        ),
        field_metadata(
            source,
            path,
            label,
            &render_value(value),
            &format!("set {label} to {expected}"),
        ),
    );
}

fn field_metadata(
    source: &str,
    path: &[Seg],
    label: &str,
    value: &str,
    suggestion: &str,
) -> DiagnosticMetadata {
    attach_evidence(
        DiagnosticMetadata::default().with_suggestion(suggestion),
        label,
        value,
    )
    .maybe_location(source, path)
}

/// Attach bounded `label = value` evidence, redacting the entire value when the
/// shared secret classifier flags the *untruncated* value — so a secret cannot
/// survive by sitting past the 80-scalar truncation window.
fn attach_evidence(metadata: DiagnosticMetadata, label: &str, value: &str) -> DiagnosticMetadata {
    if crate::sensitive::contains_sensitive_evidence(value) {
        metadata.with_redacted_evidence()
    } else {
        metadata.with_evidence(format!(
            "{label} = {}",
            truncate_scalars(value, MAX_EVIDENCE_SCALARS)
        ))
    }
}

trait MaybeLocation {
    fn maybe_location(self, source: &str, path: &[Seg]) -> Self;
}

impl MaybeLocation for DiagnosticMetadata {
    fn maybe_location(self, source: &str, path: &[Seg]) -> Self {
        match JsonScanner::locate(source, path)
            .and_then(|range| SourceSpan::from_byte_range(source, range))
        {
            Some(span) => self.with_location(span),
            None => self,
        }
    }
}

fn json_parse_error_metadata(error: &serde_json::Error) -> DiagnosticMetadata {
    let metadata =
        DiagnosticMetadata::default().with_suggestion("fix the JSON syntax so the manifest parses");
    if error.line() == 0 {
        return metadata;
    }
    let span = if error.column() > 0 {
        SourceSpan::point(error.line(), error.column())
    } else {
        SourceSpan::line(error.line())
    };
    metadata.with_location(span)
}

fn render_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn truncate_scalars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut result: String = text.chars().take(max).collect();
    result.push('…');
    result
}

fn prompt_label(path: &[Seg]) -> String {
    match path.last() {
        Some(Seg::Index(index)) => format!("interface.defaultPrompt[{index}]"),
        _ => "interface.defaultPrompt".to_string(),
    }
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

// ── minimal JSON value locator ───────────────────────────────────────────

/// One access-path segment used to locate a JSON value's source span.
#[derive(Clone, Copy)]
enum Seg<'a> {
    Key(&'a str),
    Index(usize),
}

/// Best-effort source-span locator for an already-parsed JSON document.
///
/// The document is known to parse (serde accepted it), so scanning stays lenient
/// and returns `None` rather than failing when a shape is unexpected; a missing
/// location simply omits the optional metadata. On a duplicate key the first
/// occurrence is located.
struct JsonScanner<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> JsonScanner<'a> {
    fn locate(source: &'a str, path: &[Seg]) -> Option<Range<usize>> {
        let mut scanner = Self {
            bytes: source.as_bytes(),
            pos: 0,
        };
        scanner.skip_ws();
        scanner.value_range(path)
    }

    fn value_range(&mut self, path: &[Seg]) -> Option<Range<usize>> {
        let Some((first, rest)) = path.split_first() else {
            let start = self.pos;
            self.skip_value()?;
            return Some(start..self.pos);
        };
        match *first {
            Seg::Key(key) => self.descend_object(key, rest),
            Seg::Index(index) => self.descend_array(index, rest),
        }
    }

    fn descend_object(&mut self, wanted: &str, rest: &[Seg]) -> Option<Range<usize>> {
        if self.take(b'{').is_none() {
            self.skip_value();
            return None;
        }
        loop {
            self.skip_ws();
            if self.take(b'}').is_some() {
                return None;
            }
            let key = self.parse_string()?;
            self.skip_ws();
            self.take(b':')?;
            self.skip_ws();
            if key == wanted {
                return self.value_range(rest);
            }
            self.skip_value()?;
            self.skip_ws();
            self.take(b',');
        }
    }

    fn descend_array(&mut self, wanted: usize, rest: &[Seg]) -> Option<Range<usize>> {
        if self.take(b'[').is_none() {
            self.skip_value();
            return None;
        }
        let mut index = 0;
        loop {
            self.skip_ws();
            if self.take(b']').is_some() {
                return None;
            }
            if index == wanted {
                return self.value_range(rest);
            }
            self.skip_value()?;
            self.skip_ws();
            self.take(b',');
            index += 1;
        }
    }

    fn skip_value(&mut self) -> Option<()> {
        self.skip_ws();
        match self.bytes.get(self.pos)? {
            b'"' => self.skip_string(),
            b'{' => self.skip_object(),
            b'[' => self.skip_array(),
            _ => self.skip_scalar(),
        }
    }

    fn skip_object(&mut self) -> Option<()> {
        self.take(b'{')?;
        loop {
            self.skip_ws();
            if self.take(b'}').is_some() {
                return Some(());
            }
            self.skip_string()?;
            self.skip_ws();
            self.take(b':')?;
            self.skip_value()?;
            self.skip_ws();
            self.take(b',');
        }
    }

    fn skip_array(&mut self) -> Option<()> {
        self.take(b'[')?;
        loop {
            self.skip_ws();
            if self.take(b']').is_some() {
                return Some(());
            }
            self.skip_value()?;
            self.skip_ws();
            self.take(b',');
        }
    }

    fn skip_scalar(&mut self) -> Option<()> {
        while let Some(byte) = self.bytes.get(self.pos) {
            if matches!(byte, b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r') {
                break;
            }
            self.pos += 1;
        }
        Some(())
    }

    fn skip_string(&mut self) -> Option<()> {
        self.take(b'"')?;
        loop {
            let byte = *self.bytes.get(self.pos)?;
            self.pos += 1;
            match byte {
                b'"' => return Some(()),
                b'\\' => self.pos += 1,
                _ => {}
            }
        }
    }

    fn parse_string(&mut self) -> Option<String> {
        self.take(b'"')?;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let byte = *self.bytes.get(self.pos)?;
            self.pos += 1;
            match byte {
                b'"' => return String::from_utf8(out).ok(),
                b'\\' => {
                    let escape = *self.bytes.get(self.pos)?;
                    self.pos += 1;
                    match escape {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'b' => out.push(0x08),
                        b'f' => out.push(0x0c),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            let hex = self.bytes.get(self.pos..self.pos + 4)?;
                            self.pos += 4;
                            let code =
                                u32::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
                            let mut buffer = [0u8; 4];
                            let encoded = char::from_u32(code)
                                .unwrap_or('\u{fffd}')
                                .encode_utf8(&mut buffer);
                            out.extend_from_slice(encoded.as_bytes());
                        }
                        _ => return None,
                    }
                }
                _ => out.push(byte),
            }
        }
    }

    fn skip_ws(&mut self) {
        while matches!(self.bytes.get(self.pos), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn take(&mut self, byte: u8) -> Option<()> {
        if self.bytes.get(self.pos) == Some(&byte) {
            self.pos += 1;
            Some(())
        } else {
            None
        }
    }
}

// ── Codex skill frontmatter (CX060) ──────────────────────────────────────

fn validate_codex_skill_frontmatter(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let root = Path::new(".agents/skills");
    if !root.is_dir() {
        return;
    }
    for entry in traversal::recursive_files(root, Path::new("."), Some(exclude)).entries {
        if entry.path.file_name().is_none_or(|name| name != "SKILL.md") {
            continue;
        }
        let path = &entry.path;
        let display = entry.display;
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        let Some(lines) = frontmatter::extract_frontmatter(&content) else {
            continue;
        };
        for line in lines {
            let Some((field, _)) = line.split_once(':') else {
                continue;
            };
            let field = field.trim();
            if CODEX_SKILL_UNSUPPORTED_FIELDS.contains(&field) {
                diag.report_at(LintRule::CodexSkillUnsupportedFrontmatter, &display, &format!("{display}: `{field}` is Claude-only skill frontmatter unsupported by Codex CLI"));
            }
        }
    }
}

#[cfg(test)]
mod tests;
