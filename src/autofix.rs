use crate::config::{ExcludeSet, LintConfig};
use crate::context::LintMode;
use crate::fence::CodeFenceTracker;
use crate::frontmatter;
use crate::hook_commands::extract_hook_command_paths;
use crate::platforms::ValidationTargets;
use crate::pwd_hygiene::replace_bundled_asset_prefixes;
use crate::rules::LintRule;
use crate::traversal;
use crate::validators::skill_content::security::flagged_http_offsets;
use crate::validators::skill_content::{
    RE_BACKSLASH_PATH, contains_backslash_path, is_named_tex_escape_pair,
};
use crate::validators::skills::collect_skills;
use regex::Regex;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::LazyLock;

// Reuse the same regexes validators use.
// This replacement-scoped pattern consumes every segment in a detected path
// run, so an autofix cannot leave mixed separators behind.
static RE_BACKSLASH_PATH_RUN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"[A-Za-z]:(?:\\[A-Za-z0-9_.-]+)+|\\\\[A-Za-z][A-Za-z0-9_-]*(?:\\[A-Za-z][A-Za-z0-9_-]*)+|(?:\\[A-Za-z][A-Za-z0-9_-]*){2,}",
    )
    .unwrap()
});
static RE_BASH_FENCE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^```(bash|sh|shell)\s*$").unwrap());
static RE_NAME_INVALID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^a-z0-9-]").unwrap());

/// Attempt to fix all instances of a given auto-fixable rule.
/// Returns `true` if at least one file was modified.
pub fn apply_fix(
    rule: LintRule,
    mode: LintMode,
    targets: ValidationTargets,
    exclude: &ExcludeSet,
    config: &LintConfig,
) -> bool {
    match rule {
        LintRule::HookNotExecutable => fix_executability_hooks(mode, config),
        LintRule::ScriptNotExecutable => fix_executability_scripts(mode, exclude, config),
        LintRule::FrontmatterNameMismatch => {
            fix_frontmatter_name_mismatch(mode, targets, exclude, config)
        }
        LintRule::FrontmatterFieldEmpty => fix_frontmatter_field_empty(mode, exclude, config),
        LintRule::DescHasXml => fix_desc_has_xml(mode, exclude, config),
        LintRule::ConsecutiveBash => fix_consecutive_bash(mode, exclude, config),
        LintRule::BackslashPath => fix_backslash_path(mode, exclude, config),
        LintRule::NonHttpsUrl => fix_non_https_url(mode, exclude, config),
        LintRule::FrontmatterBackslash => fix_frontmatter_backslash(mode, exclude, config),
        LintRule::ToolsListSyntax => fix_tools_list_syntax(mode, exclude, config),
        LintRule::PwdInSkill => fix_pwd_in_skill(exclude, config),
        _ => false,
    }
}

fn is_suppressed(config: &LintConfig, rule: LintRule, path: impl AsRef<Path>) -> bool {
    config.is_suppressed_at(rule, path.as_ref())
}

fn log_fix(rule: LintRule, msg: &str) {
    let msg = crate::diagnostic::sanitize_for_terminal(msg);
    let _ = writeln!(
        std::io::stderr(),
        "fixed[{}/{}]: {msg}",
        rule.code(),
        rule.name()
    );
}

// ── H005: chmod +x on hook scripts ──────────────────────────────────────

#[cfg(unix)]
fn fix_executability_hooks(mode: LintMode, config: &LintConfig) -> bool {
    use crate::context::{LintContext, ManifestState};

    let ctx = LintContext::new(Path::new("."), mode);
    let mut fixed = false;

    if mode == LintMode::Plugin
        && let ManifestState::Parsed(value) = &ctx.hooks_json
    {
        fixed |= fix_hook_config_executability(value, config);
    }
    if let ManifestState::Parsed(value) = &ctx.settings_json {
        fixed |= fix_hook_config_executability(value, config);
    }
    if let ManifestState::Parsed(value) = &ctx.settings_local_json {
        fixed |= fix_hook_config_executability(value, config);
    }
    for hook_config in &ctx.declared_hook_configs {
        if let ManifestState::Parsed(value) = &hook_config.state {
            fixed |= fix_hook_config_executability(value, config);
        }
    }
    fixed
}

#[cfg(unix)]
fn fix_hook_config_executability(value: &serde_json::Value, config: &LintConfig) -> bool {
    let mut fixed = false;
    for reference in extract_hook_command_paths(value) {
        if is_suppressed(config, LintRule::HookNotExecutable, &reference.path) {
            continue;
        }
        if make_hook_executable(&reference.path) {
            log_fix(
                LintRule::HookNotExecutable,
                &format!("made executable: {}", reference.path.display()),
            );
            fixed = true;
        }
    }
    fixed
}

#[cfg(unix)]
fn make_hook_executable(path: &Path) -> bool {
    let Ok(repo_root) = std::env::current_dir().and_then(std::fs::canonicalize) else {
        return false;
    };
    let Ok(resolved) = std::fs::canonicalize(path) else {
        return false;
    };
    if !resolved.starts_with(repo_root) {
        return false;
    }
    make_executable(resolved.to_str().unwrap_or(""))
}

#[cfg(not(unix))]
fn fix_executability_hooks(_mode: LintMode, _config: &LintConfig) -> bool {
    false
}

// ── G003: chmod +x on script files ──────────────────────────────────────

#[cfg(unix)]
fn fix_executability_scripts(mode: LintMode, exclude: &ExcludeSet, config: &LintConfig) -> bool {
    let mut fixed = false;
    for path in crate::validators::hygiene::scripts::direct_script_paths(mode, exclude) {
        let display = path.display().to_string();
        if is_suppressed(config, LintRule::ScriptNotExecutable, &path) {
            continue;
        }
        if make_executable(path.to_str().unwrap_or("")) {
            log_fix(
                LintRule::ScriptNotExecutable,
                &format!("made executable: {display}"),
            );
            fixed = true;
        }
    }
    fixed
}

#[cfg(not(unix))]
fn fix_executability_scripts(_mode: LintMode, _exclude: &ExcludeSet, _config: &LintConfig) -> bool {
    false
}

#[cfg(unix)]
fn make_executable(path: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let p = Path::new(path);
    if !p.is_file() {
        return false;
    }
    let meta = match p.metadata() {
        Ok(m) => m,
        Err(_) => return false,
    };
    let mode = meta.permissions().mode();
    if mode & 0o111 != 0 {
        return false; // Already executable
    }
    let new_mode = mode | ((mode & 0o444) >> 2);
    fs::set_permissions(p, std::os::unix::fs::PermissionsExt::from_mode(new_mode)).is_ok()
}

// ── S006: frontmatter name mismatch ─────────────────────────────────────

fn fix_frontmatter_name_mismatch(
    mode: LintMode,
    targets: ValidationTargets,
    exclude: &ExcludeSet,
    config: &LintConfig,
) -> bool {
    let mut fixed = false;
    let mut base_dirs = Vec::new();
    if mode == LintMode::Plugin {
        base_dirs.push("skills");
    }
    base_dirs.push(".claude/skills");
    if targets.agent_skills {
        base_dirs.push(".agents/skills");
    }

    for base_dir in base_dirs {
        let dir = Path::new(base_dir);
        if !dir.is_dir() {
            continue;
        }
        for entry in traversal::shallow_directories(dir, Path::new("."), None).entries {
            let path = entry.path;
            let dir_name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            if dir_name == "shared" && base_dir != ".agents/skills" {
                continue;
            }
            let skill_path = format!("{base_dir}/{dir_name}/SKILL.md");
            if exclude.is_excluded(&skill_path)
                || is_suppressed(config, LintRule::FrontmatterNameMismatch, &skill_path)
            {
                continue;
            }
            let skill_md = path.join("SKILL.md");
            if !skill_md.is_file() {
                continue;
            }

            // Validate dir_name against naming rules before using it
            if RE_NAME_INVALID.is_match(&dir_name)
                || dir_name.starts_with('-')
                || dir_name.ends_with('-')
                || dir_name.contains("--")
            {
                continue; // Dir name is invalid, skip (FINDING_9)
            }

            let content = match fs::read_to_string(&skill_md) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let fm_lines = match frontmatter::extract_frontmatter(&content) {
                Some(lines) => lines,
                None => continue,
            };
            let parsed_frontmatter = match frontmatter::parse_yaml_strict(&fm_lines) {
                Ok(yaml) => yaml,
                Err(_) => continue,
            };
            let name =
                match frontmatter::canonical_nonempty_string_field(&parsed_frontmatter, "name") {
                    Some(name) => name,
                    None => continue,
                };
            if name == dir_name {
                continue;
            }

            let Some(raw_name_index) = single_line_frontmatter_field_index(&fm_lines, "name")
            else {
                continue;
            };
            let raw_name_line = &fm_lines[raw_name_index];
            if raw_name_line["name:".len()..].trim().is_empty() {
                continue;
            }
            let new_line = format!("name: {dir_name}");
            if let Some(new_content) = replace_in_frontmatter(&content, raw_name_line, &new_line) {
                if fs::write(&skill_md, new_content).is_ok() {
                    log_fix(
                        LintRule::FrontmatterNameMismatch,
                        &format!("{skill_path}: renamed '{name}' to '{dir_name}'"),
                    );
                    fixed = true;
                }
            }
        }
    }
    fixed
}

// ── S007: empty frontmatter field ───────────────────────────────────────

fn fix_frontmatter_field_empty(mode: LintMode, exclude: &ExcludeSet, config: &LintConfig) -> bool {
    let mut fixed = false;
    let base_dirs: &[&str] = match mode {
        LintMode::Plugin => &["skills", ".claude/skills"],
        LintMode::Basic => &[".claude/skills"],
    };
    for base_dir in base_dirs {
        let skills = collect_skills(base_dir, exclude);
        for info in &skills {
            let skill_md = format!("{base_dir}/{}/SKILL.md", info.dir_name);
            if is_suppressed(config, LintRule::FrontmatterFieldEmpty, &skill_md) {
                continue;
            }
            let skill_path = Path::new(base_dir).join(&info.dir_name).join("SKILL.md");
            let content = match fs::read_to_string(&skill_path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            for field in crate::validators::skill_content::OPTIONAL_NONEMPTY_SCALAR_FIELDS {
                let parsed_frontmatter = frontmatter::parse_yaml_strict(&info.fm_lines).ok();
                if !frontmatter::optional_field_is_present(
                    &info.fm_lines,
                    parsed_frontmatter.as_ref(),
                    field,
                ) {
                    continue;
                }
                if !frontmatter::optional_field_is_empty(
                    &info.fm_lines,
                    parsed_frontmatter.as_ref(),
                    field,
                ) {
                    continue; // Not empty
                }

                // FINDING_8: skip removing argument-hint if body uses $ARGUMENTS
                if *field == "argument-hint" && info.body.contains("$ARGUMENTS") {
                    continue;
                }

                // A bare field line with no indented continuation is the only
                // unambiguous removal. In particular, never orphan a YAML
                // continuation or child block.
                let Some(index) = single_line_frontmatter_field_index(&info.fm_lines, field) else {
                    continue;
                };
                let prefix = format!("{field}:");
                if info.fm_lines[index].trim_end() != prefix {
                    continue;
                }

                if let Some(new_content) = remove_frontmatter_line(&content, &prefix) {
                    if fs::write(&skill_path, &new_content).is_ok() {
                        log_fix(
                            LintRule::FrontmatterFieldEmpty,
                            &format!("{skill_md}: removed empty '{field}'"),
                        );
                        fixed = true;
                        break; // One fix per file, re-validate
                    }
                }
            }
        }
    }
    fixed
}

/// Return a top-level frontmatter field line only when no indented YAML lines
/// belong to that field before the next top-level entry.
fn single_line_frontmatter_field_index(fm_lines: &[String], field: &str) -> Option<usize> {
    let index = frontmatter::simple_top_level_key_index(fm_lines, field)?;
    let has_continuation = fm_lines[index + 1..]
        .iter()
        .take_while(|line| line.is_empty() || line.starts_with(' ') || line.starts_with('\t'))
        .any(|line| line.starts_with(' ') || line.starts_with('\t'));
    (!has_continuation).then_some(index)
}

// ── S018: XML tags in description ───────────────────────────────────────

fn fix_desc_has_xml(mode: LintMode, exclude: &ExcludeSet, config: &LintConfig) -> bool {
    let mut fixed = false;
    let base_dirs: &[&str] = match mode {
        LintMode::Plugin => &["skills", ".claude/skills"],
        LintMode::Basic => &[".claude/skills"],
    };
    for base_dir in base_dirs {
        let skills = collect_skills(base_dir, exclude);
        for info in &skills {
            let display = format!("{base_dir}/{}/SKILL.md", info.dir_name);
            if is_suppressed(config, LintRule::DescHasXml, &display) {
                continue;
            }
            let value = match frontmatter::get_strict_string_field(&info.fm_lines, "description") {
                Some(v) => v,
                None => continue,
            };
            if !crate::validators::skill_content::description_contains_xml_tags(&value) {
                continue;
            }
            let new_value = crate::validators::skill_content::strip_description_xml_tags(&value);
            let new_value = new_value.trim().to_string();
            if new_value == value || new_value.is_empty() {
                continue;
            }

            let skill_path = Path::new(base_dir).join(&info.dir_name).join("SKILL.md");
            let content = match fs::read_to_string(&skill_path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // A multiline or quoted scalar cannot be rewritten safely with a
            // line replacement. Only rewrite a single raw value that is
            // exactly the canonical parsed scalar.
            let prefix = "description:";
            let Some(raw_index) = info
                .fm_lines
                .iter()
                .position(|line| line.starts_with(prefix))
            else {
                continue;
            };
            let raw_line = &info.fm_lines[raw_index];
            let Some(raw_value) = raw_line.strip_prefix(prefix) else {
                continue;
            };
            let has_continuation = info.fm_lines[raw_index + 1..]
                .first()
                .is_some_and(|line| line.is_empty() || line.starts_with(char::is_whitespace));
            if has_continuation || raw_value.trim_start() != value {
                continue;
            }

            let new_line = format!("description: {new_value}");
            if let Some(new_content) = replace_in_frontmatter(&content, raw_line, &new_line) {
                if fs::write(&skill_path, new_content).is_ok() {
                    log_fix(
                        LintRule::DescHasXml,
                        &format!(
                            "{base_dir}/{}/SKILL.md: stripped XML tags from description",
                            info.dir_name
                        ),
                    );
                    fixed = true;
                }
            }
        }
    }
    fixed
}

// ── S021: consecutive bash code blocks ──────────────────────────────────

fn fix_consecutive_bash(mode: LintMode, exclude: &ExcludeSet, config: &LintConfig) -> bool {
    let mut fixed = false;
    let base_dirs: &[&str] = match mode {
        LintMode::Plugin => &["skills", ".claude/skills"],
        LintMode::Basic => &[".claude/skills"],
    };
    for base_dir in base_dirs {
        let skills = collect_skills(base_dir, exclude);
        for info in &skills {
            let skill_path = Path::new(base_dir).join(&info.dir_name).join("SKILL.md");
            if is_suppressed(config, LintRule::ConsecutiveBash, &skill_path) {
                continue;
            }
            let content = match fs::read_to_string(&skill_path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            if let Some(new_content) = merge_first_consecutive_bash(&content) {
                if fs::write(&skill_path, new_content).is_ok() {
                    log_fix(
                        LintRule::ConsecutiveBash,
                        &format!(
                            "{base_dir}/{}/SKILL.md: merged consecutive bash blocks",
                            info.dir_name
                        ),
                    );
                    fixed = true;
                    break; // One fix per pass
                }
            }
        }
    }
    fixed
}

fn merge_first_consecutive_bash(content: &str) -> Option<String> {
    use crate::fence::LineClass;
    let body = frontmatter::extract_body(content);
    if body.is_empty() {
        return None;
    }
    // Count frontmatter lines to compute offset
    let fm_line_count = content.lines().count() - body.lines().count();

    let mut tracker = CodeFenceTracker::new();
    let mut last_bash_end: Option<usize> = None;
    let mut fence_is_bash = false;

    let body_lines: Vec<&str> = body.lines().collect();
    for (i, line) in body_lines.iter().enumerate() {
        let trimmed = line.trim_start();
        match tracker.process_line(line) {
            LineClass::Delimiter => {
                if !tracker.in_fence() {
                    if fence_is_bash {
                        last_bash_end = Some(i);
                    }
                    fence_is_bash = false;
                } else if RE_BASH_FENCE.is_match(trimmed) {
                    if let Some(prev_end) = last_bash_end {
                        let between_lines: Vec<&&str> =
                            body_lines[prev_end + 1..i].iter().collect();
                        let only_blank = between_lines.iter().all(|l| l.trim().is_empty());
                        if only_blank {
                            // Found consecutive bash blocks: merge them
                            // Remove lines from prev_end (closing ```) through i (opening ```bash)
                            let file_lines: Vec<&str> = content.lines().collect();
                            let remove_start = fm_line_count + prev_end;
                            let remove_end = fm_line_count + i;
                            let mut result_lines: Vec<&str> = Vec::new();
                            for (j, fl) in file_lines.iter().enumerate() {
                                if j < remove_start || j > remove_end {
                                    result_lines.push(fl);
                                }
                            }
                            // Preserve original trailing newline
                            let mut result = result_lines.join("\n");
                            if content.ends_with('\n') {
                                result.push('\n');
                            }
                            return Some(result);
                        }
                    }
                    fence_is_bash = true;
                } else {
                    fence_is_bash = false;
                }
            }
            LineClass::Inside | LineClass::Outside => {}
        }
    }
    None
}

// ── S022: backslash paths in body ───────────────────────────────────────

fn fix_backslash_path(mode: LintMode, exclude: &ExcludeSet, config: &LintConfig) -> bool {
    let mut fixed = false;
    let base_dirs: &[&str] = match mode {
        LintMode::Plugin => &["skills", ".claude/skills"],
        LintMode::Basic => &[".claude/skills"],
    };
    for base_dir in base_dirs {
        let skills = collect_skills(base_dir, exclude);
        for info in &skills {
            let skill_path = Path::new(base_dir).join(&info.dir_name).join("SKILL.md");
            if is_suppressed(config, LintRule::BackslashPath, &skill_path) {
                continue;
            }
            // Check outside code fences (matching validator)
            let has_backslash =
                crate::fence::lines_outside_fences(&info.body).any(contains_backslash_path);
            if !has_backslash {
                continue;
            }

            let content = match fs::read_to_string(&skill_path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // Replace backslash paths only in lines outside code fences
            let body = frontmatter::extract_body(&content);
            let fm_end = content.len() - body.len();
            let preamble = &content[..fm_end];

            let mut new_body = String::new();
            let mut tracker = CodeFenceTracker::new();
            for line in body.lines() {
                let class = tracker.process_line(line);
                if class == crate::fence::LineClass::Outside && contains_backslash_path(line) {
                    // Only replace backslashes within matched path patterns, not all backslashes
                    new_body.push_str(&replace_backslash_paths(line));
                } else {
                    new_body.push_str(line);
                }
                new_body.push('\n');
            }
            // Fix trailing newline
            if !body.ends_with('\n') && new_body.ends_with('\n') {
                new_body.pop();
            }

            let new_content = format!("{preamble}{new_body}");
            if new_content != content && fs::write(&skill_path, &new_content).is_ok() {
                log_fix(
                    LintRule::BackslashPath,
                    &format!(
                        "{base_dir}/{}/SKILL.md: replaced backslash paths",
                        info.dir_name
                    ),
                );
                fixed = true;
            }
        }
    }
    fixed
}

// ── S031: non-HTTPS URLs ────────────────────────────────────────────────

fn fix_non_https_url(mode: LintMode, exclude: &ExcludeSet, config: &LintConfig) -> bool {
    let mut fixed = false;
    let base_dirs: &[&str] = match mode {
        LintMode::Plugin => &["skills", ".claude/skills"],
        LintMode::Basic => &[".claude/skills"],
    };
    for base_dir in base_dirs {
        let skills = collect_skills(base_dir, exclude);
        for info in &skills {
            let skill_path = Path::new(base_dir).join(&info.dir_name).join("SKILL.md");
            if is_suppressed(config, LintRule::NonHttpsUrl, &skill_path) {
                continue;
            }
            if info.body.trim().is_empty() {
                continue;
            }
            // Only rewrite if the shared classifier would flag a match, so the
            // autofix and the S031 checker agree by construction (issue #353).
            if flagged_http_offsets(&info.body).next().is_none() {
                continue;
            }

            let content = match fs::read_to_string(&skill_path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // Replace http:// with https:// only in the body (matching validator scope)
            let body = frontmatter::extract_body(&content);
            let fm_end = content.len() - body.len();
            let preamble = &content[..fm_end];
            let new_body = replace_http_urls(body);
            let new_content = format!("{preamble}{new_body}");
            if new_content != content && fs::write(&skill_path, &new_content).is_ok() {
                log_fix(
                    LintRule::NonHttpsUrl,
                    &format!(
                        "{base_dir}/{}/SKILL.md: replaced http:// with https://",
                        info.dir_name
                    ),
                );
                fixed = true;
            }
        }
    }
    fixed
}

fn replace_http_urls(content: &str) -> String {
    let mut result = content.to_string();
    // The shared classifier decides which matches are flagged; exempt
    // identifier/reserved-host matches are left byte-identical. Rewrite in
    // reverse so earlier offsets stay valid as later ones grow by one byte.
    let offsets: Vec<usize> = flagged_http_offsets(content).collect();
    for start in offsets.into_iter().rev() {
        result = format!(
            "{}https://{}",
            &result[..start],
            &result[start + "http://".len()..]
        );
    }
    result
}

// ── S043: backslash paths in frontmatter ────────────────────────────────

fn fix_frontmatter_backslash(mode: LintMode, exclude: &ExcludeSet, config: &LintConfig) -> bool {
    let mut fixed = false;
    let base_dirs: &[&str] = match mode {
        LintMode::Plugin => &["skills", ".claude/skills"],
        LintMode::Basic => &[".claude/skills"],
    };
    for base_dir in base_dirs {
        let skills = collect_skills(base_dir, exclude);
        for info in &skills {
            let skill_path = Path::new(base_dir).join(&info.dir_name).join("SKILL.md");
            if is_suppressed(config, LintRule::FrontmatterBackslash, &skill_path) {
                continue;
            }
            let has_backslash = info
                .fm_lines
                .iter()
                .any(|line| contains_backslash_path(line));
            if !has_backslash {
                continue;
            }

            let content = match fs::read_to_string(&skill_path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // Replace backslash paths in frontmatter lines only
            let mut new_content = String::new();
            let mut in_frontmatter = false;
            let mut fm_delim_count = 0;
            let mut changed = false;

            for line in content.lines() {
                if line == "---" {
                    fm_delim_count += 1;
                    if fm_delim_count == 1 {
                        in_frontmatter = true;
                    } else if fm_delim_count == 2 {
                        in_frontmatter = false;
                    }
                    new_content.push_str(line);
                } else if in_frontmatter && contains_backslash_path(line) {
                    new_content.push_str(&replace_backslash_paths(line));
                    changed = true;
                } else {
                    new_content.push_str(line);
                }
                new_content.push('\n');
            }
            if !content.ends_with('\n') && new_content.ends_with('\n') {
                new_content.pop();
            }

            if changed && fs::write(&skill_path, &new_content).is_ok() {
                log_fix(
                    LintRule::FrontmatterBackslash,
                    &format!(
                        "{base_dir}/{}/SKILL.md: replaced backslash paths in frontmatter",
                        info.dir_name
                    ),
                );
                fixed = true;
            }
        }
    }
    fixed
}

// ── S045: YAML list syntax for allowed-tools ────────────────────────────

fn fix_tools_list_syntax(mode: LintMode, exclude: &ExcludeSet, config: &LintConfig) -> bool {
    let mut fixed = false;
    let base_dirs: &[&str] = match mode {
        LintMode::Plugin => &["skills", ".claude/skills"],
        LintMode::Basic => &[".claude/skills"],
    };
    for base_dir in base_dirs {
        let skills = collect_skills(base_dir, exclude);
        for info in &skills {
            let skill_path = Path::new(base_dir).join(&info.dir_name).join("SKILL.md");
            if is_suppressed(config, LintRule::ToolsListSyntax, &skill_path) {
                continue;
            }
            if !frontmatter::field_exists(&info.fm_lines, "allowed-tools") {
                continue;
            }
            // Check for YAML list items
            let at_idx = match info
                .fm_lines
                .iter()
                .position(|l| l.starts_with("allowed-tools:"))
            {
                Some(i) => i,
                None => continue,
            };
            let list_items: Vec<String> = info.fm_lines[at_idx + 1..]
                .iter()
                .take_while(|l| {
                    l.is_empty() || l.starts_with(' ') || l.starts_with('\t') || l.starts_with("- ")
                })
                .filter(|l| l.trim_start().starts_with("- "))
                .map(|l| {
                    l.trim_start()
                        .strip_prefix("- ")
                        .unwrap_or(l.trim())
                        .trim()
                        .to_string()
                })
                .collect();

            if list_items.is_empty() {
                continue;
            }

            let content = match fs::read_to_string(&skill_path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // Rewrite: replace the allowed-tools: line and subsequent list items
            // with a single scalar line
            let comma_list = list_items.join(", ");
            let new_content = rewrite_yaml_list_to_scalar(&content, "allowed-tools", &comma_list);
            if let Some(new_content) = new_content {
                if fs::write(&skill_path, &new_content).is_ok() {
                    log_fix(
                        LintRule::ToolsListSyntax,
                        &format!(
                            "{base_dir}/{}/SKILL.md: converted allowed-tools to scalar: {comma_list}",
                            info.dir_name
                        ),
                    );
                    fixed = true;
                }
            }
        }
    }
    fixed
}

fn rewrite_yaml_list_to_scalar(content: &str, key: &str, scalar_value: &str) -> Option<String> {
    let prefix = format!("{key}:");
    let lines: Vec<&str> = content.lines().collect();
    let mut result: Vec<String> = Vec::new();
    let mut in_frontmatter = false;
    let mut fm_delim_count = 0;
    let mut skip_list_items = false;
    let mut changed = false;

    for line in &lines {
        if *line == "---" {
            fm_delim_count += 1;
            if fm_delim_count == 1 {
                in_frontmatter = true;
            } else if fm_delim_count == 2 {
                in_frontmatter = false;
                skip_list_items = false;
            }
            result.push(line.to_string());
            continue;
        }

        if skip_list_items {
            let trimmed = line.trim();
            if trimmed.is_empty()
                || line.starts_with(' ')
                || line.starts_with('\t')
                || trimmed.starts_with("- ")
            {
                continue; // Skip list item or continuation
            }
            skip_list_items = false;
        }

        if in_frontmatter && line.starts_with(&prefix) {
            result.push(format!("{key}: {scalar_value}"));
            skip_list_items = true;
            changed = true;
        } else {
            result.push(line.to_string());
        }
    }

    if !changed {
        return None;
    }

    let mut output = result.join("\n");
    if content.ends_with('\n') {
        output.push('\n');
    }
    Some(output)
}

// ── G001: $PWD in skill content ─────────────────────────────────────────

fn fix_pwd_in_skill(exclude: &ExcludeSet, config: &LintConfig) -> bool {
    let skills_dir = Path::new("skills");
    if !skills_dir.is_dir() {
        return false;
    }
    let mut fixed = false;
    for entry in traversal::shallow_directories(skills_dir, Path::new("."), None).entries {
        let path = entry.path;
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if name == "shared" {
            continue;
        }
        let skill_path = format!("skills/{name}/SKILL.md");
        if exclude.is_excluded(&skill_path)
            || is_suppressed(config, LintRule::PwdInSkill, &skill_path)
        {
            continue;
        }
        let skill_md = path.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }
        let content = match fs::read_to_string(&skill_md) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let Some(new_content) = replace_bundled_asset_prefixes(&content) else {
            continue;
        };

        if new_content != content && fs::write(&skill_md, &new_content).is_ok() {
            log_fix(
                LintRule::PwdInSkill,
                &format!(
                    "{skill_path}: replaced bundled-asset PWD prefix with ${{CLAUDE_PLUGIN_ROOT}}/"
                ),
            );
            fixed = true;
        }
    }
    fixed
}

// ── String helpers ──────────────────────────────────────────────────────

/// Replace only backslash path patterns on a line, leaving other backslashes intact.
fn replace_backslash_paths(line: &str) -> String {
    if !RE_BACKSLASH_PATH.is_match(line) || !contains_backslash_path(line) {
        return line.to_string();
    }

    RE_BACKSLASH_PATH_RUN
        .replace_all(line, |caps: &regex::Captures| {
            if is_named_tex_escape_pair(&caps[0]) {
                caps[0].to_string()
            } else {
                caps[0].replace('\\', "/")
            }
        })
        .to_string()
}

// ── Frontmatter helpers ─────────────────────────────────────────────────

/// Replace an exact line in the frontmatter section of a file.
fn replace_in_frontmatter(content: &str, old_line: &str, new_line: &str) -> Option<String> {
    let mut result = String::new();
    let mut in_fm = false;
    let mut fm_delim_count = 0;
    let mut replaced = false;

    for line in content.lines() {
        if line == "---" {
            fm_delim_count += 1;
            if fm_delim_count == 1 {
                in_fm = true;
            } else if fm_delim_count == 2 {
                in_fm = false;
            }
            result.push_str(line);
        } else if in_fm && !replaced && line.trim() == old_line.trim() {
            result.push_str(new_line);
            replaced = true;
        } else {
            result.push_str(line);
        }
        result.push('\n');
    }
    if !content.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }
    if replaced { Some(result) } else { None }
}

/// Remove the first line matching a prefix from the frontmatter section.
fn remove_frontmatter_line(content: &str, line_prefix: &str) -> Option<String> {
    let mut result = String::new();
    let mut in_fm = false;
    let mut fm_delim_count = 0;
    let mut removed = false;

    for line in content.lines() {
        if line == "---" {
            fm_delim_count += 1;
            if fm_delim_count == 1 {
                in_fm = true;
            } else if fm_delim_count == 2 {
                in_fm = false;
            }
            result.push_str(line);
            result.push('\n');
        } else if in_fm && !removed && line.starts_with(line_prefix) {
            removed = true;
            // Don't add this line
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }
    if !content.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }
    if removed { Some(result) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn fix_executability_hooks_includes_settings_local() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude").unwrap();
        std::fs::create_dir_all("scripts").unwrap();
        std::fs::write("scripts/local-hook.sh", "#!/usr/bin/env bash\n").unwrap();
        std::fs::set_permissions(
            "scripts/local-hook.sh",
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        std::fs::write(
            ".claude/settings.local.json",
            r#"{"hooks":[{"command":"${CLAUDE_PLUGIN_ROOT}/scripts/local-hook.sh"}]}"#,
        )
        .unwrap();

        assert!(fix_executability_hooks(
            LintMode::Plugin,
            &LintConfig::default()
        ));
        assert_ne!(
            std::fs::metadata("scripts/local-hook.sh")
                .unwrap()
                .permissions()
                .mode()
                & 0o111,
            0
        );
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn fix_executability_hooks_in_basic_mode() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude/hooks").unwrap();
        std::fs::write(".claude/hooks/check.py", "#!/usr/bin/env python3\n").unwrap();
        std::fs::set_permissions(
            ".claude/hooks/check.py",
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        std::fs::write(
            ".claude/settings.json",
            r#"{"hooks":[{"command":"\"${CLAUDE_PROJECT_DIR}\"/.claude/hooks/check.py"}]}"#,
        )
        .unwrap();

        assert!(fix_executability_hooks(
            LintMode::Basic,
            &LintConfig::default()
        ));
        assert!(!fix_executability_hooks(
            LintMode::Basic,
            &LintConfig::default()
        ));
        assert_ne!(
            std::fs::metadata(".claude/hooks/check.py")
                .unwrap()
                .permissions()
                .mode()
                & 0o111,
            0
        );
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn fix_executability_hooks_does_not_follow_external_symlink() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let tmp = tempfile::tempdir().unwrap();
        let external = tempfile::NamedTempFile::new().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude").unwrap();
        std::fs::set_permissions(external.path(), std::fs::Permissions::from_mode(0o644)).unwrap();
        symlink(external.path(), ".claude/external-hook").unwrap();
        std::fs::write(
            ".claude/settings.json",
            r#"{"hooks":[{"command":"$PWD/.claude/external-hook"}]}"#,
        )
        .unwrap();

        assert!(!fix_executability_hooks(
            LintMode::Basic,
            &LintConfig::default()
        ));
        assert_eq!(
            std::fs::metadata(external.path())
                .unwrap()
                .permissions()
                .mode()
                & 0o111,
            0
        );
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn fix_executability_hooks_includes_manifest_declared_config() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude-plugin").unwrap();
        std::fs::create_dir_all("config").unwrap();
        std::fs::create_dir_all("scripts").unwrap();
        std::fs::write("scripts/declared-hook.sh", "#!/usr/bin/env bash\n").unwrap();
        std::fs::set_permissions(
            "scripts/declared-hook.sh",
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        std::fs::write(
            ".claude-plugin/plugin.json",
            r#"{"name":"declared-hooks","hooks":"config/hooks.json"}"#,
        )
        .unwrap();
        std::fs::write(
            "config/hooks.json",
            r#"{"hooks":{"PreToolUse":[{"hooks":[{"type":"command","command":"${CLAUDE_PLUGIN_ROOT}/scripts/declared-hook.sh"}]}]}}"#,
        )
        .unwrap();

        assert!(fix_executability_hooks(
            LintMode::Plugin,
            &LintConfig::default()
        ));
        assert_ne!(
            std::fs::metadata("scripts/declared-hook.sh")
                .unwrap()
                .permissions()
                .mode()
                & 0o111,
            0
        );
    }

    #[test]
    fn replace_in_frontmatter_basic() {
        let content = "---\nname: old\ndescription: test\n---\nbody\n";
        let result = replace_in_frontmatter(content, "name: old", "name: new").unwrap();
        assert!(result.contains("name: new"));
        assert!(!result.contains("name: old"));
    }

    #[test]
    fn replace_in_frontmatter_no_match() {
        let content = "---\nname: foo\n---\nbody\n";
        assert!(replace_in_frontmatter(content, "name: bar", "name: baz").is_none());
    }

    #[test]
    fn remove_frontmatter_line_basic() {
        let content = "---\nname: foo\nargument-hint:\n---\nbody\n";
        let result = remove_frontmatter_line(content, "argument-hint:").unwrap();
        assert!(!result.contains("argument-hint"));
        assert!(result.contains("name: foo"));
    }

    #[test]
    #[serial_test::serial]
    fn fix_frontmatter_field_empty_removes_a_bare_field_via_invalid_yaml_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude/skills/empty").unwrap();
        std::fs::write(
            ".claude/skills/empty/SKILL.md",
            "---\nname: empty\ndescription: A valid description\nargument-hint:\n---\nBody\n",
        )
        .unwrap();

        let skills = collect_skills(".claude/skills", &ExcludeSet::default());
        assert_eq!(skills.len(), 1);
        assert!(frontmatter::parse_yaml_strict(&skills[0].fm_lines).is_err());
        assert_eq!(
            single_line_frontmatter_field_index(&skills[0].fm_lines, "argument-hint"),
            Some(2)
        );

        assert!(fix_frontmatter_field_empty(
            LintMode::Basic,
            &ExcludeSet::default(),
            &LintConfig::default()
        ));
        assert!(
            !std::fs::read_to_string(".claude/skills/empty/SKILL.md")
                .unwrap()
                .contains("argument-hint:")
        );
    }

    #[test]
    fn merge_consecutive_bash_basic() {
        let content = "---\nname: test\n---\n```bash\necho a\n```\n\n```bash\necho b\n```\n";
        let result = merge_first_consecutive_bash(content).unwrap();
        // Should have only one bash block
        assert_eq!(result.matches("```bash").count(), 1);
        assert!(result.contains("echo a"));
        assert!(result.contains("echo b"));
    }

    #[test]
    fn merge_consecutive_bash_no_consecutive() {
        let content =
            "---\nname: test\n---\n```bash\necho a\n```\nsome text\n```bash\necho b\n```\n";
        assert!(merge_first_consecutive_bash(content).is_none());
    }

    #[test]
    fn replace_http_urls_basic() {
        let content = "Visit http://api.foo.dev for details";
        let result = replace_http_urls(content);
        assert_eq!(result, "Visit https://api.foo.dev for details");
    }

    #[test]
    fn replace_http_urls_excludes_localhost() {
        let content = "Use http://localhost:3000 for dev";
        let result = replace_http_urls(content);
        assert_eq!(result, content); // No change
    }

    #[test]
    fn replace_http_urls_excludes_example_com() {
        let content = "See http://example.com/docs for reference";
        let result = replace_http_urls(content);
        assert_eq!(result, content);
    }

    #[test]
    fn s022_imports_the_validator_detection_regex() {
        assert!(std::ptr::eq(
            &*RE_BACKSLASH_PATH,
            &*crate::validators::skill_content::RE_BACKSLASH_PATH,
        ));
    }

    #[test]
    fn replace_backslash_paths_converts_full_runs_only() {
        assert_eq!(
            replace_backslash_paths(r"Open C:\Users\name and \dir\file\last; keep \n."),
            "Open C:/Users/name and /dir/file/last; keep \\n."
        );
        assert_eq!(
            replace_backslash_paths(r"Use \alpha\beta and \\server\share."),
            r"Use \alpha\beta and //server/share."
        );
    }

    #[test]
    fn replace_http_urls_keeps_xml_identifier_but_rewrites_link() {
        // Evidence 1 (issue #353): the XML namespace identifier is untouched
        // while a genuine plain-HTTP link on another line is upgraded.
        let content = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\">\nDownload from http://api.foo.dev/asset\n";
        let once = replace_http_urls(content);
        assert!(
            once.contains("xmlns=\"http://www.w3.org/2000/svg\""),
            "identifier changed: {once}"
        );
        assert!(
            once.contains("https://api.foo.dev/asset"),
            "link not upgraded: {once}"
        );
        // Idempotent: a second pass changes nothing.
        assert_eq!(replace_http_urls(&once), once);
    }

    #[test]
    fn replace_http_urls_excludes_reserved_name_hosts() {
        for content in [
            "See http://www.example.com/guide for the walkthrough",
            "Try http://foo.test/x locally",
            "Placeholder http://demo.invalid/ endpoint",
        ] {
            assert_eq!(replace_http_urls(content), content, "content: {content}");
        }
    }

    #[test]
    fn rewrite_yaml_list_to_scalar_basic() {
        let content = "---\nname: test\nallowed-tools:\n- Bash\n- Read\n- Write\n---\nbody\n";
        let result =
            rewrite_yaml_list_to_scalar(content, "allowed-tools", "Bash, Read, Write").unwrap();
        assert!(result.contains("allowed-tools: Bash, Read, Write"));
        assert!(!result.contains("- Bash"));
    }

    #[test]
    fn rewrite_yaml_list_to_scalar_no_match() {
        let content = "---\nname: test\n---\nbody\n";
        assert!(rewrite_yaml_list_to_scalar(content, "allowed-tools", "Bash").is_none());
    }
}
