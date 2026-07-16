use crate::config::ExcludeSet;
use crate::context::{LintContext, ManifestState, collect_json_strings};
use crate::diagnostic::DiagnosticCollector;
use crate::rules::LintRule;
use regex::Regex;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::LazyLock;
use walkdir::WalkDir;

use super::scripts::{
    RE_SCRIPT_DIR_REF, RE_SCRIPT_PLACEHOLDER, RE_SCRIPTS_EXTRACT, RE_SCRIPTS_PATH,
    collect_makefile_contents, strip_yaml_comments,
};

static RE_DEAD_SCRIPT_AB: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\$(\{CLAUDE_PLUGIN_ROOT\}|PWD)/(scripts|\.claude/skills/[^/]+/scripts)/[a-zA-Z0-9._-]+\.sh",
    )
    .unwrap()
});

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
            for cap in RE_DEAD_SCRIPT_AB.find_iter(&content) {
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
            for cap in RE_SCRIPT_PLACEHOLDER.find_iter(&content) {
                let s = cap.as_str();
                let rel = s.replace("${CLAUDE_PLUGIN_ROOT_PLACEHOLDER:-$PWD}/", "");
                references.insert(rel);
            }
        }
    }

    // Extract script references from pre-parsed settings.json and hooks.json
    // via LintContext instead of reading files directly from disk.
    // Only re_ab applies here (not re_placeholder, which is directory-walk only).
    // For Invalid/Missing manifests, skip extraction -- other validators report those errors.
    for manifest in [&ctx.settings_json, &ctx.hooks_json] {
        if let ManifestState::Parsed(val) = manifest {
            for s in collect_json_strings(val) {
                for cap in RE_DEAD_SCRIPT_AB.find_iter(&s) {
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
        for cap in RE_SCRIPT_DIR_REF.find_iter(&content) {
            let s = cap.as_str();
            let name = s.replace("$SCRIPT_DIR/", "scripts/");
            references.insert(name);
        }
    }

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
                for cap in RE_SCRIPTS_PATH.find_iter(&stripped) {
                    if let Some(m) = RE_SCRIPTS_EXTRACT.find(cap.as_str()) {
                        references.insert(m.as_str().to_string());
                    }
                }
            }
        }
    }

    // Extract script references from the Makefile and any *.mk so scripts
    // invoked only from Make targets (e.g. `bash scripts/foo.sh` or
    // `${CLAUDE_PLUGIN_ROOT}/scripts/foo.sh`) are not false-flagged as dead.
    // `#` comments are stripped first (shared with YAML-workflow scanning).
    for stripped in collect_makefile_contents(exclude) {
        for cap in RE_DEAD_SCRIPT_AB.find_iter(&stripped) {
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
        for cap in RE_SCRIPTS_PATH.find_iter(&stripped) {
            if let Some(m) = RE_SCRIPTS_EXTRACT.find(cap.as_str()) {
                references.insert(m.as_str().to_string());
            }
        }
    }

    // Extract bare scripts/...sh references from pre-parsed settings/hooks manifests.
    // Only re_d applies here for bare script references.
    for manifest in [&ctx.settings_json, &ctx.hooks_json] {
        if let ManifestState::Parsed(val) = manifest {
            for s in collect_json_strings(val) {
                for cap in RE_SCRIPTS_PATH.find_iter(&s) {
                    if let Some(m) = RE_SCRIPTS_EXTRACT.find(cap.as_str()) {
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
            for cap in RE_SCRIPTS_PATH.find_iter(&fenced) {
                if let Some(m) = RE_SCRIPTS_EXTRACT.find(cap.as_str()) {
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
}
