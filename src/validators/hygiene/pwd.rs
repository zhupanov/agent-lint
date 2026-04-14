use crate::config::ExcludeSet;
use crate::diagnostic::DiagnosticCollector;
use crate::rules::LintRule;
use regex::Regex;
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

static RE_PWD_HYGIENE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[$]PWD/|[$]\{PWD\}/|/Users/|/home/|/opt/").unwrap());

/// V8: ${CLAUDE_PLUGIN_ROOT} hygiene -- public skills/*/SKILL.md must not use
/// $PWD/, ${PWD}/, or hardcoded paths (/Users/, /home/, /opt/).
pub fn validate_pwd_hygiene(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let skills_dir = Path::new("skills");
    if !skills_dir.is_dir() {
        return;
    }

    let entries = match fs::read_dir(skills_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if name == "shared" {
            continue;
        }

        let skill_path = format!("skills/{name}/SKILL.md");
        if exclude.is_excluded(&skill_path) {
            continue;
        }

        let skill_md = path.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }

        let content = match fs::read_to_string(&skill_md) {
            Ok(c) => c,
            Err(_) => continue,
        };

        if RE_PWD_HYGIENE.is_match(&content) {
            diag.report(
                LintRule::PwdInSkill,
                &format!(
                    "skills/{name}/SKILL.md uses $PWD/ or hardcoded path; use ${{CLAUDE_PLUGIN_ROOT}}/ instead"
                ),
            );
        }
    }
}
