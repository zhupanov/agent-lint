use crate::config::ExcludeSet;
use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata, SourceSpan};
use crate::frontmatter::{self, LeadingFrontmatterState};
use crate::live_instructions::{InstructionSurfaceKind, LiveInstructionDocument};
use crate::markdown::MarkdownDocument;
use crate::rules::LintRule;
use crate::traversal;
use regex::Regex;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::num::NonZeroU64;
use std::path::Path;
use std::sync::LazyLock;

use super::common::{
    NEVER_INVENT_PROHIBITION, RE_NAME_INVALID, has_bound_or_fallback, is_known_tool_name,
    is_valid_model_value, normalize_description_suffix, normalize_emphasis_for_gates,
    sentence_ranges,
};

/// Jaccard similarity threshold (strict greater-than).
const JACCARD_THRESHOLD: f64 = 0.8;
/// Descriptions with fewer than this many words are eligible for Jaccard flagging.
const MIN_DESC_WORDS: usize = 6;
const REVIEWER_TEMPLATE_PATH: &str = "skills/shared/reviewer-templates.md";
const TEMPLATE_MARKER: &str = "Derived from skills/shared/reviewer-templates.md";

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

const CURRENT_AGENT_SUBJECT: &str = r"(?:you|the\s+agent|this\s+agent|agents?|assistant|model)";
const CURRENT_AGENT_MODAL: &str = r"(?:must|shall|should|will|need\s+to)";
const SETUP_CLAUSE: &str = r"(?:if|when|before|after|unless)\b[^.!?;,:]*,\s*";
const OPTIONAL_POLITENESS: &str = r"(?:(?:always|please)\s+)?";

static EXPLICIT_READ_MANDATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?ix)^\s*(?:[-*+]\s*)?(?:
            {politeness}use\s+(?:the\s+)?Read(?:\s+tool)?\b |
            {subject}\s+{modal}\s+{politeness}use\s+(?:the\s+)?Read(?:\s+tool)?\b |
            {setup}(?:{subject}\s+{modal}\s+)?{politeness}use\s+(?:the\s+)?Read(?:\s+tool)?\b
        )",
        politeness = OPTIONAL_POLITENESS,
        subject = CURRENT_AGENT_SUBJECT,
        modal = CURRENT_AGENT_MODAL,
        setup = SETUP_CLAUSE,
    ))
    .expect("A012 explicit Read mandate regex is valid")
});

static EVIDENCE_READ_MANDATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?ix)^\s*(?:[-*+]\s*)?(?:
            {politeness}(?:read|open)\s+(?:(?:the|each|every|all|any)\s+)?[^.!?\n]{{0,80}}\b(?:file|files|bundle|bundles|path|paths|diff|diffs|body|bodies|artifact|artifacts|markdown|log|logs)\b |
            {subject}\s+{modal}\s+{politeness}(?:read|open)\s+(?:(?:the|each|every|all|any)\s+)?[^.!?\n]{{0,80}}\b(?:file|files|bundle|bundles|path|paths|diff|diffs|body|bodies|artifact|artifacts|markdown|log|logs)\b |
            {setup}(?:{subject}\s+{modal}\s+)?{politeness}(?:read|open)\s+(?:(?:the|each|every|all|any)\s+)?[^.!?\n]{{0,80}}\b(?:file|files|bundle|bundles|path|paths|diff|diffs|body|bodies|artifact|artifacts|markdown|log|logs)\b
        )",
        politeness = OPTIONAL_POLITENESS,
        subject = CURRENT_AGENT_SUBJECT,
        modal = CURRENT_AGENT_MODAL,
        setup = SETUP_CLAUSE,
    ))
    .expect("A013 evidence-read mandate regex is valid")
});

static JSON_OUTPUT_MANDATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?ix)^\s*(?:[-*+]\s*)?(?:
            {politeness}(?:emit|output|return|respond\s+with|reply\s+with)\s+(?:strict\s+|valid\s+)?JSONL?\s+only\b |
            {politeness}only\s+(?:emit|output|return)\s+(?:strict\s+|valid\s+)?JSONL?\b |
            {politeness}output\s+must\s+be\s+(?:strict\s+|valid\s+)?JSONL?\b |
            {subject}\s+{modal}\s+(?:{politeness})?(?:(?:emit|output|return|respond\s+with|reply\s+with)\s+(?:strict\s+|valid\s+)?JSONL?\s+only|only\s+(?:emit|output|return)\s+(?:strict\s+|valid\s+)?JSONL?|output\s+must\s+be\s+(?:strict\s+|valid\s+)?JSONL?)\b |
            {setup}(?:{subject}\s+{modal}\s+)?{politeness}(?:(?:emit|output|return|respond\s+with|reply\s+with)\s+(?:strict\s+|valid\s+)?JSONL?\s+only|only\s+(?:emit|output|return)\s+(?:strict\s+|valid\s+)?JSONL?|output\s+must\s+be\s+(?:strict\s+|valid\s+)?JSONL?)\b
        )",
        politeness = OPTIONAL_POLITENESS,
        subject = CURRENT_AGENT_SUBJECT,
        modal = CURRENT_AGENT_MODAL,
        setup = SETUP_CLAUSE,
    ))
    .expect("A013 JSON-only output mandate regex is valid")
});

static UNREADABLE_EVIDENCE_OUTCOME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?ix)\b(?:if|when|for)\s+(?:(?:an?|the)\s+)?(?:evidence\s+)?(?:file|files|evidence|artifact|artifacts|path|paths)?\s*(?:is\s+)?(?:unreadable|unavailable|missing)\b[^.!?\n]{0,120}\b(?:return|report|emit|output|respond|reply|stop|abort|escalate|fail)\b|\b(?:cannot|can't|could\s+not|unable\s+to)\s+(?:read|open)\b[^.!?\n]{0,120}\b(?:return|report|emit|output|respond|reply|stop|abort|escalate|fail)\b",
    )
    .expect("A013 unreadable-evidence outcome regex is valid")
});

/// Check whether an agent description is too similar to the agent name.
///
/// Returns `true` when the description adds no meaningful information beyond
/// what the name already conveys.
fn is_desc_redundant(name: &str, desc: &str) -> bool {
    let name_words: HashSet<String> = name
        .to_lowercase()
        .replace('-', " ")
        .split_whitespace()
        .map(normalize_agent_redundancy_token)
        .collect();

    // Strip leading/trailing punctuation from each token so "analyzer." matches "analyzer".
    let desc_stripped: Vec<String> = desc
        .to_lowercase()
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_string())
        .filter(|w| !w.is_empty())
        .map(|word| normalize_agent_redundancy_token(&word))
        .collect();
    let desc_word_count = desc_stripped.len();

    let stopwords: HashSet<&str> = STOPWORDS.iter().copied().collect();
    let desc_content_words: HashSet<String> = desc_stripped
        .iter()
        .filter(|w| !stopwords.contains(w.as_str()))
        .cloned()
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

/// Preserve #319's shared deterministic suffix normalization, then apply the
/// two recorded agent-role forms. This remains lexical and finite rather than
/// using a general stemmer or semantic model.
fn normalize_agent_redundancy_token(token: &str) -> String {
    let normalized = normalize_description_suffix(token);
    match normalized.as_str() {
        "reviewer" => "review".to_string(),
        "runner" => "run".to_string(),
        _ => normalized,
    }
}

/// V7: Validate agents/*.md frontmatter.
#[cfg(test)]
pub fn validate_agents(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let mut prompt_pass = super::prompt_content::PromptContentPass::default();
    validate_agents_with_prompt_pass(diag, exclude, &[], &mut prompt_pass);
}

/// V7: Validate plugin agent frontmatter and field values across the default
/// `agents/` directory and every manifest-declared agent root.
///
/// Discovery is recursive and shared through [`super::agent_discovery`], so an
/// agent nested in a subdirectory is validated exactly like a top-level one.
/// A001 fires for an explicitly declared root that is missing; A004 fires for a
/// present root that holds no agent `.md` files. The implicit default `agents/`
/// is optional: its absence is clean, and only a present-but-empty default
/// reports A004. `declared_roots` are the repository-safe, normalized,
/// deduplicated paths from plugin.json `agents`, in declaration order (the A001
/// once-per-path guarantee relies on that deduplication).
pub(crate) fn validate_agents_with_prompt_pass(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    declared_roots: &[String],
    prompt_pass: &mut super::prompt_content::PromptContentPass,
) {
    use super::agent_discovery;

    // Ordered, path-deduplicated scan set: the implicit default `agents/` first,
    // then each declared root in declaration order.
    let mut scan_order: Vec<&str> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for root in std::iter::once("agents").chain(declared_roots.iter().map(String::as_str)) {
        if seen.insert(root) {
            scan_order.push(root);
        }
    }
    let roots: Vec<agent_discovery::AgentRoot> = scan_order
        .iter()
        .map(|root| agent_discovery::discover_root(root, exclude))
        .collect();
    let exists_by_path: HashMap<&str, bool> = roots
        .iter()
        .map(|root| (root.path.as_str(), root.exists))
        .collect();

    // A001: an explicitly declared agent path that does not exist, once per
    // distinct normalized path in declaration order. The implicit default's
    // absence is legal and is never reported here.
    for declared in declared_roots {
        if exists_by_path.get(declared.as_str()) == Some(&false) {
            diag.report_at(
                LintRule::AgentsDirMissing,
                Path::new(declared),
                &format!("plugin.json declares agents path '{declared}' but it does not exist"),
            );
        }
    }

    // A004: a present root (default or declared) holding zero agent files. A root
    // whose only files are excluded is not empty — its files exist before
    // exclusion — so all-excluded roots stay silent.
    for root in &roots {
        if root.exists && root.inventory.all_files.is_empty() {
            diag.report_at(
                LintRule::NoAgentFiles,
                Path::new(&root.path),
                &format!("{} contains no agent .md files", root.path),
            );
        }
    }

    // Per-file frontmatter and field-value validation over the merged,
    // exclusion-filtered set, deduplicated across overlapping roots.
    for agent_path in &agent_discovery::merge(&roots).lint_files {
        let content = match fs::read_to_string(agent_path) {
            Ok(content) => content,
            Err(_) => continue,
        };
        diag.with_subject_path(agent_path, |diag| {
            validate_agent_file(
                diag,
                agent_path,
                &content,
                prompt_pass,
                AgentSurface::Plugin,
            );
        });
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
    validate_private_agents_with_prompt_pass(diag, exclude, &mut prompt_pass, false);
}

pub(crate) fn validate_private_agents_with_prompt_pass(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    prompt_pass: &mut super::prompt_content::PromptContentPass,
    plugin_runtime: bool,
) {
    let surface = AgentSurface::Private { plugin_runtime };
    // Recursive discovery: agents nested under `.claude/agents/` are validated
    // exactly like top-level ones. `.claude/agents/` is optional and never
    // reports A001/A004.
    let inventory = super::agent_discovery::discover_root(".claude/agents", exclude).inventory;
    for agent_path in &inventory.lint_files {
        let content = match fs::read_to_string(agent_path) {
            Ok(content) => content,
            Err(_) => continue,
        };
        diag.with_subject_path(agent_path, |diag| {
            validate_agent_file(diag, agent_path, &content, prompt_pass, surface);
        });
    }
    validate_private_agent_name_duplicates(diag, &inventory.lint_files);
}

/// Report duplicate name-based identities among private Claude agents.
///
/// Claude Code resolves `.claude/agents/` identities from frontmatter `name`,
/// so only this tree participates. Plugin `agents/` files use path-derived
/// registered IDs and deliberately remain outside A031's comparison scope.
/// Discovery has already applied exclusions, leaving the primary (first sorted)
/// path available for per-file override policy.
fn validate_private_agent_name_duplicates(diag: &mut DiagnosticCollector, paths: &[String]) {
    let mut paths_by_name: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for path in paths {
        let Ok(content) = fs::read_to_string(path) else {
            continue;
        };
        let markdown = MarkdownDocument::parse(content);
        let Some(frontmatter) = markdown.frontmatter() else {
            continue;
        };
        // A002, A003, and X001 own malformed, missing, blank, and non-string
        // names. Only their canonical strict-YAML values enter this index.
        let Some(name) = frontmatter::get_strict_string_field(frontmatter, "name") else {
            continue;
        };
        paths_by_name.entry(name).or_default().push(path);
    }

    for (name, participants) in paths_by_name {
        if participants.len() < 2 {
            continue;
        }
        let primary = participants[0];
        let related_subjects = &participants[1..];
        let count = participants.len();
        diag.report_at_with(
            LintRule::AgentNameDuplicate,
            primary,
            &format!(
                "agent name '{name}' is declared by {count} files; agent identity comes from the name field, so only one definition can be active"
            ),
            DiagnosticMetadata::default()
                .with_related_subjects(related_subjects)
                .with_suggestion("rename or remove the duplicate agent definitions"),
        );
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
    surface: AgentSurface,
) {
    let markdown = MarkdownDocument::parse(content);
    let fm_lines: Vec<String> = match frontmatter::leading_frontmatter(content) {
        LeadingFrontmatterState::Absent { .. } => {
            let bom_hint = if starts_with_bom_delimiter(content) {
                "; file starts with a UTF-8 byte-order mark; remove it"
            } else {
                ""
            };
            diag.report_with(
                LintRule::AgentFrontmatterMalformed,
                &format!("{agent_path}: frontmatter must start with '---' on line 1{bom_hint}"),
                DiagnosticMetadata::at_line(1)
                    .with_suggestion("insert '---' as the first line of the frontmatter"),
            );
            // X002–X005 still apply when frontmatter is broken.
            super::markdown_structure::check_markdown_document(agent_path, &markdown, diag);
            // Shared exact-delimiter recovery: absent openers keep live prose;
            // BOM-aware complete blocks are body-only via prompt recovery parse.
            if let Some(prompt_markdown) = MarkdownDocument::parse_for_prompt_content(content) {
                let prompt_document = LiveInstructionDocument::new(
                    Path::new(agent_path),
                    InstructionSurfaceKind::Agent,
                    &prompt_markdown,
                );
                prompt_pass.validate(&prompt_document, diag);
            }
            return;
        }
        LeadingFrontmatterState::Unterminated { .. } => {
            diag.report_with(
                LintRule::AgentFrontmatterMalformed,
                &format!("{agent_path}: frontmatter opening delimiter has no closing '---'"),
                DiagnosticMetadata::at_line(1)
                    .with_suggestion("insert a closing '---' delimiter after the frontmatter"),
            );
            super::markdown_structure::check_markdown_document(agent_path, &markdown, diag);
            // Exact opener without closer has no body boundary; Q rules skip.
            return;
        }
        LeadingFrontmatterState::Complete(block) => block.yaml.lines().map(str::to_owned).collect(),
    };

    // X001: strict YAML; CC-AG-011: hooks schema when present.
    let (parsed_frontmatter, non_mapping_frontmatter) =
        match frontmatter::parse_yaml_strict(&fm_lines) {
            Ok(yaml) => {
                let agent_frontmatter = AgentFrontmatter::from_yaml(&yaml, &fm_lines);
                // A028 owns unsupported plugin fields outright, including hooks.
                if !surface.is_plugin_agent()
                    && let Some(hooks) = yaml.get("hooks")
                {
                    super::hook_schema::validate_frontmatter_hooks(
                        hooks,
                        &format!("{agent_path} frontmatter"),
                        diag,
                    );
                }
                let non_mapping = agent_frontmatter.is_none();
                (agent_frontmatter, non_mapping)
            }
            Err(err) => {
                let metadata = match err.column {
                    Some(column) => DiagnosticMetadata::at_point(err.file_line, column),
                    None => DiagnosticMetadata::at_line(err.file_line),
                };
                diag.report_with(
                    LintRule::FrontmatterYamlInvalid,
                    &format!(
                        "{agent_path}:{}: frontmatter is not valid YAML: {}",
                        err.file_line, err.message
                    ),
                    metadata,
                );
                (None, false)
            }
        };

    // X002–X005 on the full agent markdown file.
    super::markdown_structure::check_markdown_document(agent_path, &markdown, diag);

    if let Some(frontmatter) = parsed_frontmatter.as_ref() {
        check_agent_required_fields(diag, agent_path, frontmatter);
        if let (RequiredAgentString::Valid(name), RequiredAgentString::Valid(description)) = (
            frontmatter.required_string("name"),
            frontmatter.required_string("description"),
        ) {
            check_agent_name_and_description(diag, agent_path, frontmatter, name, description);
        }
    } else if non_mapping_frontmatter {
        // Valid YAML with a non-mapping root has exactly one structural owner.
        diag.report_with(
            LintRule::AgentFieldMissing,
            &format!(
                "{agent_path}: agent frontmatter must be a mapping with required string fields"
            ),
            DiagnosticMetadata::at_line(1).with_suggestion(
                "make the frontmatter a YAML mapping with name and description strings",
            ),
        );
    }

    let max_turns = parsed_frontmatter
        .as_ref()
        .and_then(AgentFrontmatter::max_turns);
    if let Some(frontmatter) = parsed_frontmatter.as_ref() {
        check_agent_field_values(diag, agent_path, frontmatter, max_turns, surface);
        if surface.is_plugin_agent() {
            check_unsupported_plugin_fields(diag, agent_path, frontmatter);
        }
    }
    let Some(prompt_markdown) = MarkdownDocument::parse_for_prompt_content(content) else {
        return;
    };
    let prompt_document = LiveInstructionDocument::new(
        Path::new(agent_path),
        InstructionSurfaceKind::Agent,
        &prompt_markdown,
    )
    .with_outer_max_turns(max_turns);
    if let Some(parsed_frontmatter) = parsed_frontmatter.as_ref() {
        check_agent_evidence_contracts(diag, agent_path, parsed_frontmatter, &prompt_document);
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

/// Return true only for a UTF-8 BOM immediately followed by an otherwise
/// correct opening delimiter on line 1. This is source recognition for the
/// A002 hint, not a second frontmatter parser.
fn starts_with_bom_delimiter(content: &str) -> bool {
    let Some(rest) = content.strip_prefix('\u{feff}') else {
        return false;
    };
    let line = rest.split_once('\n').map_or(rest, |(line, _)| line);
    let line = line.strip_suffix('\r').unwrap_or(line);
    line == "---"
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
    "color",
    "background",
    "skills",
    "memory",
    "effort",
    "initialPrompt",
    "hooks",
    "mcpServers",
];

/// Allowed `permissionMode` values (CC-AG-004).
const VALID_PERMISSION_MODES: &[&str] = &[
    "default",
    "acceptEdits",
    "auto",
    "dontAsk",
    "bypassPermissions",
    "plan",
    "manual",
];

/// Allowed `memory` values (CC-AG-008).
const VALID_MEMORY: &[&str] = &["user", "project", "local"];

/// Allowed `effort` values (CC-AG-014; superset of the skill S025 set).
const VALID_EFFORT: &[&str] = &["low", "medium", "high", "xhigh", "max"];

#[derive(Clone, Copy, PartialEq, Eq)]
enum AgentSurface {
    Plugin,
    Private { plugin_runtime: bool },
}

impl AgentSurface {
    fn is_plugin_agent(self) -> bool {
        matches!(self, Self::Plugin)
    }

    fn uses_plugin_skill_namespace(self) -> bool {
        matches!(
            self,
            Self::Plugin
                | Self::Private {
                    plugin_runtime: true
                }
        )
    }
}

/// Canonical, strictly parsed agent frontmatter. A non-mapping YAML document
/// deliberately has no view: X001/A003 own structural YAML failures, while
/// A014-A029 only consume fields from this one mapping.
#[derive(Clone)]
struct AgentFrontmatter {
    mapping: crate::yaml::Mapping,
    key_lines: HashMap<String, usize>,
}

impl AgentFrontmatter {
    fn from_yaml(value: &crate::yaml::Value, fm_lines: &[String]) -> Option<Self> {
        let mapping = value.as_mapping()?.clone();
        // The strict Value remains the semantic authority. The CST is used
        // only to map already-accepted canonical keys back to source lines.
        let source = fm_lines.join("\n");
        let document = noyalib::cst::parse_document(&source).ok();
        let key_lines = mapping
            .keys()
            .filter_map(|key| {
                document
                    .as_ref()
                    .and_then(|document| document.span_at(key))
                    .and_then(|(start, end)| SourceSpan::from_byte_range(&source, start..end))
                    .map(|span| (key.clone(), span.start().line_number() + 1))
            })
            .collect();
        Some(Self { mapping, key_lines })
    }

    fn value(&self, key: &str) -> Option<&crate::yaml::Value> {
        self.mapping.get(key)
    }

    fn max_turns(&self) -> Option<NonZeroU64> {
        self.value("maxTurns")?.as_u64().and_then(NonZeroU64::new)
    }

    fn keys(&self) -> impl Iterator<Item = &str> {
        self.mapping.keys().map(String::as_str)
    }

    fn field_line(&self, key: &str) -> Option<usize> {
        self.key_lines.get(key).copied()
    }

    fn required_string(&self, key: &str) -> RequiredAgentString<'_> {
        match self.value(key) {
            None => RequiredAgentString::Missing,
            Some(value) => match value.as_str() {
                Some(string) if string.trim().is_empty() => RequiredAgentString::Blank,
                Some(string) => RequiredAgentString::Valid(string),
                None => RequiredAgentString::WrongType(yaml_type(value)),
            },
        }
    }
}

enum RequiredAgentString<'a> {
    Missing,
    Blank,
    WrongType(&'static str),
    Valid(&'a str),
}

fn check_agent_required_fields(
    diag: &mut DiagnosticCollector,
    agent_path: &str,
    frontmatter: &AgentFrontmatter,
) {
    for key in ["name", "description"] {
        let (message, suggestion) = match frontmatter.required_string(key) {
            RequiredAgentString::Missing => (
                format!("{agent_path}: missing required frontmatter field '{key}'"),
                format!(
                    "add {key}: <{}>",
                    if key == "name" {
                        "agent-name"
                    } else {
                        "routing description"
                    }
                ),
            ),
            RequiredAgentString::Blank => (
                format!("{agent_path}: required frontmatter field '{key}' must not be blank"),
                format!("set {key} to a non-blank string"),
            ),
            RequiredAgentString::WrongType(actual) => (
                format!(
                    "{agent_path}: required frontmatter field '{key}' must be a string (found {actual})"
                ),
                format!("replace {key} with a string value"),
            ),
            RequiredAgentString::Valid(_) => continue,
        };
        let line = frontmatter.field_line(key).unwrap_or(1);
        diag.report_with(
            LintRule::AgentFieldMissing,
            &message,
            DiagnosticMetadata::at_line(line).with_suggestion(suggestion),
        );
    }
}

fn check_agent_name_and_description(
    diag: &mut DiagnosticCollector,
    agent_path: &str,
    frontmatter: &AgentFrontmatter,
    name: &str,
    description: &str,
) {
    let name_line = frontmatter.field_line("name").unwrap_or(1);
    let description_line = frontmatter.field_line("description").unwrap_or(1);
    let character_count = description.chars().count();
    if character_count > 1024 {
        diag.report_with(
            LintRule::AgentDescLong,
            &format!("{agent_path}: description exceeds 1024 characters ({character_count})"),
            DiagnosticMetadata::at_line(description_line)
                .with_suggestion("shorten the description to 1024 characters or fewer"),
        );
    }
    let trimmed_count = description.trim().chars().count();
    if trimmed_count < 20 {
        diag.report_with(
            LintRule::AgentDescShort,
            &format!("{agent_path}: description routing-quality advisory is under 20 characters ({trimmed_count})"),
            DiagnosticMetadata::at_line(description_line)
                .with_suggestion("add concrete capability and when-to-use context"),
        );
    }
    if RE_NAME_INVALID.is_match(name) {
        diag.report_with(
            LintRule::AgentNameInvalid,
            &format!("{agent_path}: name contains characters outside [a-z0-9-]"),
            DiagnosticMetadata::at_line(name_line)
                .with_suggestion("use only lowercase letters, digits, and hyphens"),
        );
    }
    if is_desc_redundant(name.trim(), description.trim()) {
        diag.report_with(
            LintRule::AgentDescRedundant,
            &format!("{agent_path}: description substantially restates the name"),
            DiagnosticMetadata::at_line(description_line)
                .with_suggestion("add concrete capability plus when-to-use context"),
        );
    }
}

enum StringList {
    Missing,
    Valid(Vec<String>),
    Invalid,
}

fn canonical_string_list(frontmatter: &AgentFrontmatter, key: &str) -> StringList {
    let Some(value) = frontmatter.value(key) else {
        return StringList::Missing;
    };
    if let Some(scalar) = value.as_str() {
        return StringList::Valid(
            scalar
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_owned)
                .collect(),
        );
    }
    sequence_string_list(value)
}

/// Canonical reader for the agent tool fields (`tools`/`disallowedTools`).
/// Shape ownership matches [`canonical_string_list`], but a string scalar is
/// split by the shared tool tokenizer (#342) — commas and whitespace outside
/// `(...)` — so `Bash(npm install, npm test), Read` stays two declarations.
/// A sequence still contributes each string item as one entry; comments and
/// quoting are handled only by the YAML parser.
fn canonical_tool_list(frontmatter: &AgentFrontmatter, key: &str) -> StringList {
    let Some(value) = frontmatter.value(key) else {
        return StringList::Missing;
    };
    if let Some(scalar) = value.as_str() {
        return StringList::Valid(crate::validators::common::tokenize_tool_scalar(scalar));
    }
    sequence_string_list(value)
}

fn sequence_string_list(value: &crate::yaml::Value) -> StringList {
    let Some(sequence) = value.as_sequence() else {
        return StringList::Invalid;
    };
    let mut items = Vec::with_capacity(sequence.len());
    for value in sequence {
        let Some(item) = value.as_str() else {
            return StringList::Invalid;
        };
        let item = item.trim();
        if !item.is_empty() {
            items.push(item.to_owned());
        }
    }
    StringList::Valid(items)
}

fn dedupe_in_declaration_order(items: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter(|item| seen.insert(item.clone()))
        .collect()
}

fn yaml_type(value: &crate::yaml::Value) -> &'static str {
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
        "non-string value"
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

/// Return whether an explicitly declared tool can perform execution-like work.
/// MCP tools are included because their fully qualified syntax is the supported
/// declaration form and each invokes an external server operation.
fn is_execution_tool(tool: &str) -> bool {
    if !is_known_tool_name(tool) {
        return false;
    }
    let base_name = tool_base_name(tool);
    base_name.starts_with("mcp__") || EXECUTION_TOOLS.contains(&base_name)
}

fn tool_base_name(tool: &str) -> &str {
    tool.split_once('(').map_or(tool, |(base, _)| base).trim()
}

/// A012/A013: evidence-reading contracts are evaluated from the canonical
/// parsed frontmatter and source-aware live prose, never from a second YAML or
/// Markdown parse. This keeps malformed/non-mapping YAML with X001/A002 and
/// makes examples, code, quotes, and comments inert.
fn check_agent_evidence_contracts(
    diag: &mut DiagnosticCollector,
    agent_path: &str,
    parsed_frontmatter: &AgentFrontmatter,
    document: &LiveInstructionDocument<'_>,
) {
    let read_tool_mandate = first_live_sentence(document, |sentence| {
        EXPLICIT_READ_MANDATE.is_match(&normalize_emphasis_for_gates(sentence))
    });
    if let Some((line, mandate)) = read_tool_mandate {
        let has_read = matches!(canonical_tool_list(parsed_frontmatter, "tools"), StringList::Valid(tools) if tools.iter().any(|tool| is_known_tool_name(tool) && tool_base_name(tool) == "Read"));
        if !has_read {
            diag.report_with(
                LintRule::AgentReadMismatch,
                &format!(
                    "{agent_path}:{line}: explicit tools omit Read but the prompt requires the Read tool"
                ),
                DiagnosticMetadata::at_line(line)
                    .with_evidence(mandate)
                    .with_suggestion("declare Read in tools or remove the explicit Read-tool mandate"),
            );
        }
    }

    let read_evidence = first_live_sentence(document, |sentence| {
        EVIDENCE_READ_MANDATE.is_match(&normalize_emphasis_for_gates(sentence))
    });
    let json_output = first_live_sentence(document, |sentence| {
        JSON_OUTPUT_MANDATE.is_match(&normalize_emphasis_for_gates(sentence))
    });
    let has_unreadable_outcome = any_live_sentence(document, |sentence| {
        UNREADABLE_EVIDENCE_OUTCOME.is_match(&normalize_emphasis_for_gates(sentence))
    });
    let has_never_invent = any_live_sentence(document, |sentence| {
        NEVER_INVENT_PROHIBITION.is_match(&normalize_emphasis_for_gates(sentence))
    });

    if let (Some((read_line, _)), Some((output_line, _))) = (read_evidence, json_output)
        && (!has_unreadable_outcome || !has_never_invent)
    {
        diag.report_with(
            LintRule::AgentOutputUnsafe,
            &format!(
                "{agent_path}:{output_line}: machine-only output that reads evidence must define an unreadable-evidence outcome and prohibit invented evidence (read instruction at line {read_line})"
            ),
            DiagnosticMetadata::default()
                .with_location(SourceSpan::line(output_line))
                .with_evidence(format!("read instruction line {read_line}; JSON-only output line {output_line}"))
                .with_suggestion("state the unreadable-evidence outcome and directly prohibit inventing, fabricating, or guessing evidence"),
        );
    }
}

fn first_live_sentence(
    document: &LiveInstructionDocument<'_>,
    matches: impl Fn(&str) -> bool,
) -> Option<(usize, String)> {
    let example_scopes = document.example_scopes();
    let heading_lines: HashSet<_> = document
        .headings()
        .iter()
        .map(|heading| heading.line)
        .collect();
    document
        .prose_lines()
        .iter()
        .zip(example_scopes)
        .filter(|(line, is_example)| !*is_example && !heading_lines.contains(&line.line))
        .flat_map(|(line, _)| {
            sentence_ranges(&line.text)
                .into_iter()
                .map(move |range| (line.line, line.text[range].trim()))
        })
        .find(|(_, sentence)| !sentence.is_empty() && matches(sentence))
        .map(|(line, sentence)| (line, sentence.to_string()))
}

fn any_live_sentence(
    document: &LiveInstructionDocument<'_>,
    matches: impl Fn(&str) -> bool,
) -> bool {
    first_live_sentence(document, matches).is_some()
}

/// A029: tool-using agents need one concrete stop control or failure outcome.
///
/// This intentionally examines only live, operative Markdown prose. Frontmatter
/// is considered solely for a valid `maxTurns`; examples, fences, inline code,
/// block quotes, and quoted text cannot satisfy a body control.
fn check_agent_stop_control(
    diag: &mut DiagnosticCollector,
    agent_path: &str,
    parsed_frontmatter: &AgentFrontmatter,
    max_turns: Option<NonZeroU64>,
    document: &LiveInstructionDocument<'_>,
) {
    let execution_tools: Vec<_> = match canonical_tool_list(parsed_frontmatter, "tools") {
        StringList::Valid(items) => items,
        StringList::Missing | StringList::Invalid => Vec::new(),
    }
    .into_iter()
    .filter(|tool| is_execution_tool(tool))
    .collect();
    if execution_tools.is_empty() || max_turns.is_some() {
        return;
    }

    if has_operative_body_control(document) {
        return;
    }

    diag.report_with(
        LintRule::AgentStopMissing,
        &format!(
            "{agent_path}: execution tools [{}] have no maximum attempt/tool-call/step count, explicit timeout/deadline/token/cost budget, progress/failure threshold, or stop-and-report/escalation fallback; add either a concrete bound or a concrete failure outcome",
            execution_tools.join(", "),
        ),
        DiagnosticMetadata::default()
            .with_suggestion("add either a concrete bound or a concrete failure outcome"),
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
            let gate_view = normalize_emphasis_for_gates(sentence);
            OPERATIVE_CONTROL_PREFIX.is_match(&gate_view)
                && !DESCRIPTIVE_CONTROL_PREFIX.is_match(&gate_view)
                && has_bound_or_fallback(&gate_view)
        })
    })
}

/// Discover runtime skill identities rather than constructing a filesystem path
/// from an agent-controlled reference. Discovery ignores exclusions: linting a
/// skill is independent of whether Claude Code can resolve it at runtime.
fn runtime_skill_identities(surface: AgentSurface) -> HashSet<String> {
    let roots: &[&str] = if surface.uses_plugin_skill_namespace() {
        &[".claude/skills", "skills"]
    } else {
        &[".claude/skills"]
    };
    let mut identities = HashSet::new();
    for root in roots {
        for entry in traversal::shallow_directories(Path::new(root), Path::new("."), None).entries {
            let skill_dir = entry.path;
            let Ok(metadata) = fs::symlink_metadata(&skill_dir) else {
                continue;
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            let Some(dir_name) = skill_dir.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let skill_md = skill_dir.join("SKILL.md");
            let Ok(skill_metadata) = fs::symlink_metadata(&skill_md) else {
                continue;
            };
            if skill_metadata.file_type().is_symlink() || !skill_metadata.is_file() {
                continue;
            }
            // A malformed or mismatched target is still reachable by directory
            // identity, so S005/S006 remain its sole structural owner.
            identities.insert(dir_name.to_owned());
            let Ok(content) = fs::read_to_string(&skill_md) else {
                continue;
            };
            let document = MarkdownDocument::parse(content);
            let Some(lines) = document.frontmatter() else {
                continue;
            };
            let Ok(yaml) = frontmatter::parse_yaml_strict(lines) else {
                continue;
            };
            if let Some(name) = yaml
                .as_mapping()
                .and_then(|mapping| mapping.get("name"))
                .and_then(crate::yaml::Value::as_str)
                .filter(|name| !name.is_empty())
            {
                identities.insert(name.to_owned());
            }
        }
    }
    identities
}

/// A028 is evaluated from the canonical mapping and owns unsupported plugin
/// fields so no sibling agent validator contradicts it.
fn check_unsupported_plugin_fields(
    diag: &mut DiagnosticCollector,
    agent_path: &str,
    frontmatter: &AgentFrontmatter,
) {
    for field in UNSUPPORTED_PLUGIN_FIELDS {
        if frontmatter.value(field).is_some() {
            diag.report(
                LintRule::AgentFieldUnsupported,
                &format!(
                    "{agent_path}: frontmatter field '{field}' is not supported for plugin agents"
                ),
            );
        }
    }
}

/// A014-A027: field-value validation for agent frontmatter.
fn check_agent_field_values(
    diag: &mut DiagnosticCollector,
    agent_path: &str,
    frontmatter: &AgentFrontmatter,
    max_turns: Option<NonZeroU64>,
    surface: AgentSurface,
) {
    // A014: model must be a recognized value (CC-AG-003).
    if let Some(value) = frontmatter.value("model") {
        if let Some(model) = value.as_str() {
            if !is_valid_model_value(model) {
                diag.report(
                    LintRule::AgentModelInvalid,
                    &format!("{agent_path}: model '{model}' is not a recognized Claude Code model"),
                );
            }
        } else {
            diag.report(
                LintRule::AgentModelInvalid,
                &format!(
                    "{agent_path}: model must be a string (got {})",
                    yaml_type(value)
                ),
            );
        }
    }

    // A015 + A021: permissionMode enum (CC-AG-004) and bypass warning (CC-AG-012).
    if !surface.is_plugin_agent()
        && let Some(value) = frontmatter.value("permissionMode")
    {
        if let Some(mode) = value.as_str() {
            if !VALID_PERMISSION_MODES.contains(&mode) {
                diag.report(LintRule::AgentPermissionInvalid, &format!("{agent_path}: permissionMode '{mode}' is not one of [default, acceptEdits, auto, dontAsk, bypassPermissions, plan, manual]"));
            } else if mode == "bypassPermissions" {
                diag.report(
                    LintRule::AgentBypassPermissions,
                    &format!(
                        "{agent_path}: permissionMode 'bypassPermissions' disables safety checks"
                    ),
                );
            }
        } else {
            diag.report(
                LintRule::AgentPermissionInvalid,
                &format!(
                    "{agent_path}: permissionMode must be a string (got {})",
                    yaml_type(value)
                ),
            );
        }
    }

    // A018: memory must be user/project/local (CC-AG-008).
    if let Some(value) = frontmatter.value("memory") {
        if let Some(memory) = value.as_str() {
            if !VALID_MEMORY.contains(&memory) {
                diag.report(
                    LintRule::AgentMemoryInvalid,
                    &format!(
                        "{agent_path}: memory '{memory}' is not one of [user, project, local]"
                    ),
                );
            }
        } else {
            diag.report(
                LintRule::AgentMemoryInvalid,
                &format!(
                    "{agent_path}: memory must be a string (got {})",
                    yaml_type(value)
                ),
            );
        }
    }

    // A023: effort must be low/medium/high/xhigh/max (CC-AG-014).
    if let Some(value) = frontmatter.value("effort") {
        if let Some(effort) = value.as_str() {
            if !VALID_EFFORT.contains(&effort) {
                diag.report(LintRule::AgentEffortInvalid, &format!("{agent_path}: effort '{effort}' is not one of [low, medium, high, xhigh, max]"));
            }
        } else {
            diag.report(
                LintRule::AgentEffortInvalid,
                &format!(
                    "{agent_path}: effort must be a string (got {})",
                    yaml_type(value)
                ),
            );
        }
    }

    // A024: isolation must be worktree (CC-AG-015).
    if let Some(value) = frontmatter.value("isolation") {
        if let Some(isolation) = value.as_str() {
            if isolation != "worktree" {
                diag.report(
                    LintRule::AgentIsolationInvalid,
                    &format!("{agent_path}: isolation '{isolation}' is not one of [worktree]"),
                );
            }
        } else {
            diag.report(
                LintRule::AgentIsolationInvalid,
                &format!(
                    "{agent_path}: isolation must be a string (got {})",
                    yaml_type(value)
                ),
            );
        }
    }

    // A025: background must be a boolean (CC-AG-016).
    if let Some(value) = frontmatter.value("background") {
        if value.as_bool().is_none() {
            let actual_type = yaml_type(value);
            let detail = if actual_type == "string" {
                "string; use unquoted true or false — YAML 1.2 does not read yes/no as booleans"
            } else {
                actual_type
            };
            diag.report(
                LintRule::AgentBackgroundInvalid,
                &format!(
                    "{agent_path}: background must be a boolean (got {})",
                    detail
                ),
            );
        }
    }

    // A026: maxTurns must be a positive integer (CC-AG-017). Use the same
    // strict parse that owns the Q005 outer bound, never a raw line lookup.
    if frontmatter.value("maxTurns").is_some() && max_turns.is_none() {
        diag.report(
            LintRule::AgentMaxturnsInvalid,
            &format!("{agent_path}: maxTurns is not a positive integer"),
        );
    }

    let tools = match canonical_tool_list(frontmatter, "tools") {
        StringList::Missing => Vec::new(),
        StringList::Valid(items) => dedupe_in_declaration_order(items),
        StringList::Invalid => {
            diag.report(
                LintRule::AgentToolsUnknown,
                &format!("{agent_path}: tools must be a string or sequence of strings"),
            );
            Vec::new()
        }
    };
    let disallowed = match canonical_tool_list(frontmatter, "disallowedTools") {
        StringList::Missing => Vec::new(),
        StringList::Valid(items) => dedupe_in_declaration_order(items),
        StringList::Invalid => {
            diag.report(
                LintRule::AgentDisallowedUnknown,
                &format!("{agent_path}: disallowedTools must be a string or sequence of strings"),
            );
            Vec::new()
        }
    };

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

    // A017: no tool in both tools and disallowedTools (CC-AG-006). Overlap is
    // an exact full-token match in first-declaration order: restricted forms
    // are never collapsed to base names, so Bash(git *) and Bash(rm *) do not
    // overlap.
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
    let skills = match canonical_string_list(frontmatter, "skills") {
        StringList::Missing => Vec::new(),
        StringList::Valid(items) => dedupe_in_declaration_order(items),
        StringList::Invalid => {
            diag.report(
                LintRule::AgentSkillMissing,
                &format!("{agent_path}: skills must be a string or sequence of strings"),
            );
            Vec::new()
        }
    };
    let skill_identities = runtime_skill_identities(surface);
    for skill in &skills {
        if !is_kebab_case(skill) {
            diag.report(
                LintRule::AgentSkillKebab,
                &format!(
                    "{agent_path}: skills entry '{skill}' is not kebab-case ([a-z0-9-], no leading/trailing/double hyphen)",
                ),
            );
            continue;
        }
        if !skill_identities.contains(skill) {
            diag.report(
                LintRule::AgentSkillMissing,
                &format!("{agent_path}: skills entry '{skill}' has no matching runtime skill"),
            );
        }
    }

    // A027: unknown frontmatter field (CC-AG-019, typo catcher).
    for key in frontmatter.keys() {
        if !KNOWN_AGENT_FIELDS.contains(&key) {
            diag.report(
                LintRule::AgentFieldUnknown,
                &format!("{agent_path}: unrecognized frontmatter field '{key}' (possible typo)",),
            );
        }
    }
}

/// V16/V21: Validate the opt-in larch reviewer-template convention.
///
/// This convention intentionally applies only to public top-level plugin agents.
/// It is inactive until the shared template exists or an included agent makes a
/// live provenance claim. Keeping all three rules in one pass makes that
/// activation and the count's participant set deterministic.
pub fn validate_agent_template_convention(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let agents_dir = Path::new("agents");
    let template_path = Path::new(REVIEWER_TEMPLATE_PATH);
    if !agents_dir.is_dir() || exclude.is_excluded(REVIEWER_TEMPLATE_PATH) {
        return;
    }

    let mut agents = Vec::new();
    let mut has_excluded_agent = false;
    for entry in traversal::shallow_files(agents_dir, Path::new("."), None).entries {
        let path = entry.path;
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.ends_with(".md") {
            continue;
        }

        let subject_path = format!("agents/{name}");
        if exclude.is_excluded(&subject_path) {
            has_excluded_agent = true;
            continue;
        }
        let content = fs::read_to_string(&path).ok();
        let has_marker = content.as_deref().is_some_and(has_reviewer_template_marker);
        agents.push(TemplateAgent {
            subject_path,
            has_marker,
        });
    }

    let marker_activates = agents.iter().any(|agent| agent.has_marker);
    let template_activates = template_path.is_file();
    if !marker_activates && !template_activates {
        return;
    }

    let template_content = match fs::read_to_string(template_path) {
        Ok(content) => content,
        Err(_) if marker_activates => {
            diag.report_at_with(
                LintRule::TemplateFileMissing,
                template_path,
                &format!(
                    "an agent declares derivation from missing or unreadable template: {REVIEWER_TEMPLATE_PATH}"
                ),
                DiagnosticMetadata::default().with_suggestion(
                    "restore skills/shared/reviewer-templates.md or remove the stale derivation claim",
                ),
            );
            return;
        }
        Err(_) => return,
    };

    for agent in &agents {
        if !agent.has_marker {
            diag.report_at_with(
                LintRule::TemplateMarkerMissing,
                &agent.subject_path,
                &format!("{} missing '{TEMPLATE_MARKER}' marker", agent.subject_path),
                DiagnosticMetadata::default().with_suggestion(TEMPLATE_MARKER),
            );
        }
    }

    // A007 is a whole-convention fact. Once a participant is excluded, the
    // complete set cannot be observed without violating exclusion invisibility.
    if has_excluded_agent {
        return;
    }

    let template_count = reviewer_heading_count(&template_content);
    let agent_count = agents.len();
    if template_count != agent_count {
        let related_subjects: Vec<_> = agents
            .iter()
            .map(|agent| agent.subject_path.as_str())
            .collect();
        diag.report_at_with(
            LintRule::TemplateCountMismatch,
            template_path,
            &format!(
                "agent-template count mismatch: {agent_count} agent file(s) but {template_count} reviewer section(s) in {REVIEWER_TEMPLATE_PATH}"
            ),
            DiagnosticMetadata::default()
                .with_related_subjects(related_subjects)
                .with_suggestion(
                    "add or remove a reviewer section, or reconcile the participating agent set",
                ),
        );
    }
}

struct TemplateAgent {
    subject_path: String,
    has_marker: bool,
}

fn has_reviewer_template_marker(content: &str) -> bool {
    let document = MarkdownDocument::parse(content);
    document.body_prose().iter().any(|line| {
        line.masked_inline_code_columns.is_empty() && is_reviewer_template_marker_line(&line.text)
    }) || has_reviewer_template_marker_comment(&document)
}

fn is_reviewer_template_marker_line(line: &str) -> bool {
    if line.contains('`') {
        return false;
    }
    let normalized = line.trim();
    let normalized = strip_markdown_list_marker(normalized).trim_start();
    let Some(after_prefix) = strip_ascii_case_prefix(normalized, "derived from") else {
        return false;
    };
    if !after_prefix.chars().next().is_some_and(char::is_whitespace) {
        return false;
    }
    let normalized = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    contains_exact_template_path(&normalized) && !contains_marker_negation(&normalized)
}

fn strip_markdown_list_marker(line: &str) -> &str {
    if let Some(rest) = line.strip_prefix(['-', '*', '+'])
        && rest.chars().next().is_some_and(char::is_whitespace)
    {
        return rest;
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits > 0
        && matches!(line[digits..].chars().next(), Some('.' | ')'))
        && line[digits + 1..]
            .chars()
            .next()
            .is_some_and(char::is_whitespace)
    {
        return &line[digits + 1..];
    }
    line
}

fn strip_ascii_case_prefix<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.get(..prefix.len())
        .filter(|candidate| candidate.eq_ignore_ascii_case(prefix))
        .and_then(|_| text.get(prefix.len()..))
}

fn contains_exact_template_path(text: &str) -> bool {
    let path = REVIEWER_TEMPLATE_PATH;
    let mut start = 0;
    while let Some(index) = text[start..].find(path) {
        let index = start + index;
        let before = text[..index].chars().next_back();
        let after = text[index + path.len()..].chars().next();
        if before.is_none_or(is_template_path_boundary)
            && after.is_none_or(is_template_path_boundary)
        {
            return true;
        }
        start = index + path.len();
    }
    false
}

fn is_template_path_boundary(character: char) -> bool {
    !character.is_ascii_alphanumeric() && !matches!(character, '_' | '-' | '.' | '/')
}

fn contains_marker_negation(text: &str) -> bool {
    text.split(|character: char| !character.is_ascii_alphabetic())
        .any(|word| {
            matches!(
                word.to_ascii_lowercase().as_str(),
                "not" | "never" | "without" | "stale" | "independent" | "example"
            )
        })
}

/// HTML comments are intentionally absent from `body_prose`, so this small
/// recognizer reuses its source eligibility and inline-code masks to accept a
/// complete standalone provenance comment without reimplementing fence parsing.
fn has_reviewer_template_marker_comment(document: &MarkdownDocument) -> bool {
    let prose_by_line: HashMap<_, _> = document
        .body_prose()
        .iter()
        .map(|line| (line.line, line))
        .collect();
    let lines: Vec<_> = document.content().lines().collect();
    let mut comment = None::<(String, bool)>;

    for line_number in document.body_start_line()..=lines.len() {
        let raw = lines[line_number - 1];
        let Some(prose) = prose_by_line.get(&line_number) else {
            comment = None;
            continue;
        };

        if let Some((contents, contains_inline_code)) = comment.as_mut() {
            *contains_inline_code |= !prose.masked_inline_code_columns.is_empty();
            let Some(end) = visible_comment_delimiter(raw, prose, "-->", 0) else {
                contents.push('\n');
                contents.push_str(raw);
                continue;
            };
            if !raw[end + 3..].trim().is_empty() {
                comment = None;
                continue;
            }
            contents.push('\n');
            contents.push_str(&raw[..end]);
            if !*contains_inline_code && is_reviewer_template_marker_line(contents) {
                return true;
            }
            comment = None;
            continue;
        }

        let Some(start) = visible_comment_delimiter(raw, prose, "<!--", 0) else {
            continue;
        };
        if !raw[..start].trim().is_empty() {
            continue;
        }
        let after_start = start + 4;
        if let Some(end) = visible_comment_delimiter(raw, prose, "-->", after_start) {
            if raw[end + 3..].trim().is_empty()
                && prose.masked_inline_code_columns.is_empty()
                && is_reviewer_template_marker_line(&raw[after_start..end])
            {
                return true;
            }
        } else {
            comment = Some((
                raw[after_start..].to_string(),
                !prose.masked_inline_code_columns.is_empty(),
            ));
        }
    }
    false
}

fn visible_comment_delimiter(
    raw: &str,
    prose: &crate::markdown::MarkdownProseLine,
    delimiter: &str,
    from: usize,
) -> Option<usize> {
    raw[from..]
        .match_indices(delimiter)
        .find_map(|(offset, _)| {
            let index = from + offset;
            let start_column = raw[..index].chars().count() + 1;
            let end_column = start_column + delimiter.chars().count() - 1;
            (!prose
                .masked_inline_code_columns
                .iter()
                .any(|range| range.contains(&start_column) || range.contains(&end_column)))
            .then_some(index)
        })
}

fn reviewer_heading_count(content: &str) -> usize {
    let document = MarkdownDocument::parse_body(content);
    let prose_lines: HashSet<_> = document.body_prose().iter().map(|line| line.line).collect();
    document
        .headings()
        .iter()
        .filter(|heading| {
            heading.level == 2
                && prose_lines.contains(&heading.line)
                && is_reviewer_heading(&heading.text)
        })
        .count()
}

fn is_reviewer_heading(text: &str) -> bool {
    let text = text.trim();
    text == "Reviewer"
        || text.strip_prefix("Reviewer").is_some_and(|suffix| {
            suffix.chars().next().is_some_and(|character| {
                character.is_whitespace() || matches!(character, ':' | '-')
            })
        })
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
    fn test_v7_absent_default_agents_dir_is_clean() {
        // Narrowed A001: the implicit default `agents/` is optional, so its
        // absence with no declared roots reports nothing.
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.diagnostics().len(), 0);
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
        assert_eq!(diag.diagnostics()[0].rule, LintRule::NoAgentFiles);
        assert!(diag.errors()[0].contains("no agent .md files"));
    }

    // ── #321: recursive discovery, narrowed A001/A004, manifest roots ──

    /// Run the plugin agents validator with explicit manifest-declared roots.
    fn run_plugin_agents_with(
        declared: &[&str],
        exclude: &ExcludeSet,
        diag: &mut DiagnosticCollector,
    ) {
        let declared: Vec<String> = declared.iter().map(|s| (*s).to_string()).collect();
        let mut prompt_pass = super::super::prompt_content::PromptContentPass::default();
        validate_agents_with_prompt_pass(diag, exclude, &declared, &mut prompt_pass);
    }

    fn agents_of(diag: &DiagnosticCollector, rule: LintRule) -> Vec<String> {
        diag.diagnostics()
            .iter()
            .filter(|d| d.rule == rule)
            .map(|d| {
                d.subject_path
                    .as_deref()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default()
            })
            .collect()
    }

    #[test]
    #[serial_test::serial]
    fn nested_private_agent_bad_model_reports_a014() {
        // The shared recursive collector feeds sibling field-value validators, so
        // a nested private agent is checked exactly like a top-level one.
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/agents/review").unwrap();
        std::fs::write(
            ".claude/agents/review/beta.md",
            "---\nname: beta\ndescription: A general-purpose reviewer for pull requests\nmodel: not-a-model\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_private_agents(&mut diag, &ExcludeSet::default());
        assert_eq!(
            agents_of(&diag, LintRule::AgentModelInvalid),
            vec![".claude/agents/review/beta.md".to_string()]
        );
    }

    #[test]
    #[serial_test::serial]
    fn nested_private_agent_malformed_frontmatter_reports_a002() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/agents/review").unwrap();
        std::fs::write(".claude/agents/review/beta.md", "no frontmatter at all\n").unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_private_agents(&mut diag, &ExcludeSet::default());
        assert_eq!(
            agents_of(&diag, LintRule::AgentFrontmatterMalformed),
            vec![".claude/agents/review/beta.md".to_string()]
        );
    }

    #[test]
    #[serial_test::serial]
    fn deeply_nested_private_agent_is_collected() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/agents/a/b/c").unwrap();
        std::fs::write(
            ".claude/agents/a/b/c/deep.md",
            "---\nname: deep\ndescription: A general-purpose reviewer for pull requests\nmodel: not-a-model\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_private_agents(&mut diag, &ExcludeSet::default());
        assert_eq!(
            agents_of(&diag, LintRule::AgentModelInvalid),
            vec![".claude/agents/a/b/c/deep.md".to_string()]
        );
    }

    #[test]
    #[serial_test::serial]
    fn private_agent_duplicate_names_are_grouped_sorted_and_path_scoped() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/agents/review").unwrap();
        for (path, name) in [
            (".claude/agents/alpha.md", "reviewer"),
            (".claude/agents/review/beta.md", "reviewer"),
            (".claude/agents/gamma.md", "auditor"),
            (".claude/agents/zeta.md", "auditor"),
        ] {
            std::fs::write(
                path,
                format!(
                    "---\nname: {name}\ndescription: Reviews pull requests for correctness and regressions\n---\nBody\n"
                ),
            )
            .unwrap();
        }

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_private_agents(&mut diag, &ExcludeSet::default());
        let findings: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|finding| finding.rule == LintRule::AgentNameDuplicate)
            .collect();

        assert_eq!(findings.len(), 2);
        assert_eq!(
            findings[0].subject_path.as_deref(),
            Some(Path::new(".claude/agents/gamma.md"))
        );
        assert_eq!(
            findings[0].related_subjects,
            vec![Path::new(".claude/agents/zeta.md").to_path_buf()]
        );
        assert_eq!(
            findings[1].subject_path.as_deref(),
            Some(Path::new(".claude/agents/alpha.md"))
        );
        assert_eq!(
            findings[1].related_subjects,
            vec![Path::new(".claude/agents/review/beta.md").to_path_buf()]
        );
        assert!(
            findings[1]
                .message
                .contains("agent name 'reviewer' is declared by 2 files")
        );
        assert_eq!(
            findings[1].suggestion.as_deref(),
            Some("rename or remove the duplicate agent definitions")
        );
    }

    #[test]
    #[serial_test::serial]
    fn private_agent_duplicate_name_skips_invalid_names_and_honors_exclusions() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/agents").unwrap();
        std::fs::write(
            ".claude/agents/alpha.md",
            "---\nname: reviewer\ndescription: Reviews backend pull requests for correctness and regressions\n---\nBody\n",
        )
        .unwrap();
        std::fs::write(
            ".claude/agents/beta.md",
            "---\nname: reviewer\ndescription: Audits frontend accessibility and design-system conformance\n---\nBody\n",
        )
        .unwrap();
        std::fs::write(
            ".claude/agents/blank.md",
            "---\nname: \ndescription: Reviews frontend pull requests for accessibility regressions\n---\nBody\n",
        )
        .unwrap();
        std::fs::write(
            ".claude/agents/invalid.md",
            "---\nname: reviewer\ndescription: [unterminated\n---\nBody\n",
        )
        .unwrap();

        let exclude = ExcludeSet::new(&[".claude/agents/beta.md".to_string()]).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_private_agents(&mut diag, &exclude);
        assert!(
            diag.diagnostics()
                .iter()
                .all(|finding| finding.rule != LintRule::AgentNameDuplicate)
        );
    }

    #[test]
    #[serial_test::serial]
    fn plugin_agent_names_are_deliberately_excluded_from_a031() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("agents").unwrap();
        for path in ["agents/alpha.md", "agents/beta.md"] {
            std::fs::write(
                path,
                "---\nname: reviewer\ndescription: Reviews pull requests for correctness and regressions\n---\nBody\n",
            )
            .unwrap();
        }

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &ExcludeSet::default());
        assert!(
            diag.diagnostics()
                .iter()
                .all(|finding| finding.rule != LintRule::AgentNameDuplicate)
        );
    }

    #[test]
    #[serial_test::serial]
    fn declared_missing_paths_report_a001_in_declaration_order() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        // No `agents/` directory: its implicit absence must never fire A001.

        let mut diag = DiagnosticCollector::new_all_enabled();
        run_plugin_agents_with(&["custom-a", "custom-b"], &ExcludeSet::default(), &mut diag);
        assert_eq!(
            agents_of(&diag, LintRule::AgentsDirMissing),
            vec!["custom-a".to_string(), "custom-b".to_string()],
            "one A001 per declared missing path, in declaration order"
        );
    }

    #[test]
    #[serial_test::serial]
    fn custom_declared_root_reports_a002_and_no_a001() {
        // Leaf #256 reproduction: a plugin whose only agent lives under a
        // manifest-declared root must be validated there, and the absent default
        // `agents/` must not raise A001.
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("custom-agents/nested").unwrap();
        std::fs::write("custom-agents/nested/broken.md", "no frontmatter\n").unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        run_plugin_agents_with(&["custom-agents"], &ExcludeSet::default(), &mut diag);
        assert_eq!(
            agents_of(&diag, LintRule::AgentFrontmatterMalformed),
            vec!["custom-agents/nested/broken.md".to_string()]
        );
        assert!(
            agents_of(&diag, LintRule::AgentsDirMissing).is_empty(),
            "absent default agents/ must not fire A001"
        );
    }

    #[test]
    #[serial_test::serial]
    fn declared_direct_markdown_file_root_is_validated() {
        // A declared path may point directly at a single agent file; a malformed
        // one still reaches A002 and its existence means no A001.
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("custom").unwrap();
        std::fs::write("custom/agent.md", "no frontmatter\n").unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        run_plugin_agents_with(&["custom/agent.md"], &ExcludeSet::default(), &mut diag);
        assert_eq!(
            agents_of(&diag, LintRule::AgentFrontmatterMalformed),
            vec!["custom/agent.md".to_string()]
        );
        assert!(agents_of(&diag, LintRule::AgentsDirMissing).is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn default_and_two_declared_roots_validate_each_file_once_in_order() {
        // Default `agents/` plus two declared roots, one overlapping the default
        // and one spelled twice: every physical file is validated exactly once,
        // in stable sorted order.
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("agents/sub").unwrap();
        std::fs::create_dir_all("extra").unwrap();
        for path in ["agents/top.md", "agents/sub/nested.md", "extra/e.md"] {
            std::fs::write(path, "no frontmatter\n").unwrap();
        }

        let mut diag = DiagnosticCollector::new_all_enabled();
        // `agents/sub` overlaps the default `agents/`; `./extra` repeats `extra`.
        run_plugin_agents_with(
            &["agents/sub", "extra", "./extra"],
            &ExcludeSet::default(),
            &mut diag,
        );
        assert_eq!(
            agents_of(&diag, LintRule::AgentFrontmatterMalformed),
            vec![
                "agents/sub/nested.md".to_string(),
                "agents/top.md".to_string(),
                "extra/e.md".to_string(),
            ],
            "each physical file is validated once, deduplicated across overlapping roots"
        );
    }

    #[test]
    #[serial_test::serial]
    fn present_empty_default_root_reports_a004() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        run_plugin_agents_with(&[], &ExcludeSet::default(), &mut diag);
        assert_eq!(
            agents_of(&diag, LintRule::NoAgentFiles),
            vec!["agents".to_string()]
        );
    }

    #[test]
    #[serial_test::serial]
    fn present_empty_declared_root_reports_a004_not_a001() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("custom").unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        run_plugin_agents_with(&["custom"], &ExcludeSet::default(), &mut diag);
        assert_eq!(
            agents_of(&diag, LintRule::NoAgentFiles),
            vec!["custom".to_string()]
        );
        assert!(
            agents_of(&diag, LintRule::AgentsDirMissing).is_empty(),
            "an existing declared root is present, not missing"
        );
    }

    #[test]
    #[serial_test::serial]
    fn declared_non_markdown_file_reports_a004_not_a001() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::write("custom.txt", "not markdown\n").unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        run_plugin_agents_with(&["custom.txt"], &ExcludeSet::default(), &mut diag);
        assert_eq!(
            agents_of(&diag, LintRule::NoAgentFiles),
            vec!["custom.txt".to_string()]
        );
        assert!(agents_of(&diag, LintRule::AgentsDirMissing).is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn default_and_declared_same_root_emits_single_a004() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        run_plugin_agents_with(&["agents"], &ExcludeSet::default(), &mut diag);
        assert_eq!(
            agents_of(&diag, LintRule::NoAgentFiles),
            vec!["agents".to_string()],
            "a root that is both default and declared reports A004 at most once"
        );
        assert!(agents_of(&diag, LintRule::AgentsDirMissing).is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn nested_agent_suppresses_a004() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents/review").unwrap();
        std::fs::write(
            "agents/review/nested.md",
            "---\nname: nested\ndescription: A general-purpose reviewer for pull requests\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        run_plugin_agents_with(&[], &ExcludeSet::default(), &mut diag);
        assert!(
            agents_of(&diag, LintRule::NoAgentFiles).is_empty(),
            "a nested agent makes the default root non-empty"
        );
    }

    #[test]
    #[serial_test::serial]
    fn all_excluded_root_stays_silent() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::write("agents/only.md", "no frontmatter\n").unwrap();

        let exclude = ExcludeSet::new(&["agents/**".to_string()]).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        run_plugin_agents_with(&[], &exclude, &mut diag);
        assert!(
            diag.diagnostics().is_empty(),
            "an all-excluded root is neither empty (A004) nor validated: {:?}",
            diag.errors()
        );
    }

    #[test]
    #[serial_test::serial]
    fn excluding_nested_subdir_suppresses_only_nested_findings() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/agents/review").unwrap();
        let bad = "---\nname: {n}\ndescription: A general-purpose reviewer for pull requests\nmodel: not-a-model\n---\nBody\n";
        std::fs::write(".claude/agents/top.md", bad.replace("{n}", "top")).unwrap();
        std::fs::write(
            ".claude/agents/review/nested.md",
            bad.replace("{n}", "nested"),
        )
        .unwrap();

        let exclude = ExcludeSet::new(&[".claude/agents/review/**".to_string()]).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_private_agents(&mut diag, &exclude);
        assert_eq!(
            agents_of(&diag, LintRule::AgentModelInvalid),
            vec![".claude/agents/top.md".to_string()],
            "the excluded nested subdirectory is invisible; the top-level file is unchanged"
        );
    }

    #[test]
    #[serial_test::serial]
    fn non_markdown_nested_entry_is_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("agents/notes").unwrap();
        std::fs::write("agents/notes/readme.txt", "just notes\n").unwrap();
        std::fs::write(
            "agents/real.md",
            "---\nname: real\ndescription: A general-purpose reviewer for pull requests\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        run_plugin_agents_with(&[], &ExcludeSet::default(), &mut diag);
        assert!(
            agents_of(&diag, LintRule::NoAgentFiles).is_empty(),
            "the real agent keeps the root non-empty"
        );
        assert!(
            !diag.errors().iter().any(|e| e.contains("readme.txt")),
            "a non-Markdown entry is never treated as an agent: {:?}",
            diag.errors()
        );
    }

    #[test]
    #[serial_test::serial]
    fn nested_agents_do_not_change_template_arithmetic() {
        // A005-A007 stay pinned to the flat top-level `agents/*.md` larch
        // convention; a nested agent neither counts toward A007 nor gets an A006
        // marker check.
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("agents/nested").unwrap();
        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::write("skills/shared/reviewer-templates.md", "## Reviewer\n").unwrap();
        // One top-level agent with the marker == one reviewer section: aligned.
        std::fs::write(
            "agents/general.md",
            "---\nname: general\ndescription: desc\n---\nDerived from skills/shared/reviewer-templates.md\n",
        )
        .unwrap();
        // A nested agent lacking the marker must not perturb the convention.
        std::fs::write(
            "agents/nested/extra.md",
            "---\nname: extra\ndescription: desc\n---\nNo marker here\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agent_template_convention(&mut diag, &ExcludeSet::default());
        assert_eq!(
            diag.error_count(),
            0,
            "nested agents are invisible to A005-A007: {:?}",
            diag.errors()
        );
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
        std::fs::write("skills/shared/reviewer-templates.md", "## Reviewer\n").unwrap();
        std::fs::write(
            "agents/general.md",
            "---\nname: general\ndescription: desc\n---\nDerived from skills/shared/reviewer-templates.md\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agent_template_convention(&mut diag, &crate::config::ExcludeSet::default());
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
        validate_agent_template_convention(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 2);
        assert!(
            diag.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.rule == LintRule::TemplateMarkerMissing)
        );
        assert!(
            diag.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.rule == LintRule::TemplateCountMismatch)
        );
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
        std::fs::write(
            "agents/one.md",
            "---\nname: one\n---\nDerived from skills/shared/reviewer-templates.md\n",
        )
        .unwrap();
        std::fs::write(
            "agents/two.md",
            "---\nname: two\n---\nDerived from skills/shared/reviewer-templates.md\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agent_template_convention(&mut diag, &crate::config::ExcludeSet::default());
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
        std::fs::write(
            "agents/one.md",
            "---\nname: one\n---\nDerived from skills/shared/reviewer-templates.md\n",
        )
        .unwrap();
        // Only 1 agent but 2 templates

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agent_template_convention(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("mismatch"));
    }

    #[test]
    fn reviewer_template_marker_accepts_only_live_provenance() {
        for content in [
            "Derived from skills/shared/reviewer-templates.md\n",
            "- derived from skills/shared/reviewer-templates.md\n",
            "<!-- Derived from skills/shared/reviewer-templates.md -->\n",
            "<!--\nDERIVED FROM skills/shared/reviewer-templates.md\n-->\n",
        ] {
            assert!(has_reviewer_template_marker(content), "{content:?}");
        }

        for content in [
            "```text\nDerived from skills/shared/reviewer-templates.md\n```\n",
            "`Derived from skills/shared/reviewer-templates.md`\n",
            "Derived from skills/shared/reviewer-templates.md `example`\n",
            "<!-- Derived from `skills/shared/reviewer-templates.md` -->\n",
            "> Derived from skills/shared/reviewer-templates.md\n",
            "---\nDerived from skills/shared/reviewer-templates.md\n---\n",
            "Derived from skills/shared/reviewer-templates.md, but not for this agent.\n",
            "This example says Derived from skills/shared/reviewer-templates.md\n",
            "Derived from skills/shared/reviewer-templates.md.bak\n",
            "Derived from ${CLAUDE_PLUGIN_ROOT}/skills/shared/reviewer-templates.md\n",
        ] {
            assert!(!has_reviewer_template_marker(content), "{content:?}");
        }
    }

    #[test]
    fn reviewer_heading_count_uses_live_level_two_markdown_headings() {
        let content = "\
## Reviewer
## Reviewer: Security
Reviewer - Reliability
----------------------
```markdown
## Reviewer fake
```
> ## Reviewer quoted
<!-- ## Reviewer commented -->
# Reviewer
### Reviewer
## Reviewership
Body ## Reviewer
";
        assert_eq!(reviewer_heading_count(content), 3);
    }

    #[test]
    #[serial_test::serial]
    fn template_convention_is_opt_in_and_respects_exclusions() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::write("agents/independent.md", "Independent agent\n").unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agent_template_convention(&mut diag, &ExcludeSet::default());
        assert!(diag.diagnostics().is_empty());

        std::fs::write(
            "agents/declared.md",
            "Derived from skills/shared/reviewer-templates.md\n",
        )
        .unwrap();
        validate_agent_template_convention(&mut diag, &ExcludeSet::default());
        assert_eq!(diag.diagnostics().len(), 1);
        assert_eq!(diag.diagnostics()[0].rule, LintRule::TemplateFileMissing);
        assert_eq!(
            diag.diagnostics()[0].suggestion.as_deref(),
            Some(
                "restore skills/shared/reviewer-templates.md or remove the stale derivation claim"
            )
        );

        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::write("skills/shared/reviewer-templates.md", "## Reviewer\n").unwrap();
        let excluded = ExcludeSet::new(&["agents/declared.md".to_string()]).unwrap();
        let mut excluded_diag = DiagnosticCollector::new_all_enabled();
        validate_agent_template_convention(&mut excluded_diag, &excluded);
        assert!(
            excluded_diag
                .diagnostics()
                .iter()
                .all(|diagnostic| diagnostic.rule != LintRule::TemplateCountMismatch)
        );
        assert!(excluded_diag.diagnostics().iter().any(|diagnostic| {
            diagnostic.rule == LintRule::TemplateMarkerMissing
                && diagnostic.subject_path.as_deref() == Some(Path::new("agents/independent.md"))
        }));

        let template_excluded = ExcludeSet::new(&[REVIEWER_TEMPLATE_PATH.to_string()]).unwrap();
        let mut template_diag = DiagnosticCollector::new_all_enabled();
        validate_agent_template_convention(&mut template_diag, &template_excluded);
        assert!(template_diag.diagnostics().is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn template_count_metadata_is_sorted_and_actionable() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::write("skills/shared/reviewer-templates.md", "## Reviewer\n").unwrap();
        for name in ["zeta", "alpha"] {
            std::fs::write(
                format!("agents/{name}.md"),
                "Derived from skills/shared/reviewer-templates.md\n",
            )
            .unwrap();
        }

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agent_template_convention(&mut diag, &ExcludeSet::default());
        let mismatch = diag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.rule == LintRule::TemplateCountMismatch)
            .unwrap();
        assert_eq!(
            mismatch.related_subjects,
            vec![
                std::path::PathBuf::from("agents/alpha.md"),
                std::path::PathBuf::from("agents/zeta.md"),
            ]
        );
        assert_eq!(
            mismatch.suggestion.as_deref(),
            Some("add or remove a reviewer section, or reconcile the participating agent set")
        );
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
                .any(|e| e.contains("substantially restates the name"))
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
                .any(|e| e.contains("substantially restates the name"))
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
                .any(|e| e.contains("substantially restates the name")),
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

    fn run_private_agent<F: FnOnce(&mut DiagnosticCollector)>(content: &str, f: F) {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude/agents").unwrap();
        std::fs::write(".claude/agents/general.md", content).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_private_agents(&mut diag, &crate::config::ExcludeSet::default());
        f(&mut diag);
    }

    const GOOD_DESC: &str = "A general-purpose code review assistant";

    // ── A012/A013: canonical evidence contracts ────────────────────

    #[test]
    #[serial_test::serial]
    fn a012_uses_canonical_tool_lists_and_source_aware_live_prose() {
        for tools in ["tools: Bash", "tools: [Bash]", "tools:\n  - Bash"] {
            let mandate_line = 5 + tools.lines().count();
            let content = format!(
                "---\nname: general\ndescription: {GOOD_DESC}\n{tools}\n---\nPlease use the Read tool before reporting.\n"
            );
            run_private_agent(&content, |diag| {
                let finding = diag
                    .diagnostics()
                    .iter()
                    .find(|diagnostic| diagnostic.rule == LintRule::AgentReadMismatch)
                    .expect("scalar and sequence tools must be checked");
                assert_eq!(finding.location, Some(SourceSpan::line(mandate_line)));
                assert_eq!(
                    finding.evidence.as_deref(),
                    Some("Please use the Read tool before reporting.")
                );
                assert!(finding.suggestion.is_some());
            });
        }

        for tools in ["tools: Read", "tools: Read(path/**)"] {
            let content = format!(
                "---\nname: general\ndescription: {GOOD_DESC}\n{tools}\n---\nUse Read before reporting.\n"
            );
            run_private_agent(&content, |diag| {
                assert!(
                    !diag
                        .diagnostics()
                        .iter()
                        .any(|diagnostic| diagnostic.rule == LintRule::AgentReadMismatch)
                );
            });
        }

        let generic_read = format!(
            "---\nname: general\ndescription: {GOOD_DESC}\ntools: Bash, mcp__files__open\n---\nRead every evidence file before reporting.\n"
        );
        run_private_agent(&generic_read, |diag| {
            assert!(
                !diag
                    .diagnostics()
                    .iter()
                    .any(|diagnostic| diagnostic.rule == LintRule::AgentReadMismatch)
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn a012_a013_ignore_examples_and_inline_markers_but_keep_live_contracts() {
        let inert = format!(
            "---\nname: general\ndescription: {GOOD_DESC}\ntools: [Bash]\n---\n# Examples\nUse the Read tool.\nRead every evidence file. Output strict JSON only.\n\n```text\nUse the Read tool. Read every evidence file. Output strict JSON only.\n```\n> Use the Read tool.\n"
        );
        run_private_agent(&inert, |diag| {
            assert!(!diag.diagnostics().iter().any(|diagnostic| {
                matches!(
                    diagnostic.rule,
                    LintRule::AgentReadMismatch | LintRule::AgentOutputUnsafe
                )
            }));
        });

        let live = format!(
            "---\nname: general\ndescription: {GOOD_DESC}\ntools: Bash\n---\nUse the Read tool. lint-agent-tool-contract: ok because legacy.\nRead every evidence file.\nOutput strict JSON only. lint-agent-output-mandate: ok because legacy.\n"
        );
        run_private_agent(&live, |diag| {
            assert!(diag.diagnostics().iter().any(|diagnostic| {
                diagnostic.rule == LintRule::AgentReadMismatch
                    && diagnostic.location == Some(SourceSpan::line(6))
            }));
            let output = diag
                .diagnostics()
                .iter()
                .find(|diagnostic| diagnostic.rule == LintRule::AgentOutputUnsafe)
                .expect("inline marker must not suppress A013");
            assert_eq!(output.location, Some(SourceSpan::line(8)));
            assert_eq!(
                output.evidence.as_deref(),
                Some("read instruction line 7; JSON-only output line 8")
            );
            assert!(output.suggestion.is_some());
        });
    }

    #[test]
    #[serial_test::serial]
    fn a013_requires_live_unreadable_outcome_and_never_invent_prohibition() {
        let safe = format!(
            "---\nname: general\ndescription: {GOOD_DESC}\ntools: Read\n---\nRead every evidence file.\nIf a file is unreadable, return NEEDS_DEEP.\nDo not invent evidence.\nOutput strict JSONL only.\n"
        );
        run_private_agent(&safe, |diag| {
            assert!(
                !diag
                    .diagnostics()
                    .iter()
                    .any(|diagnostic| diagnostic.rule == LintRule::AgentOutputUnsafe)
            );
        });

        let quoted_safeguards = format!(
            "---\nname: general\ndescription: {GOOD_DESC}\ntools: Read\n---\nRead every evidence file.\nOutput strict JSON only.\nExample: If a file is unreadable, return NEEDS_DEEP and do not invent evidence.\n"
        );
        run_private_agent(&quoted_safeguards, |diag| {
            assert!(
                diag.diagnostics()
                    .iter()
                    .any(|diagnostic| diagnostic.rule == LintRule::AgentOutputUnsafe)
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn a012_a013_leave_invalid_yaml_to_x001() {
        let content = format!(
            "---\nname: general\n\tdescription: {GOOD_DESC}\ntools: Bash\n---\nUse the Read tool. Read every evidence file. Output strict JSON only.\n"
        );
        run_private_agent(&content, |diag| {
            assert!(
                diag.diagnostics()
                    .iter()
                    .any(|diagnostic| diagnostic.rule == LintRule::FrontmatterYamlInvalid)
            );
            assert!(!diag.diagnostics().iter().any(|diagnostic| {
                matches!(
                    diagnostic.rule,
                    LintRule::AgentReadMismatch | LintRule::AgentOutputUnsafe
                )
            }));
        });

        let non_mapping = "---\n- tools: Bash\n---\nUse the Read tool. Read every evidence file. Output strict JSON only.\n";
        run_private_agent(non_mapping, |diag| {
            assert!(!diag.diagnostics().iter().any(|diagnostic| {
                matches!(
                    diagnostic.rule,
                    LintRule::AgentReadMismatch | LintRule::AgentOutputUnsafe
                )
            }));
        });
    }

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
            assert_eq!(
                finding.suggestion.as_deref(),
                Some("add either a concrete bound or a concrete failure outcome")
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
            "**Stop after 3 attempts and report the blocker.**",
            "- **Stop after 3 attempts and report the blocker.**",
            "__Give up after 10 minutes and escalate.__",
            "**You must stop after 3 attempts.**",
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
            "**Use of a timeout of 10 minutes was common in the legacy runner.**\n\nInvestigate the failure and implement the repair.\n",
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
            assert!(is_valid_model_value(ok), "expected '{ok}' to be valid");
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
            assert!(!is_valid_model_value(bad), "expected '{bad}' to be invalid");
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

    #[test]
    #[serial_test::serial]
    fn test_aPH14PH_fable_and_claude_fable_clean() {
        for model in ["fable", "claude-fable-5"] {
            let content = format!(
                "---\nname: general\ndescription: {GOOD_DESC}\nmodel: {model}\n---\nBody\n"
            );
            run_agent(&content, |diag| {
                assert!(
                    !diag.errors().iter().any(|e| e.contains("model")),
                    "A014 must accept {model}"
                );
            });
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_aPH14PH_inherit_1m_fires() {
        let content = format!(
            "---\nname: general\ndescription: {GOOD_DESC}\nmodel: inherit[1m]\n---\nBody\n"
        );
        run_agent(&content, |diag| {
            assert!(
                diag.errors()
                    .iter()
                    .any(|e| e.contains("not a recognized Claude Code model")),
                "A014 must reject inherit[1m]"
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn test_s063_a014_model_verdicts_agree() {
        // Cross-rule consistency: skill S063 and agent A014 share is_valid_model_value.
        let cases: &[(&str, bool)] = &[
            ("fable", true),
            ("opusplan", true),
            ("best", true),
            ("claude-fable-5", true),
            ("haiku[1m]", false),
            ("inherit[1m]", false),
            ("sonet", false),
            ("", false),
        ];
        for &(model, expect_valid) in cases {
            assert_eq!(
                is_valid_model_value(model),
                expect_valid,
                "shared helper mismatch for model={model:?}"
            );
            if model.is_empty() {
                continue; // empty is FieldState::Empty on skills; helper covers both
            }
            let content = format!(
                "---\nname: general\ndescription: {GOOD_DESC}\nmodel: {model}\n---\nBody\n"
            );
            run_agent(&content, |diag| {
                let fires = diag
                    .diagnostics()
                    .iter()
                    .any(|d| d.rule == LintRule::AgentModelInvalid);
                assert_eq!(
                    !fires, expect_valid,
                    "A014 verdict mismatch for model={model:?}"
                );
            });
        }
    }

    // ── A015: agent-permission-invalid ───────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_aPH15PH_invalid_permission_fires() {
        let content = format!(
            "---\nname: general\ndescription: {GOOD_DESC}\npermissionMode: yolo\n---\nBody\n"
        );
        run_private_agent(&content, |diag| {
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

    #[test]
    #[serial_test::serial]
    fn test_aPH15PH_newly_documented_permission_modes_no_fire() {
        for mode in ["auto", "manual"] {
            let content = format!(
                "---\nname: general\ndescription: {GOOD_DESC}\npermissionMode: {mode}\n---\nBody\n"
            );
            run_agent(&content, |diag| {
                assert!(
                    !diag
                        .errors()
                        .iter()
                        .any(|e| e.contains(&format!("permissionMode '{mode}'"))),
                    "A015 must accept documented permissionMode {mode}"
                );
            });
        }
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
                    .any(|e| e.contains("no matching runtime skill"))
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

    /// #342: overlap is exact-full-token — restricted forms are never
    /// collapsed to base names, so different restrictions do not overlap
    /// while identical normalized declarations do.
    #[test]
    #[serial_test::serial]
    fn test_aPH17PH_restricted_forms_overlap_only_when_identical() {
        let content = format!(
            "---\nname: general\ndescription: {GOOD_DESC}\ntools: Bash(git *)\ndisallowedTools: Bash(rm *)\n---\nBody\n"
        );
        run_agent(&content, |diag| {
            assert!(
                !diag.errors().iter().any(|e| e.contains("appears in both")),
                "different restrictions must not overlap: {:?}",
                diag.errors()
            );
        });
        let content = format!(
            "---\nname: general\ndescription: {GOOD_DESC}\ntools: Bash(git *)\ndisallowedTools: Bash(git *)\n---\nBody\n"
        );
        run_agent(&content, |diag| {
            assert!(
                diag.errors()
                    .iter()
                    .any(|e| e.contains("tool 'Bash(git *)' appears in both")),
                "identical restricted declarations must overlap: {:?}",
                diag.errors()
            );
        });
    }

    /// #342: each exact overlap reports once, in first-declaration order.
    #[test]
    #[serial_test::serial]
    fn test_aPH17PH_overlaps_report_once_in_declaration_order() {
        let content = format!(
            "---\nname: general\ndescription: {GOOD_DESC}\ntools: Bash, Read, Bash\ndisallowedTools: Read, Bash\n---\nBody\n"
        );
        run_agent(&content, |diag| {
            let overlaps: Vec<_> = diag
                .errors()
                .iter()
                .filter(|e| e.contains("appears in both"))
                .cloned()
                .collect();
            assert_eq!(
                overlaps.len(),
                2,
                "each overlap reports exactly once: {overlaps:?}"
            );
            assert!(overlaps[0].contains("'Bash'"), "{overlaps:?}");
            assert!(overlaps[1].contains("'Read'"), "{overlaps:?}");
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
    fn test_aPH19PH_powershell_clean() {
        let content =
            format!("---\nname: general\ndescription: {GOOD_DESC}\ntools: PowerShell\n---\nBody\n");
        run_agent(&content, |diag| {
            assert!(
                !diag
                    .errors()
                    .iter()
                    .any(|e| e.contains("unrecognized tool")),
                "A019 must accept PowerShell"
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn test_aPH19PH_newly_documented_tool_no_fire() {
        let content = format!(
            "---\nname: general\ndescription: {GOOD_DESC}\ntools: ExitPlanMode\n---\nBody\n"
        );
        run_agent(&content, |diag| {
            assert!(
                !diag
                    .errors()
                    .iter()
                    .any(|e| e.contains("tools lists unrecognized tool")),
                "A019 must accept the documented ExitPlanMode tool"
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

    /// #342: agent tool scalars use the shared tokenizer — commas inside a
    /// restriction are pattern text, and whitespace separation is accepted as
    /// a conservative superset.
    #[test]
    #[serial_test::serial]
    fn test_aPH19PH_scalar_tokenizer_forms_clean() {
        for tools in [
            "tools: Bash(npm install, npm test), Read",
            "tools: Read Write",
            "tools: Bash(git add *) Bash(git commit *) Bash(git status *)",
        ] {
            let content =
                format!("---\nname: general\ndescription: {GOOD_DESC}\n{tools}\n---\nBody\n");
            run_agent(&content, |diag| {
                assert!(
                    !diag
                        .errors()
                        .iter()
                        .any(|e| e.contains("unrecognized tool")),
                    "{tools} must not fire A019: {:?}",
                    diag.errors()
                );
            });
        }
    }

    /// #342: commented flow and block lists are resolved by the YAML parser
    /// and must not produce comment-bearing tokens.
    #[test]
    #[serial_test::serial]
    fn test_aPH19PH_aPH20PH_commented_lists_clean() {
        let content = format!(
            "---\nname: general\ndescription: {GOOD_DESC}\ntools: [Bash, Read] # comment\ndisallowedTools:\n  - Write # comment\n---\nBody\n"
        );
        run_agent(&content, |diag| {
            assert!(
                !diag
                    .errors()
                    .iter()
                    .any(|e| e.contains("unrecognized tool")),
                "commented lists must stay clean: {:?}",
                diag.errors()
            );
        });
    }

    /// #342: duplicate unknown entries report once per (field, token).
    #[test]
    #[serial_test::serial]
    fn test_aPH19PH_aPH20PH_duplicate_unknowns_report_once_per_field() {
        let content = format!(
            "---\nname: general\ndescription: {GOOD_DESC}\ntools: Bsh, Bsh\ndisallowedTools: Bsh\n---\nBody\n"
        );
        run_agent(&content, |diag| {
            let tools_findings = diag
                .errors()
                .iter()
                .filter(|e| e.contains("tools lists unrecognized tool 'Bsh'"))
                .count();
            let disallowed_findings = diag
                .errors()
                .iter()
                .filter(|e| e.contains("disallowedTools lists unrecognized tool 'Bsh'"))
                .count();
            assert_eq!(
                tools_findings,
                1,
                "duplicate unknown tools entries report once: {:?}",
                diag.errors()
            );
            assert_eq!(
                disallowed_findings,
                1,
                "disallowedTools reports its own token once: {:?}",
                diag.errors()
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
        std::fs::create_dir_all(".claude/agents").unwrap();
        std::fs::write(".claude/agents/general.md", content).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_private_agents(&mut diag, &crate::config::ExcludeSet::default());
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
        for (isolation, expected) in [
            ("container", "container"),
            ("Remote", "Remote"),
            ("remote", "remote"),
            ("worktre", "worktre"),
            ("\"\"", ""),
        ] {
            let content = format!(
                "---\nname: general\ndescription: {GOOD_DESC}\nisolation: {isolation}\n---\nBody\n"
            );
            run_agent(&content, |diag| {
                assert!(
                    diag.errors()
                        .iter()
                        .any(|e| e
                            .contains(&format!("isolation '{expected}' is not one of [worktree]"))),
                    "expected invalid isolation {isolation:?} to fire"
                );
            });
        }

        for isolation in ["true", "1", "[worktree]", "{mode: worktree}"] {
            let content = format!(
                "---\nname: general\ndescription: {GOOD_DESC}\nisolation: {isolation}\n---\nBody\n"
            );
            run_agent(&content, |diag| {
                assert!(
                    diag.errors()
                        .iter()
                        .any(|e| e.contains("isolation must be a string (got ")),
                    "expected non-string isolation {isolation:?} to fire"
                );
            });
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_aPH24PH_valid_isolation_no_fire() {
        let content = format!(
            "---\nname: general\ndescription: {GOOD_DESC}\nisolation: worktree\n---\nBody\n"
        );
        run_agent(&content, |diag| {
            assert!(
                !diag.errors().iter().any(|e| e.contains("isolation")),
                "expected worktree isolation not to fire"
            );
        });
    }

    // ── A025: agent-background-invalid (warn) ────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_aPH25PH_invalid_background_fires() {
        for background in ["yes", "no", "\"true\""] {
            let content = format!(
                "---\nname: general\ndescription: {GOOD_DESC}\nbackground: {background}\n---\nBody\n"
            );
            run_agent(&content, |diag| {
                assert!(diag.errors().iter().any(|e| e.contains(
                    "background must be a boolean (got string; use unquoted true or false — YAML 1.2 does not read yes/no as booleans)"
                )));
            });
        }

        for (background, actual_type) in [
            ("null", "null"),
            ("1", "number"),
            ("[true]", "sequence"),
            ("{enabled: true}", "mapping"),
        ] {
            let content = format!(
                "---\nname: general\ndescription: {GOOD_DESC}\nbackground: {background}\n---\nBody\n"
            );
            run_agent(&content, |diag| {
                assert!(diag.errors().iter().any(|e| {
                    e.contains(&format!("background must be a boolean (got {actual_type})"))
                }));
            });
        }
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
            "---\nname: general\ndescription: {GOOD_DESC}\nmodel: sonnet\neffort: high\ninitialPrompt: Start by reading the project guide.\ncolor: cyan\n---\nBody\n"
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
    fn test_aPH27PH_color_typos_still_fire() {
        for key in ["colour", "Color"] {
            let content =
                format!("---\nname: general\ndescription: {GOOD_DESC}\n{key}: cyan\n---\nBody\n");
            run_agent(&content, |diag| {
                assert!(
                    diag.errors()
                        .iter()
                        .any(|e| e.contains(&format!("unrecognized frontmatter field '{key}'")))
                );
            });
        }
    }

    #[test]
    fn test_yaml_type_reports_string_values() {
        let value = crate::yaml::parse("value: yes\n").unwrap();
        let value = value.as_mapping().unwrap().get("value").unwrap();
        assert_eq!(yaml_type(value), "string");
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

    // ── canonical list shapes ───────────────────────────────────────

    #[test]
    fn test_canonical_string_list_scalar_and_sequences() {
        for (source, expected) in [
            ("tools: Bash, Read , Write\n", vec!["Bash", "Read", "Write"]),
            ("tools: [Bash, Read]\n", vec!["Bash", "Read"]),
            ("tools:\n  - Bash\n  - \"Read\"\n", vec!["Bash", "Read"]),
        ] {
            let yaml = crate::yaml::parse(source).unwrap();
            let lines = vec!["name: example".to_string()];
            let frontmatter = AgentFrontmatter::from_yaml(&yaml, &lines).unwrap();
            match canonical_string_list(&frontmatter, "tools") {
                StringList::Valid(items) => assert_eq!(items, expected),
                _ => panic!("expected a canonical string list"),
            }
        }

        let yaml = crate::yaml::parse("tools: [Bash, 1]\n").unwrap();
        let lines = vec!["tools: 42".to_string()];
        let frontmatter = AgentFrontmatter::from_yaml(&yaml, &lines).unwrap();
        assert!(matches!(
            canonical_string_list(&frontmatter, "tools"),
            StringList::Invalid
        ));
    }

    /// #342: the tool-field reader shares shape ownership with
    /// `canonical_string_list` but tokenizes scalars with the shared
    /// outside-parentheses splitter.
    #[test]
    fn test_canonical_tool_list_uses_shared_tokenizer() {
        for (source, expected) in [
            (
                "tools: Bash(npm install, npm test), Read\n",
                vec!["Bash(npm install, npm test)", "Read"],
            ),
            ("tools: Read Write\n", vec!["Read", "Write"]),
            // Flow-sequence commas are YAML separators; a comma-bearing
            // restriction must be quoted to stay one item.
            (
                "tools: [\"Bash(a, b)\", Read]\n",
                vec!["Bash(a, b)", "Read"],
            ),
        ] {
            let yaml = crate::yaml::parse(source).unwrap();
            let lines = vec!["name: example".to_string()];
            let frontmatter = AgentFrontmatter::from_yaml(&yaml, &lines).unwrap();
            match canonical_tool_list(&frontmatter, "tools") {
                StringList::Valid(items) => assert_eq!(items, expected, "{source}"),
                _ => panic!("expected a canonical tool list for {source}"),
            }
        }

        let yaml = crate::yaml::parse("tools: [Bash, 1]\n").unwrap();
        let lines = vec!["name: example".to_string()];
        let frontmatter = AgentFrontmatter::from_yaml(&yaml, &lines).unwrap();
        assert!(matches!(
            canonical_tool_list(&frontmatter, "tools"),
            StringList::Invalid
        ));
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
        run_private_agent(&content, |diag| {
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
    fn canonical_yaml_values_do_not_trigger_raw_line_findings() {
        let content = format!(
            "---\n\"name\": general\n\"description\": {GOOD_DESC}\nmodel: >\n  sonnet\npermissionMode: acceptEdits # comment\nmemory: project # comment\neffort: high # comment\nisolation: worktree # comment\nbackground: true # comment\ntools: [Bash, Read] # comment\ndisallowedTools: [Write]\n---\nBody\n"
        );
        run_private_agent(&content, |diag| {
            assert!(
                !diag.diagnostics().iter().any(|diagnostic| {
                    matches!(
                        diagnostic.rule,
                        LintRule::AgentModelInvalid
                            | LintRule::AgentPermissionInvalid
                            | LintRule::AgentMemoryInvalid
                            | LintRule::AgentToolsUnknown
                            | LintRule::AgentDisallowedUnknown
                            | LintRule::AgentEffortInvalid
                            | LintRule::AgentIsolationInvalid
                            | LintRule::AgentBackgroundInvalid
                            | LintRule::AgentFieldUnknown
                    )
                }),
                "canonical YAML should be the only input: {:?}",
                diag.diagnostics()
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn a002_distinguishes_opening_and_closing_delimiter_and_bom() {
        for (content, expected) in [
            ("name: general\n", "must start with '---' on line 1"),
            (
                "---\nname: general\n",
                "opening delimiter has no closing '---'",
            ),
            (
                "\u{feff}---\nname: general\ndescription: A useful routing description\n---\n",
                "file starts with a UTF-8 byte-order mark; remove it",
            ),
        ] {
            run_agent(content, |diag| {
                let findings: Vec<_> = diag
                    .diagnostics()
                    .iter()
                    .filter(|diagnostic| diagnostic.rule == LintRule::AgentFrontmatterMalformed)
                    .collect();
                assert_eq!(findings.len(), 1);
                assert!(findings[0].message.contains(expected));
                assert_eq!(findings[0].location, Some(SourceSpan::line(1)));
                assert!(findings[0].suggestion.is_some());
                assert!(!diag.diagnostics().iter().any(|diagnostic| {
                    matches!(
                        diagnostic.rule,
                        LintRule::AgentFieldMissing
                            | LintRule::AgentDescLong
                            | LintRule::AgentDescShort
                            | LintRule::AgentNameInvalid
                            | LintRule::AgentDescRedundant
                    )
                }));
            });
        }
    }

    #[test]
    #[serial_test::serial]
    fn prompt_recovery_skips_unterminated_and_bom_metadata_for_agents() {
        run_private_agent(
            "---\nname: reviewer\ndescription: malformed.\n\nRetry until success.\n",
            |diag| {
                assert!(diag.diagnostics().iter().any(|item| {
                    item.rule == LintRule::AgentFrontmatterMalformed
                        && item.message.contains("no closing")
                }));
                assert!(
                    !diag
                        .diagnostics()
                        .iter()
                        .any(|item| item.rule.code() == "Q005")
                );
            },
        );

        run_private_agent(
            "\u{feff}---\nname: reviewer\ndescription: Reviews changes with concrete test evidence\nRetry until success.: true\n---\nSafe body.\n",
            |diag| {
                assert!(diag.diagnostics().iter().any(|item| {
                    item.rule == LintRule::AgentFrontmatterMalformed
                        && item.message.contains("byte-order mark")
                }));
                assert!(
                    !diag
                        .diagnostics()
                        .iter()
                        .any(|item| item.rule.code() == "Q005"),
                    "BOM-prefixed metadata must not emit Q005: {:?}",
                    diag.diagnostics()
                );
            },
        );

        run_private_agent(
            "\u{feff}---\nname: reviewer\ndescription: Reviews changes with concrete test evidence\n---\nRetry until success.\n",
            |diag| {
                let q005 = diag
                    .diagnostics()
                    .iter()
                    .find(|item| item.rule.code() == "Q005")
                    .expect("body prose remains live after BOM-prefixed complete block");
                assert_eq!(q005.location.unwrap().start().line_number(), 5);
            },
        );
    }

    #[test]
    #[serial_test::serial]
    fn a003_a008_to_a011_consume_only_canonical_yaml_values() {
        let folded = "---\n\"name\": folded-reviewer\n\"description\": >-\n  Reviews folded YAML agent descriptions without line parser artifacts\n---\nBody\n";
        run_private_agent(folded, |diag| {
            assert!(!diag.diagnostics().iter().any(|diagnostic| {
                matches!(
                    diagnostic.rule,
                    LintRule::AgentFieldMissing | LintRule::AgentDescShort
                )
            }));
        });

        let short_with_comment = "---\nname: concise\ndescription: \"1234567890123456789\" # canonical value is 19 chars\n---\nBody\n";
        run_agent(short_with_comment, |diag| {
            assert_eq!(
                diag.diagnostics()
                    .iter()
                    .filter(|diagnostic| diagnostic.rule == LintRule::AgentDescShort)
                    .count(),
                1
            );
        });

        let blank = "---\nname: concise\ndescription: \"                    \"\n---\nBody\n";
        run_agent(blank, |diag| {
            assert_eq!(
                diag.diagnostics()
                    .iter()
                    .filter(|diagnostic| diagnostic.rule == LintRule::AgentFieldMissing)
                    .count(),
                1
            );
            assert!(
                !diag
                    .diagnostics()
                    .iter()
                    .any(|diagnostic| diagnostic.rule == LintRule::AgentDescShort)
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn a003_has_single_structural_owner_and_ordered_field_findings() {
        run_agent("---\n- not\n- a mapping\n---\nBody\n", |diag| {
            assert_eq!(
                diag.diagnostics()
                    .iter()
                    .filter(|diagnostic| diagnostic.rule == LintRule::AgentFieldMissing)
                    .count(),
                1
            );
            assert!(
                !diag
                    .diagnostics()
                    .iter()
                    .any(|diagnostic| diagnostic.rule == LintRule::FrontmatterYamlInvalid)
            );
        });
        run_agent(
            "---\nname: null\ndescription: [not, text]\n---\nBody\n",
            |diag| {
                let messages: Vec<_> = diag
                    .diagnostics()
                    .iter()
                    .filter(|diagnostic| diagnostic.rule == LintRule::AgentFieldMissing)
                    .map(|diagnostic| diagnostic.message.as_str())
                    .collect();
                assert_eq!(messages.len(), 2);
                assert!(messages[0].contains("'name' must be a string (found null)"));
                assert!(messages[1].contains("'description' must be a string (found sequence)"));
                let locations: Vec<_> = diag
                    .diagnostics()
                    .iter()
                    .filter(|diagnostic| diagnostic.rule == LintRule::AgentFieldMissing)
                    .map(|diagnostic| diagnostic.location)
                    .collect();
                assert_eq!(
                    locations,
                    vec![Some(SourceSpan::line(2)), Some(SourceSpan::line(3))]
                );
                assert!(!diag.diagnostics().iter().any(|diagnostic| {
                    matches!(
                        diagnostic.rule,
                        LintRule::AgentDescLong
                            | LintRule::AgentDescShort
                            | LintRule::AgentNameInvalid
                            | LintRule::AgentDescRedundant
                    )
                }));
            },
        );
    }

    #[test]
    fn a011_normalizes_originating_inflections() {
        assert!(is_desc_redundant("security-reviewer", "Reviews security"));
        assert!(is_desc_redundant("test-runner", "Runs tests"));
    }

    #[test]
    #[serial_test::serial]
    fn malformed_lists_report_one_owner_without_entry_cascades() {
        let content = format!(
            "---\nname: general\ndescription: {GOOD_DESC}\ntools: [Bash, 1]\ndisallowedTools: {{ bad: value }}\nskills: [bad_skill, 1]\n---\nBody\n"
        );
        run_private_agent(&content, |diag| {
            for rule in [
                LintRule::AgentToolsUnknown,
                LintRule::AgentDisallowedUnknown,
                LintRule::AgentSkillMissing,
            ] {
                assert_eq!(
                    diag.diagnostics()
                        .iter()
                        .filter(|diagnostic| diagnostic.rule == rule)
                        .count(),
                    1,
                    "expected one shape diagnostic for {rule:?}"
                );
            }
            assert!(!diag.diagnostics().iter().any(|diagnostic| {
                matches!(
                    diagnostic.rule,
                    LintRule::AgentToolsOverlap | LintRule::AgentSkillKebab
                )
            }));
        });
    }

    #[test]
    #[serial_test::serial]
    fn basic_skill_lookup_excludes_plugin_only_skills() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude/agents").unwrap();
        std::fs::create_dir_all("skills/plugin-only").unwrap();
        std::fs::write(
            "skills/plugin-only/SKILL.md",
            "---\nname: plugin-only\n---\n",
        )
        .unwrap();
        std::fs::write(
            ".claude/agents/general.md",
            format!(
                "---\nname: general\ndescription: {GOOD_DESC}\nskills: plugin-only\n---\nBody\n"
            ),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_private_agents(&mut diag, &ExcludeSet::default());
        assert!(
            diag.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.rule == LintRule::AgentSkillMissing)
        );
    }

    #[test]
    #[serial_test::serial]
    fn plugin_unsupported_fields_have_sole_ownership() {
        let content = format!(
            "---\nname: general\ndescription: {GOOD_DESC}\npermissionMode: bypassPermissions\nhooks: {{ NotAnEvent: [] }}\nmcpServers: {{}}\n---\nBody\n"
        );
        run_agent(&content, |diag| {
            assert_eq!(
                diag.diagnostics()
                    .iter()
                    .filter(|diagnostic| diagnostic.rule == LintRule::AgentFieldUnsupported)
                    .count(),
                3
            );
            assert!(!diag.diagnostics().iter().any(|diagnostic| {
                matches!(
                    diagnostic.rule,
                    LintRule::AgentPermissionInvalid
                        | LintRule::AgentBypassPermissions
                        | LintRule::HookEventInvalid
                        | LintRule::AgentFieldUnknown
                )
            }));
        });
    }

    #[test]
    #[serial_test::serial]
    fn plugin_runtime_keeps_private_agent_field_contracts() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude/agents").unwrap();
        std::fs::write(
            ".claude/agents/general.md",
            format!(
                "---\nname: general\ndescription: {GOOD_DESC}\npermissionMode: bypassPermissions\nhooks: {{ NotAnEvent: [] }}\nmcpServers: {{}}\n---\nBody\n"
            ),
        )
        .unwrap();
        let mut pass = super::super::prompt_content::PromptContentPass::default();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_private_agents_with_prompt_pass(
            &mut diag,
            &ExcludeSet::default(),
            &mut pass,
            true,
        );
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.rule == LintRule::AgentFieldUnsupported)
        );
        assert!(
            diag.diagnostics()
                .iter()
                .any(|diagnostic| { diagnostic.rule == LintRule::AgentBypassPermissions })
        );
        assert!(
            diag.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.rule == LintRule::HookEventInvalid)
        );
    }

    #[test]
    #[serial_test::serial]
    fn invalid_or_non_mapping_yaml_skips_agent_field_rules() {
        for content in [
            "---\nmodel: [sonnet\n---\nBody\n".to_string(),
            "---\n- model: sonet\n- background: nope\n---\nBody\n".to_string(),
        ] {
            run_private_agent(&content, |diag| {
                assert!(!diag.diagnostics().iter().any(|diagnostic| {
                    let code = diagnostic.rule.code();
                    matches!(
                        code,
                        "A014"
                            | "A015"
                            | "A016"
                            | "A017"
                            | "A018"
                            | "A019"
                            | "A020"
                            | "A021"
                            | "A022"
                            | "A023"
                            | "A024"
                            | "A025"
                            | "A026"
                            | "A027"
                            | "A028"
                    )
                }));
            });
        }
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
