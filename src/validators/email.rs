use crate::context::{LintContext, ManifestState, ParsedManifest};
use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata, SourceSpan};
use crate::rules::LintRule;
use serde_json::Value;
use std::ops::Range;

/// V17: Validate present plugin and marketplace contact-email metadata.
///
/// This is intentionally a small ASCII quality convention, not RFC email or
/// mailbox validation. Missing fields belong to M010/M011; a present field is
/// reported by exactly one of E001 (string format) or E002 (JSON type).
pub fn validate_email_format(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    validate_email_field(
        &ctx.marketplace_json,
        ".claude-plugin/marketplace.json",
        "owner",
        "owner.email",
        diag,
    );
    validate_email_field(
        &ctx.plugin_json,
        ".claude-plugin/plugin.json",
        "author",
        "author.email",
        diag,
    );
}

fn validate_email_field(
    manifest: &ManifestState,
    subject_path: &str,
    parent_key: &str,
    field_name: &str,
    diag: &mut DiagnosticCollector,
) {
    let ManifestState::Parsed(manifest) = manifest else {
        return;
    };
    let Some(value) = manifest
        .get(parent_key)
        .and_then(|parent| parent.get("email"))
    else {
        return;
    };

    let metadata = email_metadata(manifest, parent_key);
    match value {
        Value::String(email) if is_contact_metadata_email(email) => {}
        Value::String(_) => diag.report_at_with(
            LintRule::InvalidEmailFormat,
            subject_path,
            &format!("{subject_path} {field_name} does not meet the contact-metadata format convention"),
            metadata.with_suggestion(
                "replace this field with an ASCII contact address matching the documented convention",
            ),
        ),
        _ => diag.report_at_with(
            LintRule::EmailTypeInvalid,
            subject_path,
            &format!("{subject_path} {field_name} must be a string"),
            metadata.with_suggestion("replace this field with a string contact address"),
        ),
    }
}

fn email_metadata(manifest: &ParsedManifest, parent_key: &str) -> DiagnosticMetadata {
    let mut metadata = DiagnosticMetadata::default().with_redacted_evidence();
    if let Some(span) = manifest
        .source()
        .and_then(|source| json_member_value_range(source, parent_key, "email"))
        .and_then(|range| {
            manifest
                .source()
                .and_then(|source| SourceSpan::from_byte_range(source, range))
        })
    {
        metadata = metadata.with_location(span);
    }
    metadata
}

fn is_contact_metadata_email(email: &str) -> bool {
    let bytes = email.as_bytes();
    if !(3..=254).contains(&bytes.len())
        || !email.is_ascii()
        || bytes.iter().any(|byte| byte.is_ascii_control())
        || email.trim() != email
        || bytes.iter().filter(|byte| **byte == b'@').count() != 1
    {
        return false;
    }

    let (local, domain) = email.split_once('@').expect("exactly one @ was checked");
    if !(1..=64).contains(&local.len()) || !(1..=253).contains(&domain.len()) {
        return false;
    }
    if !local.split('.').all(is_valid_local_atom) || !domain.contains('.') {
        return false;
    }

    let mut labels = domain.split('.');
    let mut last = None;
    for label in labels.by_ref() {
        if !is_valid_domain_label(label) {
            return false;
        }
        last = Some(label);
    }
    let Some(last) = last else {
        return false;
    };
    (last.len() >= 2 && last.len() <= 63 && last.bytes().all(|byte| byte.is_ascii_alphabetic()))
        || last.starts_with("xn--")
}

fn is_valid_local_atom(atom: &str) -> bool {
    !atom.is_empty()
        && atom.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'/'
                        | b'='
                        | b'?'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'{'
                        | b'|'
                        | b'}'
                        | b'~'
                )
        })
}

fn is_valid_domain_label(label: &str) -> bool {
    let bytes = label.as_bytes();
    (1..=63).contains(&bytes.len())
        && bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
}

/// Return the final effective nested member's raw JSON value range. The
/// manifest has already parsed successfully, so this small scanner only maps
/// JSON tokens to source spans; it never makes semantic validation decisions.
fn json_member_value_range(
    source: &str,
    parent_key: &str,
    member_key: &str,
) -> Option<Range<usize>> {
    let mut scanner = JsonMemberScanner::new(source);
    scanner.scan_value(true, false, parent_key, member_key);
    scanner.found
}

struct JsonMemberScanner<'a> {
    input: &'a [u8],
    position: usize,
    found: Option<Range<usize>>,
}

impl<'a> JsonMemberScanner<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            input: source.as_bytes(),
            position: 0,
            found: None,
        }
    }

    fn scan_value(
        &mut self,
        is_root: bool,
        active_parent: bool,
        parent_key: &str,
        member_key: &str,
    ) {
        self.skip_whitespace();
        match self.input.get(self.position) {
            Some(b'{') => self.scan_object(is_root, active_parent, parent_key, member_key),
            Some(b'[') => self.scan_array(parent_key, member_key),
            Some(b'\"') => {
                self.scan_string();
            }
            Some(_) => self.scan_scalar(),
            None => {}
        }
    }

    fn scan_object(
        &mut self,
        is_root: bool,
        active_parent: bool,
        parent_key: &str,
        member_key: &str,
    ) {
        self.position += 1;
        loop {
            self.skip_whitespace();
            if self.input.get(self.position) == Some(&b'}') {
                self.position += 1;
                return;
            }
            let key = self.scan_string();
            self.skip_whitespace();
            self.position += 1; // validated JSON has a colon here
            self.skip_whitespace();
            let value_start = self.position;
            if active_parent && key == member_key {
                self.scan_value(false, false, parent_key, member_key);
                self.found = Some(value_start..self.position);
            } else {
                self.scan_value(false, is_root && key == parent_key, parent_key, member_key);
            }
            self.skip_whitespace();
            if self.input.get(self.position) == Some(&b',') {
                self.position += 1;
            }
        }
    }

    fn scan_array(&mut self, parent_key: &str, member_key: &str) {
        self.position += 1;
        loop {
            self.skip_whitespace();
            if self.input.get(self.position) == Some(&b']') {
                self.position += 1;
                return;
            }
            self.scan_value(false, false, parent_key, member_key);
            self.skip_whitespace();
            if self.input.get(self.position) == Some(&b',') {
                self.position += 1;
            }
        }
    }

    fn scan_string(&mut self) -> String {
        let start = self.position;
        self.position += 1;
        while self.position < self.input.len() {
            match self.input[self.position] {
                b'\\' => self.position += 2,
                b'\"' => {
                    self.position += 1;
                    break;
                }
                _ => self.position += 1,
            }
        }
        serde_json::from_slice(&self.input[start..self.position])
            .expect("scanner only runs after successful JSON parsing")
    }

    fn scan_scalar(&mut self) {
        while self.position < self.input.len()
            && !matches!(
                self.input[self.position],
                b',' | b']' | b'}' | b' ' | b'\n' | b'\r' | b'\t'
            )
        {
            self.position += 1;
        }
    }

    fn skip_whitespace(&mut self) {
        while self.position < self.input.len() && self.input[self.position].is_ascii_whitespace() {
            self.position += 1;
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
            declared_hook_configs: vec![],
            settings_json: ManifestState::Missing,
            settings_local_json: ManifestState::Missing,
        }
    }

    #[test]
    fn contact_metadata_grammar_has_documented_boundaries() {
        let valid_local = format!("{}@example.com", "a".repeat(64));
        let valid_total = format!(
            "a@{}.{}.{}.{}",
            "a".repeat(63),
            "a".repeat(63),
            "a".repeat(63),
            "a".repeat(60)
        );
        let cases = [
            ("user@example.com", true),
            ("a.b+tag@xn--bcher-kva.example", true),
            (&valid_local, true),
            (&valid_total, true),
            ("", false),
            (" person@example.com", false),
            ("person @example.com", false),
            ("person@@example.com", false),
            ("person@example", false),
            (".person@example.com", false),
            ("person..name@example.com", false),
            ("person@example..com", false),
            ("person@-example.com", false),
            ("person@example-.com", false),
            ("person@example.c", false),
            ("person@例え.テスト", false),
            ("\"person\"@example.com", false),
            ("person@[127.0.0.1]", false),
            ("person\n@example.com", false),
        ];
        for (email, expected) in cases {
            assert_eq!(is_contact_metadata_email(email), expected, "{email:?}");
        }
        assert!(!is_contact_metadata_email(&format!(
            "{}@example.com",
            "a".repeat(65)
        )));
        assert!(!is_contact_metadata_email(&format!(
            "a@{}.{}.{}.{}x",
            "a".repeat(63),
            "a".repeat(63),
            "a".repeat(63),
            "a".repeat(60)
        )));
    }

    #[test]
    fn format_and_type_findings_are_distinct_and_redacted() {
        let private_email = "private-routing@example";
        let ctx = make_ctx(
            ManifestState::parsed(json!({"author": {"email": private_email}})),
            ManifestState::parsed(json!({"owner": {"email": 42}})),
        );
        let mut diag = DiagnosticCollector::new();
        validate_email_format(&ctx, &mut diag);
        assert_eq!(diag.warning_count(), 1);
        assert_eq!(diag.error_count(), 1);
        assert_eq!(diag.diagnostics()[0].rule, LintRule::EmailTypeInvalid);
        assert_eq!(diag.diagnostics()[1].rule, LintRule::InvalidEmailFormat);
        for diagnostic in diag.diagnostics() {
            assert!(!diagnostic.message.contains(private_email));
            assert_eq!(
                diagnostic.evidence.as_deref(),
                Some("[redacted: possible secret]")
            );
            assert!(diagnostic.suggestion.is_some());
        }
    }

    #[test]
    fn present_wrong_types_emit_only_e002() {
        for value in [json!(null), json!(true), json!(42), json!([]), json!({})] {
            for (plugin, marketplace) in [
                (
                    ManifestState::parsed(json!({"author": {"email": value.clone()}})),
                    ManifestState::Missing,
                ),
                (
                    ManifestState::Missing,
                    ManifestState::parsed(json!({"owner": {"email": value}})),
                ),
            ] {
                let ctx = make_ctx(plugin, marketplace);
                let mut diag = DiagnosticCollector::new_all_enabled();
                validate_email_format(&ctx, &mut diag);
                assert_eq!(diag.diagnostics().len(), 1);
                assert_eq!(diag.diagnostics()[0].rule, LintRule::EmailTypeInvalid);
            }
        }
    }

    #[test]
    fn raw_member_scanner_uses_the_effective_nested_email_value() {
        let source = r#"{"email":"decoy@example.com","author":{"email":"first@example.com","nested":{"author":{"email":"nested@example.com"}},"email":"final@example"}}"#;
        let range = json_member_value_range(source, "author", "email").unwrap();
        assert_eq!(&source[range], "\"final@example\"");
    }
}
