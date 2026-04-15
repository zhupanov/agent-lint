use crate::config::ExcludeSet;
use crate::diagnostic::DiagnosticCollector;
use crate::frontmatter;
use crate::rules::LintRule;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

use super::common::RE_NAME_INVALID;

/// Jaccard similarity threshold (strict greater-than).
const JACCARD_THRESHOLD: f64 = 0.8;
/// Descriptions with fewer than this many words are eligible for Jaccard flagging.
const MIN_DESC_WORDS: usize = 6;

const STOPWORDS: &[&str] = &[
    "a", "an", "the", "is", "for", "and", "of", "to", "that", "which",
];

/// Check whether an agent description is too similar to the agent name.
///
/// Returns `true` when the description adds no meaningful information beyond
/// what the name already conveys.
fn is_desc_redundant(name: &str, desc: &str) -> bool {
    let name_lower = name.to_lowercase().replace('-', " ");
    let name_words: HashSet<&str> = name_lower.split_whitespace().collect();

    let desc_lower = desc.to_lowercase();
    // Strip leading/trailing punctuation from each token so "analyzer." matches "analyzer".
    let desc_stripped: Vec<String> = desc_lower
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_string())
        .filter(|w| !w.is_empty())
        .collect();
    let desc_word_count = desc_stripped.len();

    let stopwords: HashSet<&str> = STOPWORDS.iter().copied().collect();
    let desc_content_words: HashSet<&str> = desc_stripped
        .iter()
        .map(|w| w.as_str())
        .filter(|w| !stopwords.contains(w))
        .collect();

    // Jaccard path: flag short descriptions with high word overlap.
    if desc_word_count < MIN_DESC_WORDS && !name_words.is_empty() && !desc_content_words.is_empty()
    {
        let intersection = name_words.intersection(&desc_content_words).count();
        let union = name_words.union(&desc_content_words).count();
        if union > 0 {
            let jaccard = intersection as f64 / union as f64;
            if jaccard > JACCARD_THRESHOLD {
                return true;
            }
        }
    }

    // Token containment path: flag if all name words appear in the description
    // and the description adds at most one content word beyond the name
    // (catching filler like "tool", "agent", "helper" without listing them).
    // Require at least 2 name words to avoid false positives on single-word names
    // (e.g., name "code" with desc "code reviewer" is a valid description).
    if name_words.len() >= 2 && name_words.is_subset(&desc_content_words) {
        let extra_content = desc_content_words.difference(&name_words).count();
        if extra_content <= 1 {
            return true;
        }
    }

    false
}

/// V7: Validate agents/*.md frontmatter.
pub fn validate_agents(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let agents_dir = Path::new("agents");
    if !agents_dir.is_dir() {
        diag.report(LintRule::AgentsDirMissing, "agents/ directory is missing");
        return;
    }

    let mut found = 0;
    let mut excluded_count = 0;
    let entries = match fs::read_dir(agents_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) if n.ends_with(".md") => n.to_string(),
            _ => continue,
        };

        let agent_path = format!("agents/{name}");
        if exclude.is_excluded(&agent_path) {
            excluded_count += 1;
            continue;
        }

        found += 1;
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let fm_lines = match frontmatter::extract_frontmatter(&content) {
            Some(lines) => lines,
            None => {
                diag.report(
                    LintRule::AgentFrontmatterMalformed,
                    &format!(
                        "{agent_path}: malformed frontmatter (must start with '---' on line 1, must have closing '---')"
                    ),
                );
                continue;
            }
        };

        let fm_name = frontmatter::get_field(&fm_lines, "name");
        let fm_desc = frontmatter::get_field(&fm_lines, "description");

        if fm_name.is_none() {
            diag.report(
                LintRule::AgentFieldMissing,
                &format!("{agent_path}: missing required frontmatter field 'name'"),
            );
        }
        if fm_desc.is_none() {
            diag.report(
                LintRule::AgentFieldMissing,
                &format!("{agent_path}: missing required frontmatter field 'description'"),
            );
        }

        // A008: agent description too long
        // A009: agent description too short
        if let Some(ref desc) = fm_desc {
            let char_count = desc.chars().count();
            if char_count > 1024 {
                diag.report(
                    LintRule::AgentDescLong,
                    &format!("{agent_path}: description exceeds 1024 characters ({char_count})"),
                );
            }
            if char_count < 20 {
                diag.report(
                    LintRule::AgentDescShort,
                    &format!("{agent_path}: description is under 20 characters ({char_count})"),
                );
            }
        }

        // A011: agent description too similar to agent name
        if let Some(ref n) = fm_name {
            if let Some(ref desc) = fm_desc {
                if is_desc_redundant(n, desc) {
                    diag.report(
                        LintRule::AgentDescRedundant,
                        &format!(
                            "{agent_path}: description is too similar to the agent name '{n}'"
                        ),
                    );
                }
            }
        }

        // A010: agent name invalid characters
        if let Some(ref n) = fm_name {
            if RE_NAME_INVALID.is_match(n) {
                diag.report(
                    LintRule::AgentNameInvalid,
                    &format!(
                        "{agent_path}: name '{}' contains characters outside [a-z0-9-]",
                        n
                    ),
                );
            }
        }
    }

    if found == 0 && excluded_count == 0 {
        diag.report(LintRule::NoAgentFiles, "agents/ has no .md files");
    }
}

/// V16: Agent-template alignment — every agents/*.md must contain
/// "Derived from" marker referencing reviewer-templates.md.
/// (Larch-specific convention check.)
pub fn validate_agent_template_alignment(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let agents_dir = Path::new("agents");
    let templates = Path::new("skills/shared/reviewer-templates.md");

    if !agents_dir.is_dir() {
        return;
    }
    if !templates.is_file() {
        diag.report(
            LintRule::TemplateFileMissing,
            &format!("reviewer-templates.md missing: {}", templates.display()),
        );
        return;
    }

    let entries = match fs::read_dir(agents_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) if n.ends_with(".md") => n.to_string(),
            _ => continue,
        };

        let agent_path = format!("agents/{name}");
        if exclude.is_excluded(&agent_path) {
            continue;
        }

        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let has_marker = content.lines().any(|line| {
            let lower = line.to_lowercase();
            lower.contains("derived from") && lower.contains("reviewer-templates.md")
        });

        if !has_marker {
            diag.report(
                LintRule::TemplateMarkerMissing,
                &format!(
                    "agents/{name} missing 'Derived from skills/shared/reviewer-templates.md' marker"
                ),
            );
        }
    }
}

/// V21: Agent-template count — number of ## Reviewer sections in
/// skills/shared/reviewer-templates.md must equal number of agents/*.md files.
/// (Larch-specific convention check.)
pub fn validate_agent_template_count(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let agents_dir = Path::new("agents");
    let templates = Path::new("skills/shared/reviewer-templates.md");

    if !agents_dir.is_dir() || !templates.is_file() {
        return; // V16 catches missing template
    }

    // Count ## Reviewer sections
    let template_content = match fs::read_to_string(templates) {
        Ok(c) => c,
        Err(_) => return,
    };
    let template_count = template_content
        .lines()
        .filter(|line| line.starts_with("## Reviewer"))
        .count();

    // Count agents/*.md files
    let mut agent_count = 0;
    if let Ok(entries) = fs::read_dir(agents_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.ends_with(".md") {
                        let agent_path = format!("agents/{name}");
                        if !exclude.is_excluded(&agent_path) {
                            agent_count += 1;
                        }
                    }
                }
            }
        }
    }

    if template_count != agent_count {
        diag.report(
            LintRule::TemplateCountMismatch,
            &format!(
                "agent-template count mismatch: {agent_count} agent file(s) but {template_count} '## Reviewer' section(s) in {}",
                templates.display()
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::DiagnosticCollector;

    // V7: validate_agents
    #[test]
    #[serial_test::serial]
    fn test_v7_valid_agents() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("agents").unwrap();
        std::fs::write(
            "agents/general.md",
            "---\nname: general\ndescription: General reviewer for code quality analysis\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v7_missing_agents_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("agents/ directory is missing"));
    }

    #[test]
    #[serial_test::serial]
    fn test_v7_empty_agents_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("agents").unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("no .md files"));
    }

    #[test]
    #[serial_test::serial]
    fn test_v7_missing_frontmatter_name() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("agents").unwrap();
        std::fs::write(
            "agents/general.md",
            "---\ndescription: General reviewer for code quality analysis\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.error_count() >= 1);
        assert!(diag.errors().iter().any(|e| e.contains("name")));
    }

    // V16: validate_agent_template_alignment
    #[test]
    #[serial_test::serial]
    fn test_v16_valid_alignment() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("agents").unwrap();
        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::write("skills/shared/reviewer-templates.md", "# Templates\n").unwrap();
        std::fs::write(
            "agents/general.md",
            "---\nname: general\ndescription: desc\n---\nDerived from skills/shared/reviewer-templates.md\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agent_template_alignment(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v16_missing_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("agents").unwrap();
        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::write("skills/shared/reviewer-templates.md", "# Templates\n").unwrap();
        std::fs::write(
            "agents/general.md",
            "---\nname: general\ndescription: desc\n---\nNo marker here\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agent_template_alignment(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("missing"));
    }

    // V21: validate_agent_template_count
    #[test]
    #[serial_test::serial]
    fn test_v21_matching_count() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("agents").unwrap();
        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::write(
            "skills/shared/reviewer-templates.md",
            "## Reviewer 1\nContent\n## Reviewer 2\nContent\n",
        )
        .unwrap();
        std::fs::write("agents/one.md", "---\nname: one\n---\n").unwrap();
        std::fs::write("agents/two.md", "---\nname: two\n---\n").unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agent_template_count(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v21_mismatched_count() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("agents").unwrap();
        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::write(
            "skills/shared/reviewer-templates.md",
            "## Reviewer 1\nContent\n## Reviewer 2\nContent\n",
        )
        .unwrap();
        std::fs::write("agents/one.md", "---\nname: one\n---\n").unwrap();
        // Only 1 agent but 2 templates

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agent_template_count(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("mismatch"));
    }

    // A008: agent-desc-long
    #[test]
    #[serial_test::serial]
    fn test_a008_desc_too_long() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        let long_desc = "x".repeat(1025);
        std::fs::write(
            "agents/general.md",
            format!("---\nname: general\ndescription: {long_desc}\n---\nBody\n"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("exceeds 1024")));
    }

    // A009: agent-desc-short
    #[test]
    #[serial_test::serial]
    fn test_a009_desc_too_short() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::write(
            "agents/general.md",
            "---\nname: general\ndescription: Short\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("under 20")));
    }

    #[test]
    #[serial_test::serial]
    fn test_a008_boundary_1024_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        let desc = format!("Use when testing {}", "x".repeat(1007));
        assert_eq!(desc.chars().count(), 1024);
        std::fs::write(
            "agents/general.md",
            format!("---\nname: general\ndescription: {desc}\n---\nBody\n"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("exceeds 1024")));
    }

    #[test]
    #[serial_test::serial]
    fn test_a008_multibyte_chars_count_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        // 1025 CJK characters (3 bytes each) = 3075 bytes but only 1025 chars
        let desc = "\u{4e00}".repeat(1025);
        assert_eq!(desc.chars().count(), 1025);
        assert!(desc.len() > 1025); // bytes > chars
        std::fs::write(
            "agents/general.md",
            format!("---\nname: general\ndescription: {desc}\n---\nBody\n"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("exceeds 1024")));
    }

    #[test]
    #[serial_test::serial]
    fn test_a009_boundary_20_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        let desc = "Use when needed now!";
        assert_eq!(desc.chars().count(), 20);
        std::fs::write(
            "agents/general.md",
            format!("---\nname: general\ndescription: {desc}\n---\nBody\n"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("under 20")));
    }

    #[test]
    #[serial_test::serial]
    fn test_a009_multibyte_chars_count_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        // 19 CJK characters (3 bytes each) = 57 bytes but only 19 chars
        let desc = "\u{4e00}".repeat(19);
        assert_eq!(desc.chars().count(), 19);
        assert!(desc.len() > 19); // bytes > chars
        std::fs::write(
            "agents/general.md",
            format!("---\nname: general\ndescription: {desc}\n---\nBody\n"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("under 20")));
    }

    // A010: agent-name-invalid
    #[test]
    #[serial_test::serial]
    fn test_a010_name_invalid() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::write(
            "agents/general.md",
            "---\nname: My_Agent\ndescription: A valid agent description here\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("outside [a-z0-9-]"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_a010_valid_name_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::write(
            "agents/general.md",
            "---\nname: general-reviewer\ndescription: A valid agent description here\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("outside [a-z0-9-]"))
        );
    }

    // ── A011: agent-desc-redundant ──────────────────────────────────

    // Unit tests for is_desc_redundant helper
    #[test]
    fn test_is_desc_redundant_exact_token_match() {
        // Name tokens match desc content tokens exactly
        assert!(is_desc_redundant("code-analyzer", "A code analyzer agent"));
    }

    #[test]
    fn test_is_desc_redundant_with_only_stopwords_extra() {
        // Description has name words + only stopwords → redundant
        assert!(is_desc_redundant("test-runner", "The test runner tool"));
    }

    #[test]
    fn test_is_desc_redundant_descriptive_passes() {
        // Description adds meaningful content beyond the name
        assert!(!is_desc_redundant(
            "security-reviewer",
            "Reviews code for security vulnerabilities and auth flaws"
        ));
    }

    #[test]
    fn test_is_desc_redundant_short_name_with_context_passes() {
        // Short name with a descriptive description should NOT fire
        assert!(!is_desc_redundant(
            "api",
            "API helper tool for REST requests"
        ));
    }

    #[test]
    fn test_is_desc_redundant_overlapping_but_distinct() {
        assert!(!is_desc_redundant(
            "code-reviewer",
            "Performs deep analysis of code for bugs and security issues"
        ));
    }

    #[test]
    fn test_is_desc_redundant_token_containment_with_extra_content_passes() {
        // Name tokens present but description adds extra content words
        assert!(!is_desc_redundant(
            "test-runner",
            "Test runner for CI integration workflows"
        ));
    }

    #[test]
    fn test_is_desc_redundant_boundary_extra_words() {
        // 1 extra content word beyond name → fires (token containment)
        assert!(is_desc_redundant("code-analyzer", "code analyzer tool"));
        // 2 extra content words → does not fire
        assert!(!is_desc_redundant(
            "code-analyzer",
            "code analyzer tool agent"
        ));
    }

    #[test]
    fn test_is_desc_redundant_low_jaccard_passes() {
        // Low word overlap should not fire even with short desc.
        // name={code,analyzer}, desc={static,analysis,tool} → jaccard=0/5=0
        assert!(!is_desc_redundant("code-analyzer", "Static analysis tool"));
    }

    #[test]
    fn test_is_desc_redundant_punctuation_stripped() {
        // Trailing punctuation should be stripped — "analyzer." matches "analyzer"
        assert!(is_desc_redundant("code-analyzer", "A code analyzer."));
        // Comma after word should also be stripped
        assert!(is_desc_redundant("code-analyzer", "code analyzer, tool"));
    }

    #[test]
    fn test_is_desc_redundant_single_word_name_no_false_positive() {
        // Single-word name with meaningful two-word description should NOT fire
        assert!(!is_desc_redundant("code", "code reviewer"));
        assert!(!is_desc_redundant("api", "api gateway"));
    }

    #[test]
    fn test_is_desc_redundant_existing_fixture_safe() {
        // Existing test fixture should NOT fire A011
        assert!(!is_desc_redundant(
            "general",
            "General reviewer for code quality analysis"
        ));
    }

    // Integration tests through validate_agents
    #[test]
    #[serial_test::serial]
    fn test_a011_redundant_desc_fires() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::write(
            "agents/analyzer.md",
            "---\nname: code-analyzer\ndescription: A code analyzer agent\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("too similar to the agent name"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_a011_descriptive_desc_passes() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::write(
            "agents/reviewer.md",
            "---\nname: security-reviewer\ndescription: Reviews code for security vulnerabilities including injection and XSS flaws\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("too similar to the agent name"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_a011_existing_fixture_no_false_positive() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::write(
            "agents/general.md",
            "---\nname: general\ndescription: General reviewer for code quality analysis\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("too similar to the agent name")),
            "Existing fixture should not trigger A011"
        );
    }
}
