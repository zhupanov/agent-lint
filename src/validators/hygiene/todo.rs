use crate::config::ExcludeSet;
use crate::diagnostic::DiagnosticCollector;
use crate::rules::LintRule;
use crate::traversal;
use crate::unfinished_work::find_first_unfinished_work_marker;
use std::fs;
use std::path::Path;

/// G006: TODO/FIXME/HACK/XXX markers in published skill content.
/// Scans skills/*/SKILL.md body text outside excluded Markdown contexts.
pub fn validate_todo_in_skills(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let skills_dir = Path::new("skills");
    if !skills_dir.is_dir() {
        return;
    }

    for entry in traversal::shallow_directories(skills_dir, Path::new("."), None).entries {
        let path = entry.path;
        let dir_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if dir_name == "shared" {
            continue;
        }

        let skill_path = format!("skills/{dir_name}/SKILL.md");
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

        let Some(hit) = find_first_unfinished_work_marker(&content) else {
            continue;
        };
        diag.report_at_with(
            LintRule::TodoInSkill,
            &skill_path,
            &format!(
                "skills/{dir_name}/SKILL.md contains {} marker; remove before publishing",
                hit.marker
            ),
            hit.metadata(),
        );
    }
}

/// G007: TODO/FIXME/HACK/XXX markers in agent .md files.
/// Scans agents/*.md body text outside excluded Markdown contexts.
pub fn validate_todo_in_agents(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let agents_dir = Path::new("agents");
    if !agents_dir.is_dir() {
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

        let Some(hit) = find_first_unfinished_work_marker(&content) else {
            continue;
        };
        diag.report_at_with(
            LintRule::TodoInAgent,
            &agent_path,
            &format!(
                "agents/{name} contains {} marker; remove before publishing",
                hit.marker
            ),
            hit.metadata(),
        );
    }
}
