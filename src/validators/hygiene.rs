use crate::config::ExcludeSet;
use crate::context::LintMode;
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
pub fn validate_executability(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    check_executability_in_dirs(
        &["scripts", "skills/*/scripts", ".claude/skills/*/scripts"],
        diag,
        exclude,
    );
}

/// V10-adapted: Executability for private .claude/skills/*/scripts/*.sh only.
pub fn validate_private_executability(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    check_executability_in_dirs(&[".claude/skills/*/scripts"], diag, exclude);
}

/// Expand glob-like directory patterns into concrete directory paths.
/// Handles single-`*` patterns (e.g., `skills/*/scripts`) and plain directories.
pub fn expand_script_dirs(patterns: &[&str]) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for pattern in patterns {
        if pattern.contains('*') {
            let parts: Vec<&str> = pattern.split('*').collect();
            if parts.len() == 2 {
                let prefix = parts[0].trim_end_matches('/');
                let suffix = parts[1].trim_start_matches('/');
                let base = Path::new(prefix);
                if !base.is_dir() {
                    continue;
                }
                if let Ok(entries) = fs::read_dir(base) {
                    for entry in entries.flatten() {
                        let sub = entry.path().join(suffix);
                        if sub.is_dir() {
                            dirs.push(sub);
                        }
                    }
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

fn check_sh_executability(dir: &Path, diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
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

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = path.metadata() {
                if meta.permissions().mode() & 0o111 == 0 {
                    diag.report(
                        LintRule::ScriptNotExecutable,
                        &format!("script not executable: {}", path.display()),
                    );
                    let _ = name; // suppress unused warning
                }
            }
        }
    }
}

/// V11: Dead-script detection
pub fn validate_dead_scripts(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
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

    if Path::new(".claude/settings.json").is_file() {
        if let Ok(content) = fs::read_to_string(".claude/settings.json") {
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
        }
    }
    if Path::new("hooks/hooks.json").is_file() {
        if let Ok(content) = fs::read_to_string("hooks/hooks.json") {
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

    for json_path in &[".claude/settings.json", "hooks/hooks.json"] {
        if !Path::new(json_path).is_file() {
            continue;
        }
        let content = match fs::read_to_string(json_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for cap in re_d.find_iter(&content) {
            if let Some(m) = re_extract.find(cap.as_str()) {
                references.insert(m.as_str().to_string());
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
        let mut in_fence = false;

        for line in body.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                in_fence = !in_fence;
                continue;
            }
            if !in_fence {
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
        let mut in_fence = false;

        for line in body.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                in_fence = !in_fence;
                continue;
            }
            if !in_fence {
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
}

fn strip_yaml_comments(content: &str) -> String {
    let re_full = Regex::new(r"^[[:space:]]*#").unwrap();
    let re_trailing = Regex::new(r"[[:space:]]+#.*$").unwrap();

    content
        .lines()
        .filter(|line| !re_full.is_match(line))
        .map(|line| re_trailing.replace(line, "").to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn extract_code_fences(content: &str) -> String {
    let mut in_code = false;
    let mut result = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            result.push(line);
        }
    }

    result.join("\n")
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

        let mut diag = DiagnosticCollector::new();
        validate_dead_scripts(&mut diag, &crate::config::ExcludeSet::default());
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

        let mut diag = DiagnosticCollector::new();
        validate_dead_scripts(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("dead script"));
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
}
