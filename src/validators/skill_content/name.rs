use crate::diagnostic::DiagnosticCollector;
use crate::frontmatter;
use crate::rules::LintRule;
use crate::validators::common::RE_NAME_INVALID;
use crate::validators::skills::SkillInfo;

pub(super) const MAX_SKILL_NAME_LEN: usize = 64;

pub(super) fn check_name_format(
    info: &SkillInfo,
    plugin_mode: bool,
    diag: &mut DiagnosticCollector,
) {
    let name = match frontmatter::get_field(&info.fm_lines, "name") {
        Some(n) => n,
        None => return, // S005 fires from existing validator
    };

    // S009: name too long
    if name.len() > MAX_SKILL_NAME_LEN {
        diag.report(
            LintRule::NameTooLong,
            &format!(
                "{}: name '{}' exceeds 64 characters ({})",
                info.path,
                name,
                name.len()
            ),
        );
    }

    // S010: invalid characters
    if RE_NAME_INVALID.is_match(&name) {
        diag.report(
            LintRule::NameInvalidChars,
            &format!(
                "{}: name '{}' contains characters outside [a-z0-9-]",
                info.path, name
            ),
        );
    }

    // S011: bad hyphens
    if name.starts_with('-') || name.ends_with('-') || name.contains("--") {
        diag.report(
            LintRule::NameBadHyphens,
            &format!(
                "{}: name '{}' starts/ends with hyphen or contains consecutive hyphens",
                info.path, name
            ),
        );
    }

    // S033: vague name (plugin-only)
    if plugin_mode {
        let vague_names = [
            "helper",
            "helpers",
            "utils",
            "utility",
            "tools",
            "data",
            "files",
            "documents",
        ];
        if vague_names.contains(&name.as_str()) {
            diag.report(
                LintRule::NameVague,
                &format!(
                    "{}: name '{}' is too vague/generic for a published skill",
                    info.path, name
                ),
            );
        }
    }

    // S049: name not in gerund form (plugin-only)
    if plugin_mode {
        const NON_GERUND_ING: &[&str] = &[
            "string", "ring", "spring", "king", "thing", "bling", "sing", "wing", "ping", "sting",
            "swing", "bring", "cling", "fling", "sling", "wring",
        ];
        let has_gerund = name
            .split('-')
            .any(|word| word.ends_with("ing") && !NON_GERUND_ING.contains(&word));
        if !has_gerund {
            diag.report(
                LintRule::NameNotGerund,
                &format!(
                    "{}: name '{}' does not use gerund form (consider e.g. 'processing-pdfs', 'reviewing-code')",
                    info.path, name
                ),
            );
        }
    }
}
