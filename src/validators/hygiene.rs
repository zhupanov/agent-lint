use crate::config::ExcludeSet;
use crate::context::{LintContext, LintMode, ManifestState, collect_json_strings};
use crate::diagnostic::DiagnosticCollector;
use crate::rules::LintRule;
use regex::Regex;
use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Directory patterns for Plugin mode script discovery (V10, --list-scripts).
pub const PLUGIN_SCRIPT_DIRS: &[&str] =
    &["scripts", "skills/*/scripts", ".claude/skills/*/scripts"];

/// Directory patterns for Basic mode script discovery (V10-adapted, --list-scripts).
pub const BASIC_SCRIPT_DIRS: &[&str] = &[".claude/skills/*/scripts"];

/// V8: ${CLAUDE_PLUGIN_ROOT} hygiene — public skills/*/SKILL.md must not use
/// $PWD/, ${PWD}/, or hardcoded paths (/Users/, /home/, /opt/).
pub fn validate_pwd_hygiene(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let skills_dir = Path::new("skills");
    if !skills_dir.is_dir() {
        return;
    }

    let re = Regex::new(r"[$]PWD/|[$]\{PWD\}/|/Users/|/home/|/opt/").unwrap();

    let entries = match fs::read_dir(skills_dir) {
        Ok(e) => e,
        Err(_) => return,
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

        if re.is_match(&content) {
            diag.report(
                LintRule::PwdInSkill,
                &format!(
                    "skills/{name}/SKILL.md uses $PWD/ or hardcoded path; use ${{CLAUDE_PLUGIN_ROOT}}/ instead"
                ),
            );
        }
    }
}

/// V9: Script reference integrity.
pub fn validate_script_references(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let re_pub = Regex::new(
        r"\$\{CLAUDE_PLUGIN_ROOT\}/(scripts|skills|\.claude/skills)/[a-zA-Z0-9._/-]+\.sh",
    )
    .unwrap();
    let re_priv = Regex::new(r"\$PWD/\.claude/skills/[a-zA-Z0-9._/-]+\.sh").unwrap();
    let re_placeholder = Regex::new(
        r"\$\{CLAUDE_PLUGIN_ROOT_PLACEHOLDER:-\$PWD\}/\.claude/skills/[a-zA-Z0-9._/-]+\.sh",
    )
    .unwrap();

    let mut seen = HashSet::new();

    for dir in &["skills", ".claude/skills"] {
        let base = Path::new(dir);
        if !base.is_dir() {
            continue;
        }
        for entry in WalkDir::new(base).into_iter().flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let display_path = path.display().to_string();
            if exclude.is_excluded(&display_path) {
                continue;
            }
            let content = match fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            for cap in re_pub.find_iter(&content) {
                let reference = cap.as_str().to_string();
                if seen.insert(reference.clone()) {
                    let rel = reference.replace("${CLAUDE_PLUGIN_ROOT}/", "");
                    if !Path::new(&rel).is_file() {
                        diag.report(
                            LintRule::ScriptRefMissing,
                            &format!(
                                "script reference missing on disk: {reference} (expected {rel})"
                            ),
                        );
                    }
                }
            }

            for cap in re_priv.find_iter(&content) {
                let reference = cap.as_str().to_string();
                if seen.insert(reference.clone()) {
                    let rel = reference.replace("$PWD/", "");
                    if !Path::new(&rel).is_file() {
                        diag.report(
                            LintRule::ScriptRefMissing,
                            &format!(
                                "script reference missing on disk: {reference} (expected {rel})"
                            ),
                        );
                    }
                }
            }

            for cap in re_placeholder.find_iter(&content) {
                let reference = cap.as_str().to_string();
                if seen.insert(reference.clone()) {
                    let rel = reference.replace("${CLAUDE_PLUGIN_ROOT_PLACEHOLDER:-$PWD}/", "");
                    if !Path::new(&rel).is_file() {
                        diag.report(
                            LintRule::ScriptRefMissing,
                            &format!(
                                "script reference missing on disk: {reference} (expected {rel})"
                            ),
                        );
                    }
                }
            }
        }
    }
}

/// V9-adapted: Script reference integrity for private .claude/skills/ only.
pub fn validate_private_script_references(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let re_priv = Regex::new(r"\$PWD/\.claude/skills/[a-zA-Z0-9._/-]+\.sh").unwrap();
    let re_placeholder = Regex::new(
        r"\$\{CLAUDE_PLUGIN_ROOT_PLACEHOLDER:-\$PWD\}/\.claude/skills/[a-zA-Z0-9._/-]+\.sh",
    )
    .unwrap();

    let mut seen = HashSet::new();
    let base = Path::new(".claude/skills");
    if !base.is_dir() {
        return;
    }

    for entry in WalkDir::new(base).into_iter().flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let display_path = path.display().to_string();
        if exclude.is_excluded(&display_path) {
            continue;
        }
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        for cap in re_priv.find_iter(&content) {
            let reference = cap.as_str().to_string();
            if seen.insert(reference.clone()) {
                let rel = reference.replace("$PWD/", "");
                if !Path::new(&rel).is_file() {
                    diag.report(
                        LintRule::ScriptRefMissing,
                        &format!("script reference missing on disk: {reference} (expected {rel})"),
                    );
                }
            }
        }

        for cap in re_placeholder.find_iter(&content) {
            let reference = cap.as_str().to_string();
            if seen.insert(reference.clone()) {
                let rel = reference.replace("${CLAUDE_PLUGIN_ROOT_PLACEHOLDER:-$PWD}/", "");
                if !Path::new(&rel).is_file() {
                    diag.report(
                        LintRule::ScriptRefMissing,
                        &format!("script reference missing on disk: {reference} (expected {rel})"),
                    );
                }
            }
        }
    }
}

/// V10: Executability — every .sh file under scripts/, skills/*/scripts/,
/// and .claude/skills/*/scripts/ must be chmod +x.
#[cfg(unix)]
pub fn validate_executability(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    check_executability_in_dirs(
        &["scripts", "skills/*/scripts", ".claude/skills/*/scripts"],
        diag,
        exclude,
    );
}

#[cfg(not(unix))]
pub fn validate_executability(_diag: &mut DiagnosticCollector, _exclude: &ExcludeSet) {}

/// V10-adapted: Executability for private .claude/skills/*/scripts/*.sh only.
#[cfg(unix)]
pub fn validate_private_executability(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    check_executability_in_dirs(&[".claude/skills/*/scripts"], diag, exclude);
}

#[cfg(not(unix))]
pub fn validate_private_executability(_diag: &mut DiagnosticCollector, _exclude: &ExcludeSet) {}

/// Expand glob-like directory patterns into concrete directory paths.
/// Supports multiple `*` wildcards (e.g., `skills/*/nested/*/scripts`).
pub fn expand_script_dirs(patterns: &[&str]) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for pattern in patterns {
        if pattern.contains('*') {
            let segments: Vec<&str> = pattern.split('/').collect();
            let mut candidates = vec![PathBuf::new()];
            for seg in &segments {
                let mut next = Vec::new();
                if *seg == "*" {
                    for base in &candidates {
                        let read_dir = if base.as_os_str().is_empty() {
                            fs::read_dir(".")
                        } else {
                            fs::read_dir(base)
                        };
                        if let Ok(entries) = read_dir {
                            for entry in entries.flatten() {
                                if entry.path().is_dir() {
                                    let child = if base.as_os_str().is_empty() {
                                        PathBuf::from(entry.file_name())
                                    } else {
                                        base.join(entry.file_name())
                                    };
                                    next.push(child);
                                }
                            }
                        }
                    }
                } else {
                    for base in &candidates {
                        let child = if base.as_os_str().is_empty() {
                            PathBuf::from(seg)
                        } else {
                            base.join(seg)
                        };
                        if child.is_dir() {
                            next.push(child);
                        }
                    }
                }
                candidates = next;
            }
            for c in candidates {
                if c.is_dir() {
                    dirs.push(c);
                }
            }
        } else {
            let dir = Path::new(pattern);
            if dir.is_dir() {
                dirs.push(dir.to_path_buf());
            }
        }
    }
    dirs
}

#[cfg(unix)]
fn check_executability_in_dirs(
    patterns: &[&str],
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
) {
    for dir in expand_script_dirs(patterns) {
        check_sh_executability(&dir, diag, exclude);
    }
}

/// Collect all .sh script paths for the given lint mode.
/// Returns sorted, deduplicated repo-relative paths.
pub fn collect_script_paths(mode: LintMode, exclude: &ExcludeSet) -> Vec<String> {
    let patterns = match mode {
        LintMode::Plugin => PLUGIN_SCRIPT_DIRS,
        LintMode::Basic => BASIC_SCRIPT_DIRS,
    };
    let mut paths = BTreeSet::new();
    for dir in expand_script_dirs(patterns) {
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.ends_with(".sh") {
                    let display = path.display().to_string();
                    if !exclude.is_excluded(&display) {
                        paths.insert(display);
                    }
                }
            }
        }
    }
    paths.into_iter().collect()
}

#[cfg(unix)]
fn check_sh_executability(dir: &Path, diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    use std::os::unix::fs::PermissionsExt;

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) if n.ends_with(".sh") => n.to_string(),
            _ => continue,
        };

        let display_path = path.display().to_string();
        if exclude.is_excluded(&display_path) {
            continue;
        }

        if let Ok(meta) = path.metadata() {
            if meta.permissions().mode() & 0o111 == 0 {
                diag.report(
                    LintRule::ScriptNotExecutable,
                    &format!("script not executable: {}", path.display()),
                );
                let _ = name;
            }
        }
    }
}

/// V11: Dead-script detection
pub fn validate_dead_scripts(
    ctx: &LintContext,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
) {
    let scripts_dir = Path::new("scripts");
    if !scripts_dir.is_dir() {
        return;
    }

    let mut references: HashSet<String> = HashSet::new();

    let re_ab = Regex::new(
        r"\$(\{CLAUDE_PLUGIN_ROOT\}|PWD)/(scripts|\.claude/skills/[^/]+/scripts)/[a-zA-Z0-9._-]+\.sh",
    )
    .unwrap();

    let re_placeholder = Regex::new(
        r"\$\{CLAUDE_PLUGIN_ROOT_PLACEHOLDER:-\$PWD\}/\.claude/skills/[a-zA-Z0-9._/-]+\.sh",
    )
    .unwrap();

    for dir in &[
        "skills",
        ".claude/skills",
        "hooks",
        ".github/workflows",
        "scripts",
    ] {
        let base = Path::new(dir);
        if !base.is_dir() {
            continue;
        }
        for entry in WalkDir::new(base).into_iter().flatten() {
            if !entry.path().is_file() {
                continue;
            }
            let entry_display = entry.path().display().to_string();
            if exclude.is_excluded(&entry_display) {
                continue;
            }
            let content = match fs::read_to_string(entry.path()) {
                Ok(c) => c,
                Err(_) => continue,
            };
            for cap in re_ab.find_iter(&content) {
                let s = cap.as_str();
                let rel = if s.starts_with("${CLAUDE_PLUGIN_ROOT}/") {
                    s.replace("${CLAUDE_PLUGIN_ROOT}/", "")
                } else if s.starts_with("$PWD/") {
                    s.replace("$PWD/", "")
                } else {
                    continue;
                };
                references.insert(rel);
            }
            for cap in re_placeholder.find_iter(&content) {
                let s = cap.as_str();
                let rel = s.replace("${CLAUDE_PLUGIN_ROOT_PLACEHOLDER:-$PWD}/", "");
                references.insert(rel);
            }
        }
    }

    // Extract script references from pre-parsed settings.json and hooks.json
    // via LintContext instead of reading files directly from disk.
    // Only re_ab applies here (not re_placeholder, which is directory-walk only).
    // For Invalid/Missing manifests, skip extraction — other validators report those errors.
    for manifest in [&ctx.settings_json, &ctx.hooks_json] {
        if let ManifestState::Parsed(val) = manifest {
            for s in collect_json_strings(val) {
                for cap in re_ab.find_iter(&s) {
                    let matched = cap.as_str();
                    let rel = if matched.starts_with("${CLAUDE_PLUGIN_ROOT}/") {
                        matched.replace("${CLAUDE_PLUGIN_ROOT}/", "")
                    } else if matched.starts_with("$PWD/") {
                        matched.replace("$PWD/", "")
                    } else {
                        continue;
                    };
                    references.insert(rel);
                }
            }
        }
    }

    let re_c = Regex::new(r"\$SCRIPT_DIR/[a-zA-Z0-9._-]+\.sh").unwrap();
    for entry in WalkDir::new(scripts_dir).into_iter().flatten() {
        if !entry.path().is_file() {
            continue;
        }
        let script_display = entry.path().display().to_string();
        if exclude.is_excluded(&script_display) {
            continue;
        }
        let content = match fs::read_to_string(entry.path()) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for cap in re_c.find_iter(&content) {
            let s = cap.as_str();
            let name = s.replace("$SCRIPT_DIR/", "scripts/");
            references.insert(name);
        }
    }

    let re_d = Regex::new(r"(^|[^a-zA-Z0-9._/-])scripts/[a-zA-Z0-9._-]+\.sh").unwrap();
    let re_extract = Regex::new(r"scripts/[a-zA-Z0-9._-]+\.sh").unwrap();

    for dir in &[".github/workflows"] {
        let base = Path::new(dir);
        if !base.is_dir() {
            continue;
        }
        if let Ok(entries) = fs::read_dir(base) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !name.ends_with(".yaml") && !name.ends_with(".yml") {
                    continue;
                }
                let content = match fs::read_to_string(&path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                let stripped = strip_yaml_comments(&content);
                for cap in re_d.find_iter(&stripped) {
                    if let Some(m) = re_extract.find(cap.as_str()) {
                        references.insert(m.as_str().to_string());
                    }
                }
            }
        }
    }

    // Extract bare scripts/...sh references from pre-parsed settings/hooks manifests.
    // Only re_d applies here for bare script references.
    for manifest in [&ctx.settings_json, &ctx.hooks_json] {
        if let ManifestState::Parsed(val) = manifest {
            for s in collect_json_strings(val) {
                for cap in re_d.find_iter(&s) {
                    if let Some(m) = re_extract.find(cap.as_str()) {
                        references.insert(m.as_str().to_string());
                    }
                }
            }
        }
    }

    let shared_dir = Path::new("skills/shared");
    if shared_dir.is_dir() {
        for entry in WalkDir::new(shared_dir).into_iter().flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let shared_display = path.display().to_string();
            if exclude.is_excluded(&shared_display) {
                continue;
            }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !name.ends_with(".md") {
                continue;
            }
            let content = match fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let fenced = extract_code_fences(&content);
            for cap in re_d.find_iter(&fenced) {
                if let Some(m) = re_extract.find(cap.as_str()) {
                    references.insert(m.as_str().to_string());
                }
            }
        }
    }

    if let Ok(entries) = fs::read_dir(scripts_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) if n.ends_with(".sh") => n.to_string(),
                _ => continue,
            };
            let key = format!("scripts/{name}");
            if exclude.is_excluded(&key) {
                continue;
            }
            if !references.contains(&key) {
                diag.report(
                    LintRule::DeadScript,
                    &format!(
                        "dead script (no structured invocation reference found): scripts/{name}"
                    ),
                );
            }
        }
    }
}

/// V14: SECURITY.md presence
pub fn validate_security_md(diag: &mut DiagnosticCollector) {
    if !Path::new("SECURITY.md").is_file() {
        diag.report(
            LintRule::SecurityMdMissing,
            "SECURITY.md is missing from repo root",
        );
    }
}

/// G006: TODO/FIXME/HACK/XXX markers in published skill content.
/// Scans skills/*/SKILL.md body text outside code fences.
pub fn validate_todo_in_skills(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let skills_dir = Path::new("skills");
    if !skills_dir.is_dir() {
        return;
    }

    let re_todo = Regex::new(r"(?i)\b(TODO|FIXME|HACK|XXX)\b").unwrap();

    let entries = match fs::read_dir(skills_dir) {
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

        let skill_path = format!("skills/{dir_name}/SKILL.md");
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

        // Extract body after frontmatter
        let body = crate::frontmatter::extract_body(&content);

        for line in crate::fence::lines_outside_fences(body) {
            if let Some(m) = re_todo.find(line) {
                diag.report(
                    LintRule::TodoInSkill,
                    &format!(
                        "skills/{dir_name}/SKILL.md contains {} marker; remove before publishing",
                        m.as_str()
                    ),
                );
                break; // Report once per file
            }
        }
    }
}

/// G007: TODO/FIXME/HACK/XXX markers in agent .md files.
/// Scans agents/*.md body text outside code fences.
pub fn validate_todo_in_agents(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let agents_dir = Path::new("agents");
    if !agents_dir.is_dir() {
        return;
    }

    let re_todo = Regex::new(r"(?i)\b(TODO|FIXME|HACK|XXX)\b").unwrap();

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

        let body = crate::frontmatter::extract_body(&content);
        for line in crate::fence::lines_outside_fences(body) {
            if let Some(m) = re_todo.find(line) {
                diag.report(
                    LintRule::TodoInAgent,
                    &format!(
                        "agents/{name} contains {} marker; remove before publishing",
                        m.as_str()
                    ),
                );
                break; // Report once per file
            }
        }
    }
}

fn strip_yaml_comments(content: &str) -> String {
    let re_full = Regex::new(r"^[[:space:]]*#").unwrap();

    content
        .lines()
        .filter(|line| !re_full.is_match(line))
        .map(strip_trailing_yaml_comment)
        .collect::<Vec<_>>()
        .join("\n")
}

fn strip_trailing_yaml_comment(line: &str) -> String {
    let mut in_quote: Option<char> = None;
    let mut prev_was_ws = false;
    let mut skip_next = false;

    for (byte_pos, ch) in line.char_indices() {
        if skip_next {
            skip_next = false;
            prev_was_ws = ch.is_whitespace();
            continue;
        }
        match in_quote {
            Some(q) => {
                if q == '"' && ch == '\\' {
                    skip_next = true;
                } else if q == '\'' && ch == '\'' {
                    let rest = &line[byte_pos + ch.len_utf8()..];
                    if rest.starts_with('\'') {
                        skip_next = true;
                    } else {
                        in_quote = None;
                    }
                } else if ch == q {
                    in_quote = None;
                }
            }
            None => {
                if ch == '"' || ch == '\'' {
                    in_quote = Some(ch);
                } else if ch == '#' && prev_was_ws {
                    return line[..byte_pos].trim_end().to_string();
                }
            }
        }
        prev_was_ws = ch.is_whitespace();
    }

    line.to_string()
}

fn extract_code_fences(content: &str) -> String {
    crate::fence::lines_inside_fences(content)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_yaml_comments() {
        let input = "key: value\n# comment\nkey2: val2 # trailing\n  # indented comment\n";
        let result = strip_yaml_comments(input);
        assert!(result.contains("key: value"));
        assert!(!result.contains("# comment"));
        assert!(result.contains("key2: val2"));
        assert!(!result.contains("trailing"));
    }

    #[test]
    fn test_strip_yaml_comments_preserves_hash_in_double_quotes() {
        let result = strip_yaml_comments("key: \"value with # hash\"\n");
        assert!(result.contains("key: \"value with # hash\""));
    }

    #[test]
    fn test_strip_yaml_comments_preserves_hash_in_single_quotes() {
        let result = strip_yaml_comments("key: 'value with # hash'\n");
        assert!(result.contains("key: 'value with # hash'"));
    }

    #[test]
    fn test_strip_yaml_comments_strips_after_closing_quote() {
        let result = strip_yaml_comments("key: \"quoted\" # comment\n");
        assert!(result.contains("key: \"quoted\""));
        assert!(!result.contains("comment"));
    }

    #[test]
    fn test_strip_yaml_comments_preserves_unclosed_quote() {
        let result = strip_yaml_comments("key: \"unterminated # hash\n");
        assert!(result.contains("key: \"unterminated # hash"));
    }

    #[test]
    fn test_strip_yaml_comments_multibyte_chars() {
        let result = strip_yaml_comments("clé: \"über\" # comment\n");
        assert!(result.contains("clé: \"über\""));
        assert!(!result.contains("comment"));
    }

    #[test]
    fn test_strip_yaml_comments_escaped_double_quote() {
        let result = strip_yaml_comments("key: \"say \\\"hello\\\" # still in\" # comment\n");
        assert!(result.contains("# still in"));
        assert!(!result.contains("# comment"));
    }

    #[test]
    fn test_strip_yaml_comments_doubled_single_quote() {
        let result = strip_yaml_comments("key: 'it''s a # value' # comment\n");
        assert!(result.contains("it''s a # value"));
        assert!(!result.contains("# comment"));
    }

    #[test]
    fn test_extract_code_fences() {
        let input = "text\n```bash\nscripts/foo.sh\n```\nmore text\n~~~\nscripts/bar.sh\n~~~\n";
        let result = extract_code_fences(input);
        assert!(result.contains("scripts/foo.sh"));
        assert!(result.contains("scripts/bar.sh"));
        assert!(!result.contains("text"));
    }

    #[test]
    fn test_code_fence_with_language_tag() {
        let input = "```python\nprint('hello')\n```\n";
        let result = extract_code_fences(input);
        assert!(result.contains("print('hello')"));
    }

    // V8: validate_pwd_hygiene
    #[test]
    #[serial_test::serial]
    fn test_v8_clean_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: s\n---\nUses ${CLAUDE_PLUGIN_ROOT}/scripts/foo.sh\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_pwd_hygiene(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v8_pwd_violation() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\n---\nRun $PWD/scripts/foo.sh\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_pwd_hygiene(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("$PWD"));
    }

    #[test]
    #[serial_test::serial]
    fn test_v8_hardcoded_path_violation() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\n---\nPath /Users/somebody/code\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_pwd_hygiene(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("hardcoded path"));
    }

    // V10: validate_executability
    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn test_v10_executable_script() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("scripts").unwrap();
        let script = tmp.path().join("scripts/test.sh");
        std::fs::write(&script, "#!/bin/bash\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_executability(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn test_v10_non_executable_script() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("scripts").unwrap();
        let script = tmp.path().join("scripts/test.sh");
        std::fs::write(&script, "#!/bin/bash\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o644)).unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_executability(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("not executable"));
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn test_v10a_private_executable() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/skills/my-skill/scripts").unwrap();
        let script = tmp.path().join(".claude/skills/my-skill/scripts/helper.sh");
        std::fs::write(&script, "#!/bin/bash\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_private_executability(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn test_v10a_private_non_executable() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/skills/my-skill/scripts").unwrap();
        let script = tmp.path().join(".claude/skills/my-skill/scripts/helper.sh");
        std::fs::write(&script, "#!/bin/bash\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o644)).unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_private_executability(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("not executable"));
    }

    // V14: validate_security_md
    #[test]
    #[serial_test::serial]
    fn test_v14_security_md_present() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::write("SECURITY.md", "# Security Policy\n").unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_security_md(&mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v14_security_md_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_security_md(&mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("SECURITY.md"));
    }

    // V9: validate_script_references
    #[test]
    #[serial_test::serial]
    fn test_v9_valid_reference() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("scripts").unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write("scripts/helper.sh", "#!/bin/bash\n").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\n---\nRun ${CLAUDE_PLUGIN_ROOT}/scripts/helper.sh\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_script_references(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v9_missing_reference() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\n---\nRun ${CLAUDE_PLUGIN_ROOT}/scripts/nonexistent.sh\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_script_references(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("missing on disk"));
    }

    #[test]
    #[serial_test::serial]
    fn test_v9a_valid_private_reference() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/skills/my-skill/scripts").unwrap();
        std::fs::write(".claude/skills/my-skill/scripts/run.sh", "#!/bin/bash\n").unwrap();
        std::fs::write(
            ".claude/skills/my-skill/SKILL.md",
            "---\nname: my-skill\n---\nRun $PWD/.claude/skills/my-skill/scripts/run.sh\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_private_script_references(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v9a_missing_private_reference() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/skills/my-skill").unwrap();
        std::fs::write(
            ".claude/skills/my-skill/SKILL.md",
            "---\nname: my-skill\n---\nRun $PWD/.claude/skills/my-skill/scripts/missing.sh\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new();
        validate_private_script_references(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("missing on disk"));
    }

    // V11: validate_dead_scripts
    #[test]
    #[serial_test::serial]
    fn test_v11_referenced_script_not_dead() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("scripts").unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write("scripts/used.sh", "#!/bin/bash\n").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\n---\nRun ${CLAUDE_PLUGIN_ROOT}/scripts/used.sh\n",
        )
        .unwrap();

        let ctx = crate::context::LintContext {
            base_path: tmp.path().to_path_buf(),
            mode: crate::context::LintMode::Plugin,
            plugin_json: crate::context::ManifestState::Missing,
            marketplace_json: crate::context::ManifestState::Missing,
            hooks_json: crate::context::ManifestState::Missing,
            settings_json: crate::context::ManifestState::Missing,
        };
        let mut diag = DiagnosticCollector::new();
        validate_dead_scripts(&ctx, &mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v11_unreferenced_dead_script() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("scripts").unwrap();
        std::fs::write("scripts/orphan.sh", "#!/bin/bash\n").unwrap();

        let ctx = crate::context::LintContext {
            base_path: tmp.path().to_path_buf(),
            mode: crate::context::LintMode::Plugin,
            plugin_json: crate::context::ManifestState::Missing,
            marketplace_json: crate::context::ManifestState::Missing,
            hooks_json: crate::context::ManifestState::Missing,
            settings_json: crate::context::ManifestState::Missing,
        };
        let mut diag = DiagnosticCollector::new();
        validate_dead_scripts(&ctx, &mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("dead script"));
    }

    #[test]
    #[serial_test::serial]
    fn test_v11_script_referenced_in_hooks_json_not_dead() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("scripts").unwrap();
        std::fs::write("scripts/referenced.sh", "#!/bin/bash\n").unwrap();

        // Script referenced via ${CLAUDE_PLUGIN_ROOT}/scripts/referenced.sh in hooks.json
        let hooks_val = serde_json::json!({
            "hooks": [{"command": "${CLAUDE_PLUGIN_ROOT}/scripts/referenced.sh"}]
        });
        let ctx = crate::context::LintContext {
            base_path: tmp.path().to_path_buf(),
            mode: crate::context::LintMode::Plugin,
            plugin_json: crate::context::ManifestState::Missing,
            marketplace_json: crate::context::ManifestState::Missing,
            hooks_json: crate::context::ManifestState::Parsed(hooks_val),
            settings_json: crate::context::ManifestState::Missing,
        };
        let mut diag = DiagnosticCollector::new();
        validate_dead_scripts(&ctx, &mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(
            diag.error_count(),
            0,
            "Script referenced in hooks.json should not be reported as dead"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_v11_script_referenced_in_settings_json_not_dead() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("scripts").unwrap();
        std::fs::write("scripts/setup.sh", "#!/bin/bash\n").unwrap();

        // Script referenced via bare scripts/setup.sh in settings.json (re_d pattern)
        let settings_val = serde_json::json!({
            "permissions": {"allow": ["scripts/setup.sh"]}
        });
        let ctx = crate::context::LintContext {
            base_path: tmp.path().to_path_buf(),
            mode: crate::context::LintMode::Plugin,
            plugin_json: crate::context::ManifestState::Missing,
            marketplace_json: crate::context::ManifestState::Missing,
            hooks_json: crate::context::ManifestState::Missing,
            settings_json: crate::context::ManifestState::Parsed(settings_val),
        };
        let mut diag = DiagnosticCollector::new();
        validate_dead_scripts(&ctx, &mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(
            diag.error_count(),
            0,
            "Script referenced in settings.json should not be reported as dead"
        );
    }

    // expand_script_dirs tests
    #[test]
    #[serial_test::serial]
    fn test_expand_script_dirs_plain_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("scripts").unwrap();
        let dirs = expand_script_dirs(&["scripts"]);
        assert_eq!(dirs.len(), 1);
    }

    #[test]
    #[serial_test::serial]
    fn test_expand_script_dirs_glob() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/a/scripts").unwrap();
        std::fs::create_dir_all("skills/b/scripts").unwrap();
        let mut dirs = expand_script_dirs(&["skills/*/scripts"]);
        dirs.sort();
        assert_eq!(dirs.len(), 2);
    }

    #[test]
    #[serial_test::serial]
    fn test_expand_script_dirs_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        let dirs = expand_script_dirs(&["nonexistent"]);
        assert!(dirs.is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn test_expand_script_dirs_multi_glob() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        // Create skills/a/nested/x/scripts and skills/b/nested/y/scripts
        std::fs::create_dir_all("skills/a/nested/x/scripts").unwrap();
        std::fs::create_dir_all("skills/b/nested/y/scripts").unwrap();
        // This should NOT match (wrong intermediate dir name)
        std::fs::create_dir_all("skills/c/other/z/scripts").unwrap();

        let mut dirs = expand_script_dirs(&["skills/*/nested/*/scripts"]);
        dirs.sort();
        assert_eq!(dirs.len(), 2);
        assert!(dirs[0].ends_with("skills/a/nested/x/scripts"));
        assert!(dirs[1].ends_with("skills/b/nested/y/scripts"));
    }

    #[test]
    #[serial_test::serial]
    fn test_expand_script_dirs_glob_nonexistent_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        let dirs = expand_script_dirs(&["nonexistent/*/scripts"]);
        assert!(dirs.is_empty());
    }

    // collect_script_paths tests
    #[test]
    #[serial_test::serial]
    fn test_collect_script_paths_basic_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/skills/my-skill/scripts").unwrap();
        std::fs::write(".claude/skills/my-skill/scripts/run.sh", "#!/bin/bash\n").unwrap();
        std::fs::write(".claude/skills/my-skill/scripts/helper.sh", "#!/bin/bash\n").unwrap();
        // Non-.sh file should be ignored
        std::fs::write(".claude/skills/my-skill/scripts/readme.txt", "text\n").unwrap();

        let paths = collect_script_paths(LintMode::Basic, &crate::config::ExcludeSet::default());
        assert_eq!(paths.len(), 2);
        assert!(paths[0].ends_with("helper.sh"));
        assert!(paths[1].ends_with("run.sh"));
    }

    #[test]
    #[serial_test::serial]
    fn test_collect_script_paths_plugin_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("scripts").unwrap();
        std::fs::create_dir_all("skills/foo/scripts").unwrap();
        std::fs::create_dir_all(".claude/skills/bar/scripts").unwrap();
        std::fs::write("scripts/install.sh", "#!/bin/bash\n").unwrap();
        std::fs::write("skills/foo/scripts/build.sh", "#!/bin/bash\n").unwrap();
        std::fs::write(".claude/skills/bar/scripts/run.sh", "#!/bin/bash\n").unwrap();

        let paths = collect_script_paths(LintMode::Plugin, &crate::config::ExcludeSet::default());
        assert_eq!(paths.len(), 3);
        // Sorted by BTreeSet — paths should be in lexicographic order
        assert!(paths.iter().any(|p| p.ends_with("install.sh")));
        assert!(paths.iter().any(|p| p.ends_with("build.sh")));
        assert!(paths.iter().any(|p| p.ends_with("run.sh")));
    }

    #[test]
    #[serial_test::serial]
    fn test_collect_script_paths_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        let paths = collect_script_paths(LintMode::Basic, &crate::config::ExcludeSet::default());
        assert!(paths.is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn test_collect_script_paths_basic_excludes_top_level_scripts() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("scripts").unwrap();
        std::fs::write("scripts/install.sh", "#!/bin/bash\n").unwrap();

        // Basic mode should NOT include scripts/ (only .claude/skills/*/scripts/)
        let paths = collect_script_paths(LintMode::Basic, &crate::config::ExcludeSet::default());
        assert!(paths.is_empty());
    }

    // G006: todo-in-skill
    #[test]
    #[serial_test::serial]
    fn test_g006_todo_in_skill_body() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: desc\n---\nTODO: implement this\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_todo_in_skills(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("TODO")));
    }

    #[test]
    #[serial_test::serial]
    fn test_g006_todo_in_code_fence_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: desc\n---\n\n```bash\n# TODO: this is fine\n```\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_todo_in_skills(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("TODO")));
    }

    #[test]
    #[serial_test::serial]
    fn test_g006_todo_in_nested_fence_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        // 4-backtick fence containing 3-backtick line with TODO — should not trigger
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: desc\n---\n\n````\n```\n# TODO: nested\n```\n````\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_todo_in_skills(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag.errors().iter().any(|e| e.contains("TODO")),
            "TODO inside nested 4-backtick fence should not trigger G006"
        );
    }

    // G007: todo-in-agent
    #[test]
    #[serial_test::serial]
    fn test_g007_todo_in_agent_body() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::write(
            "agents/general.md",
            "---\nname: general\ndescription: desc\n---\nFIXME: this needs work\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_todo_in_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.errors().iter().any(|e| e.contains("FIXME")));
    }

    #[test]
    #[serial_test::serial]
    fn test_g007_todo_in_code_fence_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        std::fs::write(
            "agents/general.md",
            "---\nname: general\ndescription: desc\n---\n\n```\n# FIXME: inside fence\n```\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_todo_in_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(!diag.errors().iter().any(|e| e.contains("FIXME")));
    }

    #[test]
    #[serial_test::serial]
    fn test_g007_todo_in_nested_fence_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("agents").unwrap();
        // 4-backtick fence containing 3-backtick line with FIXME — should not trigger
        std::fs::write(
            "agents/general.md",
            "---\nname: general\ndescription: desc\n---\n\n````\n```\n# FIXME: nested\n```\n````\n",
        )
        .unwrap();
        let mut diag = DiagnosticCollector::new();
        validate_todo_in_agents(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag.errors().iter().any(|e| e.contains("FIXME")),
            "FIXME inside nested 4-backtick fence should not trigger G007"
        );
    }
}
