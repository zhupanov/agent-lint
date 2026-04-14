use crate::diagnostic::DiagnosticCollector;
use crate::frontmatter;
use crate::rules::LintRule;
use crate::validators::skills::SkillInfo;
use regex::Regex;
use std::sync::LazyLock;

// S028: $ARGUMENTS
static RE_ARGS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\$ARGUMENTS|\$\{ARGUMENTS\}").unwrap());

pub(super) fn check_cross_field(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    // S028: $ARGUMENTS in body without argument-hint (only outside code fences)
    if crate::fence::lines_outside_fences(&info.body).any(|line| RE_ARGS.is_match(line))
        && !frontmatter::field_exists(&info.fm_lines, "argument-hint")
    {
        diag.report(
            LintRule::ArgsNoHint,
            &format!(
                "{}: body uses $ARGUMENTS but frontmatter has no 'argument-hint' field",
                info.path
            ),
        );
    }
}
