use crate::diagnostic::DiagnosticCollector;
use crate::frontmatter;
use crate::rules::LintRule;
use crate::validators::common::normalize_description_suffix;
use crate::validators::skills::SkillInfo;
use regex::Regex;
use std::collections::HashSet;
use std::sync::LazyLock;

use super::description::{RE_TRIGGER, STOPWORDS};

// S028 / S069: $ARGUMENTS
static RE_ARGS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\$ARGUMENTS|\$\{ARGUMENTS\}").unwrap());

// S069: positional argument references `$1`–`$9` or `${1}`–`${9}`. The digit
// must not be followed by an ASCII alphanumeric or `_`, so `$10` (argument 10)
// and `$1x` (identifier-shaped) are not treated as positional references while
// `#$1`, `($1)`, and `$1.` are. The Rust regex crate has no look-around, so the
// trailing boundary is matched (and consumed) as a character class or line end;
// this is sound for `is_match`, which only asks whether any reference exists.
static RE_POSITIONAL_ARG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\$(?:[1-9]|\{[1-9]\})(?:[^A-Za-z0-9_]|$)").unwrap());

// S068: inline dynamic injections — !`cmd` at line start or after whitespace
static RE_INLINE_INJECT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)(?:^|[ \t])!`[^`]+`").unwrap());

/// Minimum number of keywords required from description to run S054.
const MIN_KEYWORDS: usize = 3;

/// Warn when more than this many dynamic injections appear in the body.
const MAX_DYNAMIC_INJECTIONS: usize = 3;

pub(super) fn check_cross_field(
    info: &SkillInfo,
    plugin_mode: bool,
    diag: &mut DiagnosticCollector,
) {
    // S028: $ARGUMENTS in body without argument-hint (only outside code fences).
    // The trigger set is $ARGUMENTS forms only — bare positional `$1` refs
    // deliberately do NOT trigger S028, so currency-shaped prose such as
    // "costs $1" cannot manufacture a diagnostic on the emitting side.
    let body_has_args =
        crate::fence::lines_outside_fences(&info.body).any(|line| RE_ARGS.is_match(line));

    // S028/S069 read `argument-hint` presence from the canonical mapping: a key
    // present with any non-null value counts as set; a null value counts as
    // unset (S007/S070 own empty/unknown shapes). Invalid or non-mapping
    // frontmatter is owned by X001/S004 and skips both rules.
    if let Some(map) = info.frontmatter_mapping() {
        let hint_set = map
            .get("argument-hint")
            .is_some_and(|value| !value.is_null());
        if body_has_args && !hint_set {
            diag.report(
                LintRule::ArgsNoHint,
                &format!(
                    "{}: body uses $ARGUMENTS but frontmatter has no 'argument-hint' field",
                    info.path
                ),
            );
        }
        // S069: argument-hint set but body never references its arguments
        // (smell; args also auto-append). The body references arguments when
        // $ARGUMENTS appears anywhere (including fences — presence is enough),
        // or a positional reference `$1`–`$9` / `${1}`–`${9}` appears on a line
        // outside code fences. Fenced positional refs (e.g. awk '{print $1}')
        // are excluded so S060's territory cannot silently mask hint/body drift.
        if hint_set {
            let references_args = RE_ARGS.is_match(&info.body)
                || crate::fence::lines_outside_fences(&info.body)
                    .any(|line| RE_POSITIONAL_ARG.is_match(line));
            if !references_args {
                diag.report(
                    LintRule::HintNoArgs,
                    &format!(
                        "{}: 'argument-hint' is set but body never references $ARGUMENTS",
                        info.path
                    ),
                );
            }
        }
    }

    check_injection_overflow(info, diag);

    // S054: description/body keyword alignment (plugin-only)
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
        .map(|w| normalize_description_suffix(&w))
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

#[cfg(test)]
mod stem_tests {
    use crate::validators::common::normalize_description_suffix;

    #[test]
    fn stems_inflections_for_alignment() {
        assert_eq!(normalize_description_suffix("releasing"), "releas");
        assert_eq!(normalize_description_suffix("released"), "releas");
        assert_eq!(normalize_description_suffix("processes"), "process");
        assert_eq!(normalize_description_suffix("process"), "process");
        assert_eq!(normalize_description_suffix("summaries"), "summary");
        assert_eq!(normalize_description_suffix("changelogs"), "changelog");
        assert_eq!(normalize_description_suffix("generates"), "generate");
        assert_eq!(normalize_description_suffix("diffs"), "diff");
        assert_eq!(normalize_description_suffix("versions"), "version");
    }

    #[test]
    fn ss_guard_keeps_final_s() {
        assert_eq!(normalize_description_suffix("class"), "class");
        assert_eq!(normalize_description_suffix("process"), "process");
    }
}

#[cfg(test)]
mod positional_arg_tests {
    use super::RE_POSITIONAL_ARG;

    #[test]
    fn recognizes_positional_references() {
        // Bare `$1`–`$9`, at end of line or bounded by non-word characters.
        assert!(RE_POSITIONAL_ARG.is_match("Review PR #$1"));
        assert!(RE_POSITIONAL_ARG.is_match("wrap ($1) here"));
        assert!(RE_POSITIONAL_ARG.is_match("end with $1."));
        assert!(RE_POSITIONAL_ARG.is_match("priority $2 next"));
        assert!(RE_POSITIONAL_ARG.is_match("uses $9"));
        // Braced `${1}`–`${9}`.
        assert!(RE_POSITIONAL_ARG.is_match("apply ${2} carefully"));
        assert!(RE_POSITIONAL_ARG.is_match("trailing ${1}"));
    }

    #[test]
    fn rejects_non_positional_shapes() {
        // `$10`-style: the digit is followed by another ASCII alphanumeric.
        assert!(!RE_POSITIONAL_ARG.is_match("argument $10 here"));
        assert!(!RE_POSITIONAL_ARG.is_match("token $1x here"));
        assert!(!RE_POSITIONAL_ARG.is_match("var $1_name"));
        assert!(!RE_POSITIONAL_ARG.is_match("braced ${10} here"));
        // `$0` is not a positional argument.
        assert!(!RE_POSITIONAL_ARG.is_match("script $0 name"));
        // Plain prose without any reference.
        assert!(!RE_POSITIONAL_ARG.is_match("no references at all"));
    }
}
