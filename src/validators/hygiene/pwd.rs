use crate::config::ExcludeSet;
use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata, SourceSpan};
use crate::pwd_hygiene::{PathIssueKind, find_path_issues};
use crate::rules::LintRule;
use crate::traversal;
use std::fs;
use std::path::Path;

/// Validate public skill paths without guessing their owning runtime root.
pub fn validate_pwd_hygiene(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let skills_dir = Path::new("skills");
    if !skills_dir.is_dir() {
        return;
    }

    for entry in traversal::shallow_directories(skills_dir, Path::new("."), None).entries {
        let path = entry.path;
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

        for issue in find_path_issues(&content) {
            let reference = &content[issue.range.clone()];
            let (rule, message, suggestion) = match issue.kind {
                PathIssueKind::BundledAsset => (
                    LintRule::PwdInSkill,
                    format!(
                        "skills/{name}/SKILL.md uses {reference} for a bundled plugin asset; use ${{CLAUDE_PLUGIN_ROOT}}/ instead"
                    ),
                    "Replace the $PWD prefix with ${CLAUDE_PLUGIN_ROOT} for this bundled asset.",
                ),
                PathIssueKind::HardcodedMachinePath => (
                    LintRule::HardcodedMachinePath,
                    format!(
                        "skills/{name}/SKILL.md uses machine-specific or ambiguous path {reference}"
                    ),
                    "Choose ${CLAUDE_PLUGIN_ROOT} for a bundled asset, ${CLAUDE_PROJECT_DIR} for a project file, or ${CLAUDE_PLUGIN_DATA} for persistent state.",
                ),
            };
            let metadata = SourceSpan::from_byte_range(&content, issue.range).map_or_else(
                DiagnosticMetadata::default,
                |location| {
                    DiagnosticMetadata::default()
                        .with_location(location)
                        .with_evidence(reference)
                        .with_suggestion(suggestion)
                },
            );
            diag.report_at_with(rule, &skill_path, &message, metadata);
        }
    }
}
