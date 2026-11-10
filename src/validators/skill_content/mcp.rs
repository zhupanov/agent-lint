use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata};
use crate::rules::LintRule;
use crate::validators::skills::SkillInfo;
use regex::Regex;
use std::collections::HashSet;
use std::sync::LazyLock;

// S044: Backtick-quoted snake_case identifier with at least one underscore.
static RE_SNAKE_IDENTIFIER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z][a-z0-9]*(?:_[a-z0-9]+)+$").unwrap());

// Invocation vocabulary that must sit immediately before the identifier, with
// at most one determiner between ("use the `x`", "Call `x`"). Matched on word
// boundaries so prose substrings ("use" inside "Because", "run" inside
// "prune") do not pass the gate. Case-insensitive via the inline `(?i)` flag.
static RE_INVOCATION_BEFORE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:use[sd]?|using|call(?:s|ed|ing)?|invoke[sd]?|invoking|run(?:s|ning)?|ran|execute[sd]?|executing)\s+(?:(?:the|a|an|this|that|its|their|your|our|new)\s+)?$",
    )
    .unwrap()
});

// The noun form on either side: "these tools: `x`", "the `x` tool". Nothing
// but whitespace and punctuation may sit between the noun and the identifier,
// on either side, so a later clause cannot reach back to it ("prints `job_id`,
// `owner_id` and tool versions" names no tool of its own).
static RE_TOOL_BEFORE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\btools?\b[\s:,]*$").unwrap());
static RE_TOOL_AFTER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^[\s,:;)\]]*tools?\b").unwrap());

// "the `x` tool" names a tool; "prints `x` tools are configured elsewhere"
// starts a new clause with the same two words next to each other. A
// determiner before the identifier is what separates the noun phrase from the
// accident, so the noun-after form requires one.
static RE_DETERMINER_BEFORE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:the|a|an|this|that|each|every|its|their|your|our)\s+$").unwrap()
});

// Built-in platform tools in snake_case form. Single-word tools (bash, read,
// etc.) are excluded automatically by the identifier's underscore requirement.
// This list covers multi-word built-ins that a user might write in snake_case.
// See also: KNOWN_TOOLS in common.rs (PascalCase, shared by S040 and A019/A020).
const BUILTIN_TOOLS_SNAKE: &[&str] = &[
    "task_create",
    "task_update",
    "task_list",
    "task_get",
    "task_stop",
    "task_output",
    "ask_user_question",
    "notebook_edit",
    "web_fetch",
    "web_search",
];

/// S044: Detect backtick-quoted MCP tool references that lack a `ServerName:` prefix.
///
/// An identifier is a candidate only when invocation grammar is *adjacent* to
/// it: an invocation verb immediately before it (optionally through one
/// determiner), or the noun "tool"/"tools" immediately before or after. Three
/// earlier false-positive classes follow from gating on the whole physical
/// line instead (issue #676):
///
/// 1. a context word inside a code span (`` `run` `` as a printed field name)
///    opened the gate; code spans are masked out of the prose here;
/// 2. a context word anywhere on the line opened it, so the result depended on
///    where the paragraph happened to wrap; adjacency is computed over the
///    whole paragraph, which makes rewrapping irrelevant;
/// 3. the noun opened it as well as the verb ("the run id"), which adjacency
///    rejects because the noun is not next to the identifier.
///
/// Runs in both plugin and private skill modes (no `plugin_mode` gate).
/// Scans body content only; fenced code blocks contribute no inline code.
pub(super) fn check_mcp_tool_refs(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    if info.body.trim().is_empty() {
        return;
    }

    let document = &info.document;
    let content = document.content();
    // Raw HTML blocks (`<example>…</example>`) hide their code spans from the
    // parser's inline facts, and skills do address tools inside them, so the
    // structural view that recovers those spans is the candidate source.
    let spans = document.structural_inline_code();
    let masked = mask_inline_code(content, &spans);
    let mut reported: HashSet<String> = HashSet::new();

    for span in &spans {
        if span.start_line < document.body_start_line() {
            continue;
        }
        let identifier = span.literal.as_str();
        if !RE_SNAKE_IDENTIFIER.is_match(identifier) {
            // Note: colon-qualified forms like `BigQuery:bigquery_schema` are
            // already excluded — neither `:` nor uppercase is in [a-z0-9_].
            continue;
        }
        if BUILTIN_TOOLS_SNAKE.contains(&identifier) {
            continue;
        }
        if !has_adjacent_invocation_grammar(&masked, span.byte_range.clone()) {
            continue;
        }
        // Report each distinct tool name once per file, at its first use.
        if !reported.insert(identifier.to_string()) {
            continue;
        }

        diag.report_at_with(
            LintRule::McpToolUnqualified,
            &info.path,
            &format!(
                "backtick-quoted '{identifier}' may be an MCP tool reference without ServerName: prefix (use ServerName:tool_name format)"
            ),
            DiagnosticMetadata::at_line(span.start_line),
        );
    }
}

/// Replace every inline code span, delimiters included, with filler that holds
/// its byte positions but matches no word. Prose vocabulary is then read from
/// the text alone: a printed field name such as `` `run` `` or `` `tools` ``
/// can no longer act as an invocation word for its neighbors.
fn mask_inline_code(content: &str, spans: &[crate::markdown::MarkdownInlineCode]) -> String {
    let mut masked = content.as_bytes().to_vec();
    for span in spans {
        // Byte-wise masking keeps every later span's offsets exact. A span
        // covers whole scalars, so overwriting its bytes with ASCII leaves
        // valid UTF-8; newlines survive so paragraph bounds stay readable.
        let Some(bytes) = masked.get_mut(span.byte_range.clone()) else {
            continue;
        };
        for byte in bytes {
            if *byte != b'\n' {
                *byte = b'#';
            }
        }
    }
    String::from_utf8(masked).expect("inline code spans cover whole scalars")
}

/// Whether invocation grammar sits immediately beside the span, within the
/// Markdown paragraph that holds it.
fn has_adjacent_invocation_grammar(masked: &str, span: std::ops::Range<usize>) -> bool {
    let paragraph = paragraph_bounds(masked, span.clone());
    let before = &masked[paragraph.start..span.start];
    let after = &masked[span.end..paragraph.end];
    RE_INVOCATION_BEFORE.is_match(before)
        || RE_TOOL_BEFORE.is_match(before)
        || (RE_TOOL_AFTER.is_match(after) && RE_DETERMINER_BEFORE.is_match(before))
}

/// Byte bounds of the blank-line-delimited block holding `span`.
///
/// Line breaks inside a Markdown paragraph are not semantic, so adjacency is
/// read across them; a blank line ends the block so an unrelated neighboring
/// paragraph cannot supply the grammar.
fn paragraph_bounds(masked: &str, span: std::ops::Range<usize>) -> std::ops::Range<usize> {
    let mut start = 0;
    let mut end = masked.len();
    let mut line_start = 0;
    for line in masked.split_inclusive('\n') {
        let line_end = line_start + line.len();
        if line.trim().is_empty() {
            if line_end <= span.start {
                start = line_end;
            } else if line_start >= span.end {
                end = line_start;
                break;
            }
        }
        line_start = line_end;
    }
    start..end
}
