use super::*;
use crate::test_helpers::CwdGuard;

/// Run the Codex-surface validator against a temporary repository built from
/// `files`, returning the collected diagnostics. Every caller must be
/// `#[serial_test::serial]` because the walk keys off the process directory.
fn run_in(files: &[(&str, &str)]) -> DiagnosticCollector {
    let tmp = tempfile::tempdir().unwrap();
    let _guard = CwdGuard::new();
    std::env::set_current_dir(tmp.path()).unwrap();
    for (path, contents) in files {
        let path = Path::new(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }
    let mut diag = DiagnosticCollector::new_all_enabled();
    validate(&mut diag, &ExcludeSet::default());
    diag
}

/// Run against a single root `.codex-plugin/plugin.json` with the given body.
fn run_manifest(body: &str) -> DiagnosticCollector {
    run_in(&[(".codex-plugin/plugin.json", body)])
}

fn has_rule(diag: &DiagnosticCollector, rule: LintRule) -> bool {
    diag.diagnostics().iter().any(|item| item.rule == rule)
}

fn count_rule(diag: &DiagnosticCollector, rule: LintRule) -> usize {
    diag.diagnostics()
        .iter()
        .filter(|item| item.rule == rule)
        .count()
}

fn codes(diag: &DiagnosticCollector) -> Vec<&'static str> {
    diag.diagnostics()
        .iter()
        .map(|item| item.rule.code())
        .collect()
}

fn only_skills(value: &str) -> String {
    format!(r#"{{"name":"my-plugin","description":"Valid plugin.","skills":{value}}}"#)
}

// ── discovery: alternate paths, nesting, precedence, suffix collisions ────

#[test]
#[serial_test::serial]
fn alternate_and_nested_recognized_manifests_are_valid_roots() {
    let valid = r#"{"name":"my-plugin","description":"A valid Codex plugin."}"#;
    let diag = run_in(&[
        (".codex-plugin/plugin.json", valid),
        ("plugins/nested/.codex-plugin/plugin.json", valid),
        ("packages/only-claude/.claude-plugin/plugin.json", valid),
        ("packages/only-cursor/.cursor-plugin/plugin.json", valid),
    ]);
    assert!(
        diag.diagnostics().is_empty(),
        "alternate and nested valid manifests must be clean: {:?}",
        diag.errors()
    );
    assert!(!has_rule(&diag, LintRule::CodexPluginManifestPath));
}

#[test]
#[serial_test::serial]
fn suffix_collision_manifests_are_not_recognized() {
    // Parent-directory component, not path suffix: these establish no plugin root.
    let diag = run_in(&[
        ("my.codex-plugin/plugin.json", "not json {"),
        ("x.claude-plugin/plugin.json", "not json {"),
        ("y.cursor-plugin/plugin.json", "not json {"),
    ]);
    assert!(
        diag.diagnostics().is_empty(),
        "suffix-collision files must produce no CX diagnostics: {:?}",
        codes(&diag)
    );
}

#[test]
#[serial_test::serial]
fn precedence_prefers_codex_then_claude_then_cursor() {
    // All three present in one root: only .codex-plugin is selected and validated.
    let diag = run_in(&[
        (
            ".codex-plugin/plugin.json",
            r#"{"name":"ok-plugin","description":"Good."}"#,
        ),
        (".claude-plugin/plugin.json", r#"{"name":"Bad_Name"}"#),
        (".cursor-plugin/plugin.json", r#"{"name":"Bad_Name"}"#),
    ]);
    assert!(
        diag.diagnostics().is_empty(),
        "codex manifest wins precedence and is clean: {:?}",
        codes(&diag)
    );

    // Only claude + cursor: claude is selected.
    let diag = run_in(&[
        (
            ".claude-plugin/plugin.json",
            r#"{"name":"Bad_Name","description":"x"}"#,
        ),
        (
            ".cursor-plugin/plugin.json",
            r#"{"name":"ok-plugin","description":"x"}"#,
        ),
    ]);
    assert_eq!(count_rule(&diag, LintRule::CodexPluginNameInvalid), 1);
    let subject = diag
        .diagnostics()
        .iter()
        .find(|item| item.rule == LintRule::CodexPluginNameInvalid)
        .and_then(|item| item.subject_path.clone())
        .unwrap();
    assert_eq!(
        subject.to_string_lossy(),
        ".claude-plugin/plugin.json",
        "claude precedes cursor when codex is absent"
    );
}

// ── CX047: unreadable / invalid / non-object ─────────────────────────────

#[test]
#[serial_test::serial]
fn invalid_utf8_manifest_is_cx047() {
    let tmp = tempfile::tempdir().unwrap();
    let _guard = CwdGuard::new();
    std::env::set_current_dir(tmp.path()).unwrap();
    std::fs::create_dir_all(".codex-plugin").unwrap();
    std::fs::write(".codex-plugin/plugin.json", [0xff, 0xfe, b'{', b'}']).unwrap();
    let mut diag = DiagnosticCollector::new_all_enabled();
    validate(&mut diag, &ExcludeSet::default());
    assert!(has_rule(&diag, LintRule::CodexPluginManifestInvalid));
    assert_eq!(
        diag.diagnostics().len(),
        1,
        "no cascade after unreadable file"
    );
}

#[test]
#[serial_test::serial]
fn invalid_json_is_cx047_with_location() {
    let diag = run_manifest("{\n  \"name\": \n}");
    let diagnostic = diag
        .diagnostics()
        .iter()
        .find(|item| item.rule == LintRule::CodexPluginManifestInvalid)
        .expect("CX047 fires on invalid JSON");
    assert!(
        diagnostic.location.is_some(),
        "parse error carries a location"
    );
    assert_eq!(diag.diagnostics().len(), 1, "no cascade after invalid JSON");
}

#[test]
#[serial_test::serial]
fn non_object_root_is_cx047_only() {
    for body in [r#"[1, 2, 3]"#, r#""a string""#, "42", "true", "null"] {
        let diag = run_manifest(body);
        assert!(
            has_rule(&diag, LintRule::CodexPluginManifestInvalid),
            "non-object root {body} is CX047"
        );
        assert_eq!(
            diag.diagnostics().len(),
            1,
            "no cascade for root {body}: {:?}",
            codes(&diag)
        );
    }
}

// ── CX048 / CX049 / CX047: name ──────────────────────────────────────────

#[test]
#[serial_test::serial]
fn name_missing_or_blank_is_cx048() {
    let missing = run_manifest(r#"{"description":"x"}"#);
    assert!(has_rule(&missing, LintRule::CodexPluginNameMissing));

    let blank = run_manifest(r#"{"name":"   ","description":"x"}"#);
    assert!(has_rule(&blank, LintRule::CodexPluginNameMissing));
}

#[test]
#[serial_test::serial]
fn name_kebab_case_boundaries() {
    for bad in [
        "Bad_Name",
        "bad name",
        "-bad",
        "bad-",
        "a--b",
        "UPPER",
        "with_underscore",
    ] {
        let diag = run_manifest(&format!(r#"{{"name":"{bad}","description":"x"}}"#));
        assert!(
            has_rule(&diag, LintRule::CodexPluginNameInvalid),
            "name {bad:?} must be CX049"
        );
    }
    for good in ["my-plugin", "plugin", "a1-b2-c3", "x"] {
        let diag = run_manifest(&format!(r#"{{"name":"{good}","description":"x"}}"#));
        assert!(
            !has_rule(&diag, LintRule::CodexPluginNameInvalid),
            "name {good:?} must be accepted"
        );
        assert!(!has_rule(&diag, LintRule::CodexPluginNameMissing));
    }
}

#[test]
#[serial_test::serial]
fn non_string_name_is_cx047_not_cx048() {
    let diag = run_manifest(r#"{"name":123,"description":"x"}"#);
    assert!(has_rule(&diag, LintRule::CodexPluginManifestInvalid));
    assert!(!has_rule(&diag, LintRule::CodexPluginNameMissing));
    assert!(!has_rule(&diag, LintRule::CodexPluginNameInvalid));
}

// ── CX050 / CX051 / CX052 / CX047: component paths ───────────────────────

#[test]
#[serial_test::serial]
fn component_path_containment_partition() {
    let cases = [
        (r#""skills""#, Some(LintRule::CodexPluginPathPrefix)),
        (r#"" ./skills""#, Some(LintRule::CodexPluginPathPrefix)),
        (r#""./""#, Some(LintRule::CodexPluginPathBare)),
        (r#""../x""#, Some(LintRule::CodexPluginPathTraversal)),
        (
            r#"".//etc/passwd""#,
            Some(LintRule::CodexPluginPathTraversal),
        ),
        (r#""/etc""#, Some(LintRule::CodexPluginPathTraversal)),
        (r#""C:\\Windows""#, Some(LintRule::CodexPluginPathTraversal)),
        (r#""..\\x""#, Some(LintRule::CodexPluginPathTraversal)),
        (r#""./a/../b""#, Some(LintRule::CodexPluginPathTraversal)),
        (r#""./skills""#, None),
    ];
    for (value, expected) in cases {
        let diag = run_manifest(&only_skills(value));
        match expected {
            Some(rule) => assert!(
                has_rule(&diag, rule),
                "skills={value} must emit {}",
                rule.code()
            ),
            None => assert!(
                diag.diagnostics().is_empty(),
                "skills={value} must be clean: {:?}",
                codes(&diag)
            ),
        }
    }
}

#[test]
#[serial_test::serial]
fn component_field_shapes_and_type_errors() {
    // skills string array with a non-string element → one CX047 at the index.
    let diag = run_manifest(&only_skills(r#"["./a", 123]"#));
    assert_eq!(count_rule(&diag, LintRule::CodexPluginManifestInvalid), 1);

    // Whole-field wrong type → CX047.
    let diag = run_manifest(&only_skills("123"));
    assert!(has_rule(&diag, LintRule::CodexPluginManifestInvalid));

    // apps must be a string.
    let diag = run_manifest(r#"{"name":"my-plugin","description":"x","apps":{"a":1}}"#);
    assert!(has_rule(&diag, LintRule::CodexPluginManifestInvalid));

    // mcpServers object is valid inline; array is not.
    let ok = run_manifest(r#"{"name":"my-plugin","description":"x","mcpServers":{"srv":{}}}"#);
    assert!(
        ok.diagnostics().is_empty(),
        "inline mcpServers object is valid: {:?}",
        codes(&ok)
    );
    let bad = run_manifest(r#"{"name":"my-plugin","description":"x","mcpServers":[1]}"#);
    assert!(has_rule(&bad, LintRule::CodexPluginManifestInvalid));

    // commands string is path-checked.
    let diag = run_manifest(r#"{"name":"my-plugin","description":"x","commands":"cmds"}"#);
    assert!(has_rule(&diag, LintRule::CodexPluginPathPrefix));
}

// ── CX058 soft-retire: hooks are supported ───────────────────────────────

#[test]
#[serial_test::serial]
fn hooks_are_supported_no_cx058() {
    let base = r#"{"name":"my-plugin","description":"x","hooks":HOOKS}"#;
    for hooks in [
        r#""./hooks/hooks.json""#,
        r#"["./hooks/hooks.json"]"#,
        r#"{"PreToolUse":[]}"#,
        r#"[{"event":"PreToolUse"}]"#,
    ] {
        let diag = run_manifest(&base.replace("HOOKS", hooks));
        assert!(!has_rule(&diag, LintRule::CodexPluginHooksUnsupported));
        assert!(
            diag.diagnostics().is_empty(),
            "hooks={hooks} clean: {:?}",
            codes(&diag)
        );
    }

    // Hook path strings still participate in CX050–CX052 (all three).
    for (hooks, rule) in [
        (r#""hooks/hooks.json""#, LintRule::CodexPluginPathPrefix),
        (r#""./""#, LintRule::CodexPluginPathBare),
        (r#""../escape""#, LintRule::CodexPluginPathTraversal),
        (r#"["hooks/hooks.json"]"#, LintRule::CodexPluginPathPrefix),
    ] {
        let diag = run_manifest(&base.replace("HOOKS", hooks));
        assert!(
            has_rule(&diag, rule),
            "hooks={hooks} must emit {}",
            rule.code()
        );
        assert!(!has_rule(&diag, LintRule::CodexPluginHooksUnsupported));
    }
}

// ── CX053 / CX054 / CX055 / CX047: default prompts ───────────────────────

fn with_interface(interface: &str) -> String {
    format!(r#"{{"name":"my-plugin","description":"x","interface":{interface}}}"#)
}

#[test]
#[serial_test::serial]
fn default_prompt_count_boundary() {
    let three = with_interface(r#"{"defaultPrompt":["a","b","c"]}"#);
    assert!(!has_rule(
        &run_manifest(&three),
        LintRule::CodexPluginDefaultPromptCount
    ));
    let four = with_interface(r#"{"defaultPrompt":["a","b","c","d"]}"#);
    assert!(has_rule(
        &run_manifest(&four),
        LintRule::CodexPluginDefaultPromptCount
    ));
}

#[test]
#[serial_test::serial]
fn default_prompt_length_boundary() {
    let at_limit = "x".repeat(128);
    let over_limit = "x".repeat(129);
    let ok = with_interface(&format!(r#"{{"defaultPrompt":"{at_limit}"}}"#));
    assert!(!has_rule(
        &run_manifest(&ok),
        LintRule::CodexPluginDefaultPromptLength
    ));
    let bad = with_interface(&format!(r#"{{"defaultPrompt":"{over_limit}"}}"#));
    assert!(has_rule(
        &run_manifest(&bad),
        LintRule::CodexPluginDefaultPromptLength
    ));
}

#[test]
#[serial_test::serial]
fn default_prompt_empty_and_type_rules() {
    let empty = with_interface(r#"{"defaultPrompt":["   ",""]}"#);
    assert!(has_rule(
        &run_manifest(&empty),
        LintRule::CodexPluginDefaultPromptEmpty
    ));

    // Non-string entries → CX047 per index, no false count from filtering.
    let numbers = with_interface(r#"{"defaultPrompt":[1,2,3,4,5]}"#);
    let diag = run_manifest(&numbers);
    assert_eq!(count_rule(&diag, LintRule::CodexPluginManifestInvalid), 5);
    assert!(!has_rule(&diag, LintRule::CodexPluginDefaultPromptCount));

    // Whole-value wrong type → CX047.
    let scalar = with_interface(r#"{"defaultPrompt":42}"#);
    assert!(has_rule(
        &run_manifest(&scalar),
        LintRule::CodexPluginManifestInvalid
    ));
}

// ── CX063: ignored prompt-key aliases ────────────────────────────────────

#[test]
#[serial_test::serial]
fn cx063_fires_once_per_ignored_prompt_key() {
    let singular = with_interface(r#"{"default_prompt":"hi"}"#);
    assert_eq!(
        count_rule(&run_manifest(&singular), LintRule::CodexPluginPromptField),
        1
    );

    let plural = with_interface(r#"{"default_prompts":["hi"]}"#);
    assert_eq!(
        count_rule(&run_manifest(&plural), LintRule::CodexPluginPromptField),
        1
    );

    let both = with_interface(r#"{"default_prompt":"a","default_prompts":["b"]}"#);
    assert_eq!(
        count_rule(&run_manifest(&both), LintRule::CodexPluginPromptField),
        2
    );

    // Canonical key alone → no CX063.
    let canonical = with_interface(r#"{"defaultPrompt":"hi"}"#);
    assert_eq!(
        count_rule(&run_manifest(&canonical), LintRule::CodexPluginPromptField),
        0
    );
}

// ── CX056 / CX047: interface URLs ────────────────────────────────────────

#[test]
#[serial_test::serial]
fn interface_url_both_spellings_and_boundaries() {
    // Canonical spelling, valid https → clean.
    let ok = with_interface(r#"{"websiteURL":"https://example.com/x"}"#);
    assert!(!has_rule(
        &run_manifest(&ok),
        LintRule::CodexPluginInterfaceUrl
    ));

    // Lowercase alias is still checked.
    let alias = with_interface(r#"{"websiteUrl":"http://example.com"}"#);
    assert!(has_rule(
        &run_manifest(&alias),
        LintRule::CodexPluginInterfaceUrl
    ));

    // Embedded credentials rejected.
    let creds = with_interface(r#"{"privacyPolicyURL":"https://user:pw@example.com"}"#);
    assert!(has_rule(
        &run_manifest(&creds),
        LintRule::CodexPluginInterfaceUrl
    ));

    // Over-length rejected.
    let long = format!("https://example.com/{}", "a".repeat(1024));
    let over = with_interface(&format!(r#"{{"termsOfServiceURL":"{long}"}}"#));
    assert!(has_rule(
        &run_manifest(&over),
        LintRule::CodexPluginInterfaceUrl
    ));

    // Non-string URL → CX047, not CX056.
    let typed = with_interface(r#"{"websiteURL":123}"#);
    let diag = run_manifest(&typed);
    assert!(has_rule(&diag, LintRule::CodexPluginManifestInvalid));
    assert!(!has_rule(&diag, LintRule::CodexPluginInterfaceUrl));
}

// ── CX057 / CX047: interface assets ──────────────────────────────────────

#[test]
#[serial_test::serial]
fn interface_assets_cover_every_field_and_index() {
    for field in ["composerIcon", "logo", "logoDark"] {
        let bad = with_interface(&format!(r#"{{"{field}":"../evil.svg"}}"#));
        assert!(
            has_rule(&run_manifest(&bad), LintRule::CodexPluginInterfaceAssetPath),
            "{field}"
        );
        let bare = with_interface(&format!(r#"{{"{field}":"./"}}"#));
        assert!(
            has_rule(
                &run_manifest(&bare),
                LintRule::CodexPluginInterfaceAssetPath
            ),
            "{field} bare"
        );
        let typed = with_interface(&format!(r#"{{"{field}":5}}"#));
        assert!(
            has_rule(&run_manifest(&typed), LintRule::CodexPluginManifestInvalid),
            "{field} type"
        );
    }

    // screenshots array: bad path + non-string element.
    let shots = with_interface(r#"{"screenshots":["/abs.png", 7]}"#);
    let diag = run_manifest(&shots);
    assert!(has_rule(&diag, LintRule::CodexPluginInterfaceAssetPath));
    assert!(has_rule(&diag, LintRule::CodexPluginManifestInvalid));

    // screenshots as a bare string is the wrong container → CX047.
    let single = with_interface(r#"{"screenshots":"./single.png"}"#);
    assert!(has_rule(
        &run_manifest(&single),
        LintRule::CodexPluginManifestInvalid
    ));

    // Valid assets are clean.
    let clean = with_interface(r#"{"logoDark":"./assets/dark.svg","screenshots":["./s1.png"]}"#);
    assert!(run_manifest(&clean).diagnostics().is_empty());
}

// ── CX059: description advisory ──────────────────────────────────────────

#[test]
#[serial_test::serial]
fn description_missing_blank_or_non_string_is_cx059() {
    for body in [
        r#"{"name":"my-plugin"}"#,
        r#"{"name":"my-plugin","description":"  "}"#,
        r#"{"name":"my-plugin","description":123}"#,
    ] {
        let diag = run_manifest(body);
        assert!(
            has_rule(&diag, LintRule::CodexPluginDescriptionMissing),
            "{body}"
        );
    }
    let ok = run_manifest(r#"{"name":"my-plugin","description":"A real description."}"#);
    assert!(!has_rule(&ok, LintRule::CodexPluginDescriptionMissing));
}

// ── metadata: location, evidence, redaction, determinism ─────────────────

#[test]
#[serial_test::serial]
fn diagnostics_carry_location_evidence_and_suggestion() {
    let diag = run_manifest(r#"{"name":"Bad_Name","description":"x"}"#);
    let diagnostic = diag
        .diagnostics()
        .iter()
        .find(|item| item.rule == LintRule::CodexPluginNameInvalid)
        .unwrap();
    assert!(diagnostic.location.is_some());
    assert!(
        diagnostic
            .evidence
            .as_deref()
            .is_some_and(|value| value.contains("Bad_Name"))
    );
    assert!(diagnostic.suggestion.is_some());
}

#[test]
#[serial_test::serial]
fn secret_like_evidence_is_redacted() {
    let body = only_skills(r#""token = 'this-is-a-sensitive-value'""#);
    let diag = run_manifest(&body);
    let diagnostic = diag
        .diagnostics()
        .iter()
        .find(|item| item.rule == LintRule::CodexPluginPathPrefix)
        .unwrap();
    let evidence = diagnostic.evidence.as_deref().unwrap();
    assert!(evidence.contains("redacted"), "evidence: {evidence}");
    assert!(!evidence.contains("this-is-a-sensitive-value"));
}

#[test]
#[serial_test::serial]
fn secret_past_the_truncation_window_is_still_redacted() {
    // The secret sits well past the 80-scalar evidence window; classifying the
    // full value (not the truncated prefix) must still redact it. The value omits
    // a `./` prefix so it fires CX050.
    let padding = "a".repeat(120);
    let body = only_skills(&format!(
        r#""{padding} api_key = sk-ABCDEFGHIJKLMNOP1234567890""#
    ));
    let diag = run_manifest(&body);
    let diagnostic = diag
        .diagnostics()
        .iter()
        .find(|item| item.rule == LintRule::CodexPluginPathPrefix)
        .unwrap();
    let evidence = diagnostic.evidence.as_deref().unwrap();
    assert!(evidence.contains("redacted"), "evidence: {evidence}");
    assert!(!evidence.contains("sk-ABCDEFGHIJKLMNOP1234567890"));
}

#[test]
#[serial_test::serial]
fn cx056_redacts_embedded_credentials() {
    let body = with_interface(r#"{"websiteURL":"https://alice:s3cr3tPassw0rd@example.com"}"#);
    let diag = run_manifest(&body);
    let diagnostic = diag
        .diagnostics()
        .iter()
        .find(|item| item.rule == LintRule::CodexPluginInterfaceUrl)
        .unwrap();
    let evidence = diagnostic.evidence.as_deref().unwrap_or("");
    assert!(
        !evidence.contains("s3cr3tPassw0rd"),
        "evidence leaked credentials: {evidence}"
    );
    assert!(evidence.contains("redacted"));
    // The message must not leak the value either.
    assert!(!diagnostic.message.contains("s3cr3tPassw0rd"));
}

#[test]
#[serial_test::serial]
fn oversized_manifest_is_cx047_and_not_cascaded() {
    // A manifest past the size limit is CX047 alone — no per-field cascade.
    let filler = "x".repeat(70 * 1024);
    let body = format!(r#"{{"name":"Bad_Name","description":"","note":"{filler}"}}"#);
    let diag = run_manifest(&body);
    assert!(has_rule(&diag, LintRule::CodexPluginManifestInvalid));
    assert_eq!(
        diag.diagnostics().len(),
        1,
        "oversized manifest must not cascade"
    );
    assert!(!has_rule(&diag, LintRule::CodexPluginNameInvalid));
}

#[test]
#[serial_test::serial]
fn hostile_wide_array_is_bounded() {
    // A wide array of bad paths must not emit an unbounded diagnostic count.
    let elements = std::iter::repeat_n("\"x\"", 5000)
        .collect::<Vec<_>>()
        .join(",");
    let body = only_skills(&format!("[{elements}]"));
    let diag = run_manifest(&body);
    let prefix_count = count_rule(&diag, LintRule::CodexPluginPathPrefix);
    assert!(prefix_count > 0, "some CX050 must fire");
    assert!(
        prefix_count <= super::MAX_VALIDATED_ARRAY_ELEMENTS,
        "per-array diagnostics must be bounded, got {prefix_count}"
    );
}

#[test]
#[serial_test::serial]
fn diagnostic_order_is_deterministic_across_runs() {
    let body = r#"{
      "name":"Bad_Name",
      "skills":"skills",
      "interface":{"defaultPrompt":["a","b","c","d"],"logo":"./"}
    }"#;
    let first = codes(&run_manifest(body));
    let second = codes(&run_manifest(body));
    assert_eq!(first, second);
    assert!(!first.is_empty());
}

// ── unit tests for the internal helpers ──────────────────────────────────

#[test]
fn classify_path_unit() {
    assert!(matches!(
        classify_path("skills"),
        Some(PathDefect::MissingPrefix)
    ));
    assert!(matches!(
        classify_path(" ./skills"),
        Some(PathDefect::MissingPrefix)
    ));
    assert!(matches!(classify_path("./"), Some(PathDefect::Bare)));
    assert!(matches!(classify_path("../x"), Some(PathDefect::Traversal)));
    assert!(matches!(
        classify_path(".//etc"),
        Some(PathDefect::Traversal)
    ));
    assert!(matches!(classify_path("/etc"), Some(PathDefect::Traversal)));
    assert!(matches!(
        classify_path("C:\\x"),
        Some(PathDefect::Traversal)
    ));
    assert!(matches!(
        classify_path("..\\x"),
        Some(PathDefect::Traversal)
    ));
    assert!(classify_path("./skills").is_none());
    assert!(classify_path("./a/b/c").is_none());
}

#[test]
fn is_valid_publish_url_unit() {
    assert!(is_valid_publish_url("https://example.com"));
    assert!(is_valid_publish_url("https://example.com/path?q=1"));
    assert!(!is_valid_publish_url("http://example.com"));
    assert!(!is_valid_publish_url("https://user:pw@example.com"));
    assert!(!is_valid_publish_url("https://"));
    assert!(!is_valid_publish_url("ftp://example.com"));
    assert!(!is_valid_publish_url("not a url"));
}

#[test]
fn json_scanner_locates_nested_paths() {
    let source = r#"{"name":"x","interface":{"defaultPrompt":["a","bb"]}}"#;
    // Locations point at the offending value, not the key/member.
    let name = JsonScanner::locate(source, &[Seg::Key("name")]).unwrap();
    assert_eq!(&source[name], "\"x\"");

    let prompt = JsonScanner::locate(
        source,
        &[
            Seg::Key("interface"),
            Seg::Key("defaultPrompt"),
            Seg::Index(1),
        ],
    )
    .unwrap();
    assert_eq!(&source[prompt], "\"bb\"");

    assert!(JsonScanner::locate(source, &[Seg::Key("missing")]).is_none());
}

// ── preserved surfaces: CX042 override tracking, CX060 skill frontmatter ──

#[test]
#[serial_test::serial]
fn validates_tracked_agents_override() {
    let tmp = tempfile::tempdir().unwrap();
    let _guard = CwdGuard::new();
    std::env::set_current_dir(tmp.path()).unwrap();
    std::fs::write("AGENTS.override.md", "personal settings\n").unwrap();
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .status()
            .unwrap()
            .success()
    );
    assert!(
        std::process::Command::new("git")
            .args(["add", "AGENTS.override.md"])
            .status()
            .unwrap()
            .success()
    );

    let mut diag = DiagnosticCollector::new_all_enabled();
    validate(&mut diag, &ExcludeSet::default());
    assert!(has_rule(&diag, LintRule::CodexAgentsOverrideTracked));
}

#[test]
#[serial_test::serial]
fn rejects_claude_only_codex_skill_frontmatter() {
    let diag = run_in(&[(
        ".agents/skills/example/SKILL.md",
        "---\nname: example\ndescription: Example\ncontext: fork\nagent: Explore\nhooks: {}\n---\nbody\n",
    )]);
    assert_eq!(
        diag.errors()
            .iter()
            .filter(|error| error.contains("Claude-only"))
            .count(),
        3
    );
}
