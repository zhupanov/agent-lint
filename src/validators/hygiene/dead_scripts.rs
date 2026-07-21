use crate::config::ExcludeSet;
use crate::context::{LintContext, LintMode, ManifestState};
use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata};
use crate::hook_commands::extract_hook_command_paths;
use crate::rules::LintRule;
use crate::traversal;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::scripts::collect_references;

/// G004 is deliberately a warning: lexical reachability cannot prove that a
/// dynamically dispatched script is dead. Per-file suppression and the
/// existing script inventory remain the escape hatches for intentional files.
pub fn validate_dead_scripts(
    ctx: &LintContext,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
) {
    let mut references = BTreeSet::new();
    for (source, reference) in collect_references(LintMode::Plugin, exclude) {
        if reference.path.as_os_str().is_empty() {
            continue;
        }
        // A script mentioning its own name is documentation, not reachability.
        if Path::new(&source) == reference.path {
            continue;
        }
        references.insert(reference.path);
    }
    for manifest in std::iter::once(&ctx.hooks_json)
        .chain(std::iter::once(&ctx.settings_json))
        .chain(ctx.declared_hook_configs.iter().map(|config| &config.state))
    {
        if let ManifestState::Parsed(value) = manifest {
            references.extend(
                extract_hook_command_paths(value)
                    .into_iter()
                    .map(|reference| reference.path),
            );
        }
    }
    // Claude settings permissions are executable policy, not descriptive
    // prose. A listed repository script is therefore a supported live source.
    if let ManifestState::Parsed(settings) = &ctx.settings_json
        && let Some(allow) = settings
            .pointer("/permissions/allow")
            .and_then(|value| value.as_array())
    {
        for value in allow {
            if let Some(path) = value
                .as_str()
                .and_then(crate::script_paths::normalize_repository_path)
            {
                references.insert(path);
            }
        }
    }

    let scripts = Path::new("scripts");
    if !scripts.is_dir() {
        return;
    }
    for entry in traversal::recursive_files(scripts, Path::new("."), Some(exclude)).entries {
        let path: PathBuf = entry.path;
        let display = path.display().to_string();
        if exclude.is_excluded(&display) || !path.is_file() || references.contains(&path) {
            continue;
        }
        diag.report_at_with(
            LintRule::DeadScript,
            &path,
            &format!("dead script (no executable invocation reference): {display}"),
            DiagnosticMetadata::default()
                .with_evidence("comments, prose, strings, and self-references are not reachability")
                .with_suggestion("add an executable invocation or suppress G004 for intentional inventory entries"),
        );
    }
}
