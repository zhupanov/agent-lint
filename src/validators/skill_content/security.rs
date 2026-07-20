use crate::diagnostic::DiagnosticCollector;
use crate::rules::LintRule;
use crate::sensitive::contains_possible_secret;
use crate::validators::skills::SkillInfo;
use regex::Regex;
use std::sync::LazyLock;

// S031: Non-HTTPS URLs
static RE_HTTP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"http://[a-zA-Z0-9]").unwrap());

pub(super) fn check_content_security(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    if info.body.trim().is_empty() {
        return;
    }

    // S031: non-HTTPS URLs (exclude localhost, 127.0.0.1, 0.0.0.0, example.com/org)
    for cap in RE_HTTP.find_iter(&info.body) {
        let start = cap.start();
        let after = &info.body[start + 7..]; // skip "http://"
        if after.starts_with("localhost")
            || after.starts_with("127.0.0.1")
            || after.starts_with("0.0.0.0")
            || after.starts_with("example.com")
            || after.starts_with("example.org")
        {
            continue;
        }
        diag.report(
            LintRule::NonHttpsUrl,
            &format!(
                "{}: non-HTTPS URL found; use https:// for security",
                info.path
            ),
        );
        break; // Report once per file
    }

    // S032: hardcoded secrets
    if contains_possible_secret(&info.body) {
        diag.report(
            LintRule::HardcodedSecret,
            &format!("{}: potential hardcoded secret/API key detected", info.path),
        );
    }
}
