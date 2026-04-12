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
        (
            "LARCH_SLACK_BOT_TOKEN",
            "CLAUDE_PLUGIN_OPTION_SLACK_BOT_TOKEN",
        ),
        (
            "LARCH_SLACK_CHANNEL_ID",
            "CLAUDE_PLUGIN_OPTION_SLACK_CHANNEL_ID",
        ),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::DiagnosticCollector;

    // V19: validate_slack_fallback_consistency
    #[test]
    #[serial_test::serial]
    fn test_v19_no_scripts_dir_silent() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_slack_fallback_consistency(&mut diag);
        assert_eq!(diag.error_count(), 0);

    }

    #[test]
    #[serial_test::serial]
    fn test_v19_fallback_without_plugin_var() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("scripts").unwrap();
        std::fs::write(
            "scripts/slack.sh",
            "#!/bin/bash\nTOKEN=${LARCH_SLACK_BOT_TOKEN:-default}\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_slack_fallback_consistency(&mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("CLAUDE_PLUGIN_OPTION_SLACK_BOT_TOKEN"));

    }

    #[test]
    #[serial_test::serial]
    fn test_v19_fallback_with_plugin_var_pass() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("scripts").unwrap();
        std::fs::write(
            "scripts/slack.sh",
            "#!/bin/bash\nTOKEN=${LARCH_SLACK_BOT_TOKEN:-$CLAUDE_PLUGIN_OPTION_SLACK_BOT_TOKEN}\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_slack_fallback_consistency(&mut diag);
        assert_eq!(diag.error_count(), 0);

    }

    #[test]
    #[serial_test::serial]
    fn test_v19_multiple_vars_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("scripts").unwrap();
        std::fs::write(
            "scripts/slack.sh",
            "#!/bin/bash\nTOKEN=${LARCH_SLACK_BOT_TOKEN:-x}\nCH=${LARCH_SLACK_CHANNEL_ID:-y}\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_slack_fallback_consistency(&mut diag);
        assert_eq!(diag.error_count(), 2);

    }
}
