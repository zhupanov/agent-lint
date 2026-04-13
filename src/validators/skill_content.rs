use crate::config::ExcludeSet;
use crate::diagnostic::DiagnosticCollector;
use crate::frontmatter;
use crate::rules::LintRule;
use crate::validators::skills::{SkillInfo, collect_skills};
use regex::Regex;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// Validate skill content for public skills (skills/). Runs all S009–S043 rules.
pub fn validate_skill_content(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let skills = collect_skills("skills", exclude);
    for info in &skills {
        run_content_checks(info, true, diag);
    }
    // Cross-skill checks
    validate_nested_references("skills", diag, exclude);
    validate_orphaned_skill_files("skills", diag, exclude);
    validate_ref_no_toc("skills", diag, exclude);
}

/// Validate skill content for private skills (.claude/skills/).
/// Runs only "both-mode" rules (excludes S015, S016, S017, S029, S033, S036, S037, S038).
pub fn validate_private_skill_content(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let skills = collect_skills(".claude/skills", exclude);
    for info in &skills {
        run_content_checks(info, false, diag);
    }
    validate_orphaned_skill_files(".claude/skills", diag, exclude);
}

fn run_content_checks(info: &SkillInfo, plugin_mode: bool, diag: &mut DiagnosticCollector) {
    check_name_format(info, plugin_mode, diag);
    check_description_quality(info, plugin_mode, diag);
    check_body_content(info, plugin_mode, diag);
    check_frontmatter_fields(info, diag);
    check_frontmatter_extended(info, diag);
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

fn check_body_content(info: &SkillInfo, plugin_mode: bool, diag: &mut DiagnosticCollector) {
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

    // S022: backslash paths — require path-like context to avoid false positives
    // on regex escapes (\s, \n, \t), LaTeX (\frac), etc.
    // Matches: C:\Users, \dir\file, path\to\something (letter, backslash, letter pattern)
    let re_backslash =
        Regex::new(r"[A-Za-z]:\\[A-Za-z]|\\[A-Za-z][A-Za-z0-9_-]*\\[A-Za-z]").unwrap();
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

    // S037: body-no-refs (plugin-only) — body > 300 lines with no file references
    if plugin_mode && line_count > 300 {
        let re_ref = Regex::new(
            r"\$\{CLAUDE_PLUGIN_ROOT\}|\.sh\b|\.md\b|\.py\b|\.js\b|\.ts\b|scripts/|shared/",
        )
        .unwrap();
        if !re_ref.is_match(&info.body) {
            diag.report(
                LintRule::BodyNoRefs,
                &format!(
                    "{}: body exceeds 300 lines ({}) with no file references; consider splitting into reference files",
                    info.path, line_count
                ),
            );
        }
    }

    // S038: time-sensitive (plugin-only) — date/year patterns outside code fences
    if plugin_mode {
        let re_year = Regex::new(r"\b20[2-3][0-9]\b").unwrap();
        let mut in_code = false;
        for line in info.body.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                in_code = !in_code;
                continue;
            }
            if !in_code && re_year.is_match(line) {
                diag.report(
                    LintRule::TimeSensitive,
                    &format!(
                        "{}: body contains date/year pattern that may become outdated",
                        info.path
                    ),
                );
                break; // Report once per file
            }
        }
    }

    // S041: fork-no-task — context: fork set but no task instructions in body
    if frontmatter::get_field(&info.fm_lines, "context").as_deref() == Some("fork") {
        let re_imperative = Regex::new(
            r"(?i)\b(run|execute|create|build|generate|invoke|call|launch|start|perform|apply|install|deploy|write|implement)\b",
        )
        .unwrap();
        if !re_imperative.is_match(&info.body) {
            diag.report(
                LintRule::ForkNoTask,
                &format!(
                    "{}: context: fork is set but body has no task instructions (fork subagent needs an actionable prompt)",
                    info.path
                ),
            );
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

// ── Extended frontmatter checks (S035, S039, S040, S042, S043) ───────

fn check_frontmatter_extended(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    // S035: compatibility field too long
    if let frontmatter::FieldState::Value(val) =
        frontmatter::get_field_state(&info.fm_lines, "compatibility")
    {
        if val.len() > 500 {
            diag.report(
                LintRule::CompatTooLong,
                &format!(
                    "{}: 'compatibility' exceeds 500 characters ({})",
                    info.path,
                    val.len()
                ),
            );
        }
    }

    // S039: metadata values should be strings
    // Look for metadata lines in frontmatter that have bare true/false/numeric values
    let mut in_metadata = false;
    for line in &info.fm_lines {
        if line == "metadata:" || line.starts_with("metadata:") {
            // Check for inline scalar value on the metadata: line itself
            let inline_val = line["metadata:".len()..].trim();
            if !inline_val.is_empty()
                && !inline_val.starts_with('"')
                && !inline_val.starts_with('\'')
                && (inline_val == "true"
                    || inline_val == "false"
                    || inline_val.parse::<f64>().is_ok())
            {
                diag.report(
                    LintRule::MetadataNotString,
                    &format!(
                        "{}: metadata has non-string inline value '{}' (wrap in quotes)",
                        info.path, inline_val
                    ),
                );
            }
            in_metadata = true;
            continue;
        }
        if in_metadata {
            // Metadata entries are indented (e.g., "  key: value")
            if !line.starts_with(' ') && !line.starts_with('\t') {
                break; // End of metadata block
            }
            if let Some(colon_pos) = line.find(':') {
                let val = line[colon_pos + 1..].trim();
                if !val.is_empty()
                    && !val.starts_with('"')
                    && !val.starts_with('\'')
                    && (val == "true" || val == "false" || val.parse::<f64>().is_ok())
                {
                    let key = line[..colon_pos].trim();
                    diag.report(
                        LintRule::MetadataNotString,
                        &format!(
                            "{}: metadata key '{}' has non-string value '{}' (wrap in quotes)",
                            info.path, key, val
                        ),
                    );
                }
            }
        }
    }

    // S040: allowed-tools unknown
    if let Some(tools_str) = frontmatter::get_field(&info.fm_lines, "allowed-tools") {
        let known_tools = [
            "AskUserQuestion",
            "Bash",
            "Read",
            "Edit",
            "Write",
            "Grep",
            "Glob",
            "Agent",
            "Task",
            "WebFetch",
            "WebSearch",
            "Skill",
            "NotebookEdit",
            "LSP",
            "TaskCreate",
            "TaskUpdate",
            "TaskList",
            "TaskGet",
            "TaskStop",
            "TaskOutput",
        ];
        for tool in tools_str.split(',') {
            let tool = tool.trim();
            // Skip tool patterns like "Bash(git *)" — extract base name
            let base_name = if let Some(paren) = tool.find('(') {
                tool[..paren].trim()
            } else {
                tool
            };
            if base_name.is_empty() {
                continue;
            }
            if !known_tools.contains(&base_name) {
                diag.report(
                    LintRule::ToolsUnknown,
                    &format!(
                        "{}: allowed-tools lists unrecognized tool '{}' (tool names are case-sensitive PascalCase; may be an MCP tool — verify spelling)",
                        info.path, base_name
                    ),
                );
            }
        }
    }

    // S042: disable-model-invocation: true with empty/missing description
    if frontmatter::get_field(&info.fm_lines, "disable-model-invocation").as_deref() == Some("true")
    {
        match frontmatter::get_field_state(&info.fm_lines, "description") {
            frontmatter::FieldState::Missing | frontmatter::FieldState::Empty => {
                diag.report(
                    LintRule::DmiEmptyDesc,
                    &format!(
                        "{}: disable-model-invocation: true but description is empty/missing (user-only skills need descriptions for the / menu)",
                        info.path
                    ),
                );
            }
            frontmatter::FieldState::Value(_) => {}
        }
    }

    // S043: backslash paths in frontmatter
    let re_fm_backslash =
        Regex::new(r"[A-Za-z]:\\[A-Za-z]|\\[A-Za-z][A-Za-z0-9_-]*\\[A-Za-z]").unwrap();
    for line in &info.fm_lines {
        if re_fm_backslash.is_match(line) {
            diag.report(
                LintRule::FrontmatterBackslash,
                &format!(
                    "{}: Windows-style backslash path in frontmatter; use forward slashes",
                    info.path
                ),
            );
            break; // Report once per file
        }
    }
}

// ── Cross-field checks (S028) ────────────────────────────────────────

fn check_cross_field(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    // S028: $ARGUMENTS in body without argument-hint
    let re_args = Regex::new(r"\$ARGUMENTS|\$\{ARGUMENTS\}").unwrap();
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
fn validate_nested_references(
    base_dir: &str,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
) {
    let shared_dir = Path::new(base_dir).join("shared");
    if !shared_dir.is_dir() {
        return;
    }

    let re_shared =
        Regex::new(r"\$\{CLAUDE_PLUGIN_ROOT\}/skills/shared/[a-zA-Z0-9._-]+\.md").unwrap();

    let skills = collect_skills(base_dir, exclude);
    // Cache: which shared .md files are nested (avoids re-reading files from disk)
    let mut checked: HashSet<String> = HashSet::new();
    let mut nested: HashSet<String> = HashSet::new();

    for info in &skills {
        // Find shared-md references in this skill's body
        for cap in re_shared.find_iter(&info.body) {
            let reference = cap.as_str();
            let rel = reference.replace("${CLAUDE_PLUGIN_ROOT}/", "");
            let rel_path = Path::new(&rel);

            if !rel_path.is_file() {
                continue; // S008 handles missing refs
            }

            // Check the file once for nesting, cache result
            if !checked.contains(&rel) {
                checked.insert(rel.clone());
                if let Ok(content) = fs::read_to_string(rel_path) {
                    if re_shared.is_match(&content) {
                        nested.insert(rel.clone());
                    }
                }
            }

            // Report for every referencing skill (not just the first)
            if nested.contains(&rel) {
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

/// S030: Detect orphaned files in skill scripts/ subdirectories.
fn validate_orphaned_skill_files(
    base_dir: &str,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
) {
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

        let skill_path = format!("{base_dir}/{dir_name}/SKILL.md");
        if exclude.is_excluded(&skill_path) {
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

            let display_path = format!("{base_dir}/{dir_name}/scripts/{script_name}");
            if exclude.is_excluded(&display_path) {
                continue;
            }

            // Check if the script file name is referenced anywhere in SKILL.md
            if !skill_content.contains(&script_name) {
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

/// S036: Check that referenced shared .md files > 100 lines have headings (TOC).
/// Only runs in plugin mode (called from validate_skill_content).
fn validate_ref_no_toc(base_dir: &str, diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let shared_dir = Path::new(base_dir).join("shared");
    if !shared_dir.is_dir() {
        return;
    }

    let re_shared =
        Regex::new(r"\$\{CLAUDE_PLUGIN_ROOT\}/skills/shared/[a-zA-Z0-9._-]+\.md").unwrap();

    let skills = collect_skills(base_dir, exclude);
    let mut checked: HashSet<String> = HashSet::new();

    for info in &skills {
        for cap in re_shared.find_iter(&info.body) {
            let reference = cap.as_str();
            let rel = reference.replace("${CLAUDE_PLUGIN_ROOT}/", "");

            if !checked.insert(rel.clone()) {
                continue;
            }

            let rel_path = Path::new(&rel);
            if !rel_path.is_file() {
                continue;
            }

            if let Ok(content) = fs::read_to_string(rel_path) {
                let line_count = content.lines().count();
                if line_count > 100 {
                    let has_headings = content.lines().any(|l| l.starts_with("## "));
                    if !has_headings {
                        diag.report(
                            LintRule::RefNoToc,
                            &format!(
                                "{}: references {} ({} lines) which has no ## headings for navigation",
                                info.path, reference, line_count
                            ),
                        );
                    }
                }
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
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
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
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
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
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
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
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
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
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("consecutive hyphens"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s011_leading_hyphen() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: -my-skill\ndescription: Use when testing hyphen rules\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("starts/ends with hyphen"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s011_trailing_hyphen() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill-\ndescription: Use when testing hyphen rules\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("starts/ends with hyphen"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s011_valid_hyphens_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-good-skill\ndescription: Use when testing hyphen rules\n---\nBody\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag
            .errors()
            .iter()
            .any(|e| e.contains("starts/ends with hyphen") || e.contains("consecutive hyphens")));
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
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("reserved word")));
    }

    // ── S013: name-has-xml ──────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s013_name_with_xml_tag() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-<tag>skill\ndescription: A valid skill description here\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("name") && e.contains("XML/HTML tags"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s013_name_without_xml_ok() {
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
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("XML/HTML tags") && e.contains("name"))
        );
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
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("exceeds 1024")));
    }

    // ── S015: desc-truncated ─────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s015_desc_truncated_in_plugin_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let long_desc = format!("Use when you need {}", "x".repeat(240));
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: {long_desc}\n---\nBody content\n"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("truncated")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s015_desc_250_chars_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        // "Use when the task needs " = 24 chars + 226 x's = exactly 250 chars
        let desc = format!("Use when the task needs {}", "x".repeat(226));
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: {desc}\n---\nBody content\n"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("truncated")));
    }

    // ── S016: desc-uses-person ───────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s016_desc_uses_you() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need to analyze code\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("first/second person"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s016_desc_third_person_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when the project needs code analysis and review\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("first/second person"))
        );
    }

    // ── S017: desc-no-trigger ────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s017_desc_no_trigger_context() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A skill that does things with code\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("trigger")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s017_desc_with_trigger_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when the project needs analysis\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("trigger")));
    }

    // ── S018: desc-has-xml ───────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s018_desc_with_xml() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when <b>important</b> tasks need doing\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("description") && e.contains("XML"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s018_desc_without_xml_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when important tasks need doing well\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("description") && e.contains("XML"))
        );
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
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
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
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
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
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
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
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("consecutive bash")));
    }

    // ── S022: backslash-path ────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s022_windows_path_in_body() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need path validation\n---\nUse the file at C:\\Users\\admin\\file.txt\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("backslash")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s022_forward_slash_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need path validation\n---\nUse the file at /Users/admin/file.txt\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("backslash")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s022_regex_escape_not_flagged() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need regex validation\n---\nUse regex like \\s and \\n to match patterns\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("backslash")));
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
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
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
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("must be true or false"))
        );
    }

    // ── S024: context-field-invalid ─────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s024_invalid_context() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need context testing\ncontext: invalid\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("context") && e.contains("fork"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s024_valid_context_fork() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need context testing\ncontext: fork\n---\nRun the analysis.\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("context") && e.contains("must be"))
        );
    }

    // ── S025: effort-field-invalid ───────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s025_invalid_effort() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need effort testing\neffort: extreme\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("effort") && e.contains("low/medium/high/max"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s025_valid_effort() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need effort testing\neffort: high\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("effort")));
    }

    // ── S026: shell-field-invalid ────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s026_invalid_shell() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need shell testing\nshell: zsh\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("shell") && e.contains("bash/powershell"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s026_valid_shell() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need shell testing\nshell: bash\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("shell") && e.contains("must be"))
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
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("unreachable")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s027_reachable_skill_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when testing reachability\ndisable-model-invocation: true\nuser-invocable: true\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("unreachable")));
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
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
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
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
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
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
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
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("non-HTTPS")));
    }

    // ── S029: nested-ref-deep ───────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s029_nested_reference_fires() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        // Create a shared .md that itself references another shared .md
        std::fs::write(
            "skills/shared/level1.md",
            "# Level 1\nSee ${CLAUDE_PLUGIN_ROOT}/skills/shared/level2.md for details\n",
        )
        .unwrap();
        std::fs::write("skills/shared/level2.md", "# Level 2\nContent\n").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing\n---\nRefer to ${CLAUDE_PLUGIN_ROOT}/skills/shared/level1.md\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("itself references"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s029_flat_reference_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/shared/flat.md",
            "# Flat\nNo nested references here\n",
        )
        .unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing\n---\nRefer to ${CLAUDE_PLUGIN_ROOT}/skills/shared/flat.md\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("itself references"))
        );
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
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
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
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("not referenced")));
    }

    // ── S032: hardcoded-secret ──────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s032_openai_key_pattern() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need secret detection testing\n---\nSet key to sk-aBcDeFgHiJkLmNoPqRsT1234\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("hardcoded secret")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s032_github_token_pattern() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need secret detection testing\n---\nToken is ghp_abcdefghijklmnopqrstuvwxyz1234567890\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("hardcoded secret")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s032_no_secrets_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need secret detection testing\n---\nUse the $API_KEY environment variable for authentication\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("hardcoded secret")));
    }

    // ── S033: name-vague ─────────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s033_vague_name_helper() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/helper").unwrap();
        std::fs::write(
            "skills/helper/SKILL.md",
            "---\nname: helper\ndescription: Use when you need help with various tasks\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("vague")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s033_specific_name_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/code-review").unwrap();
        std::fs::write(
            "skills/code-review/SKILL.md",
            "---\nname: code-review\ndescription: Use when code changes need thorough review\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("vague")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s033_private_mode_skips() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude/skills/helper").unwrap();
        std::fs::write(
            ".claude/skills/helper/SKILL.md",
            "---\nname: helper\ndescription: A valid skill description here\n---\nBody content\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_private_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        // S033 is plugin-only, should not fire in private mode
        assert!(!diag.errors().iter().any(|e| e.contains("vague")));
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
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
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
        validate_private_skill_content(&mut diag, &crate::config::ExcludeSet::default());
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
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
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
        // Verify S009–S043 rules round-trip via code and name lookups
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
            ("S035", "compat-too-long"),
            ("S036", "ref-no-toc"),
            ("S037", "body-no-refs"),
            ("S038", "time-sensitive"),
            ("S039", "metadata-not-string"),
            ("S040", "tools-unknown"),
            ("S041", "fork-no-task"),
            ("S042", "dmi-empty-desc"),
            ("S043", "frontmatter-backslash"),
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

    // ── S035: compat-too-long ────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s035_compat_too_long() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let long_compat = "x".repeat(501);
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: A valid skill description here\ncompatibility: {long_compat}\n---\nBody content\n"),
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("compatibility") && e.contains("500"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s035_compat_within_limit_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let compat = "x".repeat(500);
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: Use when testing compat limits\ncompatibility: {compat}\n---\nBody content\n"),
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("compatibility") && e.contains("500"))
        );
    }

    // ── S036: ref-no-toc ───────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s036_ref_no_toc() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        // Create a shared .md > 100 lines with no ## headings
        let long_content = "line\n".repeat(101);
        std::fs::write("skills/shared/big-ref.md", &long_content).unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\nSee ${CLAUDE_PLUGIN_ROOT}/skills/shared/big-ref.md\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("no ## headings")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s036_ref_with_headings_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let mut content = String::from("## Section 1\n");
        for _ in 0..100 {
            content.push_str("line\n");
        }
        std::fs::write("skills/shared/big-ref.md", &content).unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\nSee ${CLAUDE_PLUGIN_ROOT}/skills/shared/big-ref.md\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("no ## headings")));
    }

    // ── S037: body-no-refs ───────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s037_body_no_refs() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let body = "Some text without any file references\n".repeat(301);
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\n{body}"),
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("300 lines") && e.contains("file references"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s037_body_with_refs_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let mut body = "Some text\n".repeat(300);
        body.push_str("Run scripts/helper.sh to do something\n");
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\n{body}"),
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("300 lines") && e.contains("file references"))
        );
    }

    // ── S038: time-sensitive ─────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s038_time_sensitive() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\nThis expires after 2030 so plan accordingly.\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("date/year")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s038_year_in_code_fence_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when you need a skill for testing purposes\n---\n\n```bash\necho 2030\n```\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("date/year")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s038_private_mode_skips() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude/skills/my-skill").unwrap();
        std::fs::write(
            ".claude/skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\n---\nThis expires after 2030.\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_private_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("date/year")));
    }

    // ── S039: metadata-not-string ────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s039_metadata_bare_bool() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\nmetadata:\n  enabled: true\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("metadata") && e.contains("non-string"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s039_metadata_inline_value() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when testing metadata validation\nmetadata: true\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("metadata") && e.contains("non-string"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s039_metadata_quoted_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when testing metadata validation\nmetadata:\n  version: \"1.0\"\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("metadata") && e.contains("non-string"))
        );
    }

    // ── S040: tools-unknown ──────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s040_unknown_tool() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\nallowed-tools: Bash, Read, FakeToolXyz\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("FakeToolXyz")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s040_valid_tools() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\nallowed-tools: Bash, Read, Write, Grep, Glob\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("unrecognized tool"))
        );
    }

    // ── S041: fork-no-task ───────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s041_fork_no_task() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\ncontext: fork\n---\nThis is just guidelines about how to behave.\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("fork") && e.contains("task"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s041_fork_with_task_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\ncontext: fork\n---\nRun the analysis and generate a report.\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("fork") && e.contains("task"))
        );
    }

    // ── S042: dmi-empty-desc ─────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s042_dmi_empty_desc() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription:\ndisable-model-invocation: true\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("disable-model-invocation") && e.contains("empty"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s042_dmi_with_desc_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when the skill should be user-only\ndisable-model-invocation: true\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("disable-model-invocation") && e.contains("empty"))
        );
    }

    // ── S043: frontmatter-backslash ──────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn test_s043_frontmatter_backslash() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\nargument-hint: C:\\Users\\file\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("backslash") && e.contains("frontmatter"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s043_forward_slash_frontmatter_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when testing frontmatter paths\nargument-hint: /usr/local/bin/tool\n---\nBody content\n",
        ).unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("backslash") && e.contains("frontmatter"))
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Boundary tests
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    #[serial_test::serial]
    fn test_s009_boundary_64_chars_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let name64 = "a".repeat(64);
        std::fs::create_dir_all(format!("skills/{name64}")).unwrap();
        std::fs::write(
            format!("skills/{name64}/SKILL.md"),
            format!(
                "---\nname: {name64}\ndescription: Use when testing name length boundary\n---\nBody\n"
            ),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("exceeds 64")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s014_boundary_1024_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        // "Use when testing " = 17 chars + 1007 x's = exactly 1024
        let desc = format!("Use when testing {}", "x".repeat(1007));
        assert_eq!(desc.len(), 1024);
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: {desc}\n---\nBody\n"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("exceeds 1024")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s019_boundary_500_lines_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        let body = "line\n".repeat(500);
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!(
                "---\nname: my-skill\ndescription: Use when testing body length boundary\n---\n{body}"
            ),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("exceeds 500")));
    }

    #[test]
    #[serial_test::serial]
    fn test_s020_non_empty_body_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when testing body presence\n---\nHas body content\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("no content after frontmatter"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s034_boundary_20_chars_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        // exactly 20 characters
        let desc = "Use when needed now!";
        assert_eq!(desc.len(), 20);
        std::fs::write(
            "skills/my-skill/SKILL.md",
            format!("---\nname: my-skill\ndescription: {desc}\n---\nBody\n"),
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("under 20")));
    }

    // ═══════════════════════════════════════════════════════════════════
    // collect_skills edge cases
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    #[serial_test::serial]
    fn test_collect_skills_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills").unwrap();
        let skills = collect_skills("skills", &crate::config::ExcludeSet::default());
        assert!(skills.is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn test_collect_skills_missing_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        let skills = collect_skills("skills", &crate::config::ExcludeSet::default());
        assert!(skills.is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn test_collect_skills_skips_malformed_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/good-skill").unwrap();
        std::fs::create_dir_all("skills/bad-skill").unwrap();
        std::fs::write(
            "skills/good-skill/SKILL.md",
            "---\nname: good-skill\ndescription: A valid skill\n---\nBody\n",
        )
        .unwrap();
        // Malformed: no closing ---
        std::fs::write(
            "skills/bad-skill/SKILL.md",
            "---\nname: bad-skill\nno closing\n",
        )
        .unwrap();
        let skills = collect_skills("skills", &crate::config::ExcludeSet::default());
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].dir_name, "good-skill");
    }

    #[test]
    #[serial_test::serial]
    fn test_collect_skills_skips_shared() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill\n---\nBody\n",
        )
        .unwrap();
        std::fs::write("skills/shared/helpers.md", "# Helpers\n").unwrap();
        let skills = collect_skills("skills", &crate::config::ExcludeSet::default());
        assert_eq!(skills.len(), 1);
    }

    #[test]
    #[serial_test::serial]
    fn test_collect_skills_populates_body() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill\n---\nBody content here\n",
        )
        .unwrap();
        let skills = collect_skills("skills", &crate::config::ExcludeSet::default());
        assert_eq!(skills.len(), 1);
        assert!(skills[0].body.contains("Body content here"));
        assert!(!skills[0].body.contains("---"));
    }

    // ═══════════════════════════════════════════════════════════════════
    // Config integration tests
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    #[serial_test::serial]
    fn test_config_ignore_suppresses_new_rule() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        // Body empty (S020) + desc too short (S034). Use trigger context to avoid S017.
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when short\n---\n",
        )
        .unwrap();

        // Without config: S020 and S034 should fire
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("no content")));
        assert!(diag.errors().iter().any(|e| e.contains("under 20")));

        // With config ignoring S020
        let config = crate::config::LintConfig {
            ignore: std::collections::HashSet::from([LintRule::BodyEmpty]),
            warn: std::collections::HashSet::new(),
            exclude: vec![],
        };
        let mut diag2 = DiagnosticCollector::with_config(config);
        validate_skill_content(&mut diag2, &crate::config::ExcludeSet::default());
        // S020 suppressed, S034 still fires
        assert!(!diag2.errors().iter().any(|e| e.contains("no content")));
        assert!(diag2.errors().iter().any(|e| e.contains("under 20")));
        assert_eq!(
            diag2.suppressed_count(),
            1,
            "S020 should be counted as suppressed"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_config_warn_downgrades_new_rule() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Use when short\n---\n",
        )
        .unwrap();

        let config = crate::config::LintConfig {
            ignore: std::collections::HashSet::new(),
            warn: std::collections::HashSet::from([LintRule::DescTooShort]),
            exclude: vec![],
        };
        let mut diag = DiagnosticCollector::with_config(config);
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        // S034 downgraded to warning, not counted as error
        assert!(!diag.errors().iter().any(|e| e.contains("under 20")));
        assert!(diag.warnings().iter().any(|e| e.contains("under 20")));
    }

    // ═══════════════════════════════════════════════════════════════════
    // End-to-end mode dispatch integration tests
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    #[serial_test::serial]
    fn test_mixed_repo_both_modes_run() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        // Public skill with name issue (S010: uppercase)
        std::fs::create_dir_all("skills/My-Skill").unwrap();
        std::fs::write(
            "skills/My-Skill/SKILL.md",
            "---\nname: My-Skill\ndescription: Use when testing mixed mode validation\n---\nBody content\n",
        )
        .unwrap();

        // Private skill — should NOT fire S016 (plugin-only person check)
        std::fs::create_dir_all(".claude/skills/helper").unwrap();
        std::fs::write(
            ".claude/skills/helper/SKILL.md",
            "---\nname: helper\ndescription: Helps you do things more efficiently here\n---\nBody content\n",
        )
        .unwrap();

        // Plugin mode runs both public and private
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        validate_private_skill_content(&mut diag, &crate::config::ExcludeSet::default());

        // S010 fires for public "My-Skill"
        assert!(
            diag.errors()
                .iter()
                .any(|e| e.contains("outside [a-z0-9-]"))
        );
        // S016 should NOT fire for private skill (plugin_mode=false)
        assert!(
            !diag
                .errors()
                .iter()
                .any(|e| e.contains("first/second person") && e.contains(".claude"))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_valid_skill_zero_errors() {
        // A fully valid skill should produce zero errors from all content checks
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/code-review").unwrap();
        std::fs::write(
            "skills/code-review/SKILL.md",
            "---\nname: code-review\ndescription: Use when code changes need thorough review and analysis\nuser-invocable: true\neffort: high\nshell: bash\nargument-hint: <PR number or branch name>\n---\n\n# Code Review\n\nPerform a thorough code review of the specified changes.\n\n## Steps\n\n1. Run the analysis on $ARGUMENTS\n2. Generate a summary report\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_skill_content(&mut diag, &crate::config::ExcludeSet::default());
        let skill_errors: Vec<_> = diag
            .errors()
            .iter()
            .filter(|e| e.contains("skills/code-review"))
            .cloned()
            .collect();
        assert!(
            skill_errors.is_empty(),
            "Expected zero errors for valid skill, got: {skill_errors:?}"
        );
    }
}
