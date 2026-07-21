use crate::diagnostic::DiagnosticCollector;
use crate::frontmatter;
use crate::rules::LintRule;
use crate::validators::skills::SkillInfo;
use regex::Regex;
use std::collections::HashSet;
use std::sync::LazyLock;

use super::description::{RE_TRIGGER, STOPWORDS};

// S028 / S069: $ARGUMENTS
static RE_ARGS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\$ARGUMENTS|\$\{ARGUMENTS\}").unwrap());

// S068: inline dynamic injections — !`cmd` at line start or after whitespace
static RE_INLINE_INJECT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)(?:^|[ \t])!`[^`]+`").unwrap());

/// Minimum number of keywords required from description to run S053.
const MIN_KEYWORDS: usize = 3;

/// Warn when more than this many dynamic injections appear in the body.
const MAX_DYNAMIC_INJECTIONS: usize = 3;

pub(super) fn check_cross_field(
    info: &SkillInfo,
    plugin_mode: bool,
    diag: &mut DiagnosticCollector,
) {
    // S028: $ARGUMENTS in body without argument-hint (only outside code fences)
    let body_has_args =
        crate::fence::lines_outside_fences(&info.body).any(|line| RE_ARGS.is_match(line));
    if body_has_args && !frontmatter::field_exists(&info.fm_lines, "argument-hint") {
        diag.report(
            LintRule::ArgsNoHint,
            &format!(
                "{}: body uses $ARGUMENTS but frontmatter has no 'argument-hint' field",
                info.path
            ),
        );
    }

    // S069: argument-hint set but body never references $ARGUMENTS (smell; args also auto-append)
    if frontmatter::field_exists(&info.fm_lines, "argument-hint") && !body_has_args {
        // Also count $ARGUMENTS inside fences — presence anywhere in body is enough
        if !RE_ARGS.is_match(&info.body) {
            diag.report(
                LintRule::HintNoArgs,
                &format!(
                    "{}: 'argument-hint' is set but body never references $ARGUMENTS",
                    info.path
                ),
            );
        }
    }

    check_injection_overflow(info, diag);

    // S053: description/body keyword alignment (plugin-only)
    if plugin_mode {
        check_desc_body_alignment(info, diag);
    }
}

fn count_dynamic_injections(body: &str) -> usize {
    let inline = RE_INLINE_INJECT.find_iter(body).count();
    let fenced = body
        .lines()
        .filter(|l| l.trim_start().starts_with("```!"))
        .count();
    inline + fenced
}

fn check_injection_overflow(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    let count = count_dynamic_injections(&info.body);
    if count > MAX_DYNAMIC_INJECTIONS {
        diag.report(
            LintRule::InjectionOverflow,
            &format!(
                "{}: body has {count} dynamic injections (!`…` / ```!); prefer at most {MAX_DYNAMIC_INJECTIONS}",
                info.path
            ),
        );
    }
}

fn extract_keywords(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .map(|w| w.to_lowercase())
        .filter(|w| w.len() > 2 && !STOPWORDS.contains(&w.as_str()))
        .collect()
}

fn check_desc_body_alignment(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    // Bail early on empty body (S020 covers this separately)
    if info.body.trim().is_empty() {
        return;
    }

    // Alignment follows the same canonical scalar contract as the description
    // rules. Invalid YAML and missing/non-string values are diagnosed by the
    // frontmatter validators instead.
    let desc = match frontmatter::get_strict_string_field(&info.fm_lines, "description") {
        Some(d) => d,
        None => return,
    };

    // Strip trigger phrases before extracting keywords
    let stripped = RE_TRIGGER.replace_all(&desc, " ");
    let desc_keywords = extract_keywords(&stripped);

    let total = desc_keywords.len();
    if total < MIN_KEYWORDS {
        return; // Too few keywords to make a meaningful comparison
    }

    // Tokenize body text outside code fences (consistent with S028)
    let body_text: String = crate::fence::lines_outside_fences(&info.body)
        .collect::<Vec<_>>()
        .join(" ");
    let body_keywords = extract_keywords(&body_text);

    if body_keywords.is_empty() {
        return; // No prose tokens outside fences — skip alignment check
    }

    let matched = desc_keywords.intersection(&body_keywords).count();

    // Fire when fewer than 50% of description keywords appear in body
    // Using integer math: matched * 2 < total
    if matched * 2 < total {
        diag.report(
            LintRule::DescBodyMisalign,
            &format!(
                "{}: description keywords not reflected in body ({}/{} matched); \
                 body should deliver what the description promises",
                info.path, matched, total
            ),
        );
    }
}
