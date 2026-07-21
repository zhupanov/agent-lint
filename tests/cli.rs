use std::process::{Command, Output};

use serde_json::Value;

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

fn json(output: &Output) -> Value {
    let stderr = stderr(output);
    assert!(
        stderr.is_empty(),
        "machine output wrote to stderr: {stderr}"
    );
    json_document(output)
}

fn json_document(output: &Output) -> Value {
    let value: Value = serde_json::from_slice(&output.stdout).expect("stdout is one JSON document");
    let schema: Value =
        serde_json::from_str(include_str!("../schemas/diagnostic-output-v1.schema.json")).unwrap();
    let validator = jsonschema::validator_for(&schema).expect("checked-in output schema compiles");
    if let Err(error) = validator.validate(&value) {
        panic!("JSON output failed schema validation: {error}\n{value:#}");
    }
    value
}

fn init_git(path: &std::path::Path) {
    let output = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(path)
        .output()
        .expect("git init runs");
    assert!(
        output.status.success(),
        "git init failed: {}",
        stderr(&output)
    );
}

#[test]
fn help_succeeds_and_lists_supported_options() {
    let output = run(&["--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage: agent-lint"));
    assert!(stdout.contains("--autofix"));
    assert!(stdout.contains("--only"));
    assert!(stdout.contains("--format"));
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
fn format_conflicts_with_commands_that_own_stdout() {
    for operation in ["--list-scripts", "--closure-report"] {
        let output = run(&["--format", "json", operation]);
        assert_eq!(output.status.code(), Some(2));
        assert!(stderr(&output).contains("cannot be used with"));
    }
}

#[test]
fn json_clean_run_is_schema_valid_and_deterministic() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::create_dir(tmp.path().join(".claude")).unwrap();

    let first = run_in(tmp.path(), &["--format", "json", "."]);
    let second = run_in(tmp.path(), &["--format", "json", "."]);
    assert!(first.status.success());
    assert!(second.status.success());
    let first = json(&first);
    let second = json(&second);
    assert_eq!(first, second);
    assert_eq!(first["schema_version"], 1);
    assert_eq!(first["analysis_root"], ".");
    assert_eq!(first["mode"], "basic");
    assert_eq!(first["strictness"], "normal");
    assert!(first["selected_rules"].is_null());
    assert_eq!(first["active_platforms"], serde_json::json!(["claude"]));
    assert_eq!(first["status"], "clean");
    assert_eq!(first["counts"]["errors"], 0);
    assert_eq!(first["counts"]["warnings"], 0);
    assert_eq!(first["diagnostics"], serde_json::json!([]));
}

#[test]
fn q006_json_pairs_are_complete_stable_and_exclude_non_output_shape_prose() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::write(
        tmp.path().join("AGENTS.md"),
        "Return only JSON.\nReturn only XML.\nReturn only YAML.\n\
         The input contains exactly one sentence.\nThe request contains at least three paragraphs.\n",
    )
    .unwrap();

    let first = run_in(tmp.path(), &["--format", "json", "--only", "Q006", "."]);
    let second = run_in(tmp.path(), &["--format", "json", "--only", "Q006", "."]);
    assert!(first.status.success());
    assert!(second.status.success());
    let first = json(&first);
    let second = json(&second);
    assert_eq!(first, second);

    let diagnostics = first["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 3);
    let evidence = diagnostics
        .iter()
        .map(|diagnostic| diagnostic["evidence"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(evidence[0].starts_with("line 1:") && evidence[0].contains("line 2:"));
    assert!(evidence[1].starts_with("line 1:") && evidence[1].contains("line 3:"));
    assert!(evidence[2].starts_with("line 2:") && evidence[2].contains("line 3:"));
}

#[test]
fn q006_discovers_cursor_mdc_and_legacy_rules_through_the_cli() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::create_dir_all(tmp.path().join(".cursor/rules")).unwrap();
    let conflict = "Return only JSON.\nRespond in Markdown.\n";
    std::fs::write(
        tmp.path().join(".cursor/rules/project.mdc"),
        format!("---\ndescription: Enforces a response format\nalwaysApply: true\n---\n{conflict}"),
    )
    .unwrap();
    std::fs::write(tmp.path().join(".cursorrules"), conflict).unwrap();

    let report = json(&run_in(
        tmp.path(),
        &["--format", "json", "--only", "Q006", "."],
    ));
    let subjects = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|diagnostic| diagnostic["subject_path"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(subjects, vec![".cursorrules", ".cursor/rules/project.mdc"]);
}

#[test]
fn json_no_work_run_is_clean_with_no_selected_mode() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());

    let output = run_in(tmp.path(), &["--format", "json", "."]);
    assert!(output.status.success());
    let report = json(&output);
    assert_eq!(report["status"], "clean");
    assert!(report["mode"].is_null());
    assert_eq!(report["active_platforms"], serde_json::json!([]));
    assert_eq!(report["diagnostics"], serde_json::json!([]));
}

#[test]
fn json_lists_forced_platforms_in_stable_order() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[platforms]\ncursor = true\ncodex = true\n",
    )
    .unwrap();

    let output = run_in(tmp.path(), &["--format", "json", "."]);
    let report = json(&output);
    assert_eq!(report["mode"], "basic");
    assert_eq!(
        report["active_platforms"],
        serde_json::json!(["claude", "cursor", "codex"])
    );
}

#[test]
fn json_reports_focused_rule_selection_and_only_emits_selected_rules() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::create_dir(tmp.path().join(".claude-plugin")).unwrap();

    let output = run_in(
        tmp.path(),
        &["--format", "json", "--only", "H001,M001", "."],
    );
    assert_eq!(output.status.code(), Some(1));
    let report = json(&output);
    assert_eq!(
        report["selected_rules"],
        serde_json::json!([
            { "code": "M001", "name": "plugin-json-missing" },
            { "code": "H001", "name": "hooks-json-missing" }
        ])
    );
    assert!(
        report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|diagnostic| matches!(diagnostic["code"].as_str(), Some("M001" | "H001")))
    );
}

#[test]
fn json_mixed_run_has_structured_rule_fields_and_unchanged_exit_status() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::create_dir(tmp.path().join(".claude-plugin")).unwrap();

    let text = run_in(tmp.path(), &["."]);
    let output = run_in(tmp.path(), &["--format", "json", "."]);
    assert_eq!(text.status.code(), Some(1));
    assert_eq!(output.status.code(), text.status.code());
    let report = json(&output);
    assert_eq!(report["status"], "errors");
    assert!(report["counts"]["errors"].as_u64().unwrap() > 0);
    assert!(report["counts"]["warnings"].as_u64().unwrap() > 0);
    let diagnostics = report["diagnostics"].as_array().unwrap();
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic["code"] == "M001"
            && diagnostic["name"] == "plugin-json-missing"
            && diagnostic["severity"] == "error"
            && diagnostic["subject_path"] == ".claude-plugin/plugin.json"
    }));
}

#[test]
fn json_preserves_structured_locations_without_fabricating_them() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    let skill = tmp.path().join(".claude/skills/example/SKILL.md");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    std::fs::write(
        skill,
        "---\nname: example\ndescription: Use when checking an intentionally broken fenced block\n---\n```bash\necho broken\n",
    )
    .unwrap();

    let output = run_in(tmp.path(), &["--format", "json", "."]);
    let report = json(&output);
    let diagnostics = report["diagnostics"].as_array().unwrap();
    let located = diagnostics
        .iter()
        .find(|diagnostic| diagnostic["code"] == "X002")
        .expect("unclosed fence diagnostic is present");
    assert_eq!(located["subject_path"], ".claude/skills/example/SKILL.md");
    assert_eq!(located["location"]["start"]["line"], 5);
    assert!(located["location"]["start"].get("column").is_none());
}

#[test]
fn json_pathless_finding_has_no_subject_or_location() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::write(tmp.path().join("AGENTS.md"), "first line\nsecond line\n").unwrap();
    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        r#"[lint]
[[lint.prompt-source-budgets]]
name = "agents"
roots = ["AGENTS.md"]
root-max-lines = 1
"#,
    )
    .unwrap();

    let output = run_in(tmp.path(), &["--format", "json", "."]);
    let report = json(&output);
    let diagnostic = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|diagnostic| diagnostic["code"] == "S062")
        .expect("prompt budget diagnostic is present");
    assert!(diagnostic.get("subject_path").is_none());
    assert!(diagnostic.get("location").is_none());
}

#[test]
fn json_configuration_failure_is_one_document_and_exit_two() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::write(tmp.path().join("agent-lint.toml"), "not valid toml {{{\n").unwrap();

    let output = run_in(tmp.path(), &["--format", "json", "."]);
    assert_eq!(output.status.code(), Some(2));
    let report = json(&output);
    assert_eq!(report["status"], "usage-error");
    assert_eq!(report["counts"]["errors"], 0);
    assert_eq!(report["notices"][0]["kind"], "configuration");
    assert_eq!(report["notices"][0]["severity"], "error");
    let message = report["notices"][0]["message"].as_str().unwrap();
    assert!(message.starts_with("agent-lint.toml:"));
    assert!(!message.contains(tmp.path().to_string_lossy().as_ref()));

    let text = run_in(tmp.path(), &["."]);
    assert_eq!(text.status.code(), Some(2));
    assert!(stderr(&text).contains(tmp.path().to_string_lossy().as_ref()));
}

#[test]
fn json_setup_failure_is_one_document_and_exit_two() {
    let tmp = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("missing");
    let output = run(&["--format", "json", missing.to_string_lossy().as_ref()]);
    assert_eq!(output.status.code(), Some(2));
    let report = json(&output);
    assert_eq!(report["status"], "usage-error");
    assert_eq!(report["notices"][0]["kind"], "setup");
}

#[test]
fn json_invalid_only_is_schema_valid_and_preserves_prior_notices() {
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().to_string_lossy().into_owned();

    for only in ["NOT_A_RULE", "Q006,", ",Q006"] {
        let output = run(&["--format", "json", "--only", only, &target]);
        assert_eq!(output.status.code(), Some(2), "--only {only}");
        let report = json(&output);
        assert_eq!(report["status"], "usage-error");
        assert!(report["selected_rules"].is_null());
        assert_eq!(report["notices"][0]["kind"], "repository-root");
        assert_eq!(report["notices"][1]["kind"], "usage");
        let message = report["notices"][1]["message"].as_str().unwrap();
        if only == "NOT_A_RULE" {
            assert!(message.contains("invalid rule identifier 'NOT_A_RULE'"));
        } else {
            assert!(message.contains("empty rule identifier"));
        }
    }

    let text = run(&["--only", "NOT_A_RULE", &target]);
    assert_eq!(text.status.code(), Some(2));
    assert!(stderr(&text).contains("invalid rule identifier 'NOT_A_RULE'"));
}

#[test]
fn json_repository_root_notice_makes_an_otherwise_clean_run_warning_status() {
    let tmp = tempfile::tempdir().unwrap();
    let output = run(&["--format", "json", tmp.path().to_string_lossy().as_ref()]);
    assert!(output.status.success());
    let report = json(&output);
    assert_eq!(report["counts"]["warnings"], 0);
    assert_eq!(report["status"], "warnings");
    assert_eq!(report["notices"][0]["kind"], "repository-root");
}

#[test]
fn json_autofix_serializes_only_the_final_validation() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    write_skill(tmp.path(), "fixed", "http://current.invalid");

    let output = run_in(tmp.path(), &["--autofix", "--format", "json", "."]);
    assert!(output.status.success());
    assert!(stderr(&output).contains("fixed[S031/non-https-url]"));
    let report = json_document(&output);
    assert!(
        !report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["code"] == "S031")
    );
    let fixed = std::fs::read_to_string(tmp.path().join(".claude/skills/fixed/SKILL.md")).unwrap();
    assert!(fixed.contains("https://current.invalid"));
}

#[test]
fn text_and_json_strictness_modes_validate_and_preserve_gating() {
    for format in ["text", "json"] {
        for strictness in [None, Some("--pedantic"), Some("--all")] {
            let tmp = tempfile::tempdir().unwrap();
            init_git(tmp.path());
            std::fs::write(tmp.path().join("budget.md"), "first line\nsecond line\n").unwrap();
            std::fs::write(
                tmp.path().join("agent-lint.toml"),
                r#"[lint]
[[lint.prompt-source-budgets]]
name = "budget"
roots = ["budget.md"]
root-max-lines = 1
"#,
            )
            .unwrap();
            let mut args = vec!["--format", format];
            if let Some(strictness) = strictness {
                args.push(strictness);
            }
            args.push(".");
            let output = run_in(tmp.path(), &args);
            let expected_exit = if strictness.is_none() { 0 } else { 1 };
            assert_eq!(output.status.code(), Some(expected_exit));
            if format == "json" {
                let report = json(&output);
                let expected_status = if strictness.is_none() {
                    "warnings"
                } else {
                    "errors"
                };
                assert_eq!(report["status"], expected_status);
            } else {
                assert!(stderr(&output).contains("Lint:"));
            }
        }
    }
}

#[test]
fn json_unused_override_is_a_structured_notice() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        r#"[lint]
[[lint.overrides]]
files = ["missing.md"]
suppress = ["M001"]
reason = "stale exception"
"#,
    )
    .unwrap();

    let output = run_in(tmp.path(), &["--format", "json", "."]);
    assert!(output.status.success());
    let report = json(&output);
    assert_eq!(report["notices"][0]["kind"], "unused-override");
    assert_eq!(report["notices"][0]["severity"], "warning");
    assert!(
        !report["notices"][0]["message"]
            .as_str()
            .unwrap()
            .starts_with("warning[")
    );
    assert_eq!(report["counts"]["notices"], 1);
}

#[test]
fn json_counts_suppressed_diagnostics_without_rendering_them() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
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

    let output = run_in(tmp.path(), &["--format", "json", "."]);
    assert!(output.status.success());
    let report = json(&output);
    assert_eq!(report["counts"]["suppressed"], 1);
    assert!(
        !report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["code"] == "S031")
    );
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
        ("M001,,H001", "empty rule identifier"),
        ("", "empty rule identifier"),
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
fn agent_stop_missing_respects_strictness_only_and_per_file_suppression() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::create_dir_all(tmp.path().join(".claude/agents")).unwrap();
    std::fs::write(
        tmp.path().join(".claude/agents/reviewer.md"),
        "---\nname: reviewer\ndescription: Reviews changes and makes targeted repairs when needed\ntools: Bash\n---\nInvestigate the failure and implement a repair.\n",
    )
    .unwrap();

    let normal = run_in(tmp.path(), &["--only", "A029", "."]);
    let normal_stderr = stderr(&normal);
    assert!(normal.status.success(), "stderr: {normal_stderr}");
    assert!(normal_stderr.contains("warning[A029/agent-stop-missing]"));

    for strictness in ["--pedantic", "--all"] {
        let output = run_in(tmp.path(), &[strictness, "--only", "A029", "."]);
        let output_stderr = stderr(&output);
        assert_eq!(output.status.code(), Some(1), "stderr: {output_stderr}");
        assert!(output_stderr.contains("error[A029/agent-stop-missing]"));
    }

    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        r#"[lint]
[[lint.overrides]]
files = [".claude/agents/reviewer.md"]
suppress = ["agent-stop-missing"]
"#,
    )
    .unwrap();
    for args in [
        vec!["--only", "A029", "."],
        vec!["--pedantic", "--only", "A029", "."],
    ] {
        let output = run_in(tmp.path(), &args);
        let output_stderr = stderr(&output);
        assert!(output.status.success(), "stderr: {output_stderr}");
        assert!(!output_stderr.contains("A029/agent-stop-missing"));
        assert!(output_stderr.contains("(1 suppressed)"));
    }
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
