//! Overlapping routing-description checks for agents (A030) and skills (S074).
//!
//! Comparison is namespace-aware. Claude private and plugin surfaces that can be
//! loaded together in the same lint run share one runtime-union namespace.
//! Cursor `.cursor/agents/**/*.md` is a separate namespace when the Cursor
//! target is active. Cross-client `.agents/skills/` remains a separate
//! namespace. Agents are never compared with skills.
//!
//! Findings are pathless multi-source diagnostics with structured
//! `related_subjects`. Per-file overrides therefore cannot suppress them;
//! use global `suppress` instead.

use crate::config::ExcludeSet;
use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata};
use crate::frontmatter;
use crate::rules::LintRule;
use crate::traversal;
use crate::validators::skills::collect_skills;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use super::cursor::discover_cursor_agent_paths;

/// Character floor owned by A009 / S034. Shorter descriptions stay out of the
/// overlap pool so those rules keep ownership.
const MIN_DESC_CHARS: usize = 20;

/// Require at least this many content tokens after normalization.
const MIN_MEANINGFUL_TOKENS: usize = 4;

/// Conservative Jaccard threshold. Exact normalized duplicates always report
/// (score 1.0). Values at or above this threshold also report.
const JACCARD_THRESHOLD: f64 = 0.85;

const STOPWORDS: &[&str] = &[
    "the", "a", "an", "is", "are", "to", "for", "with", "and", "of", "in", "on", "it", "that",
    "this", "by", "from", "or", "as", "at", "be", "do", "so", "if", "no", "not", "but", "up",
    "out", "all", "can", "has", "had", "was", "were", "been", "have", "will", "would", "should",
    "could", "may", "might", "when", "you", "your", "use", "need", "needed", "using", "used",
];

const ROUTING_BOILERPLATE: &[&str] = &[
    "use when",
    "use this",
    "use for",
    "trigger when",
    "do not trigger",
];

#[derive(Debug, Clone)]
struct DescCandidate {
    path: String,
    tokens: Vec<String>,
    token_set: BTreeSet<String>,
}

/// Validate overlapping agent routing descriptions.
///
/// In Plugin mode, `agents/` and `.claude/agents/` form one Claude agent
/// routing namespace. In Basic mode only `.claude/agents/` is compared.
/// When `include_cursor` is true, `.cursor/agents/**/*.md` is compared in a
/// separate Cursor-only namespace (missing descriptions skip A030).
pub fn validate_agent_desc_overlap(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    plugin_mode: bool,
    include_cursor: bool,
) {
    let mut dirs = Vec::new();
    if plugin_mode {
        dirs.push("agents");
    }
    dirs.push(".claude/agents");
    report_overlaps(
        diag,
        collect_agent_candidates(&dirs, exclude),
        LintRule::AgentDescOverlap,
    );
    if include_cursor {
        report_overlaps(
            diag,
            collect_cursor_agent_candidates(exclude),
            LintRule::AgentDescOverlap,
        );
    }
}

/// Validate overlapping skill routing descriptions.
///
/// Claude private/plugin skill trees that are simultaneously available share
/// one namespace. `.agents/skills/` is compared only within itself when that
/// surface is active.
pub fn validate_skill_desc_overlap(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    plugin_mode: bool,
    include_agent_skills: bool,
) {
    let mut claude_dirs = Vec::new();
    if plugin_mode {
        claude_dirs.push("skills");
    }
    claude_dirs.push(".claude/skills");
    report_overlaps(
        diag,
        collect_skill_candidates(&claude_dirs, exclude),
        LintRule::SkillDescOverlap,
    );

    if include_agent_skills {
        report_overlaps(
            diag,
            collect_skill_candidates(&[".agents/skills"], exclude),
            LintRule::SkillDescOverlap,
        );
    }
}

fn collect_agent_candidates(dirs: &[&str], exclude: &ExcludeSet) -> Vec<DescCandidate> {
    let mut candidates = Vec::new();
    for dir in dirs {
        let root = Path::new(dir);
        if !root.is_dir() {
            continue;
        }
        for entry in traversal::shallow_files(root, Path::new("."), None).entries {
            let name = match entry.path.file_name().and_then(|n| n.to_str()) {
                Some(n) if n.ends_with(".md") => n,
                _ => continue,
            };
            let path = format!("{dir}/{name}");
            if exclude.is_excluded(&path) {
                continue;
            }
            let content = match fs::read_to_string(&entry.path) {
                Ok(content) => content,
                Err(_) => continue,
            };
            let Some(fm_lines) = frontmatter_lines(&content) else {
                continue;
            };
            let Some(description) = frontmatter::get_strict_string_field(&fm_lines, "description")
            else {
                continue;
            };
            if let Some(candidate) = candidate_from_description(path, &description) {
                candidates.push(candidate);
            }
        }
    }
    candidates.sort_by(|left, right| left.path.cmp(&right.path));
    candidates
}

fn collect_cursor_agent_candidates(exclude: &ExcludeSet) -> Vec<DescCandidate> {
    let mut candidates = Vec::new();
    for path in discover_cursor_agent_paths(exclude) {
        let content = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(_) => continue,
        };
        let Some(fm_lines) = frontmatter_lines(&content) else {
            continue;
        };
        // Missing, blank, non-string, invalid YAML, and non-mapping frontmatter
        // stay with CU014 and do not enter the A030 pool.
        let Some(description) = frontmatter::get_strict_string_field(&fm_lines, "description")
        else {
            continue;
        };
        if let Some(candidate) = candidate_from_description(path, &description) {
            candidates.push(candidate);
        }
    }
    candidates.sort_by(|left, right| left.path.cmp(&right.path));
    candidates
}

fn collect_skill_candidates(dirs: &[&str], exclude: &ExcludeSet) -> Vec<DescCandidate> {
    let mut candidates = Vec::new();
    for dir in dirs {
        for info in collect_skills(dir, exclude) {
            let Some(description) =
                frontmatter::get_strict_string_field(&info.fm_lines, "description")
            else {
                continue;
            };
            if let Some(candidate) = candidate_from_description(info.path, &description) {
                candidates.push(candidate);
            }
        }
    }
    candidates.sort_by(|left, right| left.path.cmp(&right.path));
    candidates
}

fn frontmatter_lines(content: &str) -> Option<Vec<String>> {
    let markdown = crate::markdown::MarkdownDocument::parse(content.to_string());
    markdown.frontmatter().map(|lines| lines.to_vec())
}

fn candidate_from_description(path: String, description: &str) -> Option<DescCandidate> {
    if description.chars().count() < MIN_DESC_CHARS {
        return None;
    }
    let tokens = normalize_description_tokens(description);
    if tokens.is_empty() {
        return None;
    }
    let token_set = tokens.iter().cloned().collect();
    Some(DescCandidate {
        path,
        tokens,
        token_set,
    })
}

fn report_overlaps(diag: &mut DiagnosticCollector, candidates: Vec<DescCandidate>, rule: LintRule) {
    for i in 0..candidates.len() {
        for j in (i + 1)..candidates.len() {
            let left = &candidates[i];
            let right = &candidates[j];
            let score = if left.tokens == right.tokens {
                1.0
            } else if left.tokens.len() < MIN_MEANINGFUL_TOKENS
                || right.tokens.len() < MIN_MEANINGFUL_TOKENS
            {
                continue;
            } else {
                let Some(score) = jaccard_similarity(&left.token_set, &right.token_set) else {
                    continue;
                };
                if score < JACCARD_THRESHOLD {
                    continue;
                }
                score
            };
            let message = format!(
                "{} and {} have overlapping routing descriptions (similarity {:.2})",
                left.path, right.path, score
            );
            diag.report_with(
                rule,
                &message,
                DiagnosticMetadata::default().with_related_subjects([&left.path, &right.path]),
            );
        }
    }
}

/// Unicode-safe lowercase tokenization with punctuation and stopword removal,
/// plus stripping of documented routing boilerplate phrases.
pub(crate) fn normalize_description_tokens(description: &str) -> Vec<String> {
    let stopwords: BTreeSet<&str> = STOPWORDS.iter().copied().collect();
    let tokens: Vec<_> = description
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .collect();
    let boilerplate: Vec<Vec<String>> = ROUTING_BOILERPLATE
        .iter()
        .map(|phrase| phrase.split_whitespace().map(str::to_string).collect())
        .collect();

    let mut normalized = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        if let Some(phrase) = boilerplate
            .iter()
            .find(|phrase| tokens[index..].starts_with(phrase))
        {
            index += phrase.len();
        } else {
            if !stopwords.contains(tokens[index].as_str()) {
                normalized.push(tokens[index].clone());
            }
            index += 1;
        }
    }
    normalized
}

/// Set-based Jaccard similarity. Returns `None` when either side is empty.
pub(crate) fn jaccard_similarity(left: &BTreeSet<String>, right: &BTreeSet<String>) -> Option<f64> {
    if left.is_empty() || right.is_empty() {
        return None;
    }
    let intersection = left.intersection(right).count();
    let union = left.union(right).count();
    if union == 0 {
        return None;
    }
    Some(intersection as f64 / union as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::DiagnosticCollector;

    #[test]
    fn normalization_strips_boilerplate_punctuation_and_stopwords() {
        let tokens =
            normalize_description_tokens("Use when reviewing pull requests for security issues!");
        assert_eq!(
            tokens,
            vec![
                "reviewing".to_string(),
                "pull".to_string(),
                "requests".to_string(),
                "security".to_string(),
                "issues".to_string(),
            ]
        );
    }

    #[test]
    fn normalization_removes_boilerplate_only_at_token_boundaries() {
        let without_comma = normalize_description_tokens(
            "Detects API misuse whenever clients retry aggressively in production",
        );
        let with_comma = normalize_description_tokens(
            "Detects API misuse, whenever clients retry aggressively in production",
        );
        assert_eq!(without_comma, with_comma);
        assert!(without_comma.contains(&"misuse".to_string()));
    }

    #[test]
    fn jaccard_table() {
        let cases = [
            (
                BTreeSet::from(["a".into(), "b".into(), "c".into()]),
                BTreeSet::from(["a".into(), "b".into(), "c".into()]),
                Some(1.0),
            ),
            (
                BTreeSet::from(["a".into(), "b".into(), "c".into(), "d".into()]),
                BTreeSet::from(["a".into(), "b".into(), "c".into(), "e".into()]),
                Some(0.6),
            ),
            (
                BTreeSet::from([
                    "create".into(),
                    "issue".into(),
                    "github".into(),
                    "tracker".into(),
                ]),
                BTreeSet::from([
                    "delete".into(),
                    "issue".into(),
                    "github".into(),
                    "tracker".into(),
                ]),
                Some(0.6),
            ),
            (BTreeSet::new(), BTreeSet::from(["a".into()]), None),
        ];
        for (left, right, expected) in cases {
            assert_eq!(jaccard_similarity(&left, &right), expected);
        }
    }

    #[test]
    #[serial_test::serial]
    fn exact_duplicate_agent_descriptions_warn_once_deterministically() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/agents").unwrap();
        let desc = "Reviews pull requests for security vulnerabilities and injection flaws";
        std::fs::write(
            ".claude/agents/alpha.md",
            format!("---\nname: alpha\ndescription: {desc}\n---\nBody\n"),
        )
        .unwrap();
        std::fs::write(
            ".claude/agents/beta.md",
            format!("---\nname: beta\ndescription: {desc}\n---\nBody\n"),
        )
        .unwrap();
        std::fs::write(
            ".claude/agents/gamma.md",
            "---\nname: gamma\ndescription: Creates GitHub issues from triage notes and templates\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_agent_desc_overlap(&mut diag, &ExcludeSet::default(), false, false);

        let findings: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::AgentDescOverlap)
            .collect();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].subject_path, None);
        assert_eq!(
            findings[0].related_subjects,
            vec![
                std::path::PathBuf::from(".claude/agents/alpha.md"),
                std::path::PathBuf::from(".claude/agents/beta.md"),
            ]
        );
        assert!(findings[0].message.contains("similarity 1.00"));
    }

    #[test]
    #[serial_test::serial]
    fn exact_duplicates_below_the_similarity_floor_still_warn() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/agents").unwrap();
        for name in ["alpha", "beta"] {
            std::fs::write(
                format!(".claude/agents/{name}.md"),
                format!(
                    "---\nname: {name}\ndescription: Reviews cybersecurity vulnerabilities\n---\nBody\n"
                ),
            )
            .unwrap();
        }

        let mut diag = DiagnosticCollector::new();
        validate_agent_desc_overlap(&mut diag, &ExcludeSet::default(), false, false);
        assert_eq!(
            diag.diagnostics()
                .iter()
                .filter(|item| item.rule == LintRule::AgentDescOverlap)
                .count(),
            1
        );
    }

    #[test]
    #[serial_test::serial]
    fn boilerplate_with_no_meaningful_tokens_never_enters_overlap_pool() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/agents").unwrap();
        for name in ["alpha", "beta"] {
            std::fs::write(
                format!(".claude/agents/{name}.md"),
                format!(
                    "---\nname: {name}\ndescription: Use when use this do not trigger\n---\nBody\n"
                ),
            )
            .unwrap();
        }

        let mut diag = DiagnosticCollector::new();
        validate_agent_desc_overlap(&mut diag, &ExcludeSet::default(), false, false);
        assert!(diag.diagnostics().is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn shared_boilerplate_alone_does_not_fire() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/skills/create-issue").unwrap();
        std::fs::create_dir_all(".claude/skills/delete-issue").unwrap();
        std::fs::write(
            ".claude/skills/create-issue/SKILL.md",
            "---\nname: create-issue\ndescription: Use when creating GitHub issues from triage notes\n---\nBody\n",
        )
        .unwrap();
        std::fs::write(
            ".claude/skills/delete-issue/SKILL.md",
            "---\nname: delete-issue\ndescription: Use when deleting GitHub issues from triage notes\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_skill_desc_overlap(&mut diag, &ExcludeSet::default(), false, false);
        assert!(
            diag.diagnostics()
                .iter()
                .all(|item| item.rule != LintRule::SkillDescOverlap),
            "create/delete distinction must remain a hard negative, got {:?}",
            diag.diagnostics()
                .iter()
                .map(|item| &item.message)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    #[serial_test::serial]
    fn plugin_and_private_agents_share_runtime_union_namespace() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("agents").unwrap();
        std::fs::create_dir_all(".claude/agents").unwrap();
        let desc = "Reviews pull requests for security vulnerabilities and injection flaws";
        std::fs::write(
            "agents/plugin.md",
            format!("---\nname: plugin\ndescription: {desc}\n---\nBody\n"),
        )
        .unwrap();
        std::fs::write(
            ".claude/agents/private.md",
            format!("---\nname: private\ndescription: {desc}\n---\nBody\n"),
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_agent_desc_overlap(&mut diag, &ExcludeSet::default(), true, false);
        assert_eq!(
            diag.diagnostics()
                .iter()
                .filter(|item| item.rule == LintRule::AgentDescOverlap)
                .count(),
            1
        );
    }

    #[test]
    #[serial_test::serial]
    fn agent_skills_namespace_stays_separate_from_claude_skills() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/skills/claude-one").unwrap();
        std::fs::create_dir_all(".agents/skills/agents-one").unwrap();
        let desc = "Reviews pull requests for security vulnerabilities and injection flaws";
        std::fs::write(
            ".claude/skills/claude-one/SKILL.md",
            format!("---\nname: claude-one\ndescription: {desc}\n---\nBody\n"),
        )
        .unwrap();
        std::fs::write(
            ".agents/skills/agents-one/SKILL.md",
            format!("---\nname: agents-one\ndescription: {desc}\n---\nBody\n"),
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_skill_desc_overlap(&mut diag, &ExcludeSet::default(), false, true);
        assert!(
            diag.diagnostics()
                .iter()
                .all(|item| item.rule != LintRule::SkillDescOverlap),
            "cross-namespace duplicates must not fire"
        );
    }

    #[test]
    #[serial_test::serial]
    fn short_descriptions_are_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/agents").unwrap();
        std::fs::write(
            ".claude/agents/a.md",
            "---\nname: a\ndescription: Short desc here!!\n---\nBody\n",
        )
        .unwrap();
        std::fs::write(
            ".claude/agents/b.md",
            "---\nname: b\ndescription: Short desc here!!\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_agent_desc_overlap(&mut diag, &ExcludeSet::default(), false, false);
        assert!(
            diag.diagnostics()
                .iter()
                .all(|item| item.rule != LintRule::AgentDescOverlap)
        );
    }

    #[test]
    #[serial_test::serial]
    fn high_overlap_below_exact_duplicate_still_warns() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/skills/left").unwrap();
        std::fs::create_dir_all(".claude/skills/right").unwrap();
        std::fs::write(
            ".claude/skills/left/SKILL.md",
            "---\nname: left\ndescription: Reviews pull requests for security vulnerabilities and injection flaws\n---\nBody\n",
        )
        .unwrap();
        std::fs::write(
            ".claude/skills/right/SKILL.md",
            "---\nname: right\ndescription: Reviews pull requests for security vulnerabilities including injection flaws\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_skill_desc_overlap(&mut diag, &ExcludeSet::default(), false, false);
        let finding = diag
            .diagnostics()
            .iter()
            .find(|item| item.rule == LintRule::SkillDescOverlap)
            .expect("high-overlap finding");
        assert!(finding.message.contains("similarity 0.88"));
    }

    #[test]
    #[serial_test::serial]
    fn pair_emission_is_stable_regardless_of_discovery_order() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/skills/z-skill").unwrap();
        std::fs::create_dir_all(".claude/skills/a-skill").unwrap();
        let desc = "Reviews pull requests for security vulnerabilities and injection flaws";
        std::fs::write(
            ".claude/skills/z-skill/SKILL.md",
            format!("---\nname: z-skill\ndescription: {desc}\n---\nBody\n"),
        )
        .unwrap();
        std::fs::write(
            ".claude/skills/a-skill/SKILL.md",
            format!("---\nname: a-skill\ndescription: {desc}\n---\nBody\n"),
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_skill_desc_overlap(&mut diag, &ExcludeSet::default(), false, false);
        let finding = diag
            .diagnostics()
            .iter()
            .find(|item| item.rule == LintRule::SkillDescOverlap)
            .expect("overlap finding");
        assert_eq!(
            finding.related_subjects,
            vec![
                std::path::PathBuf::from(".claude/skills/a-skill/SKILL.md"),
                std::path::PathBuf::from(".claude/skills/z-skill/SKILL.md"),
            ]
        );
        assert!(
            finding
                .message
                .starts_with(".claude/skills/a-skill/SKILL.md and .claude/skills/z-skill/SKILL.md")
        );
    }

    #[test]
    #[serial_test::serial]
    fn cursor_nested_duplicates_form_a_separate_namespace() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        let desc = "Reviews pull requests for security vulnerabilities and injection flaws";
        std::fs::create_dir_all(".cursor/agents/review").unwrap();
        std::fs::create_dir_all(".claude/agents").unwrap();
        std::fs::write(
            ".cursor/agents/review/one.md",
            format!("---\nname: one\ndescription: {desc}\n---\nBody\n"),
        )
        .unwrap();
        std::fs::write(
            ".cursor/agents/review/two.md",
            format!("---\nname: two\ndescription: {desc}\n---\nBody\n"),
        )
        .unwrap();
        std::fs::write(
            ".cursor/agents/review/three.md",
            "---\nname: three\ndescription: Creates GitHub issues from triage notes and templates\n---\nBody\n",
        )
        .unwrap();
        // Same description on Claude must not pair with Cursor.
        std::fs::write(
            ".claude/agents/claude-twin.md",
            format!("---\nname: claude-twin\ndescription: {desc}\n---\nBody\n"),
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_agent_desc_overlap(&mut diag, &ExcludeSet::default(), false, true);
        let findings: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::AgentDescOverlap)
            .collect();
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(
            findings[0].related_subjects,
            vec![
                std::path::PathBuf::from(".cursor/agents/review/one.md"),
                std::path::PathBuf::from(".cursor/agents/review/two.md"),
            ]
        );
    }

    #[test]
    #[serial_test::serial]
    fn cursor_missing_invalid_and_short_descriptions_skip_a030() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".cursor/agents").unwrap();
        let long = "Reviews pull requests for security vulnerabilities and injection flaws";
        std::fs::write(
            ".cursor/agents/missing.md",
            "---\nname: missing\n---\nBody\n",
        )
        .unwrap();
        std::fs::write(
            ".cursor/agents/blank.md",
            "---\nname: blank\ndescription: \"\"\n---\nBody\n",
        )
        .unwrap();
        std::fs::write(
            ".cursor/agents/short.md",
            "---\nname: short\ndescription: Too short to compare!!\n---\nBody\n",
        )
        .unwrap();
        std::fs::write(
            ".cursor/agents/typed.md",
            "---\nname: typed\ndescription: [not, a, string]\n---\nBody\n",
        )
        .unwrap();
        std::fs::write(
            ".cursor/agents/broken.md",
            "---\nname: broken\ndescription: [unclosed\n---\nBody\n",
        )
        .unwrap();
        std::fs::write(
            ".cursor/agents/ok.md",
            format!("---\nname: ok\ndescription: {long}\n---\nBody\n"),
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_agent_desc_overlap(&mut diag, &ExcludeSet::default(), false, true);
        assert!(
            diag.diagnostics()
                .iter()
                .all(|item| item.rule != LintRule::AgentDescOverlap)
        );
    }

    #[test]
    #[serial_test::serial]
    fn cursor_namespace_honors_exclusions() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        let desc = "Reviews pull requests for security vulnerabilities and injection flaws";
        std::fs::create_dir_all(".cursor/agents/review").unwrap();
        std::fs::write(
            ".cursor/agents/review/one.md",
            format!("---\nname: one\ndescription: {desc}\n---\nBody\n"),
        )
        .unwrap();
        std::fs::write(
            ".cursor/agents/review/two.md",
            format!("---\nname: two\ndescription: {desc}\n---\nBody\n"),
        )
        .unwrap();

        let exclude = ExcludeSet::new(&[".cursor/agents/review/two.md".into()]).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_agent_desc_overlap(&mut diag, &exclude, false, true);
        assert!(
            diag.diagnostics()
                .iter()
                .all(|item| item.rule != LintRule::AgentDescOverlap)
        );
    }
}
