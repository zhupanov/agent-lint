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
    assert!(stdout.contains("--only"));
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

#[test]
fn only_accepts_codes_names_commas_repetition_and_orders_by_registry() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join(".claude-plugin")).unwrap();

    let output = run_in(
        tmp.path(),
        &[
            "--only",
            "H001,plugin-json-missing",
            "--only",
            "hooks-json-missing",
            ".",
        ],
    );
    let stderr = stderr(&output);
    assert_eq!(output.status.code(), Some(1), "stderr: {stderr}");
    assert_eq!(stderr.matches("M001/plugin-json-missing").count(), 1);
    assert_eq!(stderr.matches("H001/hooks-json-missing").count(), 1);
    let manifest = stderr.find("M001/plugin-json-missing").unwrap();
    let hooks = stderr.find("H001/hooks-json-missing").unwrap();
    assert!(manifest < hooks, "stderr: {stderr}");
    assert!(!stderr.contains("M005/marketplace-json-missing"));
}

#[test]
fn only_rejects_unknown_and_empty_identifiers_as_usage_errors() {
    for (argument, invalid) in [
        ("X999", "X999"),
        ("M001,,H001", "invalid rule identifier ''"),
        ("", "invalid rule identifier ''"),
    ] {
        let output = run(&["--only", argument]);
        let stderr = stderr(&output);
        assert_eq!(output.status.code(), Some(2), "stderr: {stderr}");
        assert!(stderr.contains(invalid), "stderr: {stderr}");
    }
}

#[test]
fn only_does_not_bypass_invalid_agent_lint_configuration() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[lint]\nsuppress = [\"not-a-rule\"]\n",
    )
    .unwrap();

    let output = run_in(tmp.path(), &["--only", "M001", "."]);
    let stderr = stderr(&output);
    assert_eq!(output.status.code(), Some(2), "stderr: {stderr}");
    assert!(stderr.contains("not-a-rule"), "stderr: {stderr}");
}

#[test]
fn only_preserves_normal_pedantic_and_all_severity() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join(".claude-plugin")).unwrap();

    let normal = run_in(tmp.path(), &["--only", "G005", "."]);
    let normal_stderr = stderr(&normal);
    assert!(normal.status.success(), "stderr: {normal_stderr}");
    assert!(
        normal_stderr.contains("warning[G005/security-md-missing]"),
        "stderr: {normal_stderr}"
    );

    let pedantic = run_in(tmp.path(), &["--pedantic", "--only", "G005", "."]);
    let pedantic_stderr = stderr(&pedantic);
    assert_eq!(pedantic.status.code(), Some(1), "stderr: {pedantic_stderr}");
    assert!(
        pedantic_stderr.contains("error[G005/security-md-missing]"),
        "stderr: {pedantic_stderr}"
    );

    let all = run_in(tmp.path(), &["--all", "--only", "G005", "."]);
    let all_stderr = stderr(&all);
    assert_eq!(all.status.code(), Some(1), "stderr: {all_stderr}");
    assert!(
        all_stderr.contains("error[G005/security-md-missing]"),
        "stderr: {all_stderr}"
    );
    assert!(!all_stderr.contains("M001/plugin-json-missing"));
}

#[test]
fn only_preserves_configured_severity() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join(".claude-plugin")).unwrap();
    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[lint]\nwarn = [\"M001\"]\n",
    )
    .unwrap();

    let output = run_in(tmp.path(), &["--only", "M001", "."]);
    let stderr = stderr(&output);
    assert!(output.status.success(), "stderr: {stderr}");
    assert!(
        stderr.contains("warning[M001/plugin-json-missing]"),
        "stderr: {stderr}"
    );
}

#[test]
fn only_preserves_global_suppression_except_in_all_mode() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join(".claude-plugin")).unwrap();
    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[lint]\nsuppress = [\"G005\"]\n",
    )
    .unwrap();

    for args in [
        vec!["--only", "G005", "."],
        vec!["--pedantic", "--only", "G005", "."],
    ] {
        let output = run_in(tmp.path(), &args);
        let stderr = stderr(&output);
        assert!(output.status.success(), "stderr: {stderr}");
        assert!(!stderr.contains("G005/security-md-missing"));
        assert!(stderr.contains("(1 suppressed)"), "stderr: {stderr}");
    }

    let all = run_in(tmp.path(), &["--all", "--only", "G005", "."]);
    let all_stderr = stderr(&all);
    assert_eq!(all.status.code(), Some(1), "stderr: {all_stderr}");
    assert!(all_stderr.contains("error[G005/security-md-missing]"));
    assert!(!all_stderr.contains("suppressed)"));
}

#[test]
fn only_preserves_per_file_suppression() {
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

    for args in [
        vec!["--only", "M001", "."],
        vec!["--pedantic", "--only", "M001", "."],
    ] {
        let output = run_in(tmp.path(), &args);
        let stderr = stderr(&output);
        assert!(output.status.success(), "stderr: {stderr}");
        assert!(!stderr.contains("M001/plugin-json-missing"));
        assert!(stderr.contains("(1 suppressed)"), "stderr: {stderr}");
    }

    let all = run_in(tmp.path(), &["--all", "--only", "M001", "."]);
    let all_stderr = stderr(&all);
    assert_eq!(all.status.code(), Some(1), "stderr: {all_stderr}");
    assert!(all_stderr.contains("error[M001/plugin-json-missing]"));
    assert!(!all_stderr.contains("suppressed)"));
}

#[test]
fn only_excludes_unselected_rules_from_suppressed_and_unused_counts() {
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

    let output = run_in(tmp.path(), &["--only", "H001", "."]);
    let stderr = stderr(&output);
    assert_eq!(output.status.code(), Some(1), "stderr: {stderr}");
    assert!(stderr.contains("H001/hooks-json-missing"));
    assert!(!stderr.contains("suppressed)"));
    assert!(!stderr.contains("unused-override"));
}

#[test]
fn only_reports_unused_overrides_for_selected_rule_entries() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join(".claude-plugin")).unwrap();
    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        r#"[lint]
[[lint.overrides]]
files = ["missing.md"]
suppress = ["M001", "H001"]
"#,
    )
    .unwrap();

    let output = run_in(tmp.path(), &["--only", "H001", "."]);
    let stderr = stderr(&output);
    assert_eq!(output.status.code(), Some(1), "stderr: {stderr}");
    assert_eq!(stderr.matches("config/unused-override").count(), 1);
    assert!(stderr.contains("H001/hooks-json-missing"));
    assert!(!stderr.contains("M001/plugin-json-missing"));
}

#[test]
fn only_applies_and_validates_autofixes_for_selected_rules() {
    let tmp = tempfile::tempdir().unwrap();
    let content = write_skill(tmp.path(), "focused", "http://legacy.invalid")
        .replace("name: focused", "name: wrong-name");
    let skill = tmp.path().join(".claude/skills/focused/SKILL.md");
    std::fs::write(&skill, content).unwrap();

    let output = run_in(tmp.path(), &["--autofix", "--only", "S031", "."]);
    let stderr = stderr(&output);
    assert!(output.status.success(), "stderr: {stderr}");
    let fixed = std::fs::read_to_string(skill).unwrap();
    assert!(fixed.contains("https://legacy.invalid"), "content: {fixed}");
    assert!(fixed.contains("name: wrong-name"), "content: {fixed}");
    assert!(!stderr.contains("S006/frontmatter-name-mismatch"));
}

#[test]
fn only_filters_basic_plugin_codex_and_cursor_dispatch() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join(".claude-plugin")).unwrap();
    std::fs::create_dir_all(tmp.path().join(".claude")).unwrap();
    std::fs::write(tmp.path().join(".claude/settings.json"), "not json").unwrap();
    std::fs::create_dir_all(tmp.path().join(".codex")).unwrap();
    std::fs::write(tmp.path().join(".codex/config.toml"), "not = [valid").unwrap();
    std::fs::create_dir_all(tmp.path().join(".cursor/rules")).unwrap();
    std::fs::write(tmp.path().join(".cursor/rules/empty.mdc"), "").unwrap();

    let output = run_in(tmp.path(), &["--only", "CU001,CX001,H006,M001", "."]);
    let stderr = stderr(&output);
    assert_eq!(output.status.code(), Some(1), "stderr: {stderr}");
    for identifier in [
        "M001/plugin-json-missing",
        "H006/settings-json-invalid",
        "CX001/codex-toml-invalid",
        "CU001/cursor-rule-empty",
    ] {
        assert!(
            stderr.contains(identifier),
            "missing {identifier}: {stderr}"
        );
    }
    let positions =
        ["M001/", "H006/", "CX001/", "CU001/"].map(|identifier| stderr.find(identifier).unwrap());
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn only_filters_basic_mode_dispatch() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join(".claude")).unwrap();
    std::fs::write(tmp.path().join(".claude/settings.json"), "not json").unwrap();

    let output = run_in(tmp.path(), &["--only", "H006", "."]);
    let stderr = stderr(&output);
    assert_eq!(output.status.code(), Some(1), "stderr: {stderr}");
    assert!(stderr.contains("error[H006/settings-json-invalid]"));
    assert!(!stderr.contains("M001/plugin-json-missing"));
}
