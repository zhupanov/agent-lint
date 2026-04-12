use crate::context::{LintContext, ManifestState};
use crate::diagnostic::DiagnosticCollector;
use regex::Regex;

/// V17: Email format validation.
/// marketplace.json owner.email and plugin.json author.email must match
/// basic email regex (.+@.+\..+).
pub fn validate_email_format(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    let email_re = Regex::new(r"^.+@.+\..+$").unwrap();

    // marketplace.json owner.email
    if let ManifestState::Parsed(val) = &ctx.marketplace_json {
        let email = val
            .get("owner")
            .and_then(|o| o.get("email"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !email.is_empty() && !email_re.is_match(email) {
            diag.fail(&format!(
                ".claude-plugin/marketplace.json owner.email is not a valid email format: {email}"
            ));
        }
    }

    // plugin.json author.email
    if let ManifestState::Parsed(val) = &ctx.plugin_json {
        let email = val
            .get("author")
            .and_then(|o| o.get("email"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !email.is_empty() && !email_re.is_match(email) {
            diag.fail(&format!(
                ".claude-plugin/plugin.json author.email is not a valid email format: {email}"
            ));
        }
    }
}
