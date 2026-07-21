use crate::config::ExcludeSet;
use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata};
use crate::frontmatter;
use crate::live_instructions::{InstructionSurfaceKind, LiveInstructionDocument};
use crate::markdown::MarkdownDocument;
use crate::rules::LintRule;
use crate::traversal;
use regex::Regex;
use std::collections::HashSet;
use std::fs;
use std::num::NonZeroU64;
use std::path::Path;
use std::sync::LazyLock;

use super::common::{RE_NAME_INVALID, has_bound_or_fallback, is_known_tool_name, sentence_ranges};

/// Jaccard similarity threshold (strict greater-than).
const JACCARD_THRESHOLD: f64 = 0.8;
/// Descriptions with fewer than this many words are eligible for Jaccard flagging.
const MIN_DESC_WORDS: usize = 6;

const STOPWORDS: &[&str] = &[
    "a", "an", "the", "is", "for", "and", "of", "to", "that", "which",
];

/// Agent frontmatter fields that Claude Code does not honor for agents shipped
/// by a plugin. Revisit as plugin agent support matures.
const UNSUPPORTED_PLUGIN_FIELDS: &[&str] = &["hooks", "mcpServers", "permissionMode"];

/// Tools which can carry out iterative work, mutate a repository, access an
/// external system, or delegate work to another agent. Read-only discovery and
/// task-status tools are deliberately absent: declaring metadata or inspection
/// capabilities alone is not enough to trigger A029.
const EXECUTION_TOOLS: &[&str] = &[
    "Agent",
    "Bash",
    "Edit",
    "NotebookEdit",
    "Task",
    "WebFetch",
    "WebSearch",
    "Write",
];

/// A029 controls must be addressed to the current agent, not merely describe
/// a historical agent or an example. Keep this intentionally narrow: these
/// forms cover the documented direct imperative, subject directive, and
/// conditional instruction styles without guessing at descriptive prose.
static OPERATIVE_CONTROL_PREFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?ix)^\s*(?:[-*+]\s*)?(?:
            (?:you|the\s+agent|this\s+agent|agents?|assistant|model)\s+
                (?:must|should|shall|will|need\s+to|are\s+to)\b |
            (?:if|when|after|before|unless|for|during|on|upon)\b[^.!?;,:]*[,;:]\s*
                (?:(?:you|the\s+agent|this\s+agent|agents?|assistant|model)\s+
                    (?:must|should|shall|will|need\s+to|are\s+to)\s+)?
                (?:make|use|set|limit|cap|retry|abort|give\s+up|halt|stop|report|escalate|ask|handoff|return)\b |
            (?:make|use|set|limit|cap|retry|abort|give\s+up|halt|stop|report|escalate|ask|handoff|return)\b |
            (?:timeout|deadline|time[-\s]?(?:limit|budget)|token[-\s]?budget|cost[-\s]?budget|budget)\s*:
        )",
    )
    .expect("A029 operative-control prefix regex is valid")
});

static DESCRIPTIVE_CONTROL_PREFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^\s*(?:use\s+of\b|(?:timeout|deadline|budget|max(?:imum)?|limit|cap)\b.{0,80}\b(?:was|were|had|used\s+to|once\s+forced)\b)")
        .expect("A029 descriptive-control prefix regex is valid")
});

/// Check whether an agent description is too similar to the agent name.
///
/// Returns `true` when the description adds no meaningful information beyond
/// what the name already conveys.
fn is_desc_redundant(name: &str, desc: &str) -> bool {
    let name_lower = name.to_lowercase().replace('-', " ");
    let name_words: HashSet<&str> = name_lower.split_whitespace().collect();

    let desc_lower = desc.to_lowercase();
    // Strip leading/trailing punctuation from each token so "analyzer." matches "analyzer".
    let desc_stripped: Vec<String> = desc_lower
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_string())
        .filter(|w| !w.is_empty())
        .collect();
    let desc_word_count = desc_stripped.len();

    let stopwords: HashSet<&str> = STOPWORDS.iter().copied().collect();
    let desc_content_words: HashSet<&str> = desc_stripped
        .iter()
        .map(|w| w.as_str())
        .filter(|w| !stopwords.contains(w))
        .collect();

    // Jaccard path: flag short descriptions with high word overlap.
    if desc_word_count < MIN_DESC_WORDS && !name_words.is_empty() && !desc_content_words.is_empty()
    {
        let intersection = name_words.intersection(&desc_content_words).count();
        let union = name_words.union(&desc_content_words).count();
        if union > 0 {
            let jaccard = intersection as f64 / union as f64;
            if jaccard > JACCARD_THRESHOLD {
                return true;
            }
        }
    }

    // Token containment path: flag if all name words appear in the description
    // and the description adds at most one content word beyond the name
    // (catching filler like "tool", "agent", "helper" without listing them).
    // Require at least 2 name words to avoid false positives on single-word names
    // (e.g., name "code" with desc "code reviewer" is a valid description).
    if name_words.len() >= 2 && name_words.is_subset(&desc_content_words) {
        let extra_content = desc_content_words.difference(&name_words).count();
        if extra_content <= 1 {
            return true;
        }
    }

    false
}

/// V7: Validate agents/*.md frontmatter.
#[cfg(test)]
pub fn validate_agents(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let mut prompt_pass = super::prompt_content::PromptContentPass::default();
    validate_agents_with_prompt_pass(diag, exclude, &mut prompt_pass);
}

pub(crate) fn validate_agents_with_prompt_pass(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    prompt_pass: &mut super::prompt_content::PromptContentPass,
) {
    let agents_dir = Path::new("agents");
    if !agents_dir.is_dir() {
        diag.report_at(
            LintRule::AgentsDirMissing,
            agents_dir,
            "agents/ directory is missing",
        );
        return;
    }

    let mut found = 0;
    let mut excluded_count = 0;
    for entry in traversal::shallow_files(agents_dir, Path::new("."), None).entries {
        let path = entry.path;
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) if n.ends_with(".md") => n.to_string(),
            _ => continue,
        };

        let agent_path = format!("agents/{name}");
        if exclude.is_excluded(&agent_path) {
            excluded_count += 1;
            continue;
        }

        found += 1;
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        diag.with_subject_path(&agent_path, |diag| {
            validate_agent_file(diag, &agent_path, &content, prompt_pass);
            check_unsupported_plugin_fields(diag, &agent_path, &content);
        });
    }

    if found == 0 && excluded_count == 0 {
        diag.report_at(
            LintRule::NoAgentFiles,
            agents_dir,
            "agents/ has no .md files",
        );
    }
}

/// A028: frontmatter fields unsupported for plugin agents. Plugin-mode only —
/// private `.claude/agents/` files may legitimately set these fields, so this
/// is called from `validate_agents` rather than `validate_agent_file`.
fn check_unsupported_plugin_fields(
    diag: &mut DiagnosticCollector,
    agent_path: &str,
    content: &str,
) {
    let fm_lines = match frontmatter::extract_frontmatter(content) {
        Some(lines) => lines,
        None => return,
    };
    for field in UNSUPPORTED_PLUGIN_FIELDS {
        if frontmatter::field_exists(&fm_lines, field) {
            diag.report(
                LintRule::AgentFieldUnsupported,
                &format!(
                    "{agent_path}: frontmatter field '{field}' is not supported for plugin agents"
                ),
            );
        }
    }
}

/// V7-adapted: Validate `.claude/agents/*.md` (private agents) in Basic mode.
/// Runs the same per-file frontmatter and field-value checks as `agents/`
/// (A002/A003, A008-A011, A014-A027). Does not report A001/A004 (the
/// `.claude/agents/` directory is optional) nor the larch-specific
/// template rules A005-A007.
#[cfg(test)]
pub fn validate_private_agents(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let mut prompt_pass = super::prompt_content::PromptContentPass::default();
    validate_private_agents_with_prompt_pass(diag, exclude, &mut prompt_pass);
}

pub(crate) fn validate_private_agents_with_prompt_pass(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    prompt_pass: &mut super::prompt_content::PromptContentPass,
) {
    let agents_dir = Path::new(".claude/agents");
    if !agents_dir.is_dir() {
        return;
    }
    for entry in traversal::shallow_files(agents_dir, Path::new("."), None).entries {
        let path = entry.path;
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) if n.ends_with(".md") => n.to_string(),
            _ => continue,
        };

        let agent_path = format!(".claude/agents/{name}");
        if exclude.is_excluded(&agent_path) {
            continue;
        }

        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        diag.with_subject_path(&agent_path, |diag| {
            validate_agent_file(diag, &agent_path, &content, prompt_pass);
        });
    }
}

/// Run all per-file agent frontmatter checks (used for both `agents/` and
/// `.claude/agents/`). Covers A002, A003, A008-A011, and the field-value
/// rules A014-A027.
fn validate_agent_file(
    diag: &mut DiagnosticCollector,
    agent_path: &str,
    content: &str,
    prompt_pass: &mut super::prompt_content::PromptContentPass,
) {
    let markdown = MarkdownDocument::parse(content);
    let prompt_document = LiveInstructionDocument::new(
        Path::new(agent_path),
        InstructionSurfaceKind::Agent,
        &markdown,
    );
    let fm_lines = match markdown.frontmatter() {
        Some(lines) => lines,
        None => {
            diag.report(
                LintRule::AgentFrontmatterMalformed,
                &format!(
                    "{agent_path}: malformed frontmatter (must start with '---' on line 1, must have closing '---')"
                ),
            );
            // X002–X005 still apply when frontmatter is broken.
            super::markdown_structure::check_markdown_document(agent_path, &markdown, diag);
            prompt_pass.validate(&prompt_document, diag);
            return;
        }
    };

    // X001: strict YAML; CC-AG-011: hooks schema when present.
    let parsed_frontmatter = match frontmatter::parse_yaml_strict(fm_lines) {
        Ok(yaml) => {
            if let Some(hooks) = yaml.get("hooks") {
                super::hook_schema::validate_frontmatter_hooks(
                    hooks,
                    &format!("{agent_path} frontmatter"),
                    diag,
                );
            }
            Some(yaml)
        }
        Err((line, msg)) => {
            diag.report_with(
                LintRule::FrontmatterYamlInvalid,
                &format!("{agent_path}:{line}: frontmatter is not valid YAML: {msg}"),
                DiagnosticMetadata::at_line(line),
            );
            None
        }
    };

    // X002–X005 on the full agent markdown file.
    super::markdown_structure::check_markdown_document(agent_path, &markdown, diag);

    let fm_name = frontmatter::get_field(fm_lines, "name");
    let fm_desc = frontmatter::get_field(fm_lines, "description");

    if fm_name.is_none() {
        diag.report(
            LintRule::AgentFieldMissing,
            &format!("{agent_path}: missing required frontmatter field 'name'"),
        );
    }
    if fm_desc.is_none() {
        diag.report(
            LintRule::AgentFieldMissing,
            &format!("{agent_path}: missing required frontmatter field 'description'"),
        );
    }

    // A008: agent description too long
    // A009: agent description too short
    if let Some(ref desc) = fm_desc {
        let char_count = desc.chars().count();
        if char_count > 1024 {
            diag.report(
                LintRule::AgentDescLong,
                &format!("{agent_path}: description exceeds 1024 characters ({char_count})"),
            );
        }
        if char_count < 20 {
            diag.report(
                LintRule::AgentDescShort,
                &format!("{agent_path}: description is under 20 characters ({char_count})"),
            );
        }
    }

    // A011: agent description too similar to agent name
    if let Some(ref n) = fm_name {
        if let Some(ref desc) = fm_desc {
            if is_desc_redundant(n, desc) {
                diag.report(
                    LintRule::AgentDescRedundant,
                    &format!("{agent_path}: description is too similar to the agent name '{n}'"),
                );
            }
        }
    }

    // A010: agent name invalid characters
    if let Some(ref n) = fm_name {
        if RE_NAME_INVALID.is_match(n) {
            diag.report(
                LintRule::AgentNameInvalid,
                &format!(
                    "{agent_path}: name '{}' contains characters outside [a-z0-9-]",
                    n
                ),
            );
        }
    }

    let max_turns = parsed_frontmatter
        .as_ref()
        .and_then(validated_max_turns_from_yaml);
    check_agent_field_values(
        diag,
        agent_path,
        fm_lines,
        parsed_frontmatter.as_ref(),
        max_turns,
    );
    let prompt_document = prompt_document.with_outer_max_turns(max_turns);
    if let Some(parsed_frontmatter) = parsed_frontmatter.as_ref() {
        check_agent_stop_control(
            diag,
            agent_path,
            parsed_frontmatter,
            max_turns,
            &prompt_document,
        );
    }
    prompt_pass.validate(&prompt_document, diag);
}

/// Recognized agent frontmatter fields. Any other top-level key triggers A027
/// (agent-field-unknown) as a typo catcher. Matches the field set validated by
/// A002/A003 and A014-A026 plus the standard Claude Code agent schema.
const KNOWN_AGENT_FIELDS: &[&str] = &[
    "name",
    "description",
    "tools",
    "disallowedTools",
    "model",
    "permissionMode",
    "maxTurns",
    "isolation",
    "background",
    "skills",
    "memory",
    "effort",
    "hooks",
];

/// Allowed `permissionMode` values (CC-AG-004).
const VALID_PERMISSION_MODES: &[&str] = &[
    "default",
    "acceptEdits",
    "dontAsk",
    "bypassPermissions",
    "plan",
    "delegate",
];

/// Allowed `memory` values (CC-AG-008).
const VALID_MEMORY: &[&str] = &["user", "project", "local"];

/// Allowed `effort` values (CC-AG-014; superset of the skill S025 set).
const VALID_EFFORT: &[&str] = &["low", "medium", "high", "xhigh", "max"];

/// Strip one layer of matching outer quotes (double or single) from a value.
fn strip_outer_quotes(s: &str) -> &str {
    let s = s.trim();
    if s.len() >= 2
        && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')))
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Whether a string is kebab-case: non-empty, `[a-z0-9-]` only, no leading /
/// trailing hyphen, no consecutive hyphens.
fn is_kebab_case(s: &str) -> bool {
    if s.is_empty() || s.starts_with('-') || s.ends_with('-') || s.contains("--") {
        return false;
    }
    !RE_NAME_INVALID.is_match(s)
}

/// Whether a `model:` value is a recognized Claude Code model.
///
/// Accepts the aliases `sonnet`/`opus`/`haiku`/`inherit`/`default`, full model
/// IDs of the form `claude-<family>-<version>` (family in sonnet/opus/haiku),
/// and an optional `[1m]`/`[2m]` context-window suffix on any of the above.
fn is_valid_model(value: &str) -> bool {
    let v = value.trim();
    let base = v
        .strip_suffix("[1m]")
        .or_else(|| v.strip_suffix("[2m]"))
        .unwrap_or(v);
    match base {
        "sonnet" | "opus" | "haiku" | "inherit" | "default" => true,
        other => {
            for family in ["claude-sonnet-", "claude-opus-", "claude-haiku-"] {
                if let Some(rest) = other.strip_prefix(family) {
                    return !rest.is_empty() && !rest.ends_with('-');
                }
            }
            false
        }
    }
}

/// Extract items from a frontmatter field that is either a comma-separated
/// scalar (`tools: Bash, Read`) or a YAML list (`tools:\n  - Bash\n  - Read`).
/// Returns trimmed, quote-stripped, non-empty items.
fn get_field_items(fm_lines: &[String], key: &str) -> Vec<String> {
    let prefix = format!("{key}:");
    let mut items = Vec::new();
    let mut key_idx: Option<usize> = None;

    for (i, line) in fm_lines.iter().enumerate() {
        if line.starts_with(&prefix) {
            key_idx = Some(i);
            let raw = strip_outer_quotes(line[prefix.len()..].trim_start());
            // YAML flow sequence: `tools: [Bash, Read]` — strip the brackets
            // before comma-splitting so each item is parsed as a scalar entry.
            let inline = if raw.starts_with('[') && raw.ends_with(']') && raw.len() >= 2 {
                &raw[1..raw.len() - 1]
            } else {
                raw
            };
            if !inline.is_empty() {
                for part in inline.split(',') {
                    let p = strip_outer_quotes(part.trim());
                    if !p.is_empty() {
                        items.push(p.to_string());
                    }
                }
            }
            break;
        }
    }

    if let Some(idx) = key_idx {
        for line in fm_lines.iter().skip(idx + 1) {
            if line.is_empty() {
                continue;
            }
            if !(line.starts_with(' ') || line.starts_with('\t')) {
                break; // End of this key's block.
            }
            let trimmed = line.trim_start();
            if let Some(rest) = trimmed.strip_prefix("- ") {
                let item = strip_outer_quotes(rest.trim());
                if !item.is_empty() {
                    items.push(item.to_string());
                }
            }
        }
    }

    items
}

/// Read a positive integer `maxTurns` bound from successfully parsed agent YAML.
///
/// A quoted scalar, float, boolean, collection, or negative value is not an
/// agent turn bound. Keeping this at the strict-YAML ownership boundary ensures
/// prompt-content rules never infer execution limits from malformed frontmatter.
fn validated_max_turns_from_yaml(yaml: &crate::yaml::Value) -> Option<NonZeroU64> {
    yaml.as_mapping()?
        .get("maxTurns")?
        .as_u64()
        .and_then(NonZeroU64::new)
}

/// Return whether an explicitly declared tool can perform execution-like work.
/// MCP tools are included because their fully qualified syntax is the supported
/// declaration form and each invokes an external server operation.
fn is_execution_tool(tool: &str) -> bool {
    if !is_known_tool_name(tool) {
        return false;
    }
    let base_name = tool.split_once('(').map_or(tool, |(base, _)| base).trim();
    base_name.starts_with("mcp__") || EXECUTION_TOOLS.contains(&base_name)
}

/// A029: tool-using agents need one concrete stop control or failure outcome.
///
/// This intentionally examines only live, operative Markdown prose. Frontmatter
/// is considered solely for a valid `maxTurns`; examples, fences, inline code,
/// block quotes, and quoted text cannot satisfy a body control.
fn check_agent_stop_control(
    diag: &mut DiagnosticCollector,
    agent_path: &str,
    parsed_frontmatter: &crate::yaml::Value,
    max_turns: Option<NonZeroU64>,
    document: &LiveInstructionDocument<'_>,
) {
    let execution_tools: Vec<_> = frontmatter::strict_string_items(parsed_frontmatter, "tools")
        .into_iter()
        .filter(|tool| is_execution_tool(tool))
        .collect();
    if execution_tools.is_empty() || max_turns.is_some() {
        return;
    }

    if has_operative_body_control(document) {
        return;
    }

    diag.report(
        LintRule::AgentStopMissing,
        &format!(
            "{agent_path}: execution tools [{}] have no maximum attempt/tool-call/step count, explicit timeout/deadline/token/cost budget, progress/failure threshold, or stop-and-report/escalation fallback; add a concrete bound and a failure outcome",
            execution_tools.join(", "),
        ),
    );
}

fn has_operative_body_control(document: &LiveInstructionDocument<'_>) -> bool {
    let example_scopes = document.example_scopes();
    let heading_lines: HashSet<_> = document
        .headings()
        .iter()
        .map(|heading| heading.line)
        .collect();
    let mut scopes = Vec::new();
    let mut scope = Vec::new();
    let mut previous_line = None;

    for (line, is_example) in document.prose_lines().iter().zip(example_scopes) {
        let boundary = is_example
            || line.text.trim().is_empty()
            || heading_lines.contains(&line.line)
            || previous_line.is_some_and(|previous| line.line > previous + 1);
        if boundary && !scope.is_empty() {
            scopes.push(std::mem::take(&mut scope));
        }
        if !boundary {
            scope.push(line.text.as_str());
        }
        previous_line = Some(line.line);
    }
    if !scope.is_empty() {
        scopes.push(scope);
    }

    scopes.into_iter().any(|scope| {
        let scope = scope.join(" ");
        sentence_ranges(&scope).into_iter().any(|range| {
            let sentence = &scope[range];
            OPERATIVE_CONTROL_PREFIX.is_match(sentence)
                && !DESCRIPTIVE_CONTROL_PREFIX.is_match(sentence)
                && has_bound_or_fallback(sentence)
        })
    })
}

/// Collect top-level (non-indented, non-list) frontmatter keys.
fn collect_top_level_keys(fm_lines: &[String]) -> Vec<String> {
    let mut keys = Vec::new();
    for line in fm_lines {
        if line.starts_with(' ') || line.starts_with('\t') || line.trim_start().starts_with('-') {
            continue;
        }
        if let Some(pos) = line.find(':') {
            let key = line[..pos].trim();
            if !key.is_empty() {
                keys.push(key.to_string());
            }
        }
    }
    keys
}

/// Whether a referenced skill name resolves to a SKILL.md on disk under either
/// the public (`skills/`) or private (`.claude/skills/`) layout.
fn skill_exists_on_disk(skill: &str) -> bool {
    Path::new(&format!("skills/{skill}/SKILL.md")).is_file()
        || Path::new(&format!(".claude/skills/{skill}/SKILL.md")).is_file()
}

/// A014-A027: field-value validation for agent frontmatter.
fn check_agent_field_values(
    diag: &mut DiagnosticCollector,
    agent_path: &str,
    fm_lines: &[String],
    parsed_frontmatter: Option<&crate::yaml::Value>,
    max_turns: Option<NonZeroU64>,
) {
    // A014: model must be a recognized value (CC-AG-003).
    if let Some(model) = frontmatter::get_field(fm_lines, "model") {
        if !is_valid_model(&model) {
            diag.report(
                LintRule::AgentModelInvalid,
                &format!(
                    "{agent_path}: model '{}' is not a recognized Claude Code model (use sonnet/opus/haiku/inherit, a claude-<family>-<id> full ID, with optional [1m] suffix)",
                    model
                ),
            );
        }
    }

    // A015 + A021: permissionMode enum (CC-AG-004) and bypass warning (CC-AG-012).
    if let Some(mode) = frontmatter::get_field(fm_lines, "permissionMode") {
        if !VALID_PERMISSION_MODES.contains(&mode.as_str()) {
            diag.report(
                LintRule::AgentPermissionInvalid,
                &format!(
                    "{agent_path}: permissionMode '{}' is not one of [default, acceptEdits, dontAsk, bypassPermissions, plan, delegate]",
                    mode
                ),
            );
        } else if mode == "bypassPermissions" {
            diag.report(
                LintRule::AgentBypassPermissions,
                &format!(
                    "{agent_path}: permissionMode 'bypassPermissions' disables safety checks",
                ),
            );
        }
    }

    // A018: memory must be user/project/local (CC-AG-008).
    if let Some(mem) = frontmatter::get_field(fm_lines, "memory") {
        if !VALID_MEMORY.contains(&mem.as_str()) {
            diag.report(
                LintRule::AgentMemoryInvalid,
                &format!(
                    "{agent_path}: memory '{}' is not one of [user, project, local]",
                    mem
                ),
            );
        }
    }

    // A023: effort must be low/medium/high/xhigh/max (CC-AG-014).
    if let Some(eff) = frontmatter::get_field(fm_lines, "effort") {
        if !VALID_EFFORT.contains(&eff.as_str()) {
            diag.report(
                LintRule::AgentEffortInvalid,
                &format!(
                    "{agent_path}: effort '{}' is not one of [low, medium, high, xhigh, max]",
                    eff
                ),
            );
        }
    }

    // A024: isolation must be worktree (CC-AG-015).
    if let Some(iso) = frontmatter::get_field(fm_lines, "isolation") {
        if iso != "worktree" {
            diag.report(
                LintRule::AgentIsolationInvalid,
                &format!(
                    "{agent_path}: isolation '{}' is not 'worktree' (the only supported value)",
                    iso
                ),
            );
        }
    }

    // A025: background must be a boolean (CC-AG-016).
    if let Some(bg) = frontmatter::get_field(fm_lines, "background") {
        if bg != "true" && bg != "false" {
            diag.report(
                LintRule::AgentBackgroundInvalid,
                &format!(
                    "{agent_path}: background '{}' is not a boolean (use true or false)",
                    bg
                ),
            );
        }
    }

    // A026: maxTurns must be a positive integer (CC-AG-017). Use the same
    // strict parse that owns the Q005 outer bound, never a raw line lookup.
    if parsed_frontmatter
        .and_then(crate::yaml::Value::as_mapping)
        .is_some_and(|mapping| mapping.contains_key("maxTurns"))
        && max_turns.is_none()
    {
        diag.report(
            LintRule::AgentMaxturnsInvalid,
            &format!("{agent_path}: maxTurns is not a positive integer"),
        );
    }

    let tools = get_field_items(fm_lines, "tools");
    let disallowed = get_field_items(fm_lines, "disallowedTools");

    // A019: tools must be known (CC-AG-009).
    for tool in &tools {
        if !is_known_tool_name(tool) {
            diag.report(
                LintRule::AgentToolsUnknown,
                &format!(
                    "{agent_path}: tools lists unrecognized tool '{tool}' (case-sensitive PascalCase; mcp__<server>__<tool> is accepted)",
                ),
            );
        }
    }

    // A020: disallowedTools must be known (CC-AG-010).
    for tool in &disallowed {
        if !is_known_tool_name(tool) {
            diag.report(
                LintRule::AgentDisallowedUnknown,
                &format!(
                    "{agent_path}: disallowedTools lists unrecognized tool '{tool}' (case-sensitive PascalCase; mcp__<server>__<tool> is accepted)",
                ),
            );
        }
    }

    // A017: no tool in both tools and disallowedTools (CC-AG-006).
    let disallowed_set: HashSet<&str> = disallowed.iter().map(String::as_str).collect();
    for tool in &tools {
        if disallowed_set.contains(tool.as_str()) {
            diag.report(
                LintRule::AgentToolsOverlap,
                &format!("{agent_path}: tool '{tool}' appears in both tools and disallowedTools",),
            );
        }
    }

    // A016 + A022: skills must exist on disk (CC-AG-005) and be kebab-case (CC-AG-013).
    let skills = get_field_items(fm_lines, "skills");
    for skill in &skills {
        if !is_kebab_case(skill) {
            diag.report(
                LintRule::AgentSkillKebab,
                &format!(
                    "{agent_path}: skills entry '{skill}' is not kebab-case ([a-z0-9-], no leading/trailing/double hyphen)",
                ),
            );
        }
        if !skill_exists_on_disk(skill) {
            diag.report(
                LintRule::AgentSkillMissing,
                &format!(
                    "{agent_path}: skills entry '{skill}' has no matching skills/{skill}/SKILL.md or .claude/skills/{skill}/SKILL.md",
                ),
            );
        }
    }

    // A027: unknown frontmatter field (CC-AG-019, typo catcher).
    for key in collect_top_level_keys(fm_lines) {
        if !KNOWN_AGENT_FIELDS.contains(&key.as_str()) {
            diag.report(
                LintRule::AgentFieldUnknown,
                &format!("{agent_path}: unrecognized frontmatter field '{key}' (possible typo)",),
            );
        }
    }
}

/// V16: Agent-template alignment — every agents/*.md must contain
/// "Derived from" marker referencing reviewer-templates.md.
/// (Larch-specific convention check.)
pub fn validate_agent_template_alignment(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let agents_dir = Path::new("agents");
    let templates = Path::new("skills/shared/reviewer-templates.md");

    if !agents_dir.is_dir() {
        return;
    }
    if !templates.is_file() {
        diag.report_at(
            LintRule::TemplateFileMissing,
            templates,
            &format!("reviewer-templates.md missing: {}", templates.display()),
        );
        return;
    }

    for entry in traversal::shallow_files(agents_dir, Path::new("."), None).entries {
        let path = entry.path;
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) if n.ends_with(".md") => n.to_string(),
            _ => continue,
        };

        let agent_path = format!("agents/{name}");
        if exclude.is_excluded(&agent_path) {
            continue;
        }

        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let has_marker = content.lines().any(|line| {
            let lower = line.to_lowercase();
            lower.contains("derived from") && lower.contains("reviewer-templates.md")
        });

        if !has_marker {
            diag.report_at(
                LintRule::TemplateMarkerMissing,
                &agent_path,
                &format!(
                    "agents/{name} missing 'Derived from skills/shared/reviewer-templates.md' marker"
                ),
            );
        }
    }
}

/// V21: Agent-template count — number of ## Reviewer sections in
/// skills/shared/reviewer-templates.md must equal number of agents/*.md files.
/// (Larch-specific convention check.)
pub fn validate_agent_template_count(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let agents_dir = Path::new("agents");
    let templates = Path::new("skills/shared/reviewer-templates.md");

    if !agents_dir.is_dir() || !templates.is_file() {
        return; // V16 catches missing template
    }

    // Count ## Reviewer sections
    let template_content = match fs::read_to_string(templates) {
        Ok(c) => c,
        Err(_) => return,
    };
    let template_count = template_content
        .lines()
        .filter(|line| line.starts_with("## Reviewer"))
        .count();

    // Count agents/*.md files
    let mut agent_count = 0;
    for entry in traversal::shallow_files(agents_dir, Path::new("."), None).entries {
        let path = entry.path;
        if let Some(name) = path.file_name().and_then(|n| n.to_str())
            && name.ends_with(".md")
        {
            let agent_path = format!("agents/{name}");
            if !exclude.is_excluded(&agent_path) {
                agent_count += 1;
            }
        }
    }

    if template_count != agent_count {
        diag.report_at(
            LintRule::TemplateCountMismatch,
            templates,
            &format!(
                "agent-template count mismatch: {agent_count} agent file(s) but {template_count} '## Reviewer' section(s) in {}",
                templates.display()
            ),
        );
    }
}

#[cfg(test)]
#[allow(non_snake_case)] // Preserve stable test names that embed rule identifiers.
mod tests {
    use super::*;
    use crate::diagnostic::DiagnosticCollector;

    // V7: validate_agents
    #[test]
    #[serial_test::serial]
    fn test_v7_valid_agents() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("agents").unwrap();
        std::fs::write(
            "agents/general.md",
            "---\nname: general\ndescription: General reviewer for code quality analysis\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v7_missing_agents_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("agents/ directory is missing"));
    }

    #[test]
    #[serial_test::serial]
    fn test_v7_empty_agents_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("agents").unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("no .md files"));
    }

    #[test]
    #[serial_test::serial]
    fn test_v7_missing_frontmatter_name() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("agents").unwrap();
        std::fs::write(
            "agents/general.md",
            "---\ndescription: General reviewer for code quality analysis\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.error_count() >= 1);
        assert!(diag.errors().iter().any(|e| e.contains("name")));
    }

    // V16: validate_agent_template_alignment
    #[test]
    #[serial_test::serial]
    fn test_v16_valid_alignment() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("agents").unwrap();
        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::write("skills/shared/reviewer-templates.md", "# Templates\n").unwrap();
        std::fs::write(
            "agents/general.md",
            "---\nname: general\ndescription: desc\n---\nDerived from skills/shared/reviewer-templates.md\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agent_template_alignment(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v16_missing_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("agents").unwrap();
        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::write("skills/shared/reviewer-templates.md", "# Templates\n").unwrap();
        std::fs::write(
            "agents/general.md",
            "---\nname: general\ndescription: desc\n---\nNo marker here\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agent_template_alignment(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("missing"));
    }

    // V21: validate_agent_template_count
    #[test]
    #[serial_test::serial]
    fn test_v21_matching_count() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("agents").unwrap();
        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::write(
            "skills/shared/reviewer-templates.md",
            "## Reviewer 1\nContent\n## Reviewer 2\nContent\n",
        )
        .unwrap();
        std::fs::write("agents/one.md", "---\nname: one\n---\n").unwrap();
        std::fs::write("agents/two.md", "---\nname: two\n---\n").unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agent_template_count(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v21_mismatched_count() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("agents").unwrap();
        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::write(
            "skills/shared/reviewer-templates.md",
            "## Reviewer 1\nContent\n## Reviewer 2\nContent\n",
        )
        .unwrap();
        std::fs::write("agents/one.md", "---\nname: one\n---\n").unwrap();
        // Only 1 agent but 2 templates

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agent_template_count(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("mismatch"));
    }

    // A008: agent-desc-long
    #[test]
    #[serial_test::serial]
    fn test_a008_desc_too_long() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        let long_desc = "x".repeat(1025);
        std::fs::write(
            "agents/general.md",
            format!("---\nname: general\ndescription: {long_desc}\n---\nBody\n"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("exceeds 1024")));
    }

    // A009: agent-desc-short
    #[test]
    #[serial_test::serial]
    fn test_a009_desc_too_short() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::write(
            "agents/general.md",
            "---\nname: general\ndescription: Short\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("under 20")));
    }

    #[test]
    #[serial_test::serial]
    fn test_a008_boundary_1024_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        let desc = format!("Use when testing {}", "x".repeat(1007));
        assert_eq!(desc.chars().count(), 1024);
        std::fs::write(
            "agents/general.md",
            format!("---\nname: general\ndescription: {desc}\n---\nBody\n"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("exceeds 1024")));
    }

    #[test]
    #[serial_test::serial]
    fn test_a008_multibyte_chars_count_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        // 1025 CJK characters (3 bytes each) = 3075 bytes but only 1025 chars
        let desc = "\u{4e00}".repeat(1025);
        assert_eq!(desc.chars().count(), 1025);
        assert!(desc.len() > 1025); // bytes > chars
        std::fs::write(
            "agents/general.md",
            format!("---\nname: general\ndescription: {desc}\n---\nBody\n"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("exceeds 1024")));
    }

    #[test]
    #[serial_test::serial]
    fn test_a009_boundary_20_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        let desc = "Use when needed now!";
        assert_eq!(desc.chars().count(), 20);
        std::fs::write(
            "agents/general.md",
            format!("---\nname: general\ndescription: {desc}\n---\nBody\n"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("under 20")));
    }

    #[test]
    #[serial_test::serial]
    fn test_a009_multibyte_chars_count_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        // 19 CJK characters (3 bytes each) = 57 bytes but only 19 chars
        let desc = "\u{4e00}".repeat(19);
        assert_eq!(desc.chars().count(), 19);
        assert!(desc.len() > 19); // bytes > chars
        std::fs::write(
            "agents/general.md",
            format!("---\nname: general\ndescription: {desc}\n---\nBody\n"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("under 20")));
    }

    // A010: agent-name-invalid
    #[test]
    #[serial_test::serial]
    fn test_a010_name_invalid() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::write(
            "agents/general.md",
            "---\nname: My_Agent\ndescription: A valid agent description here\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("outside [a-z0-9-]"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_a010_valid_name_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::write(
            "agents/general.md",
            "---\nname: general-reviewer\ndescription: A valid agent description here\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("outside [a-z0-9-]"))
        );
    }

    // ── A011: agent-desc-redundant ──────────────────────────────────

    // Unit tests for is_desc_redundant helper
    #[test]
    fn test_is_desc_redundant_exact_token_match() {
        // Name tokens match desc content tokens exactly
        assert!(is_desc_redundant("code-analyzer", "A code analyzer agent"));
    }

    #[test]
    fn test_is_desc_redundant_with_only_stopwords_extra() {
        // Description has name words + only stopwords → redundant
        assert!(is_desc_redundant("test-runner", "The test runner tool"));
    }

    #[test]
    fn test_is_desc_redundant_descriptive_passes() {
        // Description adds meaningful content beyond the name
        assert!(!is_desc_redundant(
            "security-reviewer",
            "Reviews code for security vulnerabilities and auth flaws"
        ));
    }

    #[test]
    fn test_is_desc_redundant_short_name_with_context_passes() {
        // Short name with a descriptive description should NOT fire
        assert!(!is_desc_redundant(
            "api",
            "API helper tool for REST requests"
        ));
    }

    #[test]
    fn test_is_desc_redundant_overlapping_but_distinct() {
        assert!(!is_desc_redundant(
            "code-reviewer",
            "Performs deep analysis of code for bugs and security issues"
        ));
    }

    #[test]
    fn test_is_desc_redundant_token_containment_with_extra_content_passes() {
        // Name tokens present but description adds extra content words
        assert!(!is_desc_redundant(
            "test-runner",
            "Test runner for CI integration workflows"
        ));
    }

    #[test]
    fn test_is_desc_redundant_boundary_extra_words() {
        // 1 extra content word beyond name → fires (token containment)
        assert!(is_desc_redundant("code-analyzer", "code analyzer tool"));
        // 2 extra content words → does not fire
        assert!(!is_desc_redundant(
            "code-analyzer",
            "code analyzer tool agent"
        ));
    }

    #[test]
    fn test_is_desc_redundant_low_jaccard_passes() {
        // Low word overlap should not fire even with short desc.
        // name={code,analyzer}, desc={static,analysis,tool} → jaccard=0/5=0
        assert!(!is_desc_redundant("code-analyzer", "Static analysis tool"));
    }

    #[test]
    fn test_is_desc_redundant_punctuation_stripped() {
        // Trailing punctuation should be stripped — "analyzer." matches "analyzer"
        assert!(is_desc_redundant("code-analyzer", "A code analyzer."));
        // Comma after word should also be stripped
        assert!(is_desc_redundant("code-analyzer", "code analyzer, tool"));
    }

    #[test]
    fn test_is_desc_redundant_single_word_name_no_false_positive() {
        // Single-word name with meaningful two-word description should NOT fire
        assert!(!is_desc_redundant("code", "code reviewer"));
        assert!(!is_desc_redundant("api", "api gateway"));
    }

    #[test]
    fn test_is_desc_redundant_existing_fixture_safe() {
        // Existing test fixture should NOT fire A011
        assert!(!is_desc_redundant(
            "general",
            "General reviewer for code quality analysis"
        ));
    }

    // Integration tests through validate_agents
    #[test]
    #[serial_test::serial]
    fn test_a011_redundant_desc_fires() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::write(
            "agents/analyzer.md",
            "---\nname: code-analyzer\ndescription: A code analyzer agent\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("too similar to the agent name"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_a011_descriptive_desc_passes() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::write(
            "agents/reviewer.md",
            "---\nname: security-reviewer\ndescription: Reviews code for security vulnerabilities including injection and XSS flaws\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("too similar to the agent name"))
        );
    }

    // ── A028: agent-field-unsupported ───────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_a028_unsupported_fields_fire() {
        for field in ["hooks", "mcpServers", "permissionMode"] {
            let tmp = tempfile::tempdir().unwrap();
            let _guard = crate::test_helpers::CwdGuard::new();
            std::env::set_current_dir(tmp.path()).unwrap();
            std::fs::create_dir_all("agents").unwrap();
            std::fs::write(
                "agents/general.md",
                format!(
                    "---\nname: general\ndescription: General reviewer for code quality analysis\n{field}: something\n---\nBody\n"
                ),
            )
            .unwrap();
            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_agents(&mut diag, &crate::config::ExcludeSet::default());
            assert!(
                diag.errors()
                    .iter()
                    .any(|e| e.contains(&format!("'{field}' is not supported"))),
                "expected A028 for field '{field}', got: {:?}",
                diag.errors()
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_a028_supported_fields_pass() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::write(
            "agents/general.md",
            "---\nname: general\ndescription: General reviewer for code quality analysis\ntools: Read, Grep\nmodel: sonnet\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag.errors().iter().any(|e| e.contains("not supported")),
            "supported frontmatter should not trigger A028, got: {:?}",
            diag.errors()
        );
    }

    /// A028 is Plugin-mode only: private `.claude/agents/` files may legitimately
    /// set `hooks`/`mcpServers`/`permissionMode`, so the rule must not fire there.
    #[test]
    #[serial_test::serial]
    fn test_a028_not_reported_for_private_agents() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude/agents").unwrap();
        std::fs::write(
            ".claude/agents/private.md",
            "---\nname: private\ndescription: General reviewer for code quality analysis\npermissionMode: acceptEdits\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_private_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag.errors().iter().any(|e| e.contains("not supported")),
            "A028 must not fire for private .claude/agents/, got: {:?}",
            diag.errors()
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_a011_existing_fixture_no_false_positive() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::write(
            "agents/general.md",
            "---\nname: general\ndescription: General reviewer for code quality analysis\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("too similar to the agent name")),
            "Existing fixture should not trigger A011"
        );
    }

    // ── A014-A027: agent field-value validation ──────────────────────

    /// Run `validate_agents` against a single `agents/general.md` with the given
    /// frontmatter/body and return the resulting error messages (all rules
    /// promoted to errors via `new_all_enabled`). Temp dir + cwd are scoped to
    /// the closure so disk-backed checks (A016) see an empty skills layout.
    fn run_agent<F: FnOnce(&mut DiagnosticCollector)>(content: &str, f: F) {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::write("agents/general.md", content).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        f(&mut diag);
    }

    const GOOD_DESC: &str = "A general-purpose code review assistant";

    // ── A029: agent-stop-missing ───────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_a029_tool_using_agent_without_controls_fires() {
        let content = format!(
            "---\nname: general\ndescription: {GOOD_DESC}\ntools: Bash, Read\n---\nInvestigate the failure and implement the repair.\n"
        );
        run_agent(&content, |diag| {
            let finding = diag
                .diagnostics()
                .iter()
                .find(|diagnostic| diagnostic.rule == LintRule::AgentStopMissing)
                .expect("A029 reports an unbounded execution-capable agent");
            assert_eq!(
                finding.subject_path.as_deref(),
                Some(Path::new("agents/general.md"))
            );
            assert!(finding.message.contains("Bash"));
            assert!(finding.message.contains("failure outcome"));
        });
    }

    #[test]
    #[serial_test::serial]
    fn test_a029_uses_strict_yaml_tool_values() {
        for tools in [
            "tools: Bash # execute repository commands",
            "tools: [Bash, Read] # execute repository commands",
            "tools: \"Bash, Read\"",
            "tools:\n  - Bash\n  - Read",
            "tools: Bash(git *)",
        ] {
            let content = format!(
                "---\nname: general\ndescription: {GOOD_DESC}\n{tools}\n---\nInvestigate the failure and implement the repair.\n"
            );
            run_agent(&content, |diag| {
                assert!(
                    diag.diagnostics()
                        .iter()
                        .any(|diagnostic| diagnostic.rule == LintRule::AgentStopMissing),
                    "A029 must use the parsed execution tool declaration: {tools}"
                );
            });
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_a029_valid_max_turns_satisfies_stop_control() {
        let content = format!(
            "---\nname: general\ndescription: {GOOD_DESC}\ntools: Bash\nmaxTurns: 4\n---\nInvestigate the failure and implement the repair.\n"
        );
        run_agent(&content, |diag| {
            assert!(
                !diag
                    .diagnostics()
                    .iter()
                    .any(|diagnostic| diagnostic.rule == LintRule::AgentStopMissing)
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn test_a029_recognizes_documented_body_controls() {
        for body in [
            "Make at most 3 tool calls before returning the result.",
            "Use a timeout of\n15 minutes for the investigation.",
            "Make a maximum 3 attempts before returning the result.",
            "Use a timeout of 15 minutes for the investigation.",
            "Use a cost budget of $5 for the investigation.",
            "If there is no progress, stop and report the blocker.",
            "Stop after 3 attempts and report the remaining failure.",
            "Give up after 3 attempts and report the blocker.",
            "Set a limit of 5 attempts for the repair.",
            "On failure, escalate to the user with a summary.",
            "Upon failure, stop and report the blocker.",
            "When you cannot make progress, escalate to the user.",
            "Abort after 10 minutes and summarize progress.",
            "Retry at most 3 times, then report the failure.",
            "Make at most three attempts before reporting the failure.",
            "Timeout: 1.5 hours for the whole repair.",
            "Timeout: 2.5 minutes for the whole repair.",
            "Retry until success, but stop after 3 attempts.",
        ] {
            let content = format!(
                "---\nname: general\ndescription: {GOOD_DESC}\ntools: WebFetch\n---\n{body}\n"
            );
            run_agent(&content, |diag| {
                assert!(
                    !diag
                        .diagnostics()
                        .iter()
                        .any(|diagnostic| diagnostic.rule == LintRule::AgentStopMissing),
                    "A029 must accept documented control: {body}"
                );
            });
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_a029_rejects_vague_and_nonoperative_controls() {
        for content in [
            format!(
                "---\nname: general\ndescription: {GOOD_DESC}\ntools: Bash\n---\nBe careful, respect limits, and eventually finish.\n"
            ),
            format!(
                "---\nname: general\ndescription: {GOOD_DESC}\ntools: Bash\ncontrol: timeout 10 minutes\n---\nInvestigate the failure.\n"
            ),
            format!(
                "---\nname: general\ndescription: {GOOD_DESC}\ntools: Bash\n---\n```text\nUse a timeout of 10 minutes.\n```\nInvestigate the failure.\n"
            ),
        ] {
            run_agent(&content, |diag| {
                assert!(
                    diag.diagnostics()
                        .iter()
                        .any(|diagnostic| diagnostic.rule == LintRule::AgentStopMissing),
                    "A029 must not accept vague, frontmatter, or code-example text"
                );
            });
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_a029_rejects_example_and_descriptive_body_controls() {
        for body in [
            "# Examples\nUse a timeout of 10 minutes.\n\n# Task\nInvestigate the failure.\n",
            "The legacy runner used a timeout of 10 minutes for each investigation.\n",
            "# Example workflow\nIf there is no progress, stop and report the blocker.\n\n# Task\nInvestigate the failure and implement the repair.\n",
            "Timeout of 10 minutes was the legacy default.\n\nInvestigate the failure and implement the repair.\n",
            "Max 3 attempts was the old rule for the legacy runner.\n\nInvestigate the failure and implement the repair.\n",
            "Use of a timeout of 10 minutes was common in the legacy runner.\n\nInvestigate the failure and implement the repair.\n",
            "Budget pressures once forced a timeout of 10 minutes on the legacy runner.\n\nInvestigate the failure and implement the repair.\n",
        ] {
            let content =
                format!("---\nname: general\ndescription: {GOOD_DESC}\ntools: Bash\n---\n{body}");
            run_agent(&content, |diag| {
                assert!(
                    diag.diagnostics()
                        .iter()
                        .any(|diagnostic| diagnostic.rule == LintRule::AgentStopMissing),
                    "A029 must reject example or descriptive control text: {body}"
                );
            });
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_a029_skips_non_execution_tools_and_invalid_frontmatter() {
        for content in [
            format!(
                "---\nname: general\ndescription: {GOOD_DESC}\n---\nInvestigate the failure.\n"
            ),
            format!(
                "---\nname: general\ndescription: {GOOD_DESC}\ntools: Read, Grep, TaskList\n---\nInvestigate the failure.\n"
            ),
            format!(
                "---\nname: general\ndescription: {GOOD_DESC}\ntools: UnknownTool\n---\nInvestigate the failure.\n"
            ),
            format!(
                "---\nname: general\n\tdescription: {GOOD_DESC}\ntools: Bash\n---\nInvestigate the failure.\n"
            ),
        ] {
            run_agent(&content, |diag| {
                assert!(
                    !diag
                        .diagnostics()
                        .iter()
                        .any(|diagnostic| diagnostic.rule == LintRule::AgentStopMissing),
                    "A029 must require valid frontmatter and an explicit execution tool"
                );
            });
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_a029_applies_to_private_agents() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude/agents").unwrap();
        std::fs::write(
            ".claude/agents/general.md",
            format!(
                "---\nname: general\ndescription: {GOOD_DESC}\ntools: mcp__search__query\n---\n# Examples\nUse a timeout of 10 minutes.\n\n# Task\nInvestigate the failure.\n"
            ),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_private_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.diagnostics().iter().any(|diagnostic| {
            diagnostic.rule == LintRule::AgentStopMissing
                && diagnostic.subject_path.as_deref()
                    == Some(Path::new(".claude/agents/general.md"))
        }));
    }

    // ── A014: agent-model-invalid ────────────────────────────────────

    #[test]
    fn test_is_valid_model_aliases_and_ids() {
        for ok in [
            "sonnet",
            "opus",
            "haiku",
            "inherit",
            "default",
            "claude-sonnet-5",
            "claude-sonnet-4-5",
            "claude-sonnet-4-20250514",
            "claude-opus-4-1",
            "claude-haiku-4-5",
            "sonnet[1m]",
            "claude-sonnet-5[1m]",
            "claude-opus-4-1[2m]",
        ] {
            assert!(is_valid_model(ok), "expected '{ok}' to be valid");
        }
        for bad in [
            "sonet",
            "olus",
            "gpt-4",
            "claude-",
            "claude-sonnet-",
            "claude-sonnet",
            "",
        ] {
            assert!(!is_valid_model(bad), "expected '{bad}' to be invalid");
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_aPH14PH_invalid_model_fires() {
        let content =
            format!("---\nname: general\ndescription: {GOOD_DESC}\nmodel: sonet\n---\nBody\n");
        run_agent(&content, |diag| {
            assert!(
                diag.errors()
                    .iter()
                    .any(|e| e.contains("not a recognized Claude Code model"))
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn test_aPH14PH_valid_model_no_fire() {
        let content = format!(
            "---\nname: general\ndescription: {GOOD_DESC}\nmodel: claude-sonnet-5[1m]\n---\nBody\n"
        );
        run_agent(&content, |diag| {
            assert!(!diag.errors().iter().any(|e| e.contains("model")));
        });
    }

    #[test]
    #[serial_test::serial]
    fn test_aPH14PH_missing_model_no_fire() {
        let content = format!("---\nname: general\ndescription: {GOOD_DESC}\n---\nBody\n");
        run_agent(&content, |diag| {
            assert!(!diag.errors().iter().any(|e| e.contains("model")));
        });
    }

    // ── A015: agent-permission-invalid ───────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_aPH15PH_invalid_permission_fires() {
        let content = format!(
            "---\nname: general\ndescription: {GOOD_DESC}\npermissionMode: yolo\n---\nBody\n"
        );
        run_agent(&content, |diag| {
            assert!(
                diag.errors()
                    .iter()
                    .any(|e| e.contains("permissionMode 'yolo'"))
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn test_aPH15PH_valid_permission_no_fire() {
        let content = format!(
            "---\nname: general\ndescription: {GOOD_DESC}\npermissionMode: acceptEdits\n---\nBody\n"
        );
        run_agent(&content, |diag| {
            // A015 must not flag a valid enum value. Match the A015 message for
            // this value rather than the bare field name: A028 legitimately
            // reports `permissionMode` as unsupported for plugin agents.
            assert!(
                !diag
                    .errors()
                    .iter()
                    .any(|e| e.contains("permissionMode 'acceptEdits'"))
            );
        });
    }

    // ── A016: agent-skill-missing ────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_aPH16PH_missing_skill_fires() {
        let content = format!(
            "---\nname: general\ndescription: {GOOD_DESC}\nskills:\n  - missing-skill\n---\nBody\n"
        );
        run_agent(&content, |diag| {
            assert!(
                diag.errors()
                    .iter()
                    .any(|e| e.contains("no matching skills/missing-skill"))
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn test_aPH16PH_existing_skill_no_fire() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write("skills/my-skill/SKILL.md", "---\nname: my-skill\n---\n").unwrap();
        std::fs::write(
            "agents/general.md",
            format!(
                "---\nname: general\ndescription: {GOOD_DESC}\nskills:\n  - my-skill\n---\nBody\n"
            ),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("no matching skills"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_aPH16PH_existing_private_skill_no_fire() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::create_dir_all(".claude/skills/my-skill").unwrap();
        std::fs::write(
            ".claude/skills/my-skill/SKILL.md",
            "---\nname: my-skill\n---\n",
        )
        .unwrap();
        std::fs::write(
            "agents/general.md",
            format!("---\nname: general\ndescription: {GOOD_DESC}\nskills: my-skill\n---\nBody\n"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("no matching skills"))
        );
    }

    // ── A017: agent-tools-overlap ────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_aPH17PH_overlap_fires() {
        let content = format!(
            "---\nname: general\ndescription: {GOOD_DESC}\ntools: Bash, Read\ndisallowedTools: Read\n---\nBody\n"
        );
        run_agent(&content, |diag| {
            assert!(
                diag.errors()
                    .iter()
                    .any(|e| e.contains("appears in both tools and disallowedTools"))
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn test_aPH17PH_disjoint_no_fire() {
        let content = format!(
            "---\nname: general\ndescription: {GOOD_DESC}\ntools: Bash, Read\ndisallowedTools: Write\n---\nBody\n"
        );
        run_agent(&content, |diag| {
            assert!(!diag.errors().iter().any(|e| e.contains("appears in both")));
        });
    }

    // ── A018: agent-memory-invalid ───────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_aPH18PH_invalid_memory_fires() {
        let content =
            format!("---\nname: general\ndescription: {GOOD_DESC}\nmemory: global\n---\nBody\n");
        run_agent(&content, |diag| {
            assert!(diag.errors().iter().any(|e| e.contains("memory 'global'")));
        });
    }

    #[test]
    #[serial_test::serial]
    fn test_aPH18PH_valid_memory_no_fire() {
        let content =
            format!("---\nname: general\ndescription: {GOOD_DESC}\nmemory: project\n---\nBody\n");
        run_agent(&content, |diag| {
            assert!(!diag.errors().iter().any(|e| e.contains("memory")));
        });
    }

    // ── A019/A020: agent-tools-unknown / agent-disallowed-unknown ────

    #[test]
    #[serial_test::serial]
    fn test_aPH19PH_unknown_tool_fires() {
        let content =
            format!("---\nname: general\ndescription: {GOOD_DESC}\ntools: Bash, Bsh\n---\nBody\n");
        run_agent(&content, |diag| {
            assert!(
                diag.errors()
                    .iter()
                    .any(|e| e.contains("tools lists unrecognized tool 'Bsh'"))
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn test_aPH19PH_known_and_mcp_no_fire() {
        let content = format!(
            "---\nname: general\ndescription: {GOOD_DESC}\ntools: Bash, mcp__github__create_pr\n---\nBody\n"
        );
        run_agent(&content, |diag| {
            assert!(
                !diag
                    .errors()
                    .iter()
                    .any(|e| e.contains("unrecognized tool"))
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn test_aPH20PH_unknown_disallowed_fires() {
        let content = format!(
            "---\nname: general\ndescription: {GOOD_DESC}\ndisallowedTools: Bsh\n---\nBody\n"
        );
        run_agent(&content, |diag| {
            assert!(
                diag.errors()
                    .iter()
                    .any(|e| e.contains("disallowedTools lists unrecognized tool 'Bsh'"))
            );
        });
    }

    // ── A021: agent-bypass-permissions (warn) ────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_aPH21PH_bypass_fires_as_warning() {
        let content = format!(
            "---\nname: general\ndescription: {GOOD_DESC}\npermissionMode: bypassPermissions\n---\nBody\n"
        );
        // Under default config A021 is a warning and A015 (enum) must NOT fire.
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::write("agents/general.md", content).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
        assert!(
            diag.warnings()
                .iter()
                .any(|w| w.contains("bypassPermissions"))
        );
    }

    // ── A022: agent-skill-kebab (warn) ───────────────────────────────

    #[test]
    fn test_is_kebab_case() {
        for ok in ["my-skill", "a", "skill-1", "abc-def-ghi"] {
            assert!(is_kebab_case(ok), "expected '{ok}' kebab-case");
        }
        for bad in ["", "My_Skill", "my_skill", "-x", "x-", "a--b", "Camel"] {
            assert!(!is_kebab_case(bad), "expected '{bad}' not kebab-case");
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_aPH22PH_non_kebab_skill_fires() {
        let content =
            format!("---\nname: general\ndescription: {GOOD_DESC}\nskills: My_Skill\n---\nBody\n");
        run_agent(&content, |diag| {
            assert!(
                diag.errors()
                    .iter()
                    .any(|e| e.contains("'My_Skill' is not kebab-case"))
            );
        });
    }

    // ── A023: agent-effort-invalid ───────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_aPH23PH_invalid_effort_fires() {
        let content =
            format!("---\nname: general\ndescription: {GOOD_DESC}\neffort: turbo\n---\nBody\n");
        run_agent(&content, |diag| {
            assert!(diag.errors().iter().any(|e| e.contains("effort 'turbo'")));
        });
    }

    #[test]
    #[serial_test::serial]
    fn test_aPH23PH_valid_effort_xhigh_no_fire() {
        let content =
            format!("---\nname: general\ndescription: {GOOD_DESC}\neffort: xhigh\n---\nBody\n");
        run_agent(&content, |diag| {
            assert!(!diag.errors().iter().any(|e| e.contains("effort")));
        });
    }

    // ── A024: agent-isolation-invalid ────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_aPH24PH_invalid_isolation_fires() {
        let content = format!(
            "---\nname: general\ndescription: {GOOD_DESC}\nisolation: container\n---\nBody\n"
        );
        run_agent(&content, |diag| {
            assert!(
                diag.errors()
                    .iter()
                    .any(|e| e.contains("isolation 'container'"))
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn test_aPH24PH_valid_isolation_no_fire() {
        let content = format!(
            "---\nname: general\ndescription: {GOOD_DESC}\nisolation: worktree\n---\nBody\n"
        );
        run_agent(&content, |diag| {
            assert!(!diag.errors().iter().any(|e| e.contains("isolation")));
        });
    }

    // ── A025: agent-background-invalid (warn) ────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_aPH25PH_invalid_background_fires() {
        let content =
            format!("---\nname: general\ndescription: {GOOD_DESC}\nbackground: yes\n---\nBody\n");
        run_agent(&content, |diag| {
            assert!(diag.errors().iter().any(|e| e.contains("background 'yes'")));
        });
    }

    #[test]
    #[serial_test::serial]
    fn test_aPH25PH_valid_background_no_fire() {
        let content =
            format!("---\nname: general\ndescription: {GOOD_DESC}\nbackground: false\n---\nBody\n");
        run_agent(&content, |diag| {
            assert!(!diag.errors().iter().any(|e| e.contains("background")));
        });
    }

    // ── A026: agent-maxturns-invalid ─────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_aPH26PH_invalid_maxturns_fires() {
        for val in ["0", "-5", "abc", "3.5", "\"3\"", "true", "[3]"] {
            let content = format!(
                "---\nname: general\ndescription: {GOOD_DESC}\nmaxTurns: {val}\n---\nBody\n"
            );
            run_agent(&content, |diag| {
                assert!(
                    diag.errors()
                        .iter()
                        .any(|e| e.contains("maxTurns") && e.contains("positive integer")),
                    "expected maxTurns '{val}' to fire"
                );
            });
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_aPH26PH_valid_maxturns_no_fire() {
        let content =
            format!("---\nname: general\ndescription: {GOOD_DESC}\nmaxTurns: 5\n---\nBody\n");
        run_agent(&content, |diag| {
            assert!(!diag.errors().iter().any(|e| e.contains("maxTurns")));
        });
    }

    // ── A027: agent-field-unknown (warn) ─────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_aPH27PH_unknown_field_fires() {
        let content =
            format!("---\nname: general\ndescription: {GOOD_DESC}\nmode: plan\n---\nBody\n");
        run_agent(&content, |diag| {
            assert!(
                diag.errors()
                    .iter()
                    .any(|e| e.contains("unrecognized frontmatter field 'mode'"))
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn test_aPH27PH_known_fields_no_fire() {
        let content = format!(
            "---\nname: general\ndescription: {GOOD_DESC}\nmodel: sonnet\neffort: high\n---\nBody\n"
        );
        run_agent(&content, |diag| {
            assert!(
                !diag
                    .errors()
                    .iter()
                    .any(|e| e.contains("unrecognized frontmatter field"))
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn test_aPH27PH_warns_under_default_config() {
        let content =
            format!("---\nname: general\ndescription: {GOOD_DESC}\ntypoField: 1\n---\nBody\n");
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::write("agents/general.md", content).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
        assert!(
            diag.warnings()
                .iter()
                .any(|w| w.contains("unrecognized frontmatter field"))
        );
    }

    // ── get_field_items helper ───────────────────────────────────────

    #[test]
    fn test_get_field_items_scalar_and_list() {
        let scalar = "---\ntools: Bash, Read , Write\n---\n";
        let fm = frontmatter::extract_frontmatter(scalar).unwrap();
        assert_eq!(get_field_items(&fm, "tools"), vec!["Bash", "Read", "Write"]);

        let list = "---\ntools:\n  - Bash\n  - \"Read\"\n  - Write\n---\n";
        let fm = frontmatter::extract_frontmatter(list).unwrap();
        assert_eq!(get_field_items(&fm, "tools"), vec!["Bash", "Read", "Write"]);

        let empty = "---\ntools:\n---\n";
        let fm = frontmatter::extract_frontmatter(empty).unwrap();
        assert!(get_field_items(&fm, "tools").is_empty());

        // YAML flow sequence: `tools: [Bash, Read]` must parse as two items,
        // not as the literal strings "[Bash" and "Read]".
        let flow = "---\ntools: [Bash, Read]\n---\n";
        let fm = frontmatter::extract_frontmatter(flow).unwrap();
        assert_eq!(get_field_items(&fm, "tools"), vec!["Bash", "Read"]);

        // Single-item flow sequence with a quoted entry.
        let flow_one = "---\nskills: [\"my-skill\"]\n---\n";
        let fm = frontmatter::extract_frontmatter(flow_one).unwrap();
        assert_eq!(get_field_items(&fm, "skills"), vec!["my-skill"]);

        // Empty flow sequence.
        let flow_empty = "---\ntools: []\n---\n";
        let fm = frontmatter::extract_frontmatter(flow_empty).unwrap();
        assert!(get_field_items(&fm, "tools").is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn test_aPH19PH_flow_sequence_no_false_positive() {
        // Flow-sequence tools must not be falsely flagged as unknown.
        let content = format!(
            "---\nname: general\ndescription: {GOOD_DESC}\ntools: [Bash, Read]\n---\nBody\n"
        );
        run_agent(&content, |diag| {
            assert!(
                !diag
                    .errors()
                    .iter()
                    .any(|e| e.contains("unrecognized tool")),
                "flow-sequence tools should not fire A019: {:?}",
                diag.errors()
            );
        });
    }

    // ── Private agents (Basic mode) ──────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_private_agents_validates_claude_agents() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude/agents").unwrap();
        std::fs::write(
            ".claude/agents/general.md",
            format!("---\nname: general\ndescription: {GOOD_DESC}\nmodel: sonet\n---\nBody\n"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_private_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains(".claude/agents/general.md") && e.contains("model"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_private_agents_missing_dir_no_error() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_private_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_cc_ag_011_agent_hooks_schema() {
        let content = format!(
            "---\nname: general\ndescription: {GOOD_DESC}\nhooks:\n  NotAnEvent:\n    - hooks:\n        - type: command\n          command: echo hi\n---\nBody\n"
        );
        run_agent(&content, |diag| {
            assert!(
                diag.diagnostics().iter().any(|d| {
                    d.rule == LintRule::HookEventInvalid && d.message.contains("frontmatter")
                }),
                "expected H008 on agent frontmatter: {:?}",
                diag.diagnostics()
                    .iter()
                    .map(|d| format!("{}:{}", d.rule.code(), d.message))
                    .collect::<Vec<_>>()
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn test_x001_agent_invalid_yaml() {
        let content = format!("---\nname: general\n\tdescription: {GOOD_DESC}\n---\nBody\n");
        run_agent(&content, |diag| {
            assert!(
                diag.diagnostics()
                    .iter()
                    .any(|d| d.rule == LintRule::FrontmatterYamlInvalid)
            );
        });
    }
}
