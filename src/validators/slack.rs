use crate::diagnostic::DiagnosticCollector;
use std::fs;
use std::path::Path;

/// V19: Slack fallback consistency (larch-specific convention check).
/// For each scripts/*.sh that does a bash fallback read of LARCH_SLACK_BOT_TOKEN,
/// LARCH_SLACK_CHANNEL_ID, or LARCH_SLACK_USER_ID, verify it also references
/// the corresponding CLAUDE_PLUGIN_OPTION_* variable.
pub fn validate_slack_fallback_consistency(diag: &mut DiagnosticCollector) {
    let scripts_dir = Path::new("scripts");
    if !scripts_dir.is_dir() {
        return;
    }

    let vars = [
        ("LARCH_SLACK_BOT_TOKEN", "CLAUDE_PLUGIN_OPTION_SLACK_BOT_TOKEN"),
        ("LARCH_SLACK_CHANNEL_ID", "CLAUDE_PLUGIN_OPTION_SLACK_CHANNEL_ID"),
        ("LARCH_SLACK_USER_ID", "CLAUDE_PLUGIN_OPTION_SLACK_USER_ID"),
    ];

    let entries = match fs::read_dir(scripts_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) if n.ends_with(".sh") => n.to_string(),
            _ => continue,
        };

        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        for (larch_var, plugin_var) in &vars {
            // Check for ${VAR:- pattern (bash fallback read)
            let fallback_pattern = format!("${{{larch_var}:-");
            if content.contains(&fallback_pattern) && !content.contains(plugin_var) {
                diag.fail(&format!(
                    "scripts/{name} reads ${{{larch_var}:-...}} but does not reference {plugin_var}"
                ));
            }
        }
    }
}
