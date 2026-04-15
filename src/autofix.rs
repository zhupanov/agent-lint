use crate::config::ExcludeSet;
use crate::context::LintMode;
use crate::fence::CodeFenceTracker;
use crate::frontmatter;
use crate::rules::LintRule;
use crate::validators::skills::collect_skills;
use regex::Regex;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::LazyLock;

// Reuse the same regexes validators use.
static RE_BACKSLASH_PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[A-Za-z]:\\[A-Za-z]|\\[A-Za-z][A-Za-z0-9_-]*\\[A-Za-z]").unwrap()
});
static RE_HTTP_URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"http://[a-zA-Z0-9]").unwrap());
static RE_XML_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").unwrap());
static RE_BASH_FENCE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^```(bash|sh|shell)\s*$").unwrap());
static RE_NAME_INVALID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^a-z0-9-]").unwrap());

/// Attempt to fix all instances of a given auto-fixable rule.
/// Returns `true` if at least one file was modified.
pub fn apply_fix(rule: LintRule, mode: LintMode, exclude: &ExcludeSet) -> bool {
    match rule {
        LintRule::HookNotExecutable => fix_executability_hooks(mode),
        LintRule::ScriptNotExecutable => fix_executability_scripts(mode, exclude),
        LintRule::FrontmatterNameMismatch => fix_frontmatter_name_mismatch(exclude),
        LintRule::FrontmatterFieldEmpty => fix_frontmatter_field_empty(mode, exclude),
        LintRule::NameHasXml => fix_name_has_xml(mode, exclude),
        LintRule::DescHasXml => fix_desc_has_xml(mode, exclude),
        LintRule::ConsecutiveBash => fix_consecutive_bash(mode, exclude),
        LintRule::BackslashPath => fix_backslash_path(mode, exclude),
        LintRule::NonHttpsUrl => fix_non_https_url(mode, exclude),
        LintRule::FrontmatterBackslash => fix_frontmatter_backslash(mode, exclude),
        LintRule::ToolsListSyntax => fix_tools_list_syntax(mode, exclude),
        LintRule::PwdInSkill => fix_pwd_in_skill(exclude),
        _ => false,
    }
}

fn log_fix(rule: LintRule, msg: &str) {
    let _ = writeln!(
        std::io::stderr(),
        "fixed[{}/{}]: {msg}",
        rule.code(),
        rule.name()
    );
}

// ── H005: chmod +x on hook scripts ──────────────────────────────────────

#[cfg(unix)]
fn fix_executability_hooks(mode: LintMode) -> bool {
    use crate::context::collect_json_strings;

    if mode != LintMode::Plugin {
        return false;
    }

    let mut fixed = false;
    let re_plugin = Regex::new(r"\$\{CLAUDE_PLUGIN_ROOT\}/[a-zA-Z0-9._/-]+\.sh").unwrap();
    let re_pwd = Regex::new(r"\$PWD/[a-zA-Z0-9._/-]+\.sh").unwrap();

    // Check hooks.json
    for json_path in &["hooks/hooks.json", ".claude/settings.json"] {
        let content = match fs::read_to_string(json_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let val: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let strings = collect_json_strings(&val);

        for raw in &strings {
            for cap in re_plugin.find_iter(raw) {
                let reference = cap.as_str();
                let rel = reference.replacen("${CLAUDE_PLUGIN_ROOT}/", "", 1);
                if make_executable(&rel) {
                    log_fix(
                        LintRule::HookNotExecutable,
                        &format!("made executable: {rel}"),
                    );
                    fixed = true;
                }
            }
            for cap in re_pwd.find_iter(raw) {
                let reference = cap.as_str();
                let rel = reference.replacen("$PWD/", "", 1);
                if make_executable(&rel) {
                    log_fix(
                        LintRule::HookNotExecutable,
                        &format!("made executable: {rel}"),
                    );
                    fixed = true;
                }
            }
        }
    }
    fixed
}

#[cfg(not(unix))]
fn fix_executability_hooks(_mode: LintMode) -> bool {
    false
}

// ── G003: chmod +x on script files ──────────────────────────────────────

#[cfg(unix)]
fn fix_executability_scripts(mode: LintMode, exclude: &ExcludeSet) -> bool {
    use crate::validators::hygiene::scripts::{BASIC_SCRIPT_DIRS, PLUGIN_SCRIPT_DIRS};

    let dirs = match mode {
        LintMode::Plugin => PLUGIN_SCRIPT_DIRS,
        LintMode::Basic => BASIC_SCRIPT_DIRS,
    };

    let mut fixed = false;
    for pattern in dirs {
        for dir in glob_dirs(pattern) {
            let entries = match fs::read_dir(&dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let name = match path.file_name().and_then(|n| n.to_str()) {
                    Some(n) if n.ends_with(".sh") => n,
                    _ => continue,
                };
                let display = path.display().to_string();
                if exclude.is_excluded(&display) {
                    continue;
                }
                if make_executable(path.to_str().unwrap_or("")) {
                    log_fix(
                        LintRule::ScriptNotExecutable,
                        &format!("made executable: {display}"),
                    );
                    fixed = true;
                }
                let _ = name;
            }
        }
    }
    fixed
}

#[cfg(not(unix))]
fn fix_executability_scripts(_mode: LintMode, _exclude: &ExcludeSet) -> bool {
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
    let new_mode = mode | 0o111;
    fs::set_permissions(p, std::os::unix::fs::PermissionsExt::from_mode(new_mode)).is_ok()
}

/// Expand simple glob patterns like "scripts" or "skills/*/scripts".
fn glob_dirs(pattern: &str) -> Vec<std::path::PathBuf> {
    if !pattern.contains('*') {
        let p = Path::new(pattern);
        if p.is_dir() {
            return vec![p.to_path_buf()];
        }
        return vec![];
    }
    // Split on '*' — only support one-level wildcard
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() != 2 {
        return vec![];
    }
    let prefix = parts[0].trim_end_matches('/');
    let suffix = parts[1].trim_start_matches('/');
    let base = Path::new(prefix);
    if !base.is_dir() {
        return vec![];
    }
    let mut result = Vec::new();
    if let Ok(entries) = fs::read_dir(base) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let candidate = path.join(suffix);
            if candidate.is_dir() {
                result.push(candidate);
            }
        }
    }
    result
}

// ── S006: frontmatter name mismatch ─────────────────────────────────────

fn fix_frontmatter_name_mismatch(exclude: &ExcludeSet) -> bool {
    let mut fixed = false;
    for base_dir in &["skills", ".claude/skills"] {
        let dir = Path::new(base_dir);
        if !dir.is_dir() {
            continue;
        }
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => continue,
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
            let name = match frontmatter::get_field(&fm_lines, "name") {
                Some(n) => n,
                None => continue,
            };
            if name == dir_name {
                continue;
            }

            // Only fix in public skills (check_name_match=true only for "skills")
            if *base_dir != "skills" {
                continue;
            }

            // Replace the raw name line (handles quoted values)
            let raw_name_line = fm_lines
                .iter()
                .find(|l| l.starts_with("name:"))
                .cloned()
                .unwrap_or_default();
            let new_line = format!("name: {dir_name}");
            if let Some(new_content) = replace_in_frontmatter(&content, &raw_name_line, &new_line) {
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

fn fix_frontmatter_field_empty(mode: LintMode, exclude: &ExcludeSet) -> bool {
    let mut fixed = false;
    let base_dirs: &[&str] = match mode {
        LintMode::Plugin => &["skills", ".claude/skills"],
        LintMode::Basic => &[".claude/skills"],
    };
    for base_dir in base_dirs {
        let skills = collect_skills(base_dir, exclude);
        for info in &skills {
            let skill_md = format!("{base_dir}/{}/SKILL.md", info.dir_name);
            let skill_path = Path::new(base_dir).join(&info.dir_name).join("SKILL.md");
            let content = match fs::read_to_string(&skill_path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            for field in &["argument-hint", "allowed-tools"] {
                let prefix = format!("{field}:");
                let field_present = info.fm_lines.iter().any(|line| line.starts_with(&prefix));
                if !field_present {
                    continue;
                }
                let val = frontmatter::get_field(&info.fm_lines, field);
                if val.is_some() {
                    continue; // Not empty
                }

                // For allowed-tools: skip if YAML list items follow (S045 handles that)
                if *field == "allowed-tools" {
                    let has_list_items = info
                        .fm_lines
                        .iter()
                        .position(|l| l.starts_with("allowed-tools:"))
                        .is_some_and(|i| {
                            info.fm_lines[i + 1..]
                                .iter()
                                .take_while(|l| {
                                    l.is_empty()
                                        || l.starts_with(' ')
                                        || l.starts_with('\t')
                                        || l.starts_with("- ")
                                })
                                .any(|l| l.trim_start().starts_with("- "))
                        });
                    if has_list_items {
                        continue;
                    }
                }

                // FINDING_8: skip removing argument-hint if body uses $ARGUMENTS
                if *field == "argument-hint" && info.body.contains("$ARGUMENTS") {
                    continue;
                }

                // Remove the empty field line from the file
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

// ── S013: XML tags in name ──────────────────────────────────────────────

fn fix_name_has_xml(mode: LintMode, exclude: &ExcludeSet) -> bool {
    fix_frontmatter_field_regex(
        mode,
        exclude,
        "name",
        &RE_XML_TAG,
        "",
        LintRule::NameHasXml,
        "stripped XML tags from name",
    )
}

// ── S018: XML tags in description ───────────────────────────────────────

fn fix_desc_has_xml(mode: LintMode, exclude: &ExcludeSet) -> bool {
    fix_frontmatter_field_regex(
        mode,
        exclude,
        "description",
        &RE_XML_TAG,
        "",
        LintRule::DescHasXml,
        "stripped XML tags from description",
    )
}

/// Generic fix: apply a regex replacement on a frontmatter field value.
fn fix_frontmatter_field_regex(
    mode: LintMode,
    exclude: &ExcludeSet,
    field_name: &str,
    pattern: &Regex,
    replacement: &str,
    rule: LintRule,
    fix_desc: &str,
) -> bool {
    let mut fixed = false;
    let base_dirs: &[&str] = match mode {
        LintMode::Plugin => &["skills", ".claude/skills"],
        LintMode::Basic => &[".claude/skills"],
    };
    for base_dir in base_dirs {
        let skills = collect_skills(base_dir, exclude);
        for info in &skills {
            let value = match frontmatter::get_field(&info.fm_lines, field_name) {
                Some(v) => v,
                None => continue,
            };
            if !pattern.is_match(&value) {
                continue;
            }
            let new_value = pattern.replace_all(&value, replacement).to_string();
            let new_value = new_value.trim().to_string();
            if new_value == value || new_value.is_empty() {
                continue;
            }

            let skill_path = Path::new(base_dir).join(&info.dir_name).join("SKILL.md");
            let content = match fs::read_to_string(&skill_path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // Use raw line from fm_lines (handles quoted values)
            let prefix = format!("{field_name}:");
            let raw_line = info
                .fm_lines
                .iter()
                .find(|l| l.starts_with(&prefix))
                .cloned()
                .unwrap_or_default();
            let new_line = format!("{field_name}: {new_value}");
            if let Some(new_content) = replace_in_frontmatter(&content, &raw_line, &new_line) {
                if fs::write(&skill_path, new_content).is_ok() {
                    log_fix(
                        rule,
                        &format!("{base_dir}/{}/SKILL.md: {fix_desc}", info.dir_name),
                    );
                    fixed = true;
                }
            }
        }
    }
    fixed
}

// ── S021: consecutive bash code blocks ──────────────────────────────────

fn fix_consecutive_bash(mode: LintMode, exclude: &ExcludeSet) -> bool {
    let mut fixed = false;
    let base_dirs: &[&str] = match mode {
        LintMode::Plugin => &["skills", ".claude/skills"],
        LintMode::Basic => &[".claude/skills"],
    };
    for base_dir in base_dirs {
        let skills = collect_skills(base_dir, exclude);
        for info in &skills {
            let skill_path = Path::new(base_dir).join(&info.dir_name).join("SKILL.md");
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

fn fix_backslash_path(mode: LintMode, exclude: &ExcludeSet) -> bool {
    let mut fixed = false;
    let base_dirs: &[&str] = match mode {
        LintMode::Plugin => &["skills", ".claude/skills"],
        LintMode::Basic => &[".claude/skills"],
    };
    for base_dir in base_dirs {
        let skills = collect_skills(base_dir, exclude);
        for info in &skills {
            // Check outside code fences (matching validator)
            let has_backslash = crate::fence::lines_outside_fences(&info.body)
                .any(|line| RE_BACKSLASH_PATH.is_match(line));
            if !has_backslash {
                continue;
            }

            let skill_path = Path::new(base_dir).join(&info.dir_name).join("SKILL.md");
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
                if class == crate::fence::LineClass::Outside && RE_BACKSLASH_PATH.is_match(line) {
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

fn fix_non_https_url(mode: LintMode, exclude: &ExcludeSet) -> bool {
    let mut fixed = false;
    let base_dirs: &[&str] = match mode {
        LintMode::Plugin => &["skills", ".claude/skills"],
        LintMode::Basic => &[".claude/skills"],
    };
    for base_dir in base_dirs {
        let skills = collect_skills(base_dir, exclude);
        for info in &skills {
            if info.body.trim().is_empty() {
                continue;
            }
            // Check if there are any non-excluded http:// URLs (matching validator)
            let has_fixable = RE_HTTP_URL.find_iter(&info.body).any(|m| {
                let start = m.start();
                let after = &info.body[start + 7..]; // skip "http://"
                !after.starts_with("localhost")
                    && !after.starts_with("127.0.0.1")
                    && !after.starts_with("0.0.0.0")
                    && !after.starts_with("example.com")
                    && !after.starts_with("example.org")
            });
            if !has_fixable {
                continue;
            }

            let skill_path = Path::new(base_dir).join(&info.dir_name).join("SKILL.md");
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
    // Process in reverse order to preserve positions
    let matches: Vec<_> = RE_HTTP_URL.find_iter(content).collect();
    for m in matches.into_iter().rev() {
        let start = m.start();
        let after = &content[start + 7..]; // skip "http://"
        if after.starts_with("localhost")
            || after.starts_with("127.0.0.1")
            || after.starts_with("0.0.0.0")
            || after.starts_with("example.com")
            || after.starts_with("example.org")
        {
            continue;
        }
        // Replace "http://" with "https://"
        result = format!("{}https://{}", &result[..start], &result[start + 7..]);
    }
    result
}

// ── S043: backslash paths in frontmatter ────────────────────────────────

fn fix_frontmatter_backslash(mode: LintMode, exclude: &ExcludeSet) -> bool {
    let mut fixed = false;
    let base_dirs: &[&str] = match mode {
        LintMode::Plugin => &["skills", ".claude/skills"],
        LintMode::Basic => &[".claude/skills"],
    };
    for base_dir in base_dirs {
        let skills = collect_skills(base_dir, exclude);
        for info in &skills {
            let has_backslash = info.fm_lines.iter().any(|l| RE_BACKSLASH_PATH.is_match(l));
            if !has_backslash {
                continue;
            }

            let skill_path = Path::new(base_dir).join(&info.dir_name).join("SKILL.md");
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
                } else if in_frontmatter && RE_BACKSLASH_PATH.is_match(line) {
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

fn fix_tools_list_syntax(mode: LintMode, exclude: &ExcludeSet) -> bool {
    let mut fixed = false;
    let base_dirs: &[&str] = match mode {
        LintMode::Plugin => &["skills", ".claude/skills"],
        LintMode::Basic => &[".claude/skills"],
    };
    for base_dir in base_dirs {
        let skills = collect_skills(base_dir, exclude);
        for info in &skills {
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

            let skill_path = Path::new(base_dir).join(&info.dir_name).join("SKILL.md");
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

fn fix_pwd_in_skill(exclude: &ExcludeSet) -> bool {
    let skills_dir = Path::new("skills");
    if !skills_dir.is_dir() {
        return false;
    }
    let mut fixed = false;
    let entries = match fs::read_dir(skills_dir) {
        Ok(e) => e,
        Err(_) => return false,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if name == "shared" {
            continue;
        }
        let skill_path = format!("skills/{name}/SKILL.md");
        if exclude.is_excluded(&skill_path) {
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

        // Replace $PWD/ and ${PWD}/ with ${CLAUDE_PLUGIN_ROOT}/
        let new_content = content
            .replace("$PWD/", "${CLAUDE_PLUGIN_ROOT}/")
            .replace("${PWD}/", "${CLAUDE_PLUGIN_ROOT}/");

        if new_content != content && fs::write(&skill_md, &new_content).is_ok() {
            log_fix(
                LintRule::PwdInSkill,
                &format!("{skill_path}: replaced $PWD/ with ${{CLAUDE_PLUGIN_ROOT}}/"),
            );
            fixed = true;
        }
    }
    fixed
}

// ── String helpers ──────────────────────────────────────────────────────

/// Replace only backslash path patterns on a line, leaving other backslashes intact.
fn replace_backslash_paths(line: &str) -> String {
    RE_BACKSLASH_PATH
        .replace_all(line, |caps: &regex::Captures| caps[0].replace('\\', "/"))
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
        let content = "Visit http://example.net for details";
        let result = replace_http_urls(content);
        assert_eq!(result, "Visit https://example.net for details");
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
