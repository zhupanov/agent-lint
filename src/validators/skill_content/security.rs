use crate::diagnostic::DiagnosticCollector;
use crate::rules::LintRule;
use crate::validators::skills::SkillInfo;
use regex::Regex;
use std::sync::LazyLock;

// S031: Non-HTTPS URLs
static RE_HTTP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"http://[a-zA-Z0-9]").unwrap());

// S032: Secret patterns
static SECRET_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        Regex::new(r"sk-[a-zA-Z0-9]{20,}").unwrap(),
        Regex::new(r"ghp_[a-zA-Z0-9]{36,}").unwrap(),
        Regex::new(r"xox[bp]-[0-9][a-zA-Z0-9\-]{8,}").unwrap(),
        Regex::new(
            r#"(?i)(api[_\-]?key|api[_\-]?secret|api[_\-]?token)\s*[=:]\s*["']?[A-Za-z0-9]{20,}"#,
        )
        .unwrap(),
        Regex::new(r#"(?i)(password|secret|token)\s*[=:]\s*["'][^"']{8,}"#).unwrap(),
    ]
});

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
    for re in SECRET_PATTERNS.iter() {
        if re.is_match(&info.body) {
            diag.report(
                LintRule::HardcodedSecret,
                &format!("{}: potential hardcoded secret/API key detected", info.path),
            );
            return; // Report once per file
        }
    }
}
