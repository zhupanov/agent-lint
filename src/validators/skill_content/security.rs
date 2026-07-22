use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata};
use crate::rules::LintRule;
use crate::sensitive::{contains_sensitive_evidence, find_skill_secret};
use crate::validators::skills::SkillInfo;
use regex::Regex;
use std::sync::LazyLock;

// S031: Non-HTTPS URLs. Matches `http://` immediately followed by an
// alphanumeric authority character. This is the single owner of the pattern;
// the autofix consumes the same classifier so checker and fixer agree by
// construction (issue #353).
static RE_HTTP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"http://[a-zA-Z0-9]").unwrap());

// Attribute-value identifier contexts. An `http://` inside an `xmlns`,
// `xmlns:<prefix>`, `schemaLocation`, `xsi:schemaLocation`, or `targetNamespace`
// attribute value is an opaque XML namespace / schema identifier, never a
// fetchable link, so it must never be flagged or rewritten. Line-context
// regexes are sufficient; no XML parsing (issue #353).
//
// The leading `(?:^|[\s"'<>])` requires the attribute name to start at a real
// token boundary, so a genuine insecure URL in an unrelated attribute whose
// name merely ends in one of these tokens — `data-xmlns="http://evil.corp"`,
// `my_xmlns = "http://evil.corp"` — is not swallowed by span containment. A
// bare `\b` is insufficient because `-`/`_` already form word boundaries.
static RE_IDENTIFIER_ATTR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?:^|[\s"'<>])(?:xmlns(?::[A-Za-z0-9_-]+)?|(?:xsi:)?schemaLocation|targetNamespace)\s*=\s*(?:"[^"]*"|'[^']*')"#,
    )
    .unwrap()
});

// A `<!DOCTYPE … >` declaration. Any system/public identifier inside it is
// opaque. `[^>]*` stops at the closing `>` (consumed by `>?`) or, for a
// declaration continued on a later line, at end of line.
static RE_DOCTYPE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<!doctype[^>]*>?").unwrap());

/// Classification of an `http://` match found in skill body content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HttpMatch {
    /// Insecure URL: flag it (S031) and rewrite it (autofix).
    Flag,
    /// Opaque XML identifier or reserved/documentation host: never flag or
    /// rewrite.
    Exempt,
}

/// Decide whether an `http://` match is an insecure URL to flag and rewrite, or
/// an exempt identifier/reserved host to leave byte-identical.
///
/// `line` is the full source line containing the match; `offset_in_line` is the
/// byte offset of `http://` within that line. This is the one classifier the
/// S031 checker and the S031 autofix both consult, so they agree by
/// construction (issue #353).
pub(crate) fn classify_http_match(line: &str, offset_in_line: usize) -> HttpMatch {
    if identifier_exempt(line, offset_in_line) {
        return HttpMatch::Exempt;
    }
    let after_scheme = &line[offset_in_line + "http://".len()..];
    if host_exempt(http_host(after_scheme)) {
        return HttpMatch::Exempt;
    }
    HttpMatch::Flag
}

/// True when the match at `offset_in_line` falls inside an XML identifier
/// context (attribute value or DOCTYPE declaration) on its line.
fn identifier_exempt(line: &str, offset_in_line: usize) -> bool {
    RE_IDENTIFIER_ATTR
        .find_iter(line)
        .chain(RE_DOCTYPE.find_iter(line))
        .any(|m| m.start() <= offset_in_line && offset_in_line < m.end())
}

/// The host of an `http://` URL, given the text immediately after `http://`.
///
/// The authority ends at the first `/`, `?`, `#`, whitespace, or quote. Issue
/// #353 requirement 3 also lists `:` as a terminator, but `:` is only stripped
/// *after* removing any `user[:pass]@` userinfo: terminating at the first `:`
/// directly would cut inside the userinfo of `http://ok.example:x@evil.corp/`
/// and misread the host as the exempt `ok.example`, silently exempting an
/// insecure link to an arbitrary host. So: bound the authority (no `:`), drop
/// userinfo, then strip the port. IPv6 literals (`[::1]`) are not special-cased
/// — they remain flagged, matching prior behavior.
fn http_host(after_scheme: &str) -> &str {
    let end = after_scheme
        .find(|c: char| {
            matches!(c, '/' | '?' | '#' | '"' | '\'' | '<' | '>' | '`') || c.is_whitespace()
        })
        .unwrap_or(after_scheme.len());
    let host_and_port = {
        let authority = &after_scheme[..end];
        authority.rsplit('@').next().unwrap_or(authority)
    };
    match host_and_port.split_once(':') {
        Some((host, _port)) => host,
        None => host_and_port,
    }
}

/// True for hosts that are non-fetchable by construction: loopback/unspecified
/// addresses, the `www.w3.org` namespace/DTD authority, RFC 2606 §2 / RFC 6761
/// reserved TLDs (`test`, `example`, `invalid`, `localhost`), and the reserved
/// documentation second-level domains `example.com`/`.org`/`.net` and their
/// subdomains.
fn host_exempt(host: &str) -> bool {
    // DNS host names are case-insensitive, so compare in lower case.
    let host = host.to_ascii_lowercase();
    let host = host.as_str();
    if matches!(host, "localhost" | "127.0.0.1" | "0.0.0.0") {
        return true;
    }
    // Namespace/DTD authority; appears in skills only as an identifier.
    if host == "www.w3.org" {
        return true;
    }
    // RFC 2606 / RFC 6761 reserved top-level labels.
    let final_label = host.rsplit('.').next().unwrap_or(host);
    if matches!(final_label, "test" | "example" | "invalid" | "localhost") {
        return true;
    }
    // Reserved documentation domains, bare and any subdomain (covers `www.`).
    ["example.com", "example.org", "example.net"]
        .iter()
        .any(|base| host == *base || host.ends_with(&format!(".{base}")))
}

/// Yield the byte offset within `body` of every `http://` match that S031 flags
/// and the autofix rewrites, skipping exempt identifier/reserved-host matches.
/// The single traversal both the checker and the fixer consume.
pub(crate) fn flagged_http_offsets(body: &str) -> impl Iterator<Item = usize> + '_ {
    RE_HTTP.find_iter(body).filter_map(|m| {
        let start = m.start();
        let line_start = body[..start].rfind('\n').map_or(0, |i| i + 1);
        let line_end = body[start..].find('\n').map_or(body.len(), |i| start + i);
        let line = &body[line_start..line_end];
        match classify_http_match(line, start - line_start) {
            HttpMatch::Flag => Some(start),
            HttpMatch::Exempt => None,
        }
    })
}

/// The scheme+host+path of the `http://` URL beginning at `start` in `body`,
/// excluding any userinfo and any query/fragment. This is non-sensitive by
/// construction (issue #353: scheme+host+path is not sensitive) and is used as
/// diagnostic evidence.
fn scheme_host_path(body: &str, start: usize) -> String {
    let rest = &body[start + "http://".len()..];
    let is_delim = |c: char| {
        matches!(c, '?' | '#' | '"' | '\'' | '<' | '>' | '`' | ' ' | '\t') || c.is_whitespace()
    };
    // Authority ends at the path separator, a query/fragment, or a delimiter.
    let authority_end = rest
        .find(|c: char| c == '/' || is_delim(c))
        .unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let host_port = authority.rsplit('@').next().unwrap_or(authority);
    let path = if rest[authority_end..].starts_with('/') {
        let tail = &rest[authority_end..];
        let path_end = tail.find(is_delim).unwrap_or(tail.len());
        &tail[..path_end]
    } else {
        ""
    };
    format!("http://{host_port}{path}")
}

pub(super) fn check_content_security(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    // S031: non-HTTPS URLs. Report once per file at the first flagged match,
    // with line metadata and the offending URL as evidence.
    if let Some(start) = flagged_http_offsets(&info.body).next() {
        let body_line = info.body[..start]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
            + 1;
        // Body line 1 is the file line after the frontmatter: opening `---`,
        // the frontmatter lines, then the closing `---`.
        let line = body_line + info.fm_lines.len() + 2;
        let url = scheme_host_path(&info.body, start);
        // The rendered message carries the URL in text output, which bypasses
        // the collector's evidence redaction. Dropping the query in
        // `scheme_host_path` removes the usual secret vector, but a secret-like
        // path segment could still trip the shared heuristic; when it does,
        // keep the text message and the structured evidence consistent by
        // redacting both rather than leaking the value in one surface only.
        let suggestion = "use https:// (or remove the reference)";
        let metadata = DiagnosticMetadata::at_line(line).with_suggestion(suggestion);
        let (shown, metadata) = if contains_sensitive_evidence(&url) {
            (
                "non-HTTPS URL".to_string(),
                metadata.with_redacted_evidence(),
            )
        } else {
            (
                format!("non-HTTPS URL '{url}'"),
                metadata.with_evidence(&url),
            )
        };
        diag.report_with(
            LintRule::NonHttpsUrl,
            &format!("{}:{line}: {shown} found; {suggestion}", info.path),
            metadata,
        );
    }

    // S032 scans the complete source, rather than only the parsed Markdown
    // body: committed credentials in frontmatter are still credentials. Its
    // scanner returns source-safe evidence and an offset solely for line
    // metadata, never for rendering a candidate value.
    let source = info.document.content();
    if let Some(finding) = find_skill_secret(source) {
        let line = source[..finding.location_range.start]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
            + 1;
        let metadata = DiagnosticMetadata::at_line(line)
            .with_evidence(finding.evidence)
            .with_suggestion(
                "replace the literal with an environment-variable or secret-store reference",
            );
        diag.report_with(
            LintRule::HardcodedSecret,
            &format!("{}: potential hardcoded secret/API key detected", info.path),
            metadata,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Classify the first `http://` occurrence on `line`.
    fn classify(line: &str) -> HttpMatch {
        let offset = line
            .find("http://")
            .expect("test line has an http:// match");
        classify_http_match(line, offset)
    }

    #[test]
    fn identifier_contexts_are_exempt() {
        for line in [
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">"#,
            r#"<html xmlns="http://www.w3.org/1999/xhtml">"#,
            r#"<manifest xmlns:android="http://schemas.android.com/apk/res/android">"#,
            r#"<!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.0//EN" "http://www.w3.org/TR/xhtml1.dtd">"#,
            r#"<x xsi:schemaLocation="http://maven.apache.org/POM/4.0.0 http://maven.apache.org/xsd/maven-4.0.0.xsd">"#,
            r#"<schema targetNamespace="http://example-vendor.corp/ns">"#,
            r#"<project schemaLocation='http://internal.corp/schema.xsd'>"#,
        ] {
            assert_eq!(classify(line), HttpMatch::Exempt, "line: {line}");
        }
    }

    #[test]
    fn bare_w3_org_authority_is_exempt() {
        assert_eq!(
            classify("see http://www.w3.org/2001/XMLSchema"),
            HttpMatch::Exempt
        );
    }

    #[test]
    fn reserved_and_documentation_hosts_are_exempt() {
        for line in [
            "http://www.example.com/guide",
            "http://example.com",
            "http://example.org/docs",
            "http://example.net",
            "http://demo.example.net/x",
            "http://foo.test/x",
            "http://demo.invalid/",
            "http://service.example/",
            "http://box.localhost:9000/",
            "http://localhost:8080/data",
            "http://127.0.0.1/",
            "http://0.0.0.0/",
        ] {
            assert_eq!(classify(line), HttpMatch::Exempt, "line: {line}");
        }
    }

    #[test]
    fn genuine_insecure_urls_are_flagged() {
        for line in [
            "http://api.internal.corp/v1",
            "http://intranet:8080/",
            "http://api.example.dev/data",
            "http://api.foo.dev",
            "http://api.corp/x",
        ] {
            assert_eq!(classify(line), HttpMatch::Flag, "line: {line}");
        }
    }

    #[test]
    fn identifier_line_still_flags_a_separate_insecure_url() {
        let line = r#"<svg xmlns="http://www.w3.org/2000/svg"> then visit http://api.corp/x"#;
        let offsets: Vec<usize> = flagged_http_offsets(line).collect();
        assert_eq!(offsets.len(), 1);
        assert_eq!(
            &line[offsets[0]..offsets[0] + "http://api.corp".len()],
            "http://api.corp"
        );
    }

    #[test]
    fn evidence_excludes_userinfo_and_query() {
        let body = "grab http://user:secret@api.corp/v1?token=abc123 now";
        let start = flagged_http_offsets(body).next().unwrap();
        assert_eq!(scheme_host_path(body, start), "http://api.corp/v1");
    }

    #[test]
    fn userinfo_does_not_launder_the_host_through_an_exempt_username() {
        // The real fetch target is the userinfo-suffixed host, not the
        // exempt-looking username; a colon inside the userinfo must not cut the
        // host short (issue #353 self-review finding 1).
        for line in [
            "http://ok.example:x@evil.corp/beacon.gif",
            "http://localhost:x@evil.corp/",
            "http://www.w3.org:x@evil.corp/",
            "http://user:pass@intranet.corp/",
        ] {
            assert_eq!(classify(line), HttpMatch::Flag, "line: {line}");
        }
    }

    #[test]
    fn attribute_name_suffix_does_not_exempt_a_real_url() {
        // The token must begin at a real boundary; an unrelated attribute whose
        // name merely ends in `xmlns`/`schemaLocation` is still flagged.
        for line in [
            r#"<a data-xmlns="http://evil.corp/x">"#,
            r#"config: my_xmlns = "http://internal.corp/api""#,
            r#"<a data-schemaLocation="http://evil.corp/x">"#,
        ] {
            assert_eq!(classify(line), HttpMatch::Flag, "line: {line}");
        }
    }

    #[test]
    fn host_comparison_is_case_insensitive() {
        for line in [
            "http://EXAMPLE.COM/guide",
            "http://WWW.W3.ORG/2000/svg",
            "http://Foo.TEST/x",
        ] {
            assert_eq!(classify(line), HttpMatch::Exempt, "line: {line}");
        }
    }
}
