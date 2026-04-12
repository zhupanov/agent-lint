mod context;
mod diagnostic;
mod frontmatter;
mod validators;

use context::{LintContext, LintMode};
use diagnostic::DiagnosticCollector;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() > 2 {
        eprintln!("Usage: claude-lint [PATH]");
        std::process::exit(2);
    }

    let target = if args.len() == 2 { &args[1] } else { "." };

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
    let mode = if std::path::Path::new(".claude-plugin").is_dir() {
        LintMode::Plugin
    } else if std::path::Path::new(".claude").is_dir() {
        LintMode::Basic
    } else {
        println!("Nothing to lint (no .claude/ or .claude-plugin/ directory found).");
        std::process::exit(0);
    };

    let ctx = LintContext::new(repo_root.clone(), mode);
    let mut diag = DiagnosticCollector::new();

    validators::run_all(&ctx, &mut diag);

    let count = diag.error_count();
    if count == 0 {
        if matches!(ctx.mode, LintMode::Plugin) {
            println!("Plugin structure OK");
        } else {
            println!("Claude config OK");
        }
        std::process::exit(0);
    } else {
        eprintln!("Lint: {count} error(s)");
        std::process::exit(1);
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
