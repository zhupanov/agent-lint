use crate::context::{
    DeclaredHookConfig, DeclaredHookConfigKind, LintContext, ManifestState, ParsedManifest,
};
use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata, SourceSpan};
use crate::hook_commands::extract_hook_command_paths;
use crate::plugin_paths::{has_normalized_path_segment, is_absolute_path, path_segments};
use crate::rules::LintRule;
use crate::script_paths::Invocation;
use crate::validators::{common::manifest_error_metadata, hook_schema};
use serde_json::Value;
use std::path::Path;

/// Validate hook command paths in a parsed JSON value.
/// Verifies invoked hook script paths. Data references are deliberately
/// ignored; an interpreter or sourced script must exist but is not direct.
fn validate_hook_command_paths(
    val: &Value,
    label: &str,
    missing_rule: LintRule,
    not_exec_rule: LintRule,
    diag: &mut DiagnosticCollector,
) {
    for reference in extract_hook_command_paths(val) {
        if reference.invocation == Invocation::Mention {
            continue;
        }
        check_hook_path(
            &reference.path,
            &reference.reference,
            reference.invocation,
            label,
            missing_rule,
            not_exec_rule,
            diag,
        );
    }
}

fn check_hook_path(
    path: &Path,
    reference: &str,
    invocation: Invocation,
    label: &str,
    missing_rule: LintRule,
    not_exec_rule: LintRule,
    diag: &mut DiagnosticCollector,
) {
    if path.as_os_str().is_empty() {
        diag.report_with(
            missing_rule,
            &format!("{label}: hook command escapes the repository: {reference}"),
            DiagnosticMetadata::default()
                .with_evidence(reference)
                .with_suggestion("use an in-repository normalized path such as ${CLAUDE_PLUGIN_ROOT}/scripts/your-hook"),
        );
        return;
    }
    if !path.is_file() {
        diag.report_at_with(
            missing_rule,
            path,
            &format!("{label}: hook command missing on disk: {reference}"),
            DiagnosticMetadata::default().with_evidence(reference),
        );
        return;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if invocation == Invocation::Direct
            && let Ok(meta) = path.metadata()
            && meta.permissions().mode() & 0o111 == 0
        {
            diag.report_at_with(
                not_exec_rule,
                path,
                &format!("{label}: hook command not executable: {reference}"),
                DiagnosticMetadata::default()
                    .with_evidence(reference)
                    .with_suggestion("run chmod +x on this file"),
            );
        }
    }
}

/// V3: Validate every plugin hook configuration surface.
pub fn validate_hooks_json(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    // The conventional default is optional. A missing declared surface, on the
    // other hand, is an unresolvable manifest declaration and owns H001.
    validate_hook_config(
        &ctx.hooks_json,
        Path::new("hooks/hooks.json"),
        false,
        false,
        diag,
    );
    for config in &ctx.declared_hook_configs {
        validate_declared_hook_config(config, diag);
    }
    validate_plugin_hook_declarations(ctx, diag);
}

fn validate_declared_hook_config(config: &DeclaredHookConfig, diag: &mut DiagnosticCollector) {
    validate_hook_config(
        &config.state,
        &config.subject_path,
        true,
        config.kind == DeclaredHookConfigKind::Inline,
        diag,
    );
}

fn validate_hook_config(
    state: &ManifestState,
    subject_path: &Path,
    declared: bool,
    inline: bool,
    diag: &mut DiagnosticCollector,
) {
    let f = subject_path.display().to_string();
    let val = match state {
        ManifestState::Missing => {
            if declared {
                diag.report_at(
                    LintRule::HooksJsonMissing,
                    subject_path,
                    &format!("{f} is missing"),
                );
            }
            return;
        }
        ManifestState::Invalid(e) => {
            diag.report_at_with(
                LintRule::HooksJsonInvalid,
                subject_path,
                e.message(),
                manifest_error_metadata(e),
            );
            return;
        }
        ManifestState::Parsed(v) => v,
    };

    match val.get("hooks") {
        None if !inline => diag.report_at(
            LintRule::HooksKeyMissing,
            subject_path,
            &format!("{f} missing top-level 'hooks' key"),
        ),
        Some(_) | None => {}
    }

    diag.with_subject_path(subject_path, |diag| {
        validate_hook_command_paths(
            val,
            &f,
            LintRule::HookCommandMissing,
            LintRule::HookNotExecutable,
            diag,
        );
    });
}

fn json_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// H026 validates the declaration form before discovery. Invalid values are
/// intentionally never converted to missing hook configurations: H001 is
/// reserved for a valid declared path whose file cannot be found.
fn validate_plugin_hook_declarations(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    let ManifestState::Parsed(plugin) = &ctx.plugin_json else {
        return;
    };
    let Some(hooks) = plugin.get("hooks") else {
        return;
    };

    match hooks {
        Value::String(path) => {
            validate_hook_declaration_path(path, "hooks field", hooks, plugin, diag)
        }
        Value::Array(paths) => {
            for (index, value) in paths.iter().enumerate() {
                let label = format!("hooks array entry {}", index + 1);
                match value {
                    Value::String(path) => {
                        validate_hook_declaration_path(path, &label, value, plugin, diag)
                    }
                    value => report_plugin_declaration_malformed(
                        &format!("{label} must be a string (found {})", json_type(value)),
                        &label,
                        value,
                        plugin,
                        diag,
                    ),
                }
            }
        }
        Value::Object(_) => {}
        value => report_plugin_declaration_malformed(
            &format!(
                "hooks field must be a string, array, or object (found {})",
                json_type(value)
            ),
            "hooks field",
            value,
            plugin,
            diag,
        ),
    }
}

fn validate_hook_declaration_path(
    path: &str,
    label: &str,
    value: &Value,
    plugin: &ParsedManifest,
    diag: &mut DiagnosticCollector,
) {
    // Empty and dot-only declarations normalize to no path. Absolute and
    // traversal declarations remain exclusively M013-owned.
    if !is_absolute_path(path)
        && !path_segments(path).any(|segment| segment == "..")
        && !has_normalized_path_segment(path)
    {
        report_plugin_declaration_malformed(
            &format!("{label} must contain at least one normalized path segment"),
            label,
            value,
            plugin,
            diag,
        );
    }
}

fn report_plugin_declaration_malformed(
    detail: &str,
    label: &str,
    value: &Value,
    plugin: &ParsedManifest,
    diag: &mut DiagnosticCollector,
) {
    let mut metadata = DiagnosticMetadata::default().with_evidence(label);
    if let Some(location) = plugin
        .source()
        .and_then(|source| json_value_range(source, value))
        .and_then(|range| {
            plugin
                .source()
                .and_then(|source| SourceSpan::from_byte_range(source, range))
        })
    {
        metadata = metadata.with_location(location);
    }
    diag.report_at_with(
        LintRule::HookConfigMalformed,
        ".claude-plugin/plugin.json",
        &format!(".claude-plugin/plugin.json {detail}"),
        metadata,
    );
}

fn json_value_range(source: &str, value: &Value) -> Option<std::ops::Range<usize>> {
    let token = serde_json::to_string(value).expect("JSON values always serialize");
    source.find(&token).map(|start| start..start + token.len())
}

/// V4: Validate .claude/settings.json hook command paths
pub fn validate_settings_hooks(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    let val = match &ctx.settings_json {
        ManifestState::Missing => return, // Optional file
        ManifestState::Invalid(e) => {
            diag.report_at_with(
                LintRule::SettingsJsonInvalid,
                ".claude/settings.json",
                e.message(),
                manifest_error_metadata(e),
            );
            return;
        }
        ManifestState::Parsed(v) => v,
    };

    diag.with_subject_path(".claude/settings.json", |diag| {
        validate_hook_command_paths(
            val,
            ".claude/settings.json",
            LintRule::HookCommandMissing,
            LintRule::HookNotExecutable,
            diag,
        );
    });
}

/// V26: Validate hook object schemas in every plugin hook configuration
/// surface (H007-H026).
pub fn validate_hooks_json_schema(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    // H001/H002 already report missing or unparseable configurations.
    if let ManifestState::Parsed(val) = &ctx.hooks_json {
        diag.with_subject_path("hooks/hooks.json", |diag| {
            let result = hook_schema::validate_hook_schema(val, "hooks/hooks.json", true, diag);
            report_effectively_empty_plugin_config(result, "hooks/hooks.json", diag);
        });
    }
    for config in &ctx.declared_hook_configs {
        if let ManifestState::Parsed(val) = &config.state {
            let label = config.subject_path.display().to_string();
            diag.with_subject_path(&config.subject_path, |diag| {
                let result = hook_schema::validate_hook_schema(val, &label, true, diag);
                report_effectively_empty_plugin_config(result, &label, diag);
            });
        }
    }
}

fn report_effectively_empty_plugin_config(
    result: hook_schema::HookSchemaResult,
    label: &str,
    diag: &mut DiagnosticCollector,
) {
    if result.is_effectively_empty() {
        diag.report(
            LintRule::HooksArrayEmpty,
            &format!("{label} has no hook handler entries"),
        );
    }
}

/// V27: Validate the hook object schema in .claude/settings.json (H008-H026).
pub fn validate_settings_schema(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    // H006 already reports an unparseable settings.json.
    if let ManifestState::Parsed(val) = &ctx.settings_json {
        diag.with_subject_path(".claude/settings.json", |diag| {
            hook_schema::validate_hook_schema(val, ".claude/settings.json", false, diag);
        });
    }
}

/// V28: Validate .claude/settings.local.json (H025), hook command paths
/// (H004/H005), and hook object schema (H008-H026).
pub fn validate_settings_local(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    match &ctx.settings_local_json {
        ManifestState::Missing => {} // Optional file
        ManifestState::Invalid(e) => diag.report_at_with(
            LintRule::SettingsLocalInvalid,
            ".claude/settings.local.json",
            e.message(),
            manifest_error_metadata(e),
        ),
        ManifestState::Parsed(val) => {
            diag.with_subject_path(".claude/settings.local.json", |diag| {
                validate_hook_command_paths(
                    val,
                    ".claude/settings.local.json",
                    LintRule::HookCommandMissing,
                    LintRule::HookNotExecutable,
                    diag,
                );
            });
            diag.with_subject_path(".claude/settings.local.json", |diag| {
                hook_schema::validate_hook_schema(val, ".claude/settings.local.json", false, diag);
            });
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
            declared_hook_configs: vec![],
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
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_hooks_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_v3_missing_default_hooks_json_is_optional() {
        let ctx = make_ctx(ManifestState::Missing, ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_hooks_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_v3_invalid_hooks_json() {
        let ctx = make_ctx(ManifestState::invalid("bad json"), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_hooks_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("bad json"));
    }

    #[test]
    fn test_v3_missing_hooks_key() {
        let val = json!({"other": "stuff"});
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_hooks_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("hooks"));
    }

    #[test]
    fn test_v26_empty_legacy_hooks_array_is_h007() {
        let val = json!({"hooks": []});
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_hooks_json_schema(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert_eq!(diag.diagnostics()[0].rule, LintRule::HooksArrayEmpty);
        assert!(diag.errors()[0].contains("no hook handler entries"));
    }

    #[test]
    fn test_v26_empty_event_keyed_hooks_object_is_h007() {
        let val = json!({"hooks": {}});
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_hooks_json_schema(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert_eq!(diag.diagnostics()[0].rule, LintRule::HooksArrayEmpty);
        assert!(diag.errors()[0].contains("no hook handler entries"));
    }

    #[test]
    fn test_v26_canonical_empty_groups_are_h007_once() {
        let val = json!({"hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": []}]}});
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_hooks_json_schema(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert_eq!(diag.diagnostics()[0].rule, LintRule::HooksArrayEmpty);
    }

    #[test]
    fn test_v26_malformed_shape_does_not_cascade_to_h007() {
        let val = json!({"hooks": {"PreToolUse": ["not-an-object"]}});
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_hooks_json_schema(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert_eq!(diag.diagnostics()[0].rule, LintRule::HookConfigMalformed);
    }

    #[test]
    fn test_v26_non_collection_hooks_values_fire_h026_not_h007() {
        for value in [json!(null), json!("not-a-collection"), json!(42)] {
            let ctx = make_ctx(
                ManifestState::parsed(json!({"hooks": value})),
                ManifestState::Missing,
            );
            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_hooks_json_schema(&ctx, &mut diag);
            assert_eq!(diag.error_count(), 1);
            assert_eq!(diag.diagnostics()[0].rule, LintRule::HookConfigMalformed);
        }
    }

    #[test]
    fn plugin_declaration_malformations_use_h026_without_h001() {
        let mut ctx = make_ctx(ManifestState::Missing, ManifestState::Missing);
        ctx.plugin_json = ManifestState::parsed(json!({
            "name": "hooks-test",
            "hooks": ["", 42, null, false, {}, []]
        }));
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_hooks_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 6);
        assert!(
            diag.diagnostics()
                .iter()
                .all(|diagnostic| diagnostic.rule == LintRule::HookConfigMalformed)
        );
        assert!(diag.diagnostics().iter().all(|diagnostic| {
            diagnostic.subject_path.as_deref() == Some(Path::new(".claude-plugin/plugin.json"))
        }));
    }

    #[test]
    fn empty_plugin_declaration_array_is_valid_and_silent() {
        let mut ctx = make_ctx(ManifestState::Missing, ManifestState::Missing);
        ctx.plugin_json = ManifestState::parsed(json!({"name": "hooks-test", "hooks": []}));
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_hooks_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
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
            ManifestState::invalid("bad settings"),
        );
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_settings_hooks(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("bad settings"));
    }

    #[test]
    fn test_v4_valid_settings_no_hooks() {
        let val = json!({"permissions": {}});
        let ctx = make_ctx(ManifestState::Missing, ManifestState::parsed(val));
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
        assert_eq!(
            diag.diagnostics()[0].suggestion.as_deref(),
            Some("run chmod +x on this file")
        );
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn hook_command_paths_only_check_invoked_scripts_and_direct_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("scripts").unwrap();
        for path in [
            "scripts/direct.sh",
            "scripts/interpreted.py",
            "scripts/sourced.sh",
        ] {
            std::fs::write(path, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).unwrap();
        }

        let val = json!({"hooks": [{"command": "${CLAUDE_PLUGIN_ROOT}/scripts/direct.sh; python3 -u ${CLAUDE_PLUGIN_ROOT}/scripts/interpreted.py; source ${CLAUDE_PLUGIN_ROOT}/scripts/sourced.sh; echo ${CLAUDE_PLUGIN_ROOT}/generated/output.json; INPUT=${CLAUDE_PLUGIN_ROOT}/generated/runtime.json echo ok"}]});
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_hook_command_paths(
            &val,
            "test",
            LintRule::HookCommandMissing,
            LintRule::HookNotExecutable,
            &mut diag,
        );

        assert_eq!(diag.error_count(), 1);
        assert_eq!(diag.diagnostics()[0].rule, LintRule::HookNotExecutable);
        assert_eq!(
            diag.diagnostics()[0].subject_path.as_deref(),
            Some(Path::new("scripts/direct.sh"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn missing_invoked_paths_include_interpreters_sources_and_unsafe_direct_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        let val = json!({"hooks": [{"command": "${CLAUDE_PLUGIN_ROOT}/scripts/direct.sh; python3 ${CLAUDE_PLUGIN_ROOT}/scripts/interpreted.py; . ${CLAUDE_PLUGIN_ROOT}/scripts/sourced.sh; ${CLAUDE_PLUGIN_ROOT}/../outside"}]});
        let mut diag = DiagnosticCollector::new_all_enabled();
        diag.with_subject_path("hooks/hooks.json", |diag| {
            validate_hook_command_paths(
                &val,
                "hooks/hooks.json",
                LintRule::HookCommandMissing,
                LintRule::HookNotExecutable,
                diag,
            );
        });

        assert_eq!(diag.error_count(), 4);
        assert_eq!(
            diag.diagnostics()
                .iter()
                .map(|diagnostic| diagnostic.subject_path.as_deref())
                .collect::<Vec<_>>(),
            vec![
                Some(Path::new("scripts/direct.sh")),
                Some(Path::new("scripts/interpreted.py")),
                Some(Path::new("scripts/sourced.sh")),
                Some(Path::new("hooks/hooks.json")),
            ]
        );
        assert_eq!(
            diag.diagnostics()[3].suggestion.as_deref(),
            Some(
                "use an in-repository normalized path such as ${CLAUDE_PLUGIN_ROOT}/scripts/your-hook"
            )
        );
    }

    #[test]
    #[serial_test::serial]
    fn documented_command_paths_report_only_missing_or_non_executable_scripts() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("scripts").unwrap();
        std::fs::create_dir_all(".claude/hooks").unwrap();
        std::fs::create_dir_all("bin").unwrap();

        for path in ["scripts/check.py", "bin/check"] {
            std::fs::write(path, "#!/usr/bin/env sh\n").unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::fs::write(".claude/hooks/no-extension", "#!/usr/bin/env sh\n").unwrap();
        std::fs::set_permissions(
            ".claude/hooks/no-extension",
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();

        let val = json!({"hooks": {"PreToolUse": [{"hooks": [
            {"command": "${CLAUDE_PLUGIN_ROOT}/scripts/missing.py"},
            {"command": "\"${CLAUDE_PLUGIN_ROOT}\"/scripts/check.py"},
            {"command": "\"${CLAUDE_PROJECT_DIR}/.claude/hooks/no-extension\""},
            {"command": "$PWD/bin/check"}
        ]}]}});
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_hook_command_paths(
            &val,
            "test",
            LintRule::HookCommandMissing,
            LintRule::HookNotExecutable,
            &mut diag,
        );

        assert_eq!(diag.error_count(), 2);
        assert_eq!(
            diag.diagnostics()
                .iter()
                .map(|diagnostic| (diagnostic.rule, diagnostic.subject_path.as_deref()))
                .collect::<Vec<_>>(),
            vec![
                (
                    LintRule::HookCommandMissing,
                    Some(Path::new("scripts/missing.py")),
                ),
                (
                    LintRule::HookNotExecutable,
                    Some(Path::new(".claude/hooks/no-extension")),
                ),
            ]
        );
        assert_eq!(
            diag.diagnostics()[0].evidence.as_deref(),
            Some("${CLAUDE_PLUGIN_ROOT}/scripts/missing.py")
        );
    }

    #[test]
    fn prose_references_are_not_hook_command_paths() {
        let val = json!({
            "hooks": [{
                "command": "echo ok",
                "description": "Removed ${CLAUDE_PLUGIN_ROOT}/scripts/old-cleanup.sh"
            }]
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
            ManifestState::parsed(schema_violation()),
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
        let ctx = make_ctx(ManifestState::parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_hooks_json_schema(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_v26_unparseable_hooks_json_is_silent() {
        // H002 owns that report; the schema engine must not double-report.
        let ctx = make_ctx(ManifestState::invalid("bad"), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_hooks_json_schema(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_v27_settings_json_schema_surface() {
        let ctx = make_ctx(
            ManifestState::Missing,
            ManifestState::parsed(schema_violation()),
        );
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_settings_schema(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains(".claude/settings.json"));
    }

    #[test]
    fn test_v27_settings_json_without_hooks_passes() {
        let val = json!({"permissions": {"allow": []}});
        let ctx = make_ctx(ManifestState::Missing, ManifestState::parsed(val));
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_settings_schema(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_v27_settings_non_object_hooks_is_h026_but_empty_object_is_silent() {
        for hooks in [json!(null), json!("x"), json!([])] {
            let ctx = make_ctx(
                ManifestState::Missing,
                ManifestState::parsed(json!({"hooks": hooks})),
            );
            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_settings_schema(&ctx, &mut diag);
            assert_eq!(diag.error_count(), 1);
            assert_eq!(diag.diagnostics()[0].rule, LintRule::HookConfigMalformed);
        }

        let ctx = make_ctx(
            ManifestState::Missing,
            ManifestState::parsed(json!({"hooks": {}})),
        );
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_settings_schema(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_v28_settings_local_schema_surface() {
        let mut ctx = make_ctx(ManifestState::Missing, ManifestState::Missing);
        ctx.settings_local_json = ManifestState::parsed(schema_violation());
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
        ctx.settings_local_json = ManifestState::parsed(json!({
            "hooks": {"PreToolUse": [{"hooks": [{
                "type": "command",
                "command": "${CLAUDE_PLUGIN_ROOT}/scripts/nonexistent.sh"
            }]}]}
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
        ctx.settings_local_json = ManifestState::invalid("bad local settings");
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
