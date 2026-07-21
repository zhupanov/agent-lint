//! Shared recognition for plugin skill path-root diagnostics and their autofix.
//!
//! The validator and the fixer deliberately consume the same classifier so an
//! emitted G001 is always safe to rewrite and a G012 is never rewritten.

use crate::frontmatter;
use regex::Regex;
use std::ops::Range;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

static RE_PWD_REFERENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\$(?:PWD|\{PWD\})(?:/[^\s`"'<>()\[\]{}]*)?"#)
        .expect("PWD reference regex is valid")
});
static RE_MACHINE_PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?x)
            /(?:Users|home|root|opt)/[^\s`"'<>()\[\]{}]*
          | /var/folders/[^\s`"'<>()\[\]{}]*
          | [A-Za-z]:[\\/][^\s`"'<>()\[\]{}]*
          | \\\\[^\s\\`"'<>()\[\]{}]+\\[^\s`"'<>()\[\]{}]*
        "#,
    )
    .expect("machine path regex is valid")
});
static RE_URL_TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"[A-Za-z][A-Za-z0-9+.-]*://[^\s`"'<>()\[\]{}]*"#)
        .expect("URL token regex is valid")
});

const PLUGIN_COMPONENTS: &[&str] = &[
    "scripts",
    "skills",
    "agents",
    "commands",
    "hooks",
    "output-styles",
    "themes",
    "monitors",
    ".claude-plugin",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PathIssueKind {
    BundledAsset,
    HardcodedMachinePath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PathIssue {
    pub(crate) kind: PathIssueKind,
    /// The exact path reference, used for source location and evidence.
    pub(crate) range: Range<usize>,
    /// Only G001 has a replacement range: exactly `$PWD/` or `${PWD}/`.
    pub(crate) replacement_range: Option<Range<usize>>,
}

/// Find path-root issues in a public `SKILL.md` body in source order.
///
/// Frontmatter is intentionally outside this validator's contract. URL tokens
/// are also excluded, because a path-looking URL component is not a local path.
pub(crate) fn find_path_issues(content: &str) -> Vec<PathIssue> {
    let body_start = body_start_offset(content);
    let urls: Vec<_> = RE_URL_TOKEN
        .find_iter(content)
        .map(|found| found.range())
        .collect();
    let mut issues = Vec::new();

    for found in RE_PWD_REFERENCE.find_iter(content) {
        let range = trim_trailing_punctuation(content, found.range());
        if range.is_empty() || range.start < body_start || in_url(&urls, range.start) {
            continue;
        }
        let reference = &content[range.clone()];
        let (kind, replacement_range) = match bundled_asset_reference(reference) {
            Some(prefix_len) => (
                PathIssueKind::BundledAsset,
                Some(range.start..range.start + prefix_len),
            ),
            None => (PathIssueKind::HardcodedMachinePath, None),
        };
        issues.push(PathIssue {
            kind,
            range,
            replacement_range,
        });
    }

    for found in RE_MACHINE_PATH.find_iter(content) {
        let range = trim_trailing_punctuation(content, found.range());
        if range.is_empty() || range.start < body_start || in_url(&urls, range.start) {
            continue;
        }
        issues.push(PathIssue {
            kind: PathIssueKind::HardcodedMachinePath,
            range,
            replacement_range: None,
        });
    }

    issues.sort_by(|left, right| left.range.start.cmp(&right.range.start));
    issues.dedup_by(|left, right| left.range == right.range);
    issues
}

/// Apply the replacements identified by G001 without touching G012 references.
pub(crate) fn replace_bundled_asset_prefixes(content: &str) -> Option<String> {
    let mut replacements: Vec<_> = find_path_issues(content)
        .into_iter()
        .filter_map(|issue| {
            (issue.kind == PathIssueKind::BundledAsset)
                .then_some(issue.replacement_range)
                .flatten()
        })
        .collect();
    if replacements.is_empty() {
        return None;
    }

    replacements.sort_by(|left, right| right.start.cmp(&left.start));
    let mut fixed = content.to_string();
    for range in replacements {
        fixed.replace_range(range, "${CLAUDE_PLUGIN_ROOT}/");
    }
    Some(fixed)
}

fn body_start_offset(content: &str) -> usize {
    if !content.starts_with("---") {
        return 0;
    }
    let body = frontmatter::extract_body(content);
    if body.is_empty() {
        content.len()
    } else {
        content.len() - body.len()
    }
}

fn trim_trailing_punctuation(content: &str, mut range: Range<usize>) -> Range<usize> {
    while range.end > range.start {
        let character = content[..range.end]
            .chars()
            .next_back()
            .expect("non-empty matched range has a final character");
        if !matches!(character, '.' | ',' | ';' | ':' | '!' | '?') {
            break;
        }
        range.end -= character.len_utf8();
    }
    range
}

fn in_url(urls: &[Range<usize>], offset: usize) -> bool {
    urls.iter()
        .any(|url| url.start <= offset && offset < url.end)
}

fn bundled_asset_reference(reference: &str) -> Option<usize> {
    let prefix = if let Some(remainder) = reference.strip_prefix("$PWD/") {
        ("$PWD/".len(), remainder)
    } else if let Some(remainder) = reference.strip_prefix("${PWD}/") {
        ("${PWD}/".len(), remainder)
    } else {
        return None;
    };

    let normalized = normalize_relative_path(prefix.1)?;
    let first_component = normalized.components().next()?.as_os_str().to_str()?;
    if !PLUGIN_COMPONENTS.contains(&first_component) {
        return None;
    }
    (normalized.is_file() || normalized.is_dir()).then_some(prefix.0)
}

fn normalize_relative_path(path: &str) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(segment) => normalized.push(segment),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!normalized.as_os_str().is_empty()).then_some(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[serial_test::serial]
    fn classifies_only_existing_plugin_components_as_bundled_assets() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("scripts").unwrap();
        std::fs::write("scripts/check.sh", "#!/bin/sh\n").unwrap();
        std::fs::create_dir_all("skills/example").unwrap();

        let source = concat!(
            "---\nname: $PWD/scripts/check.sh\n---\n",
            "Run $PWD/scripts/check.sh and ${PWD}/skills/example.\n",
            "Read $PWD/package.json and ${PWD}.\n"
        );
        let issues = find_path_issues(source);

        assert_eq!(
            issues.iter().map(|issue| issue.kind).collect::<Vec<_>>(),
            vec![
                PathIssueKind::BundledAsset,
                PathIssueKind::BundledAsset,
                PathIssueKind::HardcodedMachinePath,
                PathIssueKind::HardcodedMachinePath,
            ]
        );
        assert_eq!(
            replace_bundled_asset_prefixes(source).unwrap(),
            concat!(
                "---\nname: $PWD/scripts/check.sh\n---\n",
                "Run ${CLAUDE_PLUGIN_ROOT}/scripts/check.sh and ${CLAUDE_PLUGIN_ROOT}/skills/example.\n",
                "Read $PWD/package.json and ${PWD}.\n"
            )
        );
    }

    #[test]
    fn finds_machine_paths_but_not_url_tokens_or_portable_system_paths() {
        let source = concat!(
            "See /Users/alice/project, /home/alice/project, /root/private, /opt/tool, and /var/folders/a.\n",
            "Windows C:\\\\Users\\\\alice and C:/Users/alice; UNC \\\\\\\\server\\\\share.\n",
            "URLs https://example.test/home/alice and https://example.test/Users/alice stay clean.\n",
            "Portable /tmp/cache /usr/bin /etc/hosts /var/log /bin/sh /dev/null stay clean.\n"
        );
        let issues = find_path_issues(source);
        assert_eq!(issues.len(), 8);
        assert!(
            issues
                .iter()
                .all(|issue| issue.kind == PathIssueKind::HardcodedMachinePath)
        );
    }
}
