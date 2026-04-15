mod autofix;
mod config;
mod context;
mod diagnostic;
mod fence;
mod frontmatter;
mod rules;
#[cfg(test)]
mod test_helpers;
mod validators;

use config::{CliMode, LintConfig};
use context::{LintContext, LintMode};
use diagnostic::DiagnosticCollector;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Partition args[1..] into flags and positional args.
    let mut list_scripts = false;
    let mut pedantic = false;
    let mut all = false;
    let mut autofix = false;
    let mut positional = Vec::new();
    for arg in &args[1..] {
        match arg.as_str() {
            "--help" | "-h" => {
                println!("Usage: agent-lint [OPTIONS] [PATH]");
                println!();
                println!("Options:");
                println!("  --help, -h         Print this help message");
                println!("  --version          Print version information");
                println!("  --list-scripts     List discovered script paths and exit");
                println!(
                    "  --pedantic         Promote all enabled rules to errors (except too-long rules)"
                );
                println!(
                    "  --all              Force every rule to error, ignoring config overrides"
                );
                println!(
                    "  --autofix          Fix auto-fixable violations in-place and report remaining issues"
                );
                std::process::exit(0);
            }
            "--version" => {
                println!("agent-lint {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--list-scripts" => {
                list_scripts = true;
            }
            "--pedantic" => {
                pedantic = true;
            }
            "--all" => {
                all = true;
            }
            "--autofix" => {
                autofix = true;
            }
            flag if flag.starts_with('-') => {
                eprintln!("Unknown flag: {arg}");
                eprintln!(
                    "Usage: agent-lint [--help] [--version] [--list-scripts] [--pedantic] [--all] [--autofix] [PATH]"
                );
                std::process::exit(2);
            }
            _ => {
                positional.push(arg.as_str());
            }
        }
    }

    if pedantic && all {
        eprintln!("Cannot use both --pedantic and --all");
        std::process::exit(2);
    }

    let cli_mode = if all {
        CliMode::All
    } else if pedantic {
        CliMode::Pedantic
    } else {
        CliMode::Normal
    };

    if positional.len() > 1 {
        eprintln!(
            "Usage: agent-lint [--help] [--version] [--list-scripts] [--pedantic] [--all] [--autofix] [PATH]"
        );
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
    let mut lint_config = match LintConfig::load(&repo_root) {
        Ok(cfg) => cfg,
        Err(msg) => {
            eprintln!("ERROR: {msg}");
            std::process::exit(2);
        }
    };

    lint_config.apply_cli_mode(cli_mode);

    let exclude = lint_config.build_exclude_set();

    // --list-scripts: print discovered script paths and exit.
    if list_scripts {
        let scripts = validators::hygiene::collect_script_paths(mode, &exclude);
        for path in &scripts {
            println!("{path}");
        }
        std::process::exit(0);
    }

    if autofix {
        run_autofix(&repo_root, mode, lint_config, &exclude);
    } else {
        run_lint(&repo_root, mode, lint_config, &exclude);
    }
}

fn run_lint(
    repo_root: &str,
    mode: LintMode,
    lint_config: LintConfig,
    exclude: &config::ExcludeSet,
) {
    let ctx = LintContext::new(std::path::Path::new(repo_root), mode);
    let mut diag = DiagnosticCollector::with_config(lint_config);

    validators::run_all(&ctx, &mut diag, exclude);

    let errors = diag.error_count();
    let warnings = diag.warning_count();
    let suppressed = diag.suppressed_count();

    if errors == 0 && warnings == 0 {
        if matches!(ctx.mode, LintMode::Plugin) {
            println!("Plugin structure OK");
        } else {
            println!("Config OK");
        }
        if suppressed > 0 {
            eprintln!("({suppressed} suppressed)");
        }
        std::process::exit(0);
    } else if errors == 0 {
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

const MAX_FIX_ITERATIONS: usize = 50;

fn run_autofix(
    repo_root: &str,
    mode: LintMode,
    lint_config: LintConfig,
    exclude: &config::ExcludeSet,
) {
    // Autofix loop: silently re-validate, fix one rule at a time
    for _ in 0..MAX_FIX_ITERATIONS {
        let ctx = LintContext::new(std::path::Path::new(repo_root), mode);
        let mut diag = DiagnosticCollector::with_config_silent(lint_config.clone());
        validators::run_all(&ctx, &mut diag, exclude);

        // Collect unique auto-fixable rules that have violations
        let fixable_rules: Vec<rules::LintRule> = {
            let mut seen = std::collections::HashSet::new();
            diag.diagnostics()
                .iter()
                .filter(|d| d.rule.is_autofixable())
                .filter(|d| seen.insert(d.rule))
                .map(|d| d.rule)
                .collect()
        };

        if fixable_rules.is_empty() {
            break;
        }

        let mut made_progress = false;
        for rule in fixable_rules {
            if autofix::apply_fix(rule, mode, exclude) {
                made_progress = true;
                break; // Re-validate after each fix
            }
        }
        if !made_progress {
            break;
        }
    }

    // Final validation pass with normal stderr output
    run_lint(repo_root, mode, lint_config, exclude);
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
        .output();

    if let Ok(output) = output {
        if output.status.success() {
            let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !root.is_empty() {
                return Ok(root);
            }
        }
    }

    // Git unavailable or not a git repo — fall back to the target directory.
    eprintln!("warning: not a git repository, using target directory as root");
    std::fs::canonicalize(target)
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| format!("cannot resolve path '{target}': {e}"))
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
    fn resolve_repo_root_non_git_dir_falls_back_to_target() {
        let tmp = tempfile::tempdir().unwrap();
        let result = resolve_repo_root(tmp.path().to_str().unwrap());
        assert!(result.is_ok());
        let root = result.unwrap();
        // The returned path should be the canonicalized temp dir.
        let expected = tmp.path().canonicalize().unwrap();
        assert_eq!(root, expected.to_string_lossy());
    }

    #[test]
    fn resolve_repo_root_nonexistent_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let nonexistent = tmp.path().join("does-not-exist");
        let result = resolve_repo_root(nonexistent.to_str().unwrap());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("cannot resolve path"));
    }
}
