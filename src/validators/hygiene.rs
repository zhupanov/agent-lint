use crate::diagnostic::DiagnosticCollector;
use regex::Regex;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

/// V8: ${CLAUDE_PLUGIN_ROOT} hygiene — public skills/*/SKILL.md must not use
/// $PWD/, ${PWD}/, or hardcoded paths (/Users/, /home/, /opt/).
pub fn validate_pwd_hygiene(diag: &mut DiagnosticCollector) {
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

        let skill_md = path.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }

        let content = match fs::read_to_string(&skill_md) {
            Ok(c) => c,
            Err(_) => continue,
        };

        if re.is_match(&content) {
            diag.fail(&format!(
                "skills/{name}/SKILL.md uses $PWD/ or hardcoded path; use ${{CLAUDE_PLUGIN_ROOT}}/ instead"
            ));
        }
    }
}

/// V9: Script reference integrity.
/// Every ${CLAUDE_PLUGIN_ROOT}/(scripts|skills|.claude/skills)/...sh
/// and $PWD/.claude/skills/...sh referenced from any SKILL.md or
/// skills/shared/*.md must exist on disk.
pub fn validate_script_references(diag: &mut DiagnosticCollector) {
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

    // Collect references from skills/ and .claude/skills/
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
            let content = match fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // Public references
            for cap in re_pub.find_iter(&content) {
                let reference = cap.as_str().to_string();
                if seen.insert(reference.clone()) {
                    let rel = reference.replace("${CLAUDE_PLUGIN_ROOT}/", "");
                    if !Path::new(&rel).is_file() {
                        diag.fail(&format!(
                            "script reference missing on disk: {reference} (expected {rel})"
                        ));
                    }
                }
            }

            // Private references
            for cap in re_priv.find_iter(&content) {
                let reference = cap.as_str().to_string();
                if seen.insert(reference.clone()) {
                    let rel = reference.replace("$PWD/", "");
                    if !Path::new(&rel).is_file() {
                        diag.fail(&format!(
                            "script reference missing on disk: {reference} (expected {rel})"
                        ));
                    }
                }
            }

            // Legacy placeholder references
            for cap in re_placeholder.find_iter(&content) {
                let reference = cap.as_str().to_string();
                if seen.insert(reference.clone()) {
                    let rel = reference.replace("${CLAUDE_PLUGIN_ROOT_PLACEHOLDER:-$PWD}/", "");
                    if !Path::new(&rel).is_file() {
                        diag.fail(&format!(
                            "script reference missing on disk: {reference} (expected {rel})"
                        ));
                    }
                }
            }
        }
    }
}

/// V9-adapted: Script reference integrity for private .claude/skills/ only.
pub fn validate_private_script_references(diag: &mut DiagnosticCollector) {
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
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        for cap in re_priv.find_iter(&content) {
            let reference = cap.as_str().to_string();
            if seen.insert(reference.clone()) {
                let rel = reference.replace("$PWD/", "");
                if !Path::new(&rel).is_file() {
                    diag.fail(&format!(
                        "script reference missing on disk: {reference} (expected {rel})"
                    ));
                }
            }
        }

        for cap in re_placeholder.find_iter(&content) {
            let reference = cap.as_str().to_string();
            if seen.insert(reference.clone()) {
                let rel = reference.replace("${CLAUDE_PLUGIN_ROOT_PLACEHOLDER:-$PWD}/", "");
                if !Path::new(&rel).is_file() {
                    diag.fail(&format!(
                        "script reference missing on disk: {reference} (expected {rel})"
                    ));
                }
            }
        }
    }
}

/// V10: Executability — every .sh file under scripts/, skills/*/scripts/,
/// and .claude/skills/*/scripts/ must be chmod +x.
/// NOTE: The hardcoded scripts/block-submodule-edit.sh check from the bash
/// script is intentionally NOT ported (it's larch-specific).
pub fn validate_executability(diag: &mut DiagnosticCollector) {
    check_executability_in_dirs(
        &[
            "scripts",
            "skills/*/scripts",
            ".claude/skills/*/scripts",
        ],
        diag,
    );
}

/// V10-adapted: Executability for private .claude/skills/*/scripts/*.sh only.
pub fn validate_private_executability(diag: &mut DiagnosticCollector) {
    check_executability_in_dirs(&[".claude/skills/*/scripts"], diag);
}

fn check_executability_in_dirs(patterns: &[&str], diag: &mut DiagnosticCollector) {
    for pattern in patterns {
        // Expand glob-like patterns manually
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
                            check_sh_executability(&sub, diag);
                        }
                    }
                }
            }
        } else {
            let dir = Path::new(pattern);
            if dir.is_dir() {
                check_sh_executability(dir, diag);
            }
        }
    }
}

fn check_sh_executability(dir: &Path, diag: &mut DiagnosticCollector) {
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

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = path.metadata() {
                if meta.permissions().mode() & 0o111 == 0 {
                    diag.fail(&format!("script not executable: {}", path.display()));
                    let _ = name; // suppress unused warning
                }
            }
        }
    }
}

/// V11: Dead-script detection — every scripts/*.sh must have a STRUCTURED
/// invocation reference somewhere in the codebase.
/// Implements all 5 reference patterns (A through E).
pub fn validate_dead_scripts(diag: &mut DiagnosticCollector) {
    let scripts_dir = Path::new("scripts");
    if !scripts_dir.is_dir() {
        return;
    }

    let mut references: HashSet<String> = HashSet::new();

    // Pattern A & B: ${CLAUDE_PLUGIN_ROOT}/... and $PWD/... path-shaped references
    let re_ab = Regex::new(
        r"\$(\{CLAUDE_PLUGIN_ROOT\}|PWD)/(scripts|\.claude/skills/[^/]+/scripts)/[a-zA-Z0-9._-]+\.sh",
    )
    .unwrap();

    // Legacy placeholder: ${CLAUDE_PLUGIN_ROOT_PLACEHOLDER:-$PWD}/...
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
            let content = match fs::read_to_string(entry.path()) {
                Ok(c) => c,
                Err(_) => continue,
            };
            for cap in re_ab.find_iter(&content) {
                let s = cap.as_str();
                // Strip prefix to get relative path
                let rel = if s.starts_with("${CLAUDE_PLUGIN_ROOT}/") {
                    s.replace("${CLAUDE_PLUGIN_ROOT}/", "")
                } else if s.starts_with("$PWD/") {
                    s.replace("$PWD/", "")
                } else {
                    continue;
                };
                references.insert(rel);
            }
            // Legacy placeholder pattern
            for cap in re_placeholder.find_iter(&content) {
                let s = cap.as_str();
                let rel = s.replace("${CLAUDE_PLUGIN_ROOT_PLACEHOLDER:-$PWD}/", "");
                references.insert(rel);
            }
        }
    }

    // Also check .claude/settings.json for Pattern A/B
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
    // Also check hooks/hooks.json
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

    // Pattern C: $SCRIPT_DIR/<name>.sh inside scripts/
    let re_c = Regex::new(r"\$SCRIPT_DIR/[a-zA-Z0-9._-]+\.sh").unwrap();
    for entry in WalkDir::new(scripts_dir).into_iter().flatten() {
        if !entry.path().is_file() {
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

    // Pattern D: bare scripts/<name>.sh in workflow run: blocks and JSON command fields.
    // Uses boundary guard to avoid matching subpath references.
    let re_d = Regex::new(r"(^|[^a-zA-Z0-9._/-])scripts/[a-zA-Z0-9._-]+\.sh").unwrap();
    let re_extract =
        Regex::new(r"scripts/[a-zA-Z0-9._-]+\.sh").unwrap();

    // Workflow files (strip YAML comments first)
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
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
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

    // JSON files
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

    // Pattern E: bare scripts/<name>.sh in shared markdown code fences ONLY.
    let shared_dir = Path::new("skills/shared");
    if shared_dir.is_dir() {
        for entry in WalkDir::new(shared_dir).into_iter().flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
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

    // Check each scripts/*.sh against collected references
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
            if !references.contains(&key) {
                diag.fail(&format!(
                    "dead script (no structured invocation reference found): scripts/{name}"
                ));
            }
        }
    }
}

/// V14: SECURITY.md presence
pub fn validate_security_md(diag: &mut DiagnosticCollector) {
    if !Path::new("SECURITY.md").is_file() {
        diag.fail("SECURITY.md is missing from repo root");
    }
}

/// Strip full-line and trailing # comments from YAML/shell content.
/// Lines starting with optional whitespace then `#` are dropped;
/// ` # ...` trailing comments are stripped from the rest.
/// NOTE: This is intentionally naive and does not respect YAML quoting.
/// This matches the bash script's behavior exactly.
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

/// Extract content inside fenced code blocks (``` or ~~~).
/// Toggles state on each fence line (prefix match, including language tags).
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
}
