//! Platform-neutral validation for shared `AGENTS.md` instruction files.

use crate::config::ExcludeSet;
use crate::diagnostic::DiagnosticCollector;
use crate::rules::LintRule;
use crate::traversal;
use crate::validators::skill_content::security::has_hardcoded_secret;
use std::path::Path;

const CODEX_DEFAULT_MAX_BYTES: usize = 32_768;
const CODEX_HARD_MAX_BYTES: usize = 100_000;

/// Validate every included `AGENTS.md`, applying Codex policy only when Codex is active.
pub fn validate_agents_files(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    codex_active: bool,
) {
    let codex_max_bytes = codex_active.then(|| project_doc_max_bytes(exclude));
    for entry in traversal::recursive_files(Path::new("."), Path::new("."), Some(exclude)).entries {
        if entry
            .path
            .file_name()
            .is_none_or(|name| name != "AGENTS.md")
        {
            continue;
        }
        let path = &entry.path;
        let display = entry.display;
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };

        validate_shared_rules(diag, path, &display, &content);
        if let Some(max_bytes) = codex_max_bytes {
            validate_codex_rules(diag, exclude, &display, &content, max_bytes);
        }
    }
}

fn validate_shared_rules(
    diag: &mut DiagnosticCollector,
    path: &Path,
    display: &str,
    content: &str,
) {
    if content.trim().is_empty() {
        diag.report(
            LintRule::InstructionFileEmpty,
            &format!("{display} is empty or whitespace-only"),
        );
    }
    if has_hardcoded_secret(content) {
        diag.report(
            LintRule::InstructionFileSecret,
            &format!("{display} contains a potential hardcoded secret/API key"),
        );
    }
    validate_inline_paths(diag, path, display, content);
    if is_generic_guidance(content) {
        diag.report(LintRule::InstructionFileGenericGuidance, &format!("{display} contains only generic guidance; add project-specific commands, paths, or constraints"));
    }
    if lacks_project_structure(content) {
        diag.report(
            LintRule::InstructionFileMissingStructure,
            &format!(
                "{display} lacks project-specific headings, commands, paths, or sufficient detail"
            ),
        );
    }
}

fn validate_codex_rules(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    display: &str,
    content: &str,
    max_bytes: usize,
) {
    if content.len() > CODEX_HARD_MAX_BYTES {
        diag.report(
            LintRule::CodexAgentsTooLarge,
            &format!(
                "{display} exceeds Codex's {CODEX_HARD_MAX_BYTES}-byte hard limit ({} bytes)",
                content.len()
            ),
        );
    }
    if content.len() > max_bytes {
        diag.report(LintRule::CodexAgentsDocLimit, &format!("{display} exceeds Codex's effective project document limit of {max_bytes} bytes ({} bytes)", content.len()));
    }
    if agents_conflicts_with_config(content, exclude) {
        diag.report(
            LintRule::CodexAgentsConfigConflict,
            &format!("{display} explicitly contradicts a value in .codex/config.toml"),
        );
    }
}

fn project_doc_max_bytes(exclude: &ExcludeSet) -> usize {
    if exclude.is_excluded(".codex/config.toml") {
        return CODEX_DEFAULT_MAX_BYTES;
    }
    std::fs::read_to_string(".codex/config.toml")
        .ok()
        .and_then(|content| content.parse::<toml::Value>().ok())
        .and_then(|value| {
            value
                .get("project_doc_max_bytes")
                .and_then(toml::Value::as_integer)
        })
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
        .unwrap_or(CODEX_DEFAULT_MAX_BYTES)
}

fn validate_inline_paths(
    diag: &mut DiagnosticCollector,
    agents_path: &Path,
    display: &str,
    content: &str,
) {
    for reference in backtick_tokens(content) {
        if !looks_like_path(reference) {
            continue;
        }
        let candidate = agents_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(reference);
        if !candidate.is_file() && !candidate.is_dir() {
            diag.report(
                LintRule::InstructionFilePathMissing,
                &format!("{display} references missing inline-code path `{reference}`"),
            );
            break;
        }
    }
}

fn backtick_tokens(content: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut rest = content;
    while let Some(start) = rest.find('`') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('`') else { break };
        let token = after[..end].trim();
        if !token.is_empty() && !token.contains(char::is_whitespace) {
            result.push(token);
        }
        rest = &after[end + 1..];
    }
    result
}

fn looks_like_path(token: &str) -> bool {
    !["http://", "https://", "$", "<"]
        .iter()
        .any(|prefix| token.starts_with(prefix))
        && (token.starts_with('.') || token.contains('/') || token.rsplit_once('.').is_some())
}

fn is_generic_guidance(content: &str) -> bool {
    let normalized = content.trim().to_ascii_lowercase();
    normalized.len() < 120
        && !normalized.contains(['`', '/', '\\'])
        && [
            "be helpful",
            "be accurate",
            "write good code",
            "follow best practices",
        ]
        .iter()
        .any(|phrase| normalized.contains(phrase))
}

fn lacks_project_structure(content: &str) -> bool {
    let trimmed = content.trim();
    !trimmed.is_empty()
        && trimmed.len() < 200
        && !trimmed.contains("# ")
        && !trimmed.contains('`')
        && !trimmed.contains(['/', '\\'])
}

fn agents_conflicts_with_config(content: &str, exclude: &ExcludeSet) -> bool {
    if exclude.is_excluded(".codex/config.toml") {
        return false;
    }
    let Ok(config) = std::fs::read_to_string(".codex/config.toml") else {
        return false;
    };
    let Ok(value) = config.parse::<toml::Value>() else {
        return false;
    };
    for key in ["approval_policy", "sandbox_mode", "project_doc_max_bytes"] {
        let Some(config_value) = value.get(key) else {
            continue;
        };
        let config_value = config_value.to_string().trim_matches('"').to_string();
        for line in content.lines() {
            let normalized = line.replace(['`', '"', '\''], "");
            let Some((mentioned_key, mentioned_value)) = normalized.split_once('=') else {
                continue;
            };
            if mentioned_key.trim() == key && mentioned_value.trim() != config_value {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[serial_test::serial]
    fn shared_rules_run_without_codex_policy() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("nested/generic").unwrap();
        std::fs::write(
            "AGENTS.md",
            format!(
                "# Instructions\ntoken = sk-12345678901234567890\nSee `missing.md`.\n{}",
                "x".repeat(CODEX_DEFAULT_MAX_BYTES)
            ),
        )
        .unwrap();
        std::fs::write("nested/AGENTS.md", " \n\t").unwrap();
        std::fs::write(
            "nested/generic/AGENTS.md",
            "Be helpful and write good code.",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents_files(&mut diag, &ExcludeSet::default(), false);

        for rule in [
            LintRule::InstructionFileEmpty,
            LintRule::InstructionFileSecret,
            LintRule::InstructionFilePathMissing,
            LintRule::InstructionFileGenericGuidance,
            LintRule::InstructionFileMissingStructure,
        ] {
            assert!(
                diag.diagnostics().iter().any(|item| item.rule == rule),
                "missing {}",
                rule.code()
            );
        }
        assert!(!diag.diagnostics().iter().any(|item| matches!(
            item.rule,
            LintRule::CodexAgentsTooLarge
                | LintRule::CodexAgentsDocLimit
                | LintRule::CodexAgentsConfigConflict
        )));
    }

    #[test]
    #[serial_test::serial]
    fn codex_policy_uses_effective_limit_and_config() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir(".codex").unwrap();
        std::fs::create_dir("nested").unwrap();
        std::fs::write(
            ".codex/config.toml",
            "project_doc_max_bytes = 100\napproval_policy = \"never\"\n",
        )
        .unwrap();
        std::fs::write(
            "nested/AGENTS.md",
            format!(
                "# Instructions\napproval_policy = \"on-request\"\n{}",
                "x".repeat(CODEX_HARD_MAX_BYTES + 1)
            ),
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents_files(&mut diag, &ExcludeSet::default(), true);

        assert!(diag.diagnostics().iter().any(|item| {
            item.rule == LintRule::CodexAgentsDocLimit && item.message.contains("nested/AGENTS.md")
        }));
        assert!(
            diag.diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::CodexAgentsTooLarge)
        );
        assert!(
            diag.diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::CodexAgentsConfigConflict)
        );
    }

    #[test]
    #[serial_test::serial]
    fn excluded_codex_config_does_not_affect_agents_policy() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir(".codex").unwrap();
        std::fs::write(
            ".codex/config.toml",
            "project_doc_max_bytes = 100\napproval_policy = \"never\"\n",
        )
        .unwrap();
        std::fs::write(
            "AGENTS.md",
            format!(
                "# Instructions\napproval_policy = \"on-request\"\n{}",
                "x".repeat(100)
            ),
        )
        .unwrap();
        let exclude = ExcludeSet::new(&[".codex/config.toml".into()]).unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_agents_files(&mut diag, &exclude, true);

        assert!(!diag.diagnostics().iter().any(|item| matches!(
            item.rule,
            LintRule::CodexAgentsDocLimit | LintRule::CodexAgentsConfigConflict
        )));
    }
}
