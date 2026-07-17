use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_agent-lint"))
        .args(args)
        .output()
        .expect("agent-lint binary runs")
}

fn run_in(path: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_agent-lint"))
        .current_dir(path)
        .args(args)
        .output()
        .expect("agent-lint binary runs")
}

fn write_skill(root: &std::path::Path, name: &str, url: &str) -> String {
    let relative = format!(".claude/skills/{name}/SKILL.md");
    let path = root.join(&relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let content = format!(
        "---\nname: {name}\ndescription: Use when testing secure URL handling for a configured integration\n---\n# URL handling\n\nVisit {url} for integration details.\n"
    );
    std::fs::write(path, &content).unwrap();
    content
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn help_succeeds_and_lists_supported_options() {
    let output = run(&["--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage: agent-lint"));
    assert!(stdout.contains("--autofix"));
}

#[test]
fn version_succeeds() {
    let output = run(&["--version"]);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("agent-lint {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn unknown_flag_is_a_usage_error() {
    let output = run(&["--unknown"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("unexpected argument found"));
}

#[test]
fn conflicting_strictness_modes_are_a_usage_error() {
    let output = run(&["--pedantic", "--all"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("cannot be used with"));
}

#[test]
fn conflicting_operations_are_a_usage_error() {
    let output = run(&["--list-scripts", "--closure-report"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("cannot be used with"));
}

#[test]
fn multiple_paths_are_a_usage_error() {
    let output = run(&["one", "two"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("unexpected argument found"));
}

#[test]
fn per_file_override_suppresses_one_file_but_not_another_and_counts_it() {
    let tmp = tempfile::tempdir().unwrap();
    write_skill(tmp.path(), "suppressed", "http://legacy.invalid");
    write_skill(tmp.path(), "reported", "http://current.invalid");
    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        r#"[lint]
[[lint.overrides]]
files = [".claude/skills/suppressed/SKILL.md"]
suppress = ["S031"]
reason = "legacy endpoint is externally owned"
"#,
    )
    .unwrap();

    let output = run_in(tmp.path(), &["."]);
    let stderr = stderr(&output);
    assert_eq!(output.status.code(), Some(1), "stderr: {stderr}");
    assert!(stderr.contains("S031/non-https-url"), "stderr: {stderr}");
    assert!(stderr.contains(".claude/skills/reported/SKILL.md"));
    assert!(!stderr.contains(".claude/skills/suppressed/SKILL.md: non-HTTPS"));
    assert!(stderr.contains("(1 suppressed)"), "stderr: {stderr}");
    assert!(!stderr.contains("unused-override"), "stderr: {stderr}");
}

#[test]
fn autofix_leaves_suppressed_file_unchanged_and_fixes_unsuppressed_file() {
    let tmp = tempfile::tempdir().unwrap();
    let suppressed_before = write_skill(tmp.path(), "suppressed", "http://legacy.invalid");
    write_skill(tmp.path(), "fixed", "http://current.invalid");
    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        r#"[lint]
[[lint.overrides]]
files = [".claude/skills/suppressed/SKILL.md"]
suppress = ["S031"]
"#,
    )
    .unwrap();

    let output = run_in(tmp.path(), &["--autofix", "."]);
    let stderr = stderr(&output);
    assert!(output.status.success(), "stderr: {stderr}");
    assert_eq!(
        std::fs::read_to_string(tmp.path().join(".claude/skills/suppressed/SKILL.md")).unwrap(),
        suppressed_before
    );
    let fixed = std::fs::read_to_string(tmp.path().join(".claude/skills/fixed/SKILL.md")).unwrap();
    assert!(
        fixed.contains("https://current.invalid"),
        "content: {fixed}"
    );
    assert!(!fixed.contains("http://current.invalid"));
    assert!(stderr.contains("(1 suppressed)"), "stderr: {stderr}");
}

#[test]
fn unused_override_warning_is_emitted_once_on_final_autofix_pass() {
    let tmp = tempfile::tempdir().unwrap();
    write_skill(tmp.path(), "clean", "https://current.invalid");
    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        r#"[lint]
[[lint.overrides]]
files = [".claude/skills/missing/SKILL.md"]
suppress = ["S031"]
reason = "stale exception"
"#,
    )
    .unwrap();

    let output = run_in(tmp.path(), &["--autofix", "."]);
    let stderr = stderr(&output);
    assert!(output.status.success(), "stderr: {stderr}");
    assert_eq!(stderr.matches("config/unused-override").count(), 1);
    assert!(stderr.contains("S031/non-https-url"));
    assert!(stderr.contains(".claude/skills/missing/SKILL.md"));
    assert!(stderr.contains("stale exception"));
}

#[test]
fn fixed_missing_target_uses_its_logical_subject_path() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join(".claude-plugin")).unwrap();
    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        r#"[lint]
[[lint.overrides]]
files = [".claude-plugin/plugin.json"]
suppress = ["M001"]
"#,
    )
    .unwrap();

    let output = run_in(tmp.path(), &["."]);
    let stderr = stderr(&output);
    assert_eq!(output.status.code(), Some(1), "stderr: {stderr}");
    assert!(!stderr.contains("M001/plugin-json-missing"));
    assert!(stderr.contains("(1 suppressed)"), "stderr: {stderr}");
    assert!(!stderr.contains("unused-override"), "stderr: {stderr}");
}

#[test]
fn all_mode_ignores_per_file_suppression() {
    let tmp = tempfile::tempdir().unwrap();
    write_skill(tmp.path(), "suppressed", "http://legacy.invalid");
    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        r#"[lint]
[[lint.overrides]]
files = [".claude/skills/suppressed/SKILL.md"]
suppress = ["S031"]
"#,
    )
    .unwrap();

    let output = run_in(tmp.path(), &["--all", "."]);
    let stderr = stderr(&output);
    assert_eq!(output.status.code(), Some(1), "stderr: {stderr}");
    assert!(stderr.contains("S031/non-https-url"), "stderr: {stderr}");
    assert!(stderr.contains(".claude/skills/suppressed/SKILL.md"));
    assert!(!stderr.contains("suppressed)"));
    assert!(!stderr.contains("unused-override"));
}

#[test]
fn override_only_config_reports_an_unused_entry_without_a_detected_surface() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        r#"[lint]
[[lint.overrides]]
files = ["missing.md"]
suppress = ["M001"]
"#,
    )
    .unwrap();

    let output = run_in(tmp.path(), &["."]);
    let stderr = stderr(&output);
    assert!(output.status.success(), "stderr: {stderr}");
    assert!(stderr.contains("config/unused-override"));
    assert!(stderr.contains("missing.md"));
}
