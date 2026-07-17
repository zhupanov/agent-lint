mod autofix;
mod config;
mod context;
mod diagnostic;
mod fence;
mod frontmatter;
mod platforms;
mod prompt_budget;
mod rules;
#[cfg(test)]
mod test_helpers;
mod validators;

use config::{CliMode, LintConfig};
use context::{LintContext, LintMode};
use diagnostic::DiagnosticCollector;
use platforms::{DetectedSurfaces, ValidationTargets};

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Partition args[1..] into flags and positional args.
    let mut list_scripts = false;
    let mut closure_report = false;
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
                    "  --closure-report   Print configured prompt-source budget measurements as JSON"
                );
                println!("  --pedantic         Promote warnings to errors (except too-long rules)");
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
            "--closure-report" => {
                closure_report = true;
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
                    "Usage: agent-lint [--help] [--version] [--list-scripts] [--closure-report] [--pedantic] [--all] [--autofix] [PATH]"
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
    if usize::from(list_scripts) + usize::from(closure_report) + usize::from(autofix) > 1 {
        eprintln!("Cannot combine --list-scripts, --closure-report, and --autofix");
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
            "Usage: agent-lint [--help] [--version] [--list-scripts] [--closure-report] [--pedantic] [--all] [--autofix] [PATH]"
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

    // A config file can force-enable a platform with no detected surface, so
    // it participates in deciding whether this repository has work to lint.
    // Repositories with neither a surface nor a config retain the silent
    // no-work behavior before configuration is parsed.
    if detect_mode().is_none() && !std::path::Path::new("agent-lint.toml").is_file() {
        if list_scripts {
            std::process::exit(0);
        }
        if closure_report {
            println!("[]");
            std::process::exit(0);
        }
        println!("Nothing to lint (no supported agent configuration or MCP configuration found).");
        std::process::exit(0);
    }

    let mut lint_config = match LintConfig::load(&repo_root) {
        Ok(cfg) => cfg,
        Err(msg) => {
            eprintln!("ERROR: {msg}");
            std::process::exit(2);
        }
    };

    lint_config.apply_cli_mode(cli_mode);

    if closure_report {
        run_closure_report(&lint_config);
    }

    let exclude = lint_config.build_exclude_set();
    let targets = DetectedSurfaces::discover(&exclude).resolve(lint_config.platforms);
    let mode = match detect_mode_for_targets(targets)
        .or_else(|| config_selects_basic_mode(&lint_config).then_some(LintMode::Basic))
    {
        Some(mode) => mode,
        None => {
            if list_scripts {
                std::process::exit(0);
            }
            println!("Nothing to lint (no Claude, Cursor, Codex, or MCP configuration found).");
            std::process::exit(0);
        }
    };

    // --list-scripts: print discovered script paths and exit.
    if list_scripts {
        if let Some(scripts) = &lint_config.script_inventory {
            for path in scripts
                .iter()
                .filter(|path| path.ends_with(".sh") || path.ends_with(".inc.bash"))
            {
                println!("{path}");
            }
        } else {
            let scripts = validators::hygiene::collect_script_paths(mode, &exclude);
            for path in &scripts {
                println!("{path}");
            }
        }
        std::process::exit(0);
    }

    if autofix {
        run_autofix(&repo_root, mode, lint_config, &exclude, targets);
    } else {
        run_lint(&repo_root, mode, lint_config, &exclude, targets);
    }
}

fn config_selects_basic_mode(config: &LintConfig) -> bool {
    config.script_inventory.is_some()
        || config.skill_closure_max_lines.is_some()
        || config.claude_import_max_lines.is_some()
        || config.claude_import_total_max_lines.is_some()
        || !config.claude_import_path_budgets.is_empty()
        || !config.prompt_source_budgets.is_empty()
}

fn run_closure_report(lint_config: &LintConfig) -> ! {
    let mut rows = Vec::new();
    for budget in &lint_config.prompt_source_budgets {
        let measurement = match prompt_budget::measure_budget(budget) {
            Ok(measurement) => measurement,
            Err(message) => {
                eprintln!("ERROR: prompt-source-budgets '{}': {message}", budget.name);
                std::process::exit(2);
            }
        };
        rows.extend(prompt_budget::report_rows(budget, &measurement));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&rows).expect("closure report rows serialize")
    );
    std::process::exit(0);
}

fn run_lint(
    repo_root: &str,
    mode: LintMode,
    lint_config: LintConfig,
    exclude: &config::ExcludeSet,
    targets: ValidationTargets,
) {
    let ctx = LintContext::new(std::path::Path::new(repo_root), mode);
    let mut diag = DiagnosticCollector::with_config(lint_config);

    validators::run_all_with_targets(&ctx, &mut diag, exclude, targets);

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
    targets: ValidationTargets,
) {
    // Autofix loop: silently re-validate, fix one rule at a time
    for _ in 0..MAX_FIX_ITERATIONS {
        let ctx = LintContext::new(std::path::Path::new(repo_root), mode);
        let mut diag = DiagnosticCollector::with_config_silent(lint_config.clone());
        validators::run_all_with_targets(&ctx, &mut diag, exclude, targets);

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
    run_lint(repo_root, mode, lint_config, exclude, targets);
}

/// Detect lint mode based on Claude, Codex, Cursor, or MCP configuration.
fn detect_mode() -> Option<LintMode> {
    let targets = DetectedSurfaces::discover(&config::ExcludeSet::default())
        .resolve(config::PlatformOverrides::default());
    detect_mode_for_targets(targets)
}

fn detect_mode_for_targets(targets: ValidationTargets) -> Option<LintMode> {
    if std::path::Path::new(".claude-plugin").is_dir() {
        Some(LintMode::Plugin)
    } else if std::path::Path::new(".claude").is_dir() || targets.has_work() {
        Some(LintMode::Basic)
    } else if has_mcp_config() {
        // A standalone MCP configuration is a Basic configuration project.
        Some(LintMode::Basic)
    } else {
        None
    }
}

fn has_mcp_config() -> bool {
    walkdir::WalkDir::new(".")
        .into_iter()
        .filter_entry(|entry| entry.file_name() != ".git")
        .flatten()
        .any(|entry| {
            entry.file_type().is_file()
                && entry.file_name().to_string_lossy().ends_with(".mcp.json")
        })
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
    fn explicit_contract_configuration_selects_basic_mode() {
        let config = LintConfig {
            claude_import_path_budgets: std::collections::BTreeMap::from([(
                "AGENTS.md".into(),
                10,
            )]),
            ..LintConfig::default()
        };
        assert!(config_selects_basic_mode(&config));
        assert!(!config_selects_basic_mode(&LintConfig::default()));
    }

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
    fn detect_mode_codex_config_returns_basic() {
        let _guard = test_helpers::CwdGuard::new();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir(".codex").unwrap();
        std::fs::write(".codex/config.toml", "model = 'gpt-5'\n").unwrap();
        assert_eq!(detect_mode(), Some(context::LintMode::Basic));
    }

    #[test]
    #[serial]
    fn detect_mode_shared_and_codex_surfaces_return_basic() {
        let _guard = test_helpers::CwdGuard::new();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir("nested").unwrap();
        std::fs::write("nested/AGENTS.md", "# Instructions\n").unwrap();
        assert_eq!(detect_mode(), Some(context::LintMode::Basic));

        std::fs::remove_dir_all("nested").unwrap();
        std::fs::create_dir_all(".codex-plugin").unwrap();
        std::fs::write(".codex-plugin/plugin.json", "{}").unwrap();
        assert_eq!(detect_mode(), Some(context::LintMode::Basic));

        std::fs::remove_dir_all(".codex-plugin").unwrap();
        std::fs::create_dir_all(".agents/skills/example").unwrap();
        std::fs::write(".agents/skills/example/SKILL.md", "# Skill\n").unwrap();
        assert_eq!(detect_mode(), Some(context::LintMode::Basic));
    }

    #[test]
    #[serial]
    fn detect_mode_cursor_surfaces_return_basic() {
        let _guard = test_helpers::CwdGuard::new();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".cursor/rules").unwrap();
        std::fs::write(".cursor/rules/project.mdc", "# Rule\n").unwrap();
        assert_eq!(detect_mode(), Some(context::LintMode::Basic));
    }

    #[test]
    #[serial]
    fn detect_mode_legacy_cursor_rules_return_basic() {
        let _guard = test_helpers::CwdGuard::new();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::write(".cursorrules", "Use strict mode.\n").unwrap();
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

    #[test]
    #[serial]
    fn detect_mode_root_mcp_config_returns_basic() {
        let _guard = test_helpers::CwdGuard::new();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::write(".mcp.json", "{}").unwrap();
        assert_eq!(detect_mode(), Some(context::LintMode::Basic));
    }

    #[test]
    #[serial]
    fn detect_mode_nested_mcp_config_returns_basic() {
        let _guard = test_helpers::CwdGuard::new();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir("config").unwrap();
        std::fs::write("config/development.mcp.json", "{}").unwrap();
        assert_eq!(detect_mode(), Some(context::LintMode::Basic));
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
