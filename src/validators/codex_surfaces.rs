//! Validation for Codex plugin manifests and skills.
//!
//! Codex plugin manifest linting is rebuilt around Codex's real manifest
//! discovery and schema. Classification of checks:
//!
//! - Runtime compatibility (parse/shape/path/prompt): CX047, CX050–CX055.
//! - Public authoring policy: CX048 (name present), CX049 (kebab-case name).
//! - Publishing / install quality: CX056 (interface URLs), CX057 (asset paths),
//!   CX059 (description), CX063 (ignored prompt-key migration aid).
//! - Skill frontmatter compatibility (CX060): ignored Claude / Agent Skills
//!   behavior fields on `.agents/skills/<skill>/SKILL.md` and selected plugin
//!   skill roots; strict top-level YAML only; default warning, non-autofixable.
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

use crate::config::{ExcludeSet, normalize_path};
use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata, SourceSpan};
use crate::frontmatter;
use crate::live_instructions::{InstructionSurfaceKind, LiveInstructionDocument};
use crate::markdown::MarkdownDocument;
use crate::plugin_paths::safe_component_path;
use crate::rules::LintRule;
use crate::traversal;
use crate::validators::json_locate::{JsonScanner, Seg};
use regex::Regex;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
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
/// Fixed CX060 set: Agent Skills experimental `allowed-tools` plus Claude
/// behavior fields that Codex's skill loader ignores (reads only `name`,
/// `description`, and `metadata.short-description`). Verified against
/// openai/codex `7442f5f` `core-skills` frontmatter parsing and the Agent
/// Skills / Claude Code frontmatter references on 2026-07-21.
const CODEX_SKILL_UNSUPPORTED_FIELDS: &[&str] = &[
    "allowed-tools",
    "when_to_use",
    "argument-hint",
    "arguments",
    "disable-model-invocation",
    "user-invocable",
    "model",
    "effort",
    "context",
    "agent",
    "hooks",
    "paths",
    "shell",
];

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
    validate_override_prompt(diag, exclude, prompt_pass);
    validate_plugin_manifests(diag, exclude);
    validate_codex_skill_frontmatter(diag, exclude);
}

/// Prompt-content rules retain their long-standing root override surface.
/// CX040/CX045 selection is separately owned by `instruction_files`.
fn validate_override_prompt(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    prompt_pass: &mut super::prompt_content::PromptContentPass,
) {
    let path = "AGENTS.override.md";
    if exclude.is_excluded(path) || !Path::new(path).is_file() {
        return;
    }
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    let markdown = MarkdownDocument::parse_body(content);
    let document = LiveInstructionDocument::new(
        Path::new(path),
        InstructionSurfaceKind::CodexAgentsOverride,
        &markdown,
    );
    diag.with_subject_path(path, |diag| prompt_pass.validate(&document, diag));
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
            for (index, item) in items.iter().enumerate() {
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
    for (index, item) in items.iter().enumerate() {
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
            for (index, item) in items.iter().enumerate() {
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
            for (index, item) in items.iter().enumerate() {
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

// ── Codex skill frontmatter (CX060) ──────────────────────────────────────

/// CX060: warn when a Codex-discovered skill declares behavior fields that
/// Codex's loader ignores. Discovery covers every repository-owned
/// `.agents/skills/<skill>/SKILL.md` tree and every selected plugin skill root
/// from [`crate::platforms::codex_plugin_manifests`] (declared `skills` paths
/// when non-empty, otherwise default `skills/`). Invalid YAML is owned by X001;
/// CX060 inspects only a successfully parsed top-level mapping.
fn validate_codex_skill_frontmatter(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    for (display, path) in discover_codex_skill_files(exclude) {
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Some(lines) = frontmatter::extract_frontmatter(&content) else {
            continue;
        };
        let Ok(yaml) = frontmatter::parse_yaml_strict(&lines) else {
            continue;
        };
        let Some(mapping) = yaml.as_mapping() else {
            continue;
        };
        for (key, value) in mapping.iter() {
            let field = key.as_str();
            if !CODEX_SKILL_UNSUPPORTED_FIELDS.contains(&field) {
                continue;
            }
            let (message, suggestion) = unsupported_skill_field_guidance(&display, field);
            let mut metadata = DiagnosticMetadata::default()
                .with_evidence(format!("{field} ({})", yaml_value_type(value)))
                .with_suggestion(suggestion);
            if let Some((line, column)) = top_level_key_location(&lines, field) {
                metadata = metadata.with_location(SourceSpan::point(line, column));
            }
            diag.report_at_with(
                LintRule::CodexSkillUnsupportedFrontmatter,
                &display,
                &message,
                metadata,
            );
        }
    }
}

/// Deterministic Codex skill inventory: repository `.agents/skills` trees plus
/// selected-plugin skill roots. Ordered by physical display path.
fn discover_codex_skill_files(exclude: &ExcludeSet) -> Vec<(String, PathBuf)> {
    let mut by_display: BTreeMap<String, PathBuf> = BTreeMap::new();
    for entry in crate::platforms::agent_skill_candidates(exclude) {
        if !is_direct_skill_md_under_agents_skills(&entry.path) {
            continue;
        }
        by_display.insert(entry.display, entry.path);
    }
    for skills_root in selected_plugin_skill_roots(exclude) {
        for entry in skill_md_files_under_root(&skills_root, exclude) {
            by_display.insert(entry.display, entry.path);
        }
    }
    by_display.into_iter().collect()
}

/// `…/.agents/skills/<skill>/SKILL.md` only — not deeper fixture/example copies
/// nested under a skill directory.
fn is_direct_skill_md_under_agents_skills(path: &Path) -> bool {
    let components: Vec<_> = path.components().collect();
    if components.len() < 4 {
        return false;
    }
    matches!(
        &components[components.len() - 4..],
        [
            Component::Normal(agents),
            Component::Normal(skills),
            Component::Normal(_skill),
            Component::Normal(file)
        ] if *agents == ".agents" && *skills == "skills" && *file == "SKILL.md"
    )
}

/// Skill roots for every selected Codex plugin manifest. Consumes
/// [`crate::platforms::codex_plugin_manifests`] for precedence/exclusions and
/// [`safe_component_path`] for path safety — no second resolver.
fn selected_plugin_skill_roots(exclude: &ExcludeSet) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for manifest in crate::platforms::codex_plugin_manifests(exclude) {
        let plugin_root = manifest
            .path
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let Ok(content) = std::fs::read_to_string(&manifest.path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&content) else {
            continue;
        };
        let declared = declared_plugin_skill_roots(&plugin_root, &value);
        if declared.is_empty() {
            roots.push(plugin_root.join("skills"));
        } else {
            roots.extend(declared);
        }
    }
    roots.sort();
    roots.dedup();
    roots
}

/// Safe `skills` string / string-array declarations relative to the plugin
/// root. Empty or all-unsafe declarations yield an empty list so the caller
/// can fall back to default `skills/` (Codex replacement semantics).
fn declared_plugin_skill_roots(plugin_root: &Path, value: &Value) -> Vec<PathBuf> {
    let Some(skills) = value.get("skills") else {
        return Vec::new();
    };
    let raw_paths: Vec<&str> = match skills {
        Value::String(path) => vec![path.as_str()],
        Value::Array(items) => items.iter().filter_map(Value::as_str).collect(),
        _ => return Vec::new(),
    };
    raw_paths
        .into_iter()
        .filter_map(safe_component_path)
        .map(|relative| plugin_root.join(relative))
        .collect()
}

fn skill_md_files_under_root(
    skills_root: &Path,
    exclude: &ExcludeSet,
) -> Vec<traversal::WalkEntry> {
    if !skills_root.is_dir() {
        return Vec::new();
    }
    let mut files = Vec::new();
    for dir in traversal::shallow_directories(skills_root, Path::new("."), Some(exclude)).entries {
        let path = dir.path.join("SKILL.md");
        if !path.is_file() {
            continue;
        }
        let display = normalize_path(&format!("{}/SKILL.md", dir.display));
        if exclude.is_excluded(&display) {
            continue;
        }
        files.push(traversal::WalkEntry { path, display });
    }
    files.sort_by(|left, right| left.display.cmp(&right.display));
    files
}

fn unsupported_skill_field_guidance(display: &str, field: &str) -> (String, String) {
    match field {
        "when_to_use" => (
            format!(
                "{display}: `when_to_use` is ignored by Codex and does not control skill selection"
            ),
            "merge the trigger text into `description`".to_string(),
        ),
        "allowed-tools" => (
            format!(
                "{display}: `allowed-tools` is ignored by Codex and does not grant tool permission"
            ),
            "remove `allowed-tools` and rely on Codex sandbox/approval configuration".to_string(),
        ),
        "disable-model-invocation" => (
            format!(
                "{display}: `disable-model-invocation` is ignored by Codex and does not control implicit invocation"
            ),
            "move invocation policy to `agents/openai.yaml` (`policy.allow_implicit_invocation`)"
                .to_string(),
        ),
        "user-invocable" => (
            format!(
                "{display}: `user-invocable` is ignored by Codex; `user-invocable: false` has no Codex equivalent"
            ),
            "remove `user-invocable`; use `agents/openai.yaml` (`policy.allow_implicit_invocation`) only when that mapping applies"
                .to_string(),
        ),
        "argument-hint" | "arguments" | "model" | "effort" | "context" | "agent" | "hooks"
        | "paths" | "shell" => (
            format!(
                "{display}: `{field}` is ignored by Codex and does not enforce the declared control"
            ),
            format!(
                "remove `{field}` and move the required behavior into skill instructions or repository Codex configuration"
            ),
        ),
        other => (
            format!("{display}: `{other}` is ignored by Codex"),
            format!("remove `{other}`"),
        ),
    }
}

fn yaml_value_type(value: &crate::yaml::Value) -> &'static str {
    if value.is_null() {
        "null"
    } else if value.as_str().is_some() {
        "string"
    } else if value.as_bool().is_some() {
        "boolean"
    } else if value.as_i64().is_some() || value.as_u64().is_some() || value.as_f64().is_some() {
        "number"
    } else if value.as_sequence().is_some() {
        "sequence"
    } else if value.as_mapping().is_some() {
        "mapping"
    } else {
        "value"
    }
}

/// Locate a top-level YAML mapping key in frontmatter lines.
///
/// Returns 1-based file line and Unicode-scalar column of the key's first
/// source character. Agrees with the strict parser for plain keys (any legal
/// spacing before `:`), single/double/escaped quoted keys, root flow mappings,
/// CRLF-normalized lines, and Unicode content before the key token. Nested
/// block mappings and value-level flow maps are ignored.
fn top_level_key_location(fm_lines: &[String], key: &str) -> Option<(usize, usize)> {
    if fm_lines_are_root_flow_mapping(fm_lines) {
        return flow_mapping_key_location(fm_lines, key);
    }
    block_top_level_key_location(fm_lines, key)
}

fn fm_lines_are_root_flow_mapping(fm_lines: &[String]) -> bool {
    fm_lines
        .iter()
        .map(|line| line.trim_start_matches('\u{feff}'))
        .find(|line| {
            let trimmed = line.trim_start();
            !trimmed.is_empty() && !trimmed.starts_with('#')
        })
        .is_some_and(|line| line.trim_start().starts_with('{'))
}

fn block_top_level_key_location(fm_lines: &[String], key: &str) -> Option<(usize, usize)> {
    for (index, line) in fm_lines.iter().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let mut start = 0usize;
        if chars.first() == Some(&'\u{feff}') {
            start = 1;
        }
        // Nested block entries are indented; skip them (and comments/blank).
        if chars.get(start).is_some_and(|ch| ch.is_whitespace()) {
            continue;
        }
        let Some(rest) = chars.get(start..) else {
            continue;
        };
        if rest.first() == Some(&'#') || rest.is_empty() {
            continue;
        }
        if let Some(column) = mapping_key_column(rest, key) {
            return Some((index + 2, start + column));
        }
    }
    None
}

fn flow_mapping_key_location(fm_lines: &[String], key: &str) -> Option<(usize, usize)> {
    let mut flow_depth = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    let mut escape = false;
    let mut expect_key = false;

    for (line_idx, line) in fm_lines.iter().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let mut idx = 0usize;
        while idx < chars.len() {
            let ch = chars[idx];
            if in_double {
                if escape {
                    escape = false;
                } else if ch == '\\' {
                    escape = true;
                } else if ch == '"' {
                    in_double = false;
                }
                idx += 1;
                continue;
            }
            if in_single {
                if ch == '\'' {
                    if chars.get(idx + 1) == Some(&'\'') {
                        idx += 2;
                        continue;
                    }
                    in_single = false;
                }
                idx += 1;
                continue;
            }
            match ch {
                '#' if flow_depth == 0 => break,
                '"' => {
                    if expect_key && flow_depth == 1 {
                        if let Some(column) = mapping_key_column(&chars[idx..], key) {
                            return Some((line_idx + 2, idx + column));
                        }
                    }
                    in_double = true;
                    expect_key = false;
                    idx += 1;
                }
                '\'' => {
                    if expect_key && flow_depth == 1 {
                        if let Some(column) = mapping_key_column(&chars[idx..], key) {
                            return Some((line_idx + 2, idx + column));
                        }
                    }
                    in_single = true;
                    expect_key = false;
                    idx += 1;
                }
                '{' => {
                    flow_depth += 1;
                    expect_key = flow_depth == 1;
                    idx += 1;
                }
                '}' => {
                    flow_depth = flow_depth.saturating_sub(1);
                    expect_key = false;
                    idx += 1;
                }
                '[' => {
                    flow_depth += 1;
                    expect_key = false;
                    idx += 1;
                }
                ']' => {
                    flow_depth = flow_depth.saturating_sub(1);
                    expect_key = false;
                    idx += 1;
                }
                ',' if flow_depth == 1 => {
                    expect_key = true;
                    idx += 1;
                }
                ':' if flow_depth == 1 => {
                    expect_key = false;
                    idx += 1;
                }
                ch if ch.is_whitespace() => idx += 1,
                _ if expect_key && flow_depth == 1 => {
                    if let Some(column) = mapping_key_column(&chars[idx..], key) {
                        return Some((line_idx + 2, idx + column));
                    }
                    // Consume a non-matching key token so later keys still scan.
                    expect_key = false;
                    idx += 1;
                }
                _ => idx += 1,
            }
        }
    }
    None
}

/// If `chars` begins with mapping key `key` followed by optional whitespace and
/// `:`, return the 1-based Unicode-scalar column of the key's first character
/// within `chars`.
fn mapping_key_column(chars: &[char], key: &str) -> Option<usize> {
    let (decoded, key_end) = parse_yaml_key_token(chars)?;
    if decoded != key {
        return None;
    }
    let mut colon_at = key_end;
    while colon_at < chars.len() && matches!(chars[colon_at], ' ' | '\t') {
        colon_at += 1;
    }
    if chars.get(colon_at) != Some(&':') {
        return None;
    }
    Some(1)
}

/// Parse a YAML mapping-key token at the start of `chars`.
/// Returns the decoded key text and the index just past the token.
fn parse_yaml_key_token(chars: &[char]) -> Option<(String, usize)> {
    match chars.first()? {
        '"' => parse_double_quoted_key(chars),
        '\'' => parse_single_quoted_key(chars),
        '#' | '{' | '}' | '[' | ']' | ',' | ':' => None,
        _ => parse_plain_key(chars),
    }
}

fn parse_plain_key(chars: &[char]) -> Option<(String, usize)> {
    let mut end = 0usize;
    while end < chars.len() {
        let ch = chars[end];
        if ch == ':' || ch == '#' || ch == ',' || ch == '{' || ch == '}' || ch == '[' || ch == ']' {
            break;
        }
        if ch == ' ' || ch == '\t' {
            // Allow internal spaces only when more key text follows before `:`.
            let mut look = end;
            while look < chars.len() && matches!(chars[look], ' ' | '\t') {
                look += 1;
            }
            if chars.get(look) == Some(&':') || look == chars.len() {
                break;
            }
        }
        end += 1;
    }
    if end == 0 {
        return None;
    }
    let key: String = chars[..end].iter().collect();
    if key.chars().all(|ch| ch.is_whitespace()) {
        return None;
    }
    Some((key, end))
}

fn parse_single_quoted_key(chars: &[char]) -> Option<(String, usize)> {
    if chars.first() != Some(&'\'') {
        return None;
    }
    let mut out = String::new();
    let mut idx = 1usize;
    while idx < chars.len() {
        let ch = chars[idx];
        if ch == '\'' {
            if chars.get(idx + 1) == Some(&'\'') {
                out.push('\'');
                idx += 2;
                continue;
            }
            return Some((out, idx + 1));
        }
        out.push(ch);
        idx += 1;
    }
    None
}

fn parse_double_quoted_key(chars: &[char]) -> Option<(String, usize)> {
    if chars.first() != Some(&'"') {
        return None;
    }
    let mut out = String::new();
    let mut idx = 1usize;
    while idx < chars.len() {
        let ch = chars[idx];
        if ch == '"' {
            return Some((out, idx + 1));
        }
        if ch == '\\' {
            idx += 1;
            let escaped = chars.get(idx)?;
            match escaped {
                '0' => out.push('\0'),
                'a' => out.push('\u{07}'),
                'b' => out.push('\u{08}'),
                't' => out.push('\t'),
                'n' => out.push('\n'),
                'v' => out.push('\u{0b}'),
                'f' => out.push('\u{0c}'),
                'r' => out.push('\r'),
                'e' => out.push('\u{1b}'),
                ' ' => out.push(' '),
                '"' => out.push('"'),
                '/' => out.push('/'),
                '\\' => out.push('\\'),
                'N' => out.push('\u{85}'),
                '_' => out.push('\u{a0}'),
                'L' => out.push('\u{2028}'),
                'P' => out.push('\u{2029}'),
                'x' => {
                    let value = parse_hex_escape(chars, idx + 1, 2)?;
                    out.push(char::from_u32(value)?);
                    idx += 2;
                }
                'u' => {
                    let value = parse_hex_escape(chars, idx + 1, 4)?;
                    out.push(char::from_u32(value)?);
                    idx += 4;
                }
                'U' => {
                    let value = parse_hex_escape(chars, idx + 1, 8)?;
                    out.push(char::from_u32(value)?);
                    idx += 8;
                }
                other => out.push(*other),
            }
            idx += 1;
            continue;
        }
        out.push(ch);
        idx += 1;
    }
    None
}

fn parse_hex_escape(chars: &[char], start: usize, width: usize) -> Option<u32> {
    let slice = chars.get(start..start + width)?;
    let text: String = slice.iter().collect();
    u32::from_str_radix(&text, 16).ok()
}

#[cfg(test)]
mod tests;
