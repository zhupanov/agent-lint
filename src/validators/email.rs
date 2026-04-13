use crate::context::{LintContext, ManifestState};
use crate::diagnostic::DiagnosticCollector;
use crate::rules::LintRule;
use regex::Regex;

/// V17: Email format validation.
pub fn validate_email_format(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    let email_re = Regex::new(r"^.+@.+\..+$").unwrap();

    if let ManifestState::Parsed(val) = &ctx.marketplace_json {
        let email = val
            .get("owner")
            .and_then(|o| o.get("email"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !email.is_empty() && !email_re.is_match(email) {
            diag.report(
                LintRule::InvalidEmailFormat,
                &format!(".claude-plugin/marketplace.json owner.email is not a valid email format: {email}"),
            );
        }
    }

    if let ManifestState::Parsed(val) = &ctx.plugin_json {
        let email = val
            .get("author")
            .and_then(|o| o.get("email"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !email.is_empty() && !email_re.is_match(email) {
            diag.report(
                LintRule::InvalidEmailFormat,
                &format!(
                    ".claude-plugin/plugin.json author.email is not a valid email format: {email}"
                ),
            );
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
            repo_root: String::new(),
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
        let mut diag = DiagnosticCollector::new();
        validate_email_format(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_v17_invalid_marketplace_email() {
        let ctx = make_ctx(
            ManifestState::Missing,
            ManifestState::Parsed(json!({"owner": {"email": "not-an-email"}})),
        );
        let mut diag = DiagnosticCollector::new();
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
        let mut diag = DiagnosticCollector::new();
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
        let mut diag = DiagnosticCollector::new();
        validate_email_format(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_v17_skips_when_not_parsed() {
        let ctx = make_ctx(ManifestState::Missing, ManifestState::Missing);
        let mut diag = DiagnosticCollector::new();
        validate_email_format(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }
}
