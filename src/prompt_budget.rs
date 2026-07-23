//! Shared prompt-source discovery and size measurement for D004 and S062.

use crate::config::{PromptMetricCaps, PromptSourceBudget};
use crate::fence::lines_outside_fences;
use crate::live_instructions::example_scopes_for;
use crate::markdown::MarkdownDocument;
use crate::markdown_refs::{
    MarkdownRefKind, clause_is_mandatory_load, clause_rejects_mandatory_at_import,
    is_root_plain_md_prefix, markdown_references as structured_refs, percent_decode_once,
    prompt_resolution_base,
};
use crate::repo_path::{PathProbe, ResolutionBase, normalize_separators, resolve_repo_path};
use regex::Regex;
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

static PLAIN_MD_PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?:^|[\s`'(])((?:skills|\.claude/skills|docs|agents|scripts)/[A-Za-z0-9._/-]+\.md)\b",
    )
    .unwrap()
});

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SourceMetrics {
    pub lines: usize,
    pub estimated_tokens: usize,
    pub content_tokens: usize,
}

impl std::ops::AddAssign for SourceMetrics {
    fn add_assign(&mut self, other: Self) {
        self.lines += other.lines;
        self.estimated_tokens += other.estimated_tokens;
        self.content_tokens += other.content_tokens;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetMeasurement {
    pub root: SourceMetrics,
    pub closure: SourceMetrics,
    pub conditional: SourceMetrics,
    pub closure_files: BTreeSet<PathBuf>,
    pub conditional_files: BTreeSet<PathBuf>,
}

/// One stable machine-readable measurement emitted by `--closure-report`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BudgetReportRow {
    pub group: String,
    pub source_set: &'static str,
    pub scope: &'static str,
    pub metric: &'static str,
    pub measured_value: usize,
    pub cap: Option<usize>,
}

/// Lexical repository-relative normalization used by import-graph callers.
pub fn normalize_repo_relative(path: &Path) -> Option<PathBuf> {
    crate::repo_path::normalize_repo_relative(path)
}

/// Discover mandatory Markdown prompt references in deterministic path order.
pub fn markdown_references(source_path: &Path, content: &str) -> Vec<PathBuf> {
    let mut refs = BTreeSet::new();
    for reference in structured_refs(content) {
        if reference.excluded_from_always_load {
            continue;
        }
        let Some(clause) = reference.clause.as_deref() else {
            continue;
        };
        if !clause_is_mandatory_load(clause) {
            continue;
        }
        let decoded = if reference.kind == MarkdownRefKind::Link {
            percent_decode_once(&reference.decoded)
        } else {
            reference.decoded.clone()
        };
        let path = decoded.split(['#', ':']).next().unwrap_or(&decoded);
        if !path.ends_with(".md") || path.contains(['$', '{', '}', '<', '>', '*']) {
            continue;
        }
        let base = prompt_resolution_base(reference.kind, path);
        add_resolved_reference(source_path, path, base, &mut refs);
    }
    for line in lines_outside_fences(content) {
        for capture in PLAIN_MD_PATH.captures_iter(line) {
            let raw = &capture[1];
            if !is_root_plain_md_prefix(raw) {
                continue;
            }
            // Plain paths are only collected from mandatory-looking lines: reuse
            // the same clause classifier against the full physical line.
            if !clause_is_mandatory_load(line) {
                continue;
            }
            add_resolved_reference(source_path, raw, ResolutionBase::RepositoryRoot, &mut refs);
        }
    }
    collect_live_raw_at_md_references(source_path, content, &mut refs);
    refs.into_iter().collect()
}

/// Live raw `@path.md` directives are mandatory without strength cues, resolve
/// source-relatively only, and honor the shared live-prose exclusions.
fn collect_live_raw_at_md_references(
    source_path: &Path,
    content: &str,
    refs: &mut BTreeSet<PathBuf>,
) {
    let document = MarkdownDocument::parse(content);
    let example_scopes = example_scopes_for(&document);
    for (line, is_example) in document.body_prose().iter().zip(example_scopes) {
        if is_example {
            continue;
        }
        if clause_rejects_mandatory_at_import(&line.text) {
            continue;
        }
        let chars: Vec<char> = line.text.chars().collect();
        let mut index = 0;
        while index < chars.len() {
            if chars[index] != '@'
                || !raw_at_token_boundary(chars.get(index.wrapping_sub(1)).copied())
                || document.links().iter().any(|link| {
                    (link.line..=link.end_line).contains(&line.line)
                        && (line.line != link.line || index + 1 >= link.start_column)
                        && (line.line != link.end_line || index < link.end_column)
                })
            {
                index += 1;
                continue;
            }
            index += 1;
            let token_start = index;
            while index < chars.len() && !chars[index].is_whitespace() {
                index += 1;
            }
            let mut token_end = index;
            while token_end > token_start
                && matches!(
                    chars[token_end - 1],
                    ')' | ']' | '}' | '>' | ',' | '.' | ';' | ':' | '!' | '?'
                )
            {
                token_end -= 1;
            }
            if token_end == token_start {
                continue;
            }
            let raw: String = chars[token_start..token_end].iter().collect();
            if raw.contains('@') || raw.starts_with('/') || raw.starts_with("~/") {
                continue;
            }
            let path = raw.split(['#', ':']).next().unwrap_or(&raw);
            if !path.ends_with(".md") || path.contains(['$', '{', '}', '<', '>', '*']) {
                continue;
            }
            add_resolved_reference(source_path, path, ResolutionBase::SourceRelative, refs);
        }
    }
}

fn raw_at_token_boundary(previous: Option<char>) -> bool {
    previous.is_none_or(|ch| ch.is_whitespace() || matches!(ch, '(' | '[' | '{' | '<' | '"' | '\''))
}

fn add_resolved_reference(
    source: &Path,
    raw: &str,
    base: ResolutionBase,
    refs: &mut BTreeSet<PathBuf>,
) {
    let normalized = normalize_separators(raw);
    if !normalized.ends_with(".md") {
        return;
    }
    if let PathProbe::File(path) = resolve_repo_path(source, &normalized, base) {
        refs.insert(path);
    }
}

pub fn source_metrics(content: &str) -> SourceMetrics {
    let content_chars = content.chars().count();
    let nonblank_chars = content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.chars().count() + 1)
        .sum::<usize>();
    SourceMetrics {
        lines: content.lines().count(),
        estimated_tokens: content_chars.div_ceil(4),
        content_tokens: nonblank_chars.div_ceil(4),
    }
}

pub fn measure_budget(budget: &PromptSourceBudget) -> Result<BudgetMeasurement, String> {
    let roots: Vec<PathBuf> = budget.roots.iter().map(PathBuf::from).collect();
    let conditional: Vec<PathBuf> = budget
        .conditional_sources
        .iter()
        .map(PathBuf::from)
        .collect();
    let (root, _) = measure_paths(&roots, false, &BTreeSet::new())?;
    let (closure, closure_files) = measure_paths(&roots, true, &BTreeSet::new())?;
    let (conditional, conditional_files) = measure_paths(&conditional, true, &closure_files)?;
    Ok(BudgetMeasurement {
        root,
        closure,
        conditional,
        closure_files,
        conditional_files,
    })
}

fn measure_paths(
    roots: &[PathBuf],
    transitive: bool,
    excluded: &BTreeSet<PathBuf>,
) -> Result<(SourceMetrics, BTreeSet<PathBuf>), String> {
    let mut pending: BTreeSet<PathBuf> = roots.iter().cloned().collect();
    let mut seen = BTreeSet::new();
    let mut metrics = SourceMetrics::default();
    while let Some(path) = pending.pop_first() {
        let Some(path) = normalize_repo_relative(&path) else {
            return Err(format!(
                "prompt source '{}' escapes the repository",
                path.display()
            ));
        };
        if excluded.contains(&path) || !seen.insert(path.clone()) {
            continue;
        }
        let content = match resolve_repo_path(
            Path::new("."),
            &path.to_string_lossy(),
            ResolutionBase::RepositoryRoot,
        ) {
            PathProbe::File(_) => fs::read_to_string(&path).map_err(|error| {
                format!("cannot read prompt source '{}': {error}", path.display())
            })?,
            _ => {
                return Err(format!(
                    "cannot read prompt source '{}': missing, unsafe, or unreadable",
                    path.display()
                ));
            }
        };
        metrics += source_metrics(&content);
        if transitive {
            pending.extend(markdown_references(&path, &content));
        }
    }
    Ok((metrics, seen))
}

pub fn report_rows(
    budget: &PromptSourceBudget,
    measurement: &BudgetMeasurement,
) -> Vec<BudgetReportRow> {
    let mut rows = Vec::new();
    push_metric_rows(
        &mut rows,
        &budget.name,
        "always",
        "root",
        measurement.root,
        budget.root_caps,
    );
    push_metric_rows(
        &mut rows,
        &budget.name,
        "always",
        "closure",
        measurement.closure,
        budget.closure_caps,
    );
    if !budget.conditional_sources.is_empty() {
        push_metric_rows(
            &mut rows,
            &budget.name,
            "conditional",
            "closure",
            measurement.conditional,
            budget.conditional_caps,
        );
    }
    rows
}

fn push_metric_rows(
    rows: &mut Vec<BudgetReportRow>,
    group: &str,
    source_set: &'static str,
    scope: &'static str,
    metrics: SourceMetrics,
    caps: PromptMetricCaps,
) {
    for (metric, measured_value, cap) in [
        ("lines", metrics.lines, caps.lines),
        (
            "estimated_tokens",
            metrics.estimated_tokens,
            caps.estimated_tokens,
        ),
        (
            "content_tokens",
            metrics.content_tokens,
            caps.content_tokens,
        ),
    ] {
        rows.push(BudgetReportRow {
            group: group.to_string(),
            source_set,
            scope,
            metric,
            measured_value,
            cap,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[serial_test::serial]
    fn measures_root_closure_and_blank_neutral_tokens() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all("skills/demo").unwrap();
        fs::write(
            "skills/demo/SKILL.md",
            "Read `shared.md` completely.\n\nroot\n",
        )
        .unwrap();
        fs::write("skills/demo/shared.md", "child\n").unwrap();
        fs::write("skills/demo/conditional.md", "branch\n\n").unwrap();
        let budget = PromptSourceBudget {
            name: "demo".into(),
            roots: vec!["skills/demo/SKILL.md".into()],
            conditional_sources: vec!["skills/demo/conditional.md".into()],
            root_caps: PromptMetricCaps::default(),
            closure_caps: PromptMetricCaps {
                estimated_tokens: Some(99),
                ..PromptMetricCaps::default()
            },
            conditional_caps: PromptMetricCaps {
                content_tokens: Some(77),
                ..PromptMetricCaps::default()
            },
        };

        let result = measure_budget(&budget).unwrap();

        assert_eq!(result.root.lines, 3);
        assert_eq!(result.closure.lines, 4);
        assert_eq!(result.conditional.lines, 2);
        assert_eq!(
            source_metrics("one\n\ntwo\n").content_tokens,
            source_metrics("one\ntwo\n").content_tokens
        );
        let rows = report_rows(&budget, &result);
        assert_eq!(rows.len(), 9);
        assert_eq!(rows[4].metric, "estimated_tokens");
        assert_eq!(rows[4].cap, Some(99));
        assert_eq!(rows[8].source_set, "conditional");
        assert_eq!(rows[8].metric, "content_tokens");
        assert_eq!(rows[8].cap, Some(77));
    }

    #[test]
    #[serial_test::serial]
    fn source_relative_references_are_not_root_shadowed() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all(".claude/skills/s062/references").unwrap();
        fs::create_dir_all("references").unwrap();
        fs::write(
            ".claude/skills/s062/SKILL.md",
            "Read `references/shared.md` completely.\n",
        )
        .unwrap();
        fs::write(
            ".claude/skills/s062/references/shared.md",
            "one\ntwo\nthree\n",
        )
        .unwrap();
        fs::write("references/shared.md", "shadow\n").unwrap();
        let refs = markdown_references(
            Path::new(".claude/skills/s062/SKILL.md"),
            "Read `references/shared.md` completely.\n",
        );
        assert_eq!(
            refs,
            vec![PathBuf::from(".claude/skills/s062/references/shared.md")]
        );
    }

    #[test]
    #[serial_test::serial]
    fn decoded_link_destinations_contribute_to_prompt_closure() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all("skills/demo/references").unwrap();
        fs::write("skills/demo/references/a(b).md", "child\n").unwrap();
        let refs = markdown_references(
            Path::new("skills/demo/SKILL.md"),
            "Read [the nested guide](references/a\\(b\\).md) completely.\n",
        );
        assert_eq!(refs, vec![PathBuf::from("skills/demo/references/a(b).md")]);
    }

    #[test]
    #[serial_test::serial]
    fn prohibited_references_stay_out_of_closure() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all("skills/demo").unwrap();
        fs::write(
            "skills/demo/SKILL.md",
            "Do not read `optional.md` completely.\nline\nline\nline\nline\n",
        )
        .unwrap();
        fs::write("skills/demo/optional.md", "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n").unwrap();
        let budget = PromptSourceBudget {
            name: "demo".into(),
            roots: vec!["skills/demo/SKILL.md".into()],
            conditional_sources: vec![],
            root_caps: PromptMetricCaps::default(),
            closure_caps: PromptMetricCaps {
                lines: Some(8),
                ..PromptMetricCaps::default()
            },
            conditional_caps: PromptMetricCaps::default(),
        };
        let result = measure_budget(&budget).unwrap();
        assert_eq!(result.closure.lines, 5);
        assert_eq!(result.closure_files.len(), 1);
    }

    #[test]
    #[serial_test::serial]
    fn raw_at_imports_are_mandatory_and_source_relative() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all(".claude/skills/demo").unwrap();
        fs::write(
            ".claude/skills/demo/SKILL.md",
            "line\nline\nline\nline\n@reference.md\n",
        )
        .unwrap();
        fs::write(
            ".claude/skills/demo/reference.md",
            "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n",
        )
        .unwrap();
        fs::write("reference.md", "shadow\n").unwrap();

        let equality = PromptSourceBudget {
            name: "demo".into(),
            roots: vec![".claude/skills/demo/SKILL.md".into()],
            conditional_sources: vec![],
            root_caps: PromptMetricCaps::default(),
            closure_caps: PromptMetricCaps {
                lines: Some(15),
                ..PromptMetricCaps::default()
            },
            conditional_caps: PromptMetricCaps::default(),
        };
        let measured = measure_budget(&equality).unwrap();
        assert_eq!(measured.closure.lines, 15);
        assert_eq!(
            measured.closure_files,
            [
                PathBuf::from(".claude/skills/demo/SKILL.md"),
                PathBuf::from(".claude/skills/demo/reference.md"),
            ]
            .into_iter()
            .collect()
        );
        let over = PromptSourceBudget {
            name: "demo".into(),
            roots: vec![".claude/skills/demo/SKILL.md".into()],
            conditional_sources: vec![],
            root_caps: PromptMetricCaps::default(),
            closure_caps: PromptMetricCaps {
                lines: Some(14),
                ..PromptMetricCaps::default()
            },
            conditional_caps: PromptMetricCaps::default(),
        };
        assert!(measure_budget(&over).unwrap().closure.lines > 14);

        let refs = markdown_references(
            Path::new(".claude/skills/demo/SKILL.md"),
            "See @reference.md for details.\n",
        );
        assert_eq!(
            refs,
            vec![PathBuf::from(".claude/skills/demo/reference.md")]
        );
    }

    #[test]
    #[serial_test::serial]
    fn raw_at_link_exclusions_are_scalar_stable() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all("skills/demo").unwrap();
        fs::write("skills/demo/live.md", "live\n").unwrap();
        let source_path = Path::new("skills/demo/SKILL.md");

        for prefix in ["", "\u{e9}\u{2194}\u{1f680} "] {
            let content =
                format!("{prefix}See [@linked.md](https://example.test), then load @live.md.\n");
            let refs = markdown_references(source_path, &content);
            assert_eq!(
                refs,
                vec![PathBuf::from("skills/demo/live.md")],
                "{prefix:?}"
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn raw_at_hard_negatives_stay_out_of_closure() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::create_dir_all("skills/demo").unwrap();
        fs::write("skills/demo/optional.md", "child\n").unwrap();
        fs::write("skills/demo/notes.txt", "not markdown\n").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("optional.md", "skills/demo/link.md").unwrap();
        }

        let content = "\
---
name: demo
description: fixture
---
Do not load @optional.md
@optional.md if needed
> @optional.md
<!-- @optional.md -->
`@optional.md`
[@optional.md](@optional.md)
```text
@optional.md
```
@notes.txt
@/tmp/outside.md
@~/outside.md
@../outside.md
";
        #[cfg(unix)]
        let content = format!("{content}@link.md\n");
        fs::write("skills/demo/SKILL.md", &content).unwrap();

        let refs = markdown_references(Path::new("skills/demo/SKILL.md"), &content);
        assert!(refs.is_empty(), "unexpected refs: {refs:?}");
    }
}
