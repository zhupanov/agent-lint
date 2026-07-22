//! L006: npm scripts referenced from instruction files must exist in root package.json.

use crate::config::ExcludeSet;
use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata, SourceSpan};
use crate::markdown::MarkdownDocument;
#[cfg(test)]
use crate::markdown_commands::CommandSurface;
use crate::markdown_commands::{ShellToken, command_fragments, tokenize_shell_commands};
use crate::rules::LintRule;
use std::collections::BTreeMap;
use std::fs;
use std::ops::Range;
use std::path::Path;

const L006_SUGGESTION: &str = "add this script to the root package.json or correct the command";

/// Qualifying flags that select a non-root package/context. Root `package.json`
/// is not authoritative for these commands, so they are skipped entirely.
const QUALIFIER_FLAGS: &[&str] = &[
    "--workspace",
    "-w",
    "--workspaces",
    "--prefix",
    "--global",
    "-g",
];

/// Documented bare npm flags whose following token is a value, not the script.
/// All other bare `--name` flags are treated as no-value. Long forms only.
const VALUE_TAKING_FLAGS: &[&str] = &[
    "--loglevel",
    "--registry",
    "--script-shell",
    "--otp",
    "--userconfig",
    "--cache",
];

/// L006: `npm run` / `npm run-script` referenced from a configured instruction
/// file must exist in the root `package.json` `scripts` map.
///
/// Silent when root `package.json` is absent, unreadable, invalid JSON, or
/// lacks an object-valued `scripts` field. Emits once per distinct missing
/// script per source file, ordered by the first command span. Not autofixable.
///
/// Extraction, qualification, and skip boundaries are documented on
/// [`crate::markdown_commands`] and in `docs/rules.md`.
pub fn validate_npm_scripts(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let Ok(pkg_text) = fs::read_to_string("package.json") else {
        return;
    };
    let Ok(pkg) = serde_json::from_str::<serde_json::Value>(&pkg_text) else {
        return;
    };
    let Some(scripts) = pkg.get("scripts").and_then(|value| value.as_object()) else {
        return;
    };

    for relpath in diag.config().instruction_files.clone() {
        if exclude.is_excluded(&relpath) {
            continue;
        }
        let path = Path::new(&relpath);
        if path.is_symlink() {
            continue;
        }
        let Ok(content) = fs::read_to_string(path) else {
            continue;
        };
        let document = MarkdownDocument::parse(&content);
        let mut findings: BTreeMap<String, ScriptFinding> = BTreeMap::new();

        for fragment in command_fragments(&document) {
            let Some(commands) = tokenize_shell_commands(&content, &fragment) else {
                continue;
            };
            for tokens in commands {
                if let Some(finding) = find_missing_npm_script(&tokens, scripts) {
                    findings
                        .entry(finding.script.clone())
                        .and_modify(|existing| {
                            if finding.source_range.start < existing.source_range.start {
                                *existing = finding.clone();
                            }
                        })
                        .or_insert(finding);
                }
            }
        }

        let mut ordered: Vec<_> = findings.into_values().collect();
        ordered.sort_by_key(|finding| finding.source_range.start);
        for finding in ordered {
            let mut metadata = DiagnosticMetadata::default()
                .with_evidence(&finding.script)
                .with_suggestion(L006_SUGGESTION);
            if let Some(span) = SourceSpan::from_byte_range(&content, finding.source_range.clone())
            {
                metadata = metadata.with_location(span);
            }
            diag.report_at_with(
                LintRule::NpmScriptMissing,
                &relpath,
                &format!(
                    "npm run {} is not defined in package.json scripts",
                    finding.script
                ),
                metadata,
            );
        }
    }
}

#[derive(Debug, Clone)]
struct ScriptFinding {
    script: String,
    source_range: Range<usize>,
}

fn find_missing_npm_script(
    tokens: &[ShellToken],
    scripts: &serde_json::Map<String, serde_json::Value>,
) -> Option<ScriptFinding> {
    let mut index = 0;
    while index < tokens.len() {
        if strip_quotes(&tokens[index].text) != "npm" {
            index += 1;
            continue;
        }
        match parse_npm_run(&tokens[index..]) {
            NpmRunParse::Missing { script, range } => {
                if scripts.contains_key(&script) {
                    return None;
                }
                return Some(ScriptFinding {
                    script,
                    source_range: range,
                });
            }
            NpmRunParse::Skip => return None,
            NpmRunParse::NotNpmRun => index += 1,
        }
    }
    None
}

#[derive(Debug)]
enum NpmRunParse {
    Missing { script: String, range: Range<usize> },
    Skip,
    NotNpmRun,
}

/// Recognize `npm run` / `npm run-script` with no-value flags in `--name` or
/// `--name=value` form before `run` and between `run` and the script token.
/// Arguments after the script do not affect identity. Qualifier flags anywhere
/// before the script skip the command. Bare flags in [`VALUE_TAKING_FLAGS`]
/// consume the following token; every other bare `--name` flag is no-value.
fn parse_npm_run(tokens: &[ShellToken]) -> NpmRunParse {
    if tokens
        .first()
        .is_none_or(|token| strip_quotes(&token.text) != "npm")
    {
        return NpmRunParse::NotNpmRun;
    }

    let mut index = 1;
    let mut qualified = false;

    loop {
        let Some(token) = tokens.get(index) else {
            return NpmRunParse::NotNpmRun;
        };
        let text = strip_quotes(&token.text);
        if text == "run" || text == "run-script" {
            index += 1;
            break;
        }
        match consume_pre_script_flag(tokens, index, text) {
            FlagConsume::Qualifier { next_index } => {
                qualified = true;
                index = next_index;
            }
            FlagConsume::Advance { next_index } => index = next_index,
            FlagConsume::NotFlag => return NpmRunParse::NotNpmRun,
        }
    }

    loop {
        let Some(token) = tokens.get(index) else {
            return NpmRunParse::Skip;
        };
        let text = strip_quotes(&token.text);
        match consume_pre_script_flag(tokens, index, text) {
            FlagConsume::Qualifier { next_index } => {
                qualified = true;
                index = next_index;
                continue;
            }
            FlagConsume::Advance { next_index } => {
                index = next_index;
                continue;
            }
            FlagConsume::NotFlag => {}
        }
        if qualified {
            return NpmRunParse::Skip;
        }
        if !is_script_name(text) {
            return NpmRunParse::Skip;
        }
        return NpmRunParse::Missing {
            script: text.to_string(),
            range: token.source_range.clone(),
        };
    }
}

enum FlagConsume {
    Qualifier { next_index: usize },
    Advance { next_index: usize },
    NotFlag,
}

/// Consume a qualifier, `--name=value`, value-taking, or no-value flag before
/// the script token. Returns [`FlagConsume::NotFlag`] when `text` is not a flag.
fn consume_pre_script_flag(tokens: &[ShellToken], index: usize, text: &str) -> FlagConsume {
    if is_qualifier(text) {
        let mut next_index = index + 1;
        if qualifier_consumes_value(text)
            && tokens
                .get(next_index)
                .is_some_and(|next| !is_flag_token(strip_quotes(&next.text)))
        {
            next_index += 1;
        }
        return FlagConsume::Qualifier { next_index };
    }
    if is_self_contained_flag(text) {
        return FlagConsume::Advance {
            next_index: index + 1,
        };
    }
    if !is_flag_token(text) {
        return FlagConsume::NotFlag;
    }
    if is_value_taking_flag(text) {
        let mut next_index = index + 1;
        if tokens
            .get(next_index)
            .is_some_and(|next| !is_flag_token(strip_quotes(&next.text)))
        {
            next_index += 1;
        }
        return FlagConsume::Advance { next_index };
    }
    FlagConsume::Advance {
        next_index: index + 1,
    }
}

fn is_qualifier(text: &str) -> bool {
    let name = flag_name(text);
    QUALIFIER_FLAGS.contains(&name)
}

fn is_self_contained_flag(text: &str) -> bool {
    is_flag_token(text) && text.contains('=')
}

fn is_value_taking_flag(text: &str) -> bool {
    VALUE_TAKING_FLAGS.contains(&flag_name(text))
}

fn qualifier_consumes_value(flag: &str) -> bool {
    matches!(flag_name(flag), "--workspace" | "-w" | "--prefix")
}

fn is_flag_token(text: &str) -> bool {
    text.starts_with('-') && text != "-" && text != "--"
}

fn flag_name(text: &str) -> &str {
    text.split_once('=').map_or(text, |(name, _)| name)
}

fn strip_quotes(text: &str) -> &str {
    if text.len() >= 2 {
        let bytes = text.as_bytes();
        if (bytes[0] == b'\'' && bytes[text.len() - 1] == b'\'')
            || (bytes[0] == b'"' && bytes[text.len() - 1] == b'"')
        {
            return &text[1..text.len() - 1];
        }
    }
    text
}

fn is_script_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | ':' | '.'))
}

/// Test helper: collect missing script names from Markdown against a scripts map.
#[cfg(test)]
pub(crate) fn missing_scripts_for(
    content: &str,
    scripts: &serde_json::Map<String, serde_json::Value>,
) -> Vec<(String, Range<usize>, CommandSurface)> {
    let document = MarkdownDocument::parse(content);
    let mut findings: BTreeMap<String, (Range<usize>, CommandSurface)> = BTreeMap::new();
    for fragment in command_fragments(&document) {
        let Some(commands) = tokenize_shell_commands(content, &fragment) else {
            continue;
        };
        for tokens in commands {
            if let Some(finding) = find_missing_npm_script(&tokens, scripts) {
                findings
                    .entry(finding.script.clone())
                    .and_modify(|existing| {
                        if finding.source_range.start < existing.0.start {
                            *existing = (finding.source_range.clone(), fragment.surface);
                        }
                    })
                    .or_insert((finding.source_range.clone(), fragment.surface));
            }
        }
    }
    let mut ordered: Vec<_> = findings
        .into_iter()
        .map(|(script, (range, surface))| (script, range, surface))
        .collect();
    ordered.sort_by_key(|item| item.1.start);
    ordered
}

#[cfg(test)]
fn scripts_map(keys: &[&str]) -> serde_json::Map<String, serde_json::Value> {
    keys.iter()
        .map(|key| ((*key).to_string(), serde_json::json!("echo")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LintConfig;
    use crate::test_helpers::CwdGuard;
    use std::fs;

    #[test]
    fn npm_grammar_covers_flags_qualifiers_and_names() {
        let present = scripts_map(&["build", "build:css", "test.unit"]);
        assert!(missing_scripts_for("`npm run build`\n", &present).is_empty());
        assert!(missing_scripts_for("`npm run build:css`\n", &present).is_empty());
        assert!(missing_scripts_for("`npm run test.unit`\n", &present).is_empty());

        let missing = scripts_map(&["test"]);
        let found = missing_scripts_for("`npm run-script build`\n", &missing);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "build");

        assert!(missing_scripts_for("`npm run`\n", &missing).is_empty());
        assert_eq!(
            missing_scripts_for("`npm --silent run build`\n", &missing).len(),
            1
        );
        assert_eq!(
            missing_scripts_for("`npm run --silent build`\n", &missing).len(),
            1
        );
        assert_eq!(
            missing_scripts_for("`npm run --silent build target`\n", &missing).len(),
            1
        );
        assert_eq!(
            missing_scripts_for("`npm run --silent build -- target`\n", &missing).len(),
            1
        );
        assert_eq!(
            missing_scripts_for("`npm run-script --silent build target`\n", &missing).len(),
            1
        );
        assert_eq!(
            missing_scripts_for("`npm run build -- --flag`\n", &missing).len(),
            1
        );
        assert_eq!(
            missing_scripts_for("`npm run --loglevel silent build`\n", &missing).len(),
            1
        );
        assert_eq!(
            missing_scripts_for("`npm --registry https://example.com run build`\n", &missing).len(),
            1
        );
        assert_eq!(
            missing_scripts_for("`npm run --userconfig ./rc build`\n", &missing).len(),
            1
        );
        assert_eq!(
            missing_scripts_for("`npm --userconfig ./rc run build`\n", &missing).len(),
            1
        );
        assert_eq!(
            missing_scripts_for("`npm run --cache /tmp/npm build`\n", &missing).len(),
            1
        );
        assert_eq!(
            missing_scripts_for("`npm run --otp 123456 build`\n", &missing).len(),
            1
        );
        assert_eq!(
            missing_scripts_for("`npm run --script-shell /bin/bash build`\n", &missing).len(),
            1
        );
        assert_eq!(
            missing_scripts_for("`npm run --silent --production build extra`\n", &missing).len(),
            1
        );
        assert!(
            missing_scripts_for("`npm run --silent build`\n", &present).is_empty(),
            "present script with no-value flag stays clean"
        );
        assert!(
            missing_scripts_for("`npm run --silent build target`\n", &present).is_empty(),
            "present script with trailing args stays clean"
        );

        for command in [
            "`npm --workspace pkg run build`\n",
            "`npm -w pkg run build`\n",
            "`npm --workspaces run build`\n",
            "`npm --prefix ./packages/app run build`\n",
            "`npm --global run build`\n",
            "`npm -g run build`\n",
            "`npm --workspace=pkg run build`\n",
            "`npm --workspace pkg run build --silent arg`\n",
        ] {
            assert!(
                missing_scripts_for(command, &missing).is_empty(),
                "qualified must stay clean: {command}"
            );
        }

        assert!(missing_scripts_for("Do not run npm run build\n", &missing).is_empty());
        assert_eq!(
            missing_scripts_for("Never invent. Run npm run build\n", &missing)
                .iter()
                .map(|item| item.0.as_str())
                .collect::<Vec<_>>(),
            ["build"]
        );
        let quoted = missing_scripts_for("`npm run \"build\" target`\n", &missing);
        assert_eq!(quoted.len(), 1);
        assert_eq!(quoted[0].0, "build");
        assert_eq!(
            missing_scripts_for("`npm run --loglevel silent --otp 99 build`\n", &missing).len(),
            1
        );
    }

    #[test]
    #[serial_test::serial]
    fn package_states_and_dedupe_use_first_span() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        fs::write(
            "CLAUDE.md",
            "```bash\nnpm run missing-fenced\n```\n\nAlso `npm run missing-fenced` and `npm run other`.\n",
        )
        .unwrap();
        fs::write(
            "package.json",
            r#"{"name":"demo","scripts":{"test":"echo hi"}}"#,
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_npm_scripts(&mut diag, &ExcludeSet::default());
        let missing: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::NpmScriptMissing)
            .collect();
        assert_eq!(missing.len(), 2, "{missing:?}");
        assert_eq!(missing[0].evidence.as_deref(), Some("missing-fenced"));
        assert_eq!(missing[0].suggestion.as_deref(), Some(L006_SUGGESTION));
        assert!(missing[0].location.is_some());
        assert_eq!(missing[1].evidence.as_deref(), Some("other"));

        fs::remove_file("package.json").unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_npm_scripts(&mut diag, &ExcludeSet::default());
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::NpmScriptMissing)
        );

        fs::write("package.json", "{not json").unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_npm_scripts(&mut diag, &ExcludeSet::default());
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::NpmScriptMissing)
        );

        fs::write("package.json", r#"{"name":"demo","scripts":["oops"]}"#).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_npm_scripts(&mut diag, &ExcludeSet::default());
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::NpmScriptMissing)
        );
    }

    #[test]
    #[serial_test::serial]
    fn l006_flags_npm_run_script_missing_from_package_json() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::write(
            "package.json",
            "{\"name\":\"demo\",\"scripts\":{\"test\":\"echo hi\"}}",
        )
        .unwrap();
        fs::write("CLAUDE.md", "Run `npm run build` to compile.\n").unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_npm_scripts(&mut diag, &ExcludeSet::default());
        let missing: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|item| item.rule == LintRule::NpmScriptMissing)
            .collect();
        assert_eq!(missing.len(), 1);
        assert!(missing[0].message.contains("build"));
    }

    #[test]
    #[serial_test::serial]
    fn l006_accepts_colon_namespaced_script_names() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::write(
            "package.json",
            "{\"name\":\"demo\",\"scripts\":{\"build:css\":\"postcss\"}}",
        )
        .unwrap();
        fs::write("CLAUDE.md", "Run `npm run build:css` to compile styles.\n").unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_npm_scripts(&mut diag, &ExcludeSet::default());
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::NpmScriptMissing)
        );
    }

    #[test]
    #[serial_test::serial]
    fn l006_silent_when_no_package_json_or_no_scripts() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::write("CLAUDE.md", "Run `npm run build` to compile.\n").unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_npm_scripts(&mut diag, &ExcludeSet::default());
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::NpmScriptMissing)
        );

        fs::write("package.json", "{\"name\":\"demo\"}").unwrap();
        let mut diag2 = DiagnosticCollector::new_all_enabled();
        validate_npm_scripts(&mut diag2, &ExcludeSet::default());
        assert!(
            !diag2
                .diagnostics()
                .iter()
                .any(|item| item.rule == LintRule::NpmScriptMissing)
        );
    }

    #[test]
    #[serial_test::serial]
    fn l006_honors_exclusion_and_suppression() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        fs::write(
            "package.json",
            r#"{"name":"demo","scripts":{"test":"echo"}}"#,
        )
        .unwrap();
        fs::write("CLAUDE.md", "```bash\nnpm run missing\n```\n").unwrap();
        fs::write("NOTES.md", "```bash\nnpm run missing\n```\n").unwrap();

        let config = LintConfig {
            instruction_files: vec!["CLAUDE.md".into(), "NOTES.md".into()],
            ..LintConfig::default()
        };
        let exclude = ExcludeSet::new(&["NOTES.md".into()]).unwrap();
        let mut diag = DiagnosticCollector::with_config(config);
        validate_npm_scripts(&mut diag, &exclude);
        assert_eq!(
            diag.diagnostics()
                .iter()
                .filter(|item| item.rule == LintRule::NpmScriptMissing)
                .count(),
            1
        );

        let config = LintConfig {
            instruction_files: vec!["CLAUDE.md".into()],
            suppress: [LintRule::NpmScriptMissing].into_iter().collect(),
            ..LintConfig::default()
        };
        let mut diag = DiagnosticCollector::with_config(config);
        validate_npm_scripts(&mut diag, &ExcludeSet::default());
        assert!(diag.diagnostics().is_empty());
    }
}
