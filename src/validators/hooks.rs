use crate::context::{LintContext, ManifestState, collect_json_strings};
use crate::diagnostic::DiagnosticCollector;
use crate::rules::LintRule;
use crate::validators::hook_schema;
use regex::Regex;
use serde_json::Value;
use std::path::Path;
use std::sync::LazyLock;

static RE_PLUGIN_ROOT_SH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\$\{CLAUDE_PLUGIN_ROOT\}/[a-zA-Z0-9._/-]+\.sh").unwrap());
static RE_PWD_SH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\$PWD/[a-zA-Z0-9._/-]+\.sh").unwrap());

/// Validate hook command paths in a parsed JSON value.
/// Extracts script paths matching ${CLAUDE_PLUGIN_ROOT}/...sh or $PWD/...sh
/// from all string values, then verifies each resolved path exists and is executable.
fn validate_hook_command_paths(
    val: &Value,
    label: &str,
    missing_rule: LintRule,
    not_exec_rule: LintRule,
    diag: &mut DiagnosticCollector,
) {
    let strings = collect_json_strings(val);
    for raw in &strings {
        // Extract script paths from the string using regex (handles commands with arguments)
        for cap in RE_PLUGIN_ROOT_SH.find_iter(raw) {
            let reference = cap.as_str();
            let rel = reference.replacen("${CLAUDE_PLUGIN_ROOT}/", "", 1);
            check_hook_path(&rel, reference, label, missing_rule, not_exec_rule, diag);
        }
        for cap in RE_PWD_SH.find_iter(raw) {
            let reference = cap.as_str();
            let rel = reference.replacen("$PWD/", "", 1);
            check_hook_path(&rel, reference, label, missing_rule, not_exec_rule, diag);
        }
    }
}

fn check_hook_path(
    rel: &str,
    reference: &str,
    label: &str,
    missing_rule: LintRule,
    not_exec_rule: LintRule,
    diag: &mut DiagnosticCollector,
) {
    let path = Path::new(rel);
    if !path.is_file() {
        diag.report(
            missing_rule,
            &format!("{label}: hook command missing on disk: {reference}"),
        );
        return;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = path.metadata() {
            if meta.permissions().mode() & 0o111 == 0 {
                diag.report(
                    not_exec_rule,
                    &format!("{label}: hook command not executable: {reference}"),
                );
            }
        }
    }
}

/// V3: Validate hooks/hooks.json
pub fn validate_hooks_json(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    let f = "hooks/hooks.json";
    let val = match &ctx.hooks_json {
        ManifestState::Missing => {
            diag.report(LintRule::HooksJsonMissing, &format!("{f} is missing"));
            return;
        }
        ManifestState::Invalid(e) => {
            diag.report(LintRule::HooksJsonInvalid, e);
            return;
        }
        ManifestState::Parsed(v) => v,
    };

    match val.get("hooks") {
        None => diag.report(
            LintRule::HooksKeyMissing,
            &format!("{f} missing top-level 'hooks' key"),
        ),
        // Plugin hooks use an event-keyed object. Retain the legacy flat-array
        // check so existing configurations continue to receive H007 as well.
        Some(hooks)
            if hooks.as_object().is_some_and(|events| events.is_empty())
                || hooks.as_array().is_some_and(|entries| entries.is_empty()) =>
        {
            diag.report(
                LintRule::HooksArrayEmpty,
                &format!("{f} has empty 'hooks' collection"),
            );
        }
        Some(_) => {}
    }

    validate_hook_command_paths(
        val,
        f,
        LintRule::HookCommandMissing,
        LintRule::HookNotExecutable,
        diag,
    );
}

/// V4: Validate .claude/settings.json hook command paths
pub fn validate_settings_hooks(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    let val = match &ctx.settings_json {
        ManifestState::Missing => return, // Optional file
        ManifestState::Invalid(e) => {
            diag.report(LintRule::SettingsJsonInvalid, e);
            return;
        }
        ManifestState::Parsed(v) => v,
    };

    validate_hook_command_paths(
        val,
        ".claude/settings.json",
        LintRule::HookCommandMissing,
        LintRule::HookNotExecutable,
        diag,
    );
}

/// V26: Validate the hook object schema in hooks/hooks.json (H008-H024).
pub fn validate_hooks_json_schema(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    // H001/H002 already report a missing or unparseable hooks.json.
    if let ManifestState::Parsed(val) = &ctx.hooks_json {
        hook_schema::validate_hook_schema(val, "hooks/hooks.json", diag);
    }
}

/// V27: Validate the hook object schema in .claude/settings.json (H008-H024).
pub fn validate_settings_schema(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    // H006 already reports an unparseable settings.json.
    if let ManifestState::Parsed(val) = &ctx.settings_json {
        hook_schema::validate_hook_schema(val, ".claude/settings.json", diag);
    }
}

/// V28: Validate .claude/settings.local.json (H025), hook command paths
/// (H004/H005), and hook object schema (H008-H024).
pub fn validate_settings_local(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    match &ctx.settings_local_json {
        ManifestState::Missing => {} // Optional file
        ManifestState::Invalid(e) => diag.report(LintRule::SettingsLocalInvalid, e),
        ManifestState::Parsed(val) => {
            validate_hook_command_paths(
                val,
                ".claude/settings.local.json",
                LintRule::HookCommandMissing,
                LintRule::HookNotExecutable,
                diag,
            );
            hook_schema::validate_hook_schema(val, ".claude/settings.local.json", diag);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::LintMode;
    use serde_json::json;

    fn make_ctx(hooks: ManifestState, settings: ManifestState) -> LintContext {
        LintContext {
            base_path: std::path::PathBuf::new(),
            mode: LintMode::Plugin,
            plugin_json: ManifestState::Missing,
            marketplace_json: ManifestState::Missing,
            hooks_json: hooks,
            settings_json: settings,
            settings_local_json: ManifestState::Missing,
        }
    }

    // V3: validate_hooks_json
    #[test]
    fn test_v3_valid_hooks_json() {
        let val = json!({
            "hooks": {
                "PreToolUse": [{
                    "matcher": "Bash",
                    "hooks": [{"type": "command", "command": "echo test"}]
                }]
            }
        });
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_hooks_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_v3_missing_hooks_json() {
        let ctx = make_ctx(ManifestState::Missing, ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_hooks_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("is missing"));
    }

    #[test]
    fn test_v3_invalid_hooks_json() {
        let ctx = make_ctx(
            ManifestState::Invalid("bad json".to_string()),
            ManifestState::Missing,
        );
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_hooks_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("bad json"));
    }

    #[test]
    fn test_v3_missing_hooks_key() {
        let val = json!({"other": "stuff"});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_hooks_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("hooks"));
    }

    #[test]
    fn test_v3_empty_hooks_array() {
        let val = json!({"hooks": []});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_hooks_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert_eq!(diag.diagnostics()[0].rule, LintRule::HooksArrayEmpty);
        assert!(diag.errors()[0].contains("empty"));
    }

    #[test]
    fn test_v3_empty_event_keyed_hooks_object() {
        let val = json!({"hooks": {}});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_hooks_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert_eq!(diag.diagnostics()[0].rule, LintRule::HooksArrayEmpty);
        assert!(diag.errors()[0].contains("empty"));
    }

    // V4: validate_settings_hooks
    #[test]
    fn test_v4_missing_settings_silent_pass() {
        let ctx = make_ctx(ManifestState::Missing, ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_settings_hooks(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_v4_invalid_settings() {
        let ctx = make_ctx(
            ManifestState::Missing,
            ManifestState::Invalid("bad settings".to_string()),
        );
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_settings_hooks(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("bad settings"));
    }

    #[test]
    fn test_v4_valid_settings_no_hooks() {
        let val = json!({"permissions": {}});
        let ctx = make_ctx(ManifestState::Missing, ManifestState::Parsed(val));
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_settings_hooks(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    // Hook command path validation with fixtures
    #[test]
    #[serial_test::serial]
    fn test_hook_command_path_missing_script() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        let val = json!({
            "hooks": [{"command": "${CLAUDE_PLUGIN_ROOT}/scripts/nonexistent.sh"}]
        });
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_hook_command_paths(
            &val,
            "test",
            LintRule::HookCommandMissing,
            LintRule::HookNotExecutable,
            &mut diag,
        );
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("missing on disk"));
    }

    #[test]
    #[serial_test::serial]
    fn test_hook_command_path_existing_script() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("scripts").unwrap();
        let script = tmp.path().join("scripts/test.sh");
        std::fs::write(&script, "#!/bin/bash\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let val = json!({
            "hooks": [{"command": "${CLAUDE_PLUGIN_ROOT}/scripts/test.sh arg1"}]
        });
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_hook_command_paths(
            &val,
            "test",
            LintRule::HookCommandMissing,
            LintRule::HookNotExecutable,
            &mut diag,
        );
        assert_eq!(diag.error_count(), 0);
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn test_hook_command_path_not_executable() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("scripts").unwrap();
        let script = tmp.path().join("scripts/noexec.sh");
        std::fs::write(&script, "#!/bin/bash\n").unwrap();
        // Explicitly set non-executable
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o644)).unwrap();

        let val = json!({
            "hooks": [{"command": "${CLAUDE_PLUGIN_ROOT}/scripts/noexec.sh"}]
        });
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_hook_command_paths(
            &val,
            "test",
            LintRule::HookCommandMissing,
            LintRule::HookNotExecutable,
            &mut diag,
        );
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("not executable"));
    }

    // ── Hook schema surfaces (H008-H025) ────────────────────────────
    //
    // Engine behavior is covered in hook_schema.rs; these verify that each
    // surface is wired to the engine and labels its diagnostics correctly.

    /// An event-keyed config whose only hook object is missing `type` (H010).
    fn schema_violation() -> Value {
        json!({"hooks": {"PreToolUse": [{"hooks": [{"command": "echo hi"}]}]}})
    }

    #[test]
    fn test_v26_hooks_json_schema_surface() {
        let ctx = make_ctx(
            ManifestState::Parsed(schema_violation()),
            ManifestState::Missing,
        );
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_hooks_json_schema(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("hooks/hooks.json"));
        assert!(diag.errors()[0].contains("'type'"));
    }

    #[test]
    fn test_v26_legacy_array_hooks_json_skipped() {
        // The shape H001-H007 model: no event context, so the engine skips it.
        let val = json!({"hooks": [{"command": "echo test"}]});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_hooks_json_schema(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_v26_unparseable_hooks_json_is_silent() {
        // H002 owns that report; the schema engine must not double-report.
        let ctx = make_ctx(
            ManifestState::Invalid("bad".to_string()),
            ManifestState::Missing,
        );
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_hooks_json_schema(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_v27_settings_json_schema_surface() {
        let ctx = make_ctx(
            ManifestState::Missing,
            ManifestState::Parsed(schema_violation()),
        );
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_settings_schema(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains(".claude/settings.json"));
    }

    #[test]
    fn test_v27_settings_json_without_hooks_passes() {
        let val = json!({"permissions": {"allow": []}});
        let ctx = make_ctx(ManifestState::Missing, ManifestState::Parsed(val));
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_settings_schema(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_v28_settings_local_schema_surface() {
        let mut ctx = make_ctx(ManifestState::Missing, ManifestState::Missing);
        ctx.settings_local_json = ManifestState::Parsed(schema_violation());
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_settings_local(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains(".claude/settings.local.json"));
    }

    #[test]
    #[serial_test::serial]
    fn test_v28_settings_local_checks_hook_command_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        let mut ctx = make_ctx(ManifestState::Missing, ManifestState::Missing);
        ctx.settings_local_json = ManifestState::Parsed(json!({
            "hooks": [{"command": "${CLAUDE_PLUGIN_ROOT}/scripts/nonexistent.sh"}]
        }));
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_settings_local(&ctx, &mut diag);

        assert_eq!(diag.error_count(), 1);
        assert_eq!(diag.diagnostics()[0].rule, LintRule::HookCommandMissing);
        assert!(diag.errors()[0].contains(".claude/settings.local.json"));
    }

    #[test]
    fn test_v28_settings_local_invalid_fires_h025() {
        let mut ctx = make_ctx(ManifestState::Missing, ManifestState::Missing);
        ctx.settings_local_json = ManifestState::Invalid("bad local settings".to_string());
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_settings_local(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("bad local settings"));
        assert_eq!(
            diag.diagnostics()[0].rule,
            LintRule::SettingsLocalInvalid,
            "must use the new H025 code, not H006"
        );
    }

    #[test]
    fn test_v28_settings_local_missing_silent_pass() {
        let ctx = make_ctx(ManifestState::Missing, ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_settings_local(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }
}
