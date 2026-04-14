mod config;
mod context;
mod diagnostic;
mod fence;
mod frontmatter;
mod rules;
#[cfg(test)]
mod test_helpers;
mod validators;

use config::LintConfig;
use context::{LintContext, LintMode};
use diagnostic::DiagnosticCollector;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Partition args[1..] into flags and positional args.
    let mut list_scripts = false;
    let mut positional = Vec::new();
    for arg in &args[1..] {
        if arg == "--list-scripts" {
            list_scripts = true;
        } else if arg.starts_with("--") {
            eprintln!("Unknown flag: {arg}");
            eprintln!("Usage: claude-lint [--list-scripts] [PATH]");
            std::process::exit(2);
        } else {
            positional.push(arg.as_str());
        }
    }

    if positional.len() > 1 {
        eprintln!("Usage: claude-lint [--list-scripts] [PATH]");
        std::process::exit(2);
    }

    let target = positional.first().copied().unwrap_or(".");

    // Resolve repo root from the target path.
    let repo_root = match resolve_repo_root(target) {
        Ok(root) => root,
        Err(msg) => {
            eprintln!("ERROR: {msg}");
            std::process::exit(2);
        }
    };

    if std::env::set_current_dir(&repo_root).is_err() {
        eprintln!("ERROR: cannot cd to repo root: {repo_root}");
        std::process::exit(2);
    }

    // Detect lint mode.
    let mode = match detect_mode() {
        Some(m) => m,
        None => {
            if list_scripts {
                // Silent exit — no stdout contamination for pipe consumers.
                std::process::exit(0);
            }
            println!("Nothing to lint (no .claude/ or .claude-plugin/ directory found).");
            std::process::exit(0);
        }
    };

    // Load configuration AFTER mode detection (so "Nothing to lint" repos
    // are not affected by malformed config files).
    let lint_config = match LintConfig::load(&repo_root) {
        Ok(cfg) => cfg,
        Err(msg) => {
            eprintln!("ERROR: {msg}");
            std::process::exit(2);
        }
    };

    let exclude = lint_config.build_exclude_set();

    // --list-scripts: print discovered script paths and exit.
    if list_scripts {
        let scripts = validators::hygiene::collect_script_paths(mode, &exclude);
        for path in &scripts {
            println!("{path}");
        }
        std::process::exit(0);
    }

    let ctx = LintContext::new(repo_root.clone(), mode);
    let mut diag = DiagnosticCollector::with_config(lint_config);

    validators::run_all(&ctx, &mut diag, &exclude);

    let errors = diag.error_count();
    let warnings = diag.warning_count();
    let suppressed = diag.suppressed_count();

    if errors == 0 && warnings == 0 {
        if matches!(ctx.mode, LintMode::Plugin) {
            println!("Plugin structure OK");
        } else {
            println!("Claude config OK");
        }
        if suppressed > 0 {
            eprintln!("({suppressed} suppressed)");
        }
        std::process::exit(0);
    } else if errors == 0 {
        // Only warnings — exit 0
        eprintln!("Lint: {warnings} warning(s)");
        if suppressed > 0 {
            eprintln!("({suppressed} suppressed)");
        }
        std::process::exit(0);
    } else {
        eprintln!("Lint: {errors} error(s), {warnings} warning(s)");
        if suppressed > 0 {
            eprintln!("({suppressed} suppressed)");
        }
        std::process::exit(1);
    }
}

/// Detect lint mode based on directory presence.
fn detect_mode() -> Option<LintMode> {
    if std::path::Path::new(".claude-plugin").is_dir() {
        Some(LintMode::Plugin)
    } else if std::path::Path::new(".claude").is_dir() {
        Some(LintMode::Basic)
    } else {
        None
    }
}

fn resolve_repo_root(target: &str) -> Result<String, String> {
    let output = std::process::Command::new("git")
        .args(["-C", target, "rev-parse", "--show-toplevel"])
        .output()
        .map_err(|e| format!("failed to run git: {e}"))?;

    if !output.status.success() {
        return Err("not inside a git repository".to_string());
    }

    let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if root.is_empty() {
        return Err("git rev-parse returned empty path".to_string());
    }
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    // ── detect_mode ──────────────────────────────────────────────────

    #[test]
    #[serial]
    fn detect_mode_plugin_dir_returns_plugin() {
        let _guard = test_helpers::CwdGuard::new();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir(".claude-plugin").unwrap();
        assert_eq!(detect_mode(), Some(context::LintMode::Plugin));
    }

    #[test]
    #[serial]
    fn detect_mode_basic_dir_returns_basic() {
        let _guard = test_helpers::CwdGuard::new();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir(".claude").unwrap();
        assert_eq!(detect_mode(), Some(context::LintMode::Basic));
    }

    #[test]
    #[serial]
    fn detect_mode_plugin_takes_precedence_over_basic() {
        let _guard = test_helpers::CwdGuard::new();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir(".claude-plugin").unwrap();
        std::fs::create_dir(".claude").unwrap();
        assert_eq!(detect_mode(), Some(context::LintMode::Plugin));
    }

    #[test]
    #[serial]
    fn detect_mode_neither_dir_returns_none() {
        let _guard = test_helpers::CwdGuard::new();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        assert_eq!(detect_mode(), None);
    }

    // ── resolve_repo_root ────────────────────────────────────────────

    #[test]
    fn resolve_repo_root_valid_git_repo() {
        // Use CARGO_MANIFEST_DIR (absolute) to avoid CWD races with serial tests.
        let result = resolve_repo_root(env!("CARGO_MANIFEST_DIR"));
        assert!(result.is_ok());
        let root = result.unwrap();
        assert!(!root.is_empty());
        assert!(std::path::Path::new(&root).join(".git").exists());
    }

    #[test]
    fn resolve_repo_root_non_git_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let result = resolve_repo_root(tmp.path().to_str().unwrap());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not inside a git repository"));
    }

    #[test]
    fn resolve_repo_root_nonexistent_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let nonexistent = tmp.path().join("does-not-exist");
        let result = resolve_repo_root(nonexistent.to_str().unwrap());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not inside a git repository"));
    }
}
