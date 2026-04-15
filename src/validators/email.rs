use crate::context::{LintContext, ManifestState};
use crate::diagnostic::DiagnosticCollector;
use crate::rules::LintRule;
use regex::Regex;
use std::sync::LazyLock;

static RE_EMAIL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^.+@.+\..+$").unwrap());

/// V17: Email format validation.
pub fn validate_email_format(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    if let ManifestState::Parsed(val) = &ctx.marketplace_json {
        let email_val = val.get("owner").and_then(|o| o.get("email"));
        if let Some(v) = email_val {
            match v.as_str() {
                Some(s) if !s.is_empty() && !RE_EMAIL.is_match(s) => {
                    diag.report(
                        LintRule::InvalidEmailFormat,
                        &format!(".claude-plugin/marketplace.json owner.email is not a valid email format: {s}"),
                    );
                }
                Some(_) => {} // empty string or valid: skip
                None => {
                    diag.report(
                        LintRule::InvalidEmailFormat,
                        ".claude-plugin/marketplace.json owner.email is not a string",
                    );
                }
            }
        }
    }

    if let ManifestState::Parsed(val) = &ctx.plugin_json {
        let email_val = val.get("author").and_then(|o| o.get("email"));
        if let Some(v) = email_val {
            match v.as_str() {
                Some(s) if !s.is_empty() && !RE_EMAIL.is_match(s) => {
                    diag.report(
                        LintRule::InvalidEmailFormat,
                        &format!(
                            ".claude-plugin/plugin.json author.email is not a valid email format: {s}"
                        ),
                    );
                }
                Some(_) => {} // empty string or valid: skip
                None => {
                    diag.report(
                        LintRule::InvalidEmailFormat,
                        ".claude-plugin/plugin.json author.email is not a string",
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::LintMode;
    use serde_json::json;

    fn make_ctx(plugin: ManifestState, marketplace: ManifestState) -> LintContext {
        LintContext {
            base_path: std::path::PathBuf::new(),
            mode: LintMode::Plugin,
            plugin_json: plugin,
            marketplace_json: marketplace,
            hooks_json: ManifestState::Missing,
            settings_json: ManifestState::Missing,
        }
    }

    #[test]
    fn test_v17_valid_emails() {
        let ctx = make_ctx(
            ManifestState::Parsed(json!({"author": {"email": "user@example.com"}})),
            ManifestState::Parsed(json!({"owner": {"email": "admin@test.org"}})),
        );
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_email_format(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_v17_invalid_marketplace_email() {
        let ctx = make_ctx(
            ManifestState::Missing,
            ManifestState::Parsed(json!({"owner": {"email": "not-an-email"}})),
        );
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_email_format(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("marketplace.json"));
    }

    #[test]
    fn test_v17_invalid_plugin_email() {
        let ctx = make_ctx(
            ManifestState::Parsed(json!({"author": {"email": "bad"}})),
            ManifestState::Missing,
        );
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_email_format(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("plugin.json"));
    }

    #[test]
    fn test_v17_empty_email_no_error() {
        let ctx = make_ctx(
            ManifestState::Parsed(json!({"author": {"email": ""}})),
            ManifestState::Parsed(json!({"owner": {"email": ""}})),
        );
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_email_format(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_v17_non_string_marketplace_email() {
        let ctx = make_ctx(
            ManifestState::Missing,
            ManifestState::Parsed(json!({"owner": {"email": 42}})),
        );
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_email_format(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("not a string"));
    }

    #[test]
    fn test_v17_non_string_plugin_email() {
        let ctx = make_ctx(
            ManifestState::Parsed(json!({"author": {"email": true}})),
            ManifestState::Missing,
        );
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_email_format(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("not a string"));
    }

    #[test]
    fn test_v17_null_email_reported() {
        let ctx = make_ctx(
            ManifestState::Parsed(json!({"author": {"email": null}})),
            ManifestState::Parsed(json!({"owner": {"email": null}})),
        );
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_email_format(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 2);
    }

    #[test]
    fn test_v17_array_email_reported() {
        let ctx = make_ctx(
            ManifestState::Parsed(json!({"author": {"email": ["a@b.com"]}})),
            ManifestState::Missing,
        );
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_email_format(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("not a string"));
    }

    #[test]
    fn test_v17_skips_when_not_parsed() {
        let ctx = make_ctx(ManifestState::Missing, ManifestState::Missing);
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_email_format(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }
}
