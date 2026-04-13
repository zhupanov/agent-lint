use crate::diagnostic::DiagnosticCollector;
use crate::frontmatter;
use crate::rules::LintRule;
use crate::validators::skills::{SkillInfo, collect_skills};
use regex::Regex;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// Validate skill content for public skills (skills/). Runs all 26 rules.
pub fn validate_skill_content(diag: &mut DiagnosticCollector) {
    let skills = collect_skills("skills");
    for info in &skills {
        run_content_checks(info, true, diag);
    }
    // Cross-skill checks
    validate_nested_references("skills", diag);
    validate_orphaned_skill_files("skills", diag);
}

/// Validate skill content for private skills (.claude/skills/).
/// Runs only "both-mode" rules (excludes S015, S016, S017, S029, S033).
pub fn validate_private_skill_content(diag: &mut DiagnosticCollector) {
    let skills = collect_skills(".claude/skills");
    for info in &skills {
        run_content_checks(info, false, diag);
    }
    validate_orphaned_skill_files(".claude/skills", diag);
}

fn run_content_checks(info: &SkillInfo, plugin_mode: bool, diag: &mut DiagnosticCollector) {
    check_name_format(info, plugin_mode, diag);
    check_description_quality(info, plugin_mode, diag);
    check_body_content(info, diag);
    check_frontmatter_fields(info, diag);
    check_cross_field(info, diag);
    check_content_security(info, diag);
}

// ── Name validation (S009–S013, S033) ────────────────────────────────

fn check_name_format(info: &SkillInfo, plugin_mode: bool, diag: &mut DiagnosticCollector) {
    let name = match frontmatter::get_field(&info.fm_lines, "name") {
        Some(n) => n,
        None => return, // S005 fires from existing validator
    };

    // S009: name too long
    if name.len() > 64 {
        diag.report(
            LintRule::NameTooLong,
            &format!(
                "{}: name '{}' exceeds 64 characters ({})",
                info.path,
                name,
                name.len()
            ),
        );
    }

    // S010: invalid characters
    let re_invalid = Regex::new(r"[^a-z0-9-]").unwrap();
    if re_invalid.is_match(&name) {
        diag.report(
            LintRule::NameInvalidChars,
            &format!(
                "{}: name '{}' contains characters outside [a-z0-9-]",
                info.path, name
            ),
        );
    }

    // S011: bad hyphens
    if name.starts_with('-') || name.ends_with('-') || name.contains("--") {
        diag.report(
            LintRule::NameBadHyphens,
            &format!(
                "{}: name '{}' starts/ends with hyphen or contains consecutive hyphens",
                info.path, name
            ),
        );
    }

    // S012: reserved words
    let lower = name.to_lowercase();
    if lower.contains("anthropic") || lower.contains("claude") {
        diag.report(
            LintRule::NameReservedWord,
            &format!(
                "{}: name '{}' contains reserved word ('anthropic' or 'claude')",
                info.path, name
            ),
        );
    }

    // S013: XML tags in name
    let re_xml = Regex::new(r"<[^>]+>").unwrap();
    if re_xml.is_match(&name) {
        diag.report(
            LintRule::NameHasXml,
            &format!("{}: name '{}' contains XML/HTML tags", info.path, name),
        );
    }

    // S033: vague name (plugin-only)
    if plugin_mode {
        let vague_names = [
            "helper",
            "helpers",
            "utils",
            "utility",
            "tools",
            "data",
            "files",
            "documents",
        ];
        if vague_names.contains(&name.as_str()) {
            diag.report(
                LintRule::NameVague,
                &format!(
                    "{}: name '{}' is too vague/generic for a published skill",
                    info.path, name
                ),
            );
        }
    }
}

// ── Description validation (S014–S018, S034) ─────────────────────────

fn check_description_quality(info: &SkillInfo, plugin_mode: bool, diag: &mut DiagnosticCollector) {
    let desc = match frontmatter::get_field(&info.fm_lines, "description") {
        Some(d) => d,
        None => return, // S005 fires from existing validator
    };

    // S014: description too long
    if desc.len() > 1024 {
        diag.report(
            LintRule::DescTooLong,
            &format!(
                "{}: description exceeds 1024 characters ({})",
                info.path,
                desc.len()
            ),
        );
    }

    // S034: description too short
    if desc.len() < 20 {
        diag.report(
            LintRule::DescTooShort,
            &format!(
                "{}: description is under 20 characters ({})",
                info.path,
                desc.len()
            ),
        );
    }

    // S015: description truncated in listing (plugin-only)
    if plugin_mode && desc.len() > 250 {
        diag.report(
            LintRule::DescTruncated,
            &format!(
                "{}: description exceeds 250 characters ({}) and will be truncated in skill listing",
                info.path,
                desc.len()
            ),
        );
    }

    // S016: uses first/second person (plugin-only)
    if plugin_mode {
        let re_person = Regex::new(r"(?i)\b(I|you|we|my|your|our)\b").unwrap();
        if re_person.is_match(&desc) {
            diag.report(
                LintRule::DescUsesPerson,
                &format!(
                    "{}: description uses first/second person; use third person for published skills",
                    info.path
                ),
            );
        }
    }

    // S017: no trigger context (plugin-only)
    if plugin_mode {
        let re_trigger = Regex::new(
            r"(?i)(use\s+when|use\s+this|use\s+for|trigger\s+when|do\s+not\s+trigger|\bwhen\b)",
        )
        .unwrap();
        if !re_trigger.is_match(&desc) {
            diag.report(
                LintRule::DescNoTrigger,
                &format!(
                    "{}: description lacks trigger/usage context (e.g., 'Use when...', 'Trigger when...')",
                    info.path
                ),
            );
        }
    }

    // S018: XML tags in description
    let re_xml = Regex::new(r"<[^>]+>").unwrap();
    if re_xml.is_match(&desc) {
        diag.report(
            LintRule::DescHasXml,
            &format!("{}: description contains XML/HTML tags", info.path),
        );
    }
}

// ── Body content (S019–S022) ─────────────────────────────────────────

fn check_body_content(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    // S020: empty body
    if info.body.trim().is_empty() {
        diag.report(
            LintRule::BodyEmpty,
            &format!("{}: no content after frontmatter", info.path),
        );
        return; // No point checking other body rules
    }

    // S019: body too long
    let line_count = info.body.lines().count();
    if line_count > 500 {
        diag.report(
            LintRule::BodyTooLong,
            &format!(
                "{}: body exceeds 500 lines ({} lines)",
                info.path, line_count
            ),
        );
    }

    // S021: consecutive bash code blocks
    check_consecutive_bash(info, diag);

    // S022: backslash paths
    let re_backslash = Regex::new(r"\\[a-zA-Z]").unwrap();
    // Only check outside code fences to reduce false positives
    let mut in_fence = false;
    for line in info.body.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if !in_fence && re_backslash.is_match(line) {
            diag.report(
                LintRule::BackslashPath,
                &format!(
                    "{}: Windows-style backslash path detected; use forward slashes",
                    info.path
                ),
            );
            break; // Report once per file
        }
    }
}

fn check_consecutive_bash(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    let re_bash_fence = Regex::new(r"^```(bash|sh|shell)\s*$").unwrap();

    let mut last_bash_end: Option<usize> = None;
    let mut in_fence = false;
    let mut fence_is_bash = false;

    for (i, line) in info.body.lines().enumerate() {
        let trimmed = line.trim_start();
        if !in_fence {
            if re_bash_fence.is_match(trimmed) {
                // Opening a bash fence
                if let Some(prev_end) = last_bash_end {
                    // Check if only blank lines between prev close and this open
                    let between_lines: Vec<&str> = info
                        .body
                        .lines()
                        .skip(prev_end + 1)
                        .take(i - prev_end - 1)
                        .collect();
                    let only_blank = between_lines.iter().all(|l| l.trim().is_empty());
                    if only_blank {
                        diag.report(
                            LintRule::ConsecutiveBash,
                            &format!(
                                "{}: consecutive bash code blocks (lines {} and {}) could be combined into one",
                                info.path, prev_end + 1, i + 1
                            ),
                        );
                        return; // Report once per file
                    }
                }
                in_fence = true;
                fence_is_bash = true;
            } else if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                in_fence = true;
                fence_is_bash = false;
            }
        } else if trimmed == "```" || trimmed == "~~~" {
            // Closing fence
            if fence_is_bash {
                last_bash_end = Some(i);
            }
            in_fence = false;
            fence_is_bash = false;
        }
    }
}

// ── Frontmatter field types (S023–S027) ──────────────────────────────

fn check_frontmatter_fields(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    // S023: boolean fields
    for field_name in &["user-invocable", "disable-model-invocation"] {
        match frontmatter::get_field_state(&info.fm_lines, field_name) {
            frontmatter::FieldState::Value(val) => {
                if val != "true" && val != "false" {
                    diag.report(
                        LintRule::BoolFieldInvalid,
                        &format!(
                            "{}: '{}' must be true or false, got '{}'",
                            info.path, field_name, val
                        ),
                    );
                }
            }
            frontmatter::FieldState::Empty => {
                diag.report(
                    LintRule::BoolFieldInvalid,
                    &format!(
                        "{}: '{}' is present but empty (must be true or false)",
                        info.path, field_name
                    ),
                );
            }
            frontmatter::FieldState::Missing => {} // Not required
        }
    }

    // S024: context field
    match frontmatter::get_field_state(&info.fm_lines, "context") {
        frontmatter::FieldState::Value(val) => {
            if val != "fork" {
                diag.report(
                    LintRule::ContextFieldInvalid,
                    &format!("{}: 'context' must be 'fork', got '{}'", info.path, val),
                );
            }
        }
        frontmatter::FieldState::Empty => {
            diag.report(
                LintRule::ContextFieldInvalid,
                &format!(
                    "{}: 'context' is present but empty (must be 'fork')",
                    info.path
                ),
            );
        }
        frontmatter::FieldState::Missing => {}
    }

    // S025: effort field
    match frontmatter::get_field_state(&info.fm_lines, "effort") {
        frontmatter::FieldState::Value(val) => {
            if !["low", "medium", "high", "max"].contains(&val.as_str()) {
                diag.report(
                    LintRule::EffortFieldInvalid,
                    &format!(
                        "{}: 'effort' must be low/medium/high/max, got '{}'",
                        info.path, val
                    ),
                );
            }
        }
        frontmatter::FieldState::Empty => {
            diag.report(
                LintRule::EffortFieldInvalid,
                &format!("{}: 'effort' is present but empty", info.path),
            );
        }
        frontmatter::FieldState::Missing => {}
    }

    // S026: shell field
    match frontmatter::get_field_state(&info.fm_lines, "shell") {
        frontmatter::FieldState::Value(val) => {
            if !["bash", "powershell"].contains(&val.as_str()) {
                diag.report(
                    LintRule::ShellFieldInvalid,
                    &format!(
                        "{}: 'shell' must be bash/powershell, got '{}'",
                        info.path, val
                    ),
                );
            }
        }
        frontmatter::FieldState::Empty => {
            diag.report(
                LintRule::ShellFieldInvalid,
                &format!("{}: 'shell' is present but empty", info.path),
            );
        }
        frontmatter::FieldState::Missing => {}
    }

    // S027: unreachable skill
    let dmi = frontmatter::get_field(&info.fm_lines, "disable-model-invocation");
    let ui = frontmatter::get_field(&info.fm_lines, "user-invocable");
    if dmi.as_deref() == Some("true") && ui.as_deref() == Some("false") {
        diag.report(
            LintRule::SkillUnreachable,
            &format!(
                "{}: skill is unreachable (disable-model-invocation: true and user-invocable: false)",
                info.path
            ),
        );
    }
}

// ── Cross-field checks (S028) ────────────────────────────────────────

fn check_cross_field(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    // S028: $ARGUMENTS in body without argument-hint
    let re_args = Regex::new(r"\$ARGUMENTS|\$\{ARGUMENTS\}|\$0|\$1|\$2").unwrap();
    if re_args.is_match(&info.body) && !frontmatter::field_exists(&info.fm_lines, "argument-hint") {
        diag.report(
            LintRule::ArgsNoHint,
            &format!(
                "{}: body uses $ARGUMENTS but frontmatter has no 'argument-hint' field",
                info.path
            ),
        );
    }
}

// ── Content security (S031–S032) ─────────────────────────────────────

fn check_content_security(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    if info.body.trim().is_empty() {
        return;
    }

    // S031: non-HTTPS URLs (exclude localhost, 127.0.0.1, 0.0.0.0, example.com/org)
    let re_http = Regex::new(r"http://[a-zA-Z0-9]").unwrap();
    for cap in re_http.find_iter(&info.body) {
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
    let secret_patterns = [
        r"sk-[a-zA-Z0-9]{20,}",
        r"ghp_[a-zA-Z0-9]{36,}",
        r"xox[bp]-[0-9][a-zA-Z0-9\-]{8,}",
        r#"(?i)(api[_\-]?key|api[_\-]?secret|api[_\-]?token)\s*[=:]\s*["']?[A-Za-z0-9]{20,}"#,
        r#"(?i)(password|secret|token)\s*[=:]\s*["'][^"']{8,}"#,
    ];
    for pattern in &secret_patterns {
        let re = Regex::new(pattern).unwrap();
        if re.is_match(&info.body) {
            diag.report(
                LintRule::HardcodedSecret,
                &format!("{}: potential hardcoded secret/API key detected", info.path),
            );
            return; // Report once per file
        }
    }
}

// ── Cross-skill validators (S029, S030) ──────────────────────────────

/// S029: Check for deeply nested shared markdown references.
/// Only follows canonical ${CLAUDE_PLUGIN_ROOT}/skills/shared/*.md syntax.
fn validate_nested_references(base_dir: &str, diag: &mut DiagnosticCollector) {
    let shared_dir = Path::new(base_dir).join("shared");
    if !shared_dir.is_dir() {
        return;
    }

    let re_shared =
        Regex::new(r"\$\{CLAUDE_PLUGIN_ROOT\}/skills/shared/[a-zA-Z0-9._-]+\.md").unwrap();

    let skills = collect_skills(base_dir);
    let mut visited = HashSet::new();

    for info in &skills {
        // Find shared-md references in this skill's body
        for cap in re_shared.find_iter(&info.body) {
            let reference = cap.as_str();
            let rel = reference.replace("${CLAUDE_PLUGIN_ROOT}/", "");
            let rel_path = Path::new(&rel);

            if !rel_path.is_file() {
                continue; // S008 handles missing refs
            }

            if !visited.insert(rel.clone()) {
                continue; // Already checked this file
            }

            // Read the referenced file and check for further references
            if let Ok(content) = fs::read_to_string(rel_path) {
                if re_shared.is_match(&content) {
                    diag.report(
                        LintRule::NestedRefDeep,
                        &format!(
                            "{}: references {} which itself references other shared .md files (keep references one level deep)",
                            info.path, reference
                        ),
                    );
                }
            }
        }
    }
}

/// S030: Detect orphaned files in skill scripts/ subdirectories.
fn validate_orphaned_skill_files(base_dir: &str, diag: &mut DiagnosticCollector) {
    let dir = Path::new(base_dir);
    if !dir.is_dir() {
        return;
    }

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let dir_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if dir_name == "shared" {
            continue;
        }

        let scripts_dir = path.join("scripts");
        if !scripts_dir.is_dir() {
            continue;
        }

        let skill_md = path.join("SKILL.md");
        let skill_content = match fs::read_to_string(&skill_md) {
            Ok(c) => c,
            Err(_) => continue,
        };

        // Check each file in scripts/
        let script_entries = match fs::read_dir(&scripts_dir) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for script_entry in script_entries.flatten() {
            let script_path = script_entry.path();
            if !script_path.is_file() {
                continue;
            }
            let script_name = match script_path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };

            // Check if the script file name is referenced anywhere in SKILL.md
            if !skill_content.contains(&script_name) {
                let display_path = format!("{base_dir}/{dir_name}/scripts/{script_name}");
                diag.report(
                    LintRule::OrphanedSkillFiles,
                    &format!(
                        "{}: not referenced from {base_dir}/{dir_name}/SKILL.md",
                        display_path
                    ),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::DiagnosticCollector;

    // ── S009: name-too-long ──────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s009_name_within_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\nBody content\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag);
        assert!(!diag.errors().iter().any(|e| e.contains("exceeds 64")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s009_name_too_long() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let long_name = "a".repeat(65);
        std::fs::create_dir_all(format!("skills/{long_name}")).unwrap();
        std::fs::write(
            format!("skills/{long_name}/SKILL.md"),
            format!(
                "---\nname: {long_name}\ndescription: A valid skill description here\n---\nBody\n"
            ),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag);
        assert!(diag.errors().iter().any(|e| e.contains("exceeds 64")));
    }

    // ── S010: name-invalid-chars ─────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s010_valid_name() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill-123").unwrap();
        std::fs::write(
            "skills/my-skill-123/SKILL.md",
            "---\nname: my-skill-123\ndescription: A valid skill description here\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag);
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("outside [a-z0-9-]"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s010_uppercase_name() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: My-Skill\ndescription: A valid skill description here\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag);
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("outside [a-z0-9-]"))
        );
    }

    // ── S011: name-bad-hyphens ───────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s011_consecutive_hyphens() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my--skill\ndescription: A valid skill description here\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag);
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("consecutive hyphens"))
        );
    }

    // ── S012: name-reserved-word ─────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s012_reserved_word() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: claude-helper\ndescription: A valid skill description here\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag);
        assert!(diag.errors().iter().any(|e| e.contains("reserved word")));
    }

    // ── S014: desc-too-long ──────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s014_desc_too_long() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let long_desc = "x".repeat(1025);
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: {long_desc}\n---\nBody\n"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag);
        assert!(diag.errors().iter().any(|e| e.contains("exceeds 1024")));
    }

    // ── S019: body-too-long ──────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s019_body_too_long() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let body = "line\n".repeat(501);
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!(
                "---\nname: my-skill\ndescription: A valid skill description here\n---\n{body}"
            ),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag);
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("exceeds 500 lines"))
        );
    }

    // ── S020: body-empty ─────────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s020_body_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag);
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("no content after frontmatter"))
        );
    }

    // ── S021: consecutive-bash ───────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s021_consecutive_bash_blocks() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n\n```bash\necho hello\n```\n\n```bash\necho world\n```\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag);
        assert!(diag.errors().iter().any(|e| e.contains("consecutive bash")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s021_bash_blocks_with_prose_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\n\n```bash\necho hello\n```\n\nThen run the second command:\n\n```bash\necho world\n```\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag);
        assert!(!diag.errors().iter().any(|e| e.contains("consecutive bash")));
    }

    // ── S023: bool-field-invalid ─────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s023_invalid_bool() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\nuser-invocable: yes\n---\nBody\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag);
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("must be true or false"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s023_valid_bool() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\nuser-invocable: true\n---\nBody\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag);
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("must be true or false"))
        );
    }

    // ── S027: skill-unreachable ──────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s027_unreachable_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\ndisable-model-invocation: true\nuser-invocable: false\n---\nBody\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag);
        assert!(diag.errors().iter().any(|e| e.contains("unreachable")));
    }

    // ── S028: args-no-hint ───────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s028_args_without_hint() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\nUse $ARGUMENTS as input\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag);
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("$ARGUMENTS") && e.contains("argument-hint"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s028_args_with_hint_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\nargument-hint: <feature>\n---\nUse $ARGUMENTS as input\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag);
        assert!(!diag.errors().iter().any(|e| e.contains("argument-hint")));
    }

    // ── S031: non-https-url ──────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s031_http_url() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\nFetch from http://api.example.net/data\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag);
        assert!(diag.errors().iter().any(|e| e.contains("non-HTTPS")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s031_localhost_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\nFetch from http://localhost:8080/data\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag);
        assert!(!diag.errors().iter().any(|e| e.contains("non-HTTPS")));
    }

    // ── S030: orphaned-skill-files ───────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s030_orphaned_script() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts").unwrap();
        std::fs::write("skills/my-skill/scripts/orphan.sh", "#!/bin/bash\n").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\nNo script refs\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag);
        assert!(diag.errors().iter().any(|e| e.contains("not referenced")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s030_referenced_script_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill/scripts").unwrap();
        std::fs::write("skills/my-skill/scripts/helper.sh", "#!/bin/bash\n").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\nRun helper.sh\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag);
        assert!(!diag.errors().iter().any(|e| e.contains("not referenced")));
    }

    // ── S034: desc-too-short ─────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s034_desc_too_short() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Short\n---\nBody content\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag);
        assert!(diag.errors().iter().any(|e| e.contains("under 20")));
    }

    // ── Private skill (basic mode) excludes plugin-only rules ────────

    #[test]
    #[serial_test::serial]
    fn test_private_skill_skips_plugin_only_rules() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude/skills/my-skill").unwrap();
        // This description uses "you" (would trigger S016 in plugin mode) and is >250 chars
        let long_desc = format!("Use when you need to {}", "x".repeat(250));
        std::fs::write(
            ".claude/skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: {long_desc}\n---\nBody content\n"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_private_skill_content(&mut diag);
        // S016 (person) and S015 (truncated) should NOT fire in basic mode
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("first/second person"))
        );
        assert!(!diag.errors().iter().any(|e| e.contains("truncated")));
    }

    // ── Integration: mode dispatch ───────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_integration_plugin_mode_runs_all_rules() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        // Name with uppercase (S010) + uses "you" in desc (S016)
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: My-Skill\ndescription: I help you do things and more stuff here\n---\nBody content here\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag);
        // Both S010 and S016 should fire in plugin mode
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("outside [a-z0-9-]"))
        );
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("first/second person"))
        );
    }

    // ── Integration: config round-tripping ───────────────────────────

    #[test]
    fn test_new_rules_lookup_by_code_and_name() {
        use crate::rules::LintRule;
        // Verify all 26 new rules can be looked up by code and name
        let new_rules = [
            ("S009", "name-too-long"),
            ("S010", "name-invalid-chars"),
            ("S011", "name-bad-hyphens"),
            ("S012", "name-reserved-word"),
            ("S013", "name-has-xml"),
            ("S014", "desc-too-long"),
            ("S015", "desc-truncated"),
            ("S016", "desc-uses-person"),
            ("S017", "desc-no-trigger"),
            ("S018", "desc-has-xml"),
            ("S019", "body-too-long"),
            ("S020", "body-empty"),
            ("S021", "consecutive-bash"),
            ("S022", "backslash-path"),
            ("S023", "bool-field-invalid"),
            ("S024", "context-field-invalid"),
            ("S025", "effort-field-invalid"),
            ("S026", "shell-field-invalid"),
            ("S027", "skill-unreachable"),
            ("S028", "args-no-hint"),
            ("S029", "nested-ref-deep"),
            ("S030", "orphaned-skill-files"),
            ("S031", "non-https-url"),
            ("S032", "hardcoded-secret"),
            ("S033", "name-vague"),
            ("S034", "desc-too-short"),
        ];
        for (code, name) in &new_rules {
            assert!(
                LintRule::from_code_or_name(code).is_some(),
                "Failed to look up rule by code: {code}"
            );
            assert!(
                LintRule::from_code_or_name(name).is_some(),
                "Failed to look up rule by name: {name}"
            );
            // Round-trip
            let rule = LintRule::from_code_or_name(code).unwrap();
            assert_eq!(rule.code(), *code);
            assert_eq!(rule.name(), *name);
        }
    }
}
