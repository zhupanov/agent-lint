use crate::rules::RETIRED_IDENTIFIERS;
use crate::rules::{ALL_RULES, LintRule};
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet, HashMap};

const CARGO_MANIFEST: &str = include_str!("../Cargo.toml");
const PACKAGE_MANIFEST: &str = include_str!("../package.json");
const README: &str = include_str!("../README.md");
const RULES_DOCUMENTATION: &str = include_str!("../docs/rules.md");
const RELEASE_EXAMPLE_DOCUMENTS: &[(&str, &str)] = &[
    ("README.md", README),
    (
        "docs/github-action.md",
        include_str!("../docs/github-action.md"),
    ),
    (
        "docs/development.md",
        include_str!("../docs/development.md"),
    ),
];

fn release_example_files() -> Result<Vec<(String, String)>, String> {
    // Documentation examples must track the package version and floating major.
    // CI workflow e2e pins intentionally stay on the latest *published* release
    // so they keep downloading during unreleased major bumps.
    let mut files: Vec<_> = RELEASE_EXAMPLE_DOCUMENTS
        .iter()
        .map(|(path, contents)| ((*path).to_owned(), (*contents).to_owned()))
        .collect();
    files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}

fn matching_manifest_version(cargo: &str, package: &str) -> Result<String, String> {
    let cargo: toml::Value =
        toml::from_str(cargo).map_err(|error| format!("invalid Cargo.toml: {error}"))?;
    let package: serde_json::Value =
        serde_json::from_str(package).map_err(|error| format!("invalid package.json: {error}"))?;

    let cargo_version = cargo
        .get("package")
        .and_then(|package| package.get("version"))
        .and_then(toml::Value::as_str)
        .ok_or_else(|| "Cargo.toml is missing package.version".to_owned())?;
    let package_version = package
        .get("version")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "package.json is missing version".to_owned())?;

    if cargo_version != package_version {
        return Err(format!(
            "Cargo.toml version {cargo_version} differs from package.json version {package_version}"
        ));
    }

    Ok(cargo_version.to_owned())
}

fn registry_prefix_counts() -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for rule in ALL_RULES {
        let prefix = rule
            .code()
            .trim_end_matches(|character: char| character.is_ascii_digit())
            .trim_end_matches('-');
        *counts.entry(prefix.to_owned()).or_default() += 1;
    }
    counts
}

fn readme_prefix_counts(readme: &str) -> Result<BTreeMap<String, usize>, String> {
    let mut in_table = false;
    let mut counts = BTreeMap::new();

    for line in readme.lines() {
        if line == "| Category | Prefix | Rules | Description |" {
            in_table = true;
            continue;
        }
        if !in_table || line.starts_with("|---") {
            continue;
        }
        if !line.starts_with('|') {
            break;
        }

        let cells: Vec<_> = line.trim_matches('|').split('|').map(str::trim).collect();
        if cells.len() != 4 {
            return Err(format!("invalid README rule-summary row: {line}"));
        }
        let count = cells[2]
            .parse::<usize>()
            .map_err(|_| format!("invalid README rule count in row: {line}"))?;
        if counts.insert(cells[1].to_owned(), count).is_some() {
            return Err(format!("README rule summary repeats prefix {}", cells[1]));
        }
    }

    if counts.is_empty() {
        return Err("README rule-summary table was not found".to_owned());
    }
    Ok(counts)
}

fn documented_rule_summary(document: &str, path: &str) -> Result<(usize, usize), String> {
    let summary = Regex::new(
        r"Agent Lint ships ([0-9]+) rules organized into ([0-9]+) code-prefix categories",
    )
    .expect("rule summary regex is valid");
    let captures = summary
        .captures(document)
        .ok_or_else(|| format!("{path} is missing the canonical rule summary"))?;
    let total = captures[1]
        .parse()
        .map_err(|_| format!("{path} has an invalid documented rule total"))?;
    let categories = captures[2]
        .parse()
        .map_err(|_| format!("{path} has an invalid documented category total"))?;
    Ok((total, categories))
}

fn validate_rule_summaries(readme: &str, rules_documentation: &str) -> Result<(), String> {
    let registry = registry_prefix_counts();
    let documented = readme_prefix_counts(readme)?;
    if documented != registry {
        return Err(format!(
            "README prefix counts {documented:?} differ from registry counts {registry:?}"
        ));
    }

    for (path, document) in [
        ("README.md", readme),
        ("docs/rules.md", rules_documentation),
    ] {
        let (total, categories) = documented_rule_summary(document, path)?;
        if total != ALL_RULES.len() || categories != registry.len() {
            return Err(format!(
                "{path} documents {total} rules in {categories} categories; registry has {} rules in {} categories",
                ALL_RULES.len(),
                registry.len()
            ));
        }
    }

    Ok(())
}

fn validate_autofix_documentation(rules_documentation: &str) -> Result<(), String> {
    let summary = Regex::new(r"Auto-fixable rules \(([0-9]+) of ([0-9]+)\)")
        .expect("autofix summary regex is valid");
    let summaries: Vec<_> = summary.captures_iter(rules_documentation).collect();
    if summaries.len() != 1 {
        return Err(format!(
            "docs/rules.md must contain exactly one autofix summary, found {}",
            summaries.len()
        ));
    }
    let captures = &summaries[0];
    let documented_fixable: usize = captures[1]
        .parse()
        .map_err(|_| "docs/rules.md has an invalid autofix count".to_owned())?;
    let documented_total: usize = captures[2]
        .parse()
        .map_err(|_| "docs/rules.md has an invalid autofix denominator".to_owned())?;
    let live_fixable: BTreeSet<_> = ALL_RULES
        .iter()
        .filter(|rule| rule.fix_kind().is_some())
        .map(|rule| rule.code())
        .collect();
    if documented_fixable != live_fixable.len() || documented_total != ALL_RULES.len() {
        return Err(format!(
            "docs/rules.md documents {documented_fixable} autofixable rules of {documented_total}; registry has {} of {}",
            live_fixable.len(),
            ALL_RULES.len()
        ));
    }

    if rules_documentation
        .lines()
        .filter(|line| *line == "## Auto-Fixable Rules")
        .count()
        != 1
    {
        return Err("docs/rules.md must contain exactly one Auto-Fixable Rules section".to_owned());
    }
    if rules_documentation
        .lines()
        .filter(|line| *line == "| Rule | Code | Fix |")
        .count()
        != 1
    {
        return Err("docs/rules.md must contain exactly one autofix table".to_owned());
    }

    let mut in_table = false;
    let mut documented_codes = BTreeSet::new();
    let mut documented_names = BTreeSet::new();
    for line in rules_documentation.lines() {
        if line == "| Rule | Code | Fix |" {
            in_table = true;
            continue;
        }
        if !in_table || line.starts_with("|---") {
            continue;
        }
        if !line.starts_with('|') {
            break;
        }
        let cells: Vec<_> = line.trim_matches('|').split('|').map(str::trim).collect();
        if cells.len() != 3 {
            return Err(format!("invalid autofix documentation row: {line}"));
        }
        if cells.iter().any(|cell| cell.is_empty()) {
            return Err(format!("autofix documentation has an empty cell: {line}"));
        }
        if !documented_codes.insert(cells[1]) {
            return Err(format!("duplicate autofix documentation for {}", cells[1]));
        }
        if !documented_names.insert(cells[0]) {
            return Err(format!("duplicate autofix documentation for {}", cells[0]));
        }
        let rule = LintRule::from_code_or_name(cells[1]).ok_or_else(|| {
            format!(
                "autofix documentation names non-live rule code {}",
                cells[1]
            )
        })?;
        if rule.name() != cells[0] {
            return Err(format!(
                "autofix documentation pairs {} with {}, but the canonical name is {}",
                cells[1],
                cells[0],
                rule.name()
            ));
        }
        if rule.fix_kind().is_none() {
            return Err(format!(
                "autofix documentation lists non-fixable rule {} ({})",
                rule.code(),
                rule.name()
            ));
        }
    }
    if documented_codes != live_fixable {
        return Err(format!(
            "documented autofix codes {documented_codes:?} differ from registry codes {live_fixable:?}"
        ));
    }
    Ok(())
}

#[derive(Debug)]
struct DocumentedRule {
    line_number: usize,
    name: String,
    description: String,
    mode: String,
    default: String,
}

fn rule_code_parts(code: &str) -> Option<(&str, u16)> {
    let digit_start = code
        .char_indices()
        .rev()
        .take_while(|(_, character)| character.is_ascii_digit())
        .last()
        .map(|(index, _)| index)?;
    let (prefix, suffix) = code.split_at(digit_start);
    if prefix.is_empty()
        || !prefix
            .chars()
            .all(|character| character.is_ascii_uppercase() || character == '-')
        || suffix.len() != 3
    {
        return None;
    }
    suffix.parse().ok().map(|number| (prefix, number))
}

fn documented_rule_rows(documentation: &str) -> Result<HashMap<String, DocumentedRule>, String> {
    let mut rows = HashMap::new();
    let mut in_table = false;
    for (index, line) in documentation.lines().enumerate() {
        if line == "| Code | Name | Description | Mode | Default |" {
            in_table = true;
            continue;
        }
        if !in_table || line.starts_with("|---") {
            continue;
        }
        if !line.starts_with('|') {
            in_table = false;
            continue;
        }
        let cells: Vec<_> = line.trim_matches('|').split('|').map(str::trim).collect();
        if cells.len() != 5 || cells.iter().any(|cell| cell.is_empty()) {
            return Err(format!(
                "docs/rules.md:{}: malformed rule row: {line}",
                index + 1
            ));
        }
        let (first, last) = cells[0]
            .split_once('–')
            .or_else(|| cells[0].split_once("--"))
            .map_or((cells[0], None), |(first, last)| (first, Some(last)));
        let (prefix, first_number) = rule_code_parts(first).ok_or_else(|| {
            format!(
                "docs/rules.md:{}: invalid rule code {}",
                index + 1,
                cells[0]
            )
        })?;
        let last_number = match last {
            Some(last) => {
                let (last_prefix, last_number) = rule_code_parts(last).ok_or_else(|| {
                    format!(
                        "docs/rules.md:{}: invalid rule code range {}",
                        index + 1,
                        cells[0]
                    )
                })?;
                if prefix != last_prefix || first_number > last_number {
                    return Err(format!(
                        "docs/rules.md:{}: invalid rule code range {}",
                        index + 1,
                        cells[0]
                    ));
                }
                last_number
            }
            None => first_number,
        };
        let name = cells[1]
            .strip_prefix('`')
            .and_then(|name| name.strip_suffix('`'))
            .unwrap_or(cells[1]);
        if first_number == last_number && !cells[1].starts_with('`') {
            return Err(format!(
                "docs/rules.md:{}: rule name must be backticked",
                index + 1
            ));
        }
        if first_number != last_number && name != "—" {
            return Err(format!(
                "docs/rules.md:{}: range row must use an em dash name",
                index + 1
            ));
        }
        for number in first_number..=last_number {
            let code = format!("{prefix}{number:03}");
            if rows
                .insert(
                    code.clone(),
                    DocumentedRule {
                        line_number: index + 1,
                        name: name.into(),
                        description: cells[2].into(),
                        mode: cells[3].into(),
                        default: cells[4].into(),
                    },
                )
                .is_some()
            {
                return Err(format!(
                    "docs/rules.md:{}: duplicate rule code {code}",
                    index + 1
                ));
            }
        }
    }
    Ok(rows)
}

fn validate_rule_documentation_rows(documentation: &str) -> Result<(), String> {
    let rows = documented_rule_rows(documentation)?;
    for rule in ALL_RULES {
        let row = rows
            .get(rule.code())
            .ok_or_else(|| format!("docs/rules.md is missing {}", rule.code()))?;
        if row.name != "—" && row.name != rule.name() {
            return Err(format!(
                "docs/rules.md:{}: {} has name {}, expected {}",
                row.line_number,
                rule.code(),
                row.name,
                rule.name()
            ));
        }
        if row.description.is_empty() {
            return Err(format!(
                "docs/rules.md:{}: {} has an empty description",
                row.line_number,
                rule.code()
            ));
        }
        if row.mode != rule.applicability().documentation_label() {
            return Err(format!(
                "docs/rules.md:{}: {} has mode {}, expected {}",
                row.line_number,
                rule.code(),
                row.mode,
                rule.applicability().documentation_label()
            ));
        }
        let expected = match rule.default_severity() {
            crate::rules::DefaultSeverity::Error => "error",
            crate::rules::DefaultSeverity::Warning => "warn",
            crate::rules::DefaultSeverity::Suppressed => "suppressed",
        };
        if !matches!(row.default.as_str(), "error" | "warn" | "suppressed")
            || row.default != expected
        {
            return Err(format!(
                "docs/rules.md:{}: {} has default {}, expected {}",
                row.line_number,
                rule.code(),
                row.default,
                expected
            ));
        }
    }
    for (code, row) in rows {
        if LintRule::from_code_or_name(&code).is_none() {
            return Err(format!(
                "docs/rules.md:{}: non-live rule code {code}",
                row.line_number
            ));
        }
    }
    Ok(())
}

fn validate_no_retired_rule_references(documents: &[(String, String)]) -> Result<(), String> {
    fn contains_identifier(document: &str, identifier: &str) -> bool {
        let bytes = document.as_bytes();
        document.match_indices(identifier).any(|(start, matched)| {
            let end = start + matched.len();
            let is_identifier_byte = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'-';
            let starts_at_boundary = start == 0 || !is_identifier_byte(bytes[start - 1]);
            let ends_at_boundary = end == bytes.len() || !is_identifier_byte(bytes[end]);
            starts_at_boundary && ends_at_boundary
        })
    }

    for (path, document) in documents {
        for identifier in RETIRED_IDENTIFIERS {
            if contains_identifier(document, identifier) {
                return Err(format!(
                    "{path} references retired rule identifier {identifier}"
                ));
            }
        }
    }
    Ok(())
}

fn current_documentation_files() -> Vec<(String, String)> {
    fn visit(
        directory: &std::path::Path,
        root: &std::path::Path,
        files: &mut Vec<(String, String)>,
    ) {
        for entry in std::fs::read_dir(directory).expect("documentation directory is readable") {
            let entry = entry.expect("documentation entry is readable");
            let path = entry.path();
            if path.is_dir() {
                visit(&path, root, files);
            } else if path.extension().is_some_and(|extension| extension == "md") {
                let relative = path
                    .strip_prefix(root)
                    .expect("documentation path is beneath repository root")
                    .to_string_lossy()
                    .into_owned();
                if relative != "CHANGELOG.md" {
                    let contents = std::fs::read_to_string(&path).unwrap_or_else(|error| {
                        panic!("cannot read documentation {relative}: {error}")
                    });
                    files.push((relative, contents));
                }
            }
        }
    }

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for entry in std::fs::read_dir(root).expect("repository root is readable") {
        let entry = entry.expect("repository root entry is readable");
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|extension| extension == "md") {
            let name = path
                .file_name()
                .expect("root documentation has a name")
                .to_string_lossy()
                .into_owned();
            if name != "CHANGELOG.md" {
                files.push((
                    name,
                    std::fs::read_to_string(&path).expect("root documentation is readable"),
                ));
            }
        }
    }
    visit(&root.join("docs"), root, &mut files);
    let proposal = root.join("PROPOSED_AGNIX_CHANGES.txt");
    files.push((
        "PROPOSED_AGNIX_CHANGES.txt".to_owned(),
        std::fs::read_to_string(proposal).expect("proposal document is readable"),
    ));
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

fn validate_release_examples(
    expected_version: &str,
    files: &[(String, String)],
) -> Result<(), String> {
    let expected_major = expected_version
        .split('.')
        .next()
        .ok_or_else(|| format!("invalid package version {expected_version}"))?;
    let pinned_version =
        Regex::new(r#"(?:^|[\s`])(?:version:\s*"|rev:\s*v)([0-9]+\.[0-9]+\.[0-9]+)"#)
            .expect("pinned version regex is valid");
    let action_major =
        Regex::new(r"zhupanov/agent-lint@v([0-9]+)").expect("action major-version regex is valid");
    let mut examples = 0;

    for (path, contents) in files {
        for capture in pinned_version.captures_iter(contents) {
            examples += 1;
            if &capture[1] != expected_version {
                return Err(format!(
                    "{path} pins agent-lint {} instead of {expected_version}",
                    &capture[1]
                ));
            }
        }
        for capture in action_major.captures_iter(contents) {
            examples += 1;
            if &capture[1] != expected_major {
                return Err(format!(
                    "{path} uses agent-lint@v{} instead of @v{expected_major}",
                    &capture[1]
                ));
            }
        }
    }

    if examples == 0 {
        return Err("no release examples were checked".to_owned());
    }
    Ok(())
}

#[test]
fn package_and_crate_versions_match() {
    matching_manifest_version(CARGO_MANIFEST, PACKAGE_MANIFEST)
        .expect("Cargo.toml and package.json versions must agree");
}

#[test]
fn package_version_mismatch_fixture_is_rejected() {
    let current_version = matching_manifest_version(CARGO_MANIFEST, PACKAGE_MANIFEST)
        .expect("fixture starts with matching versions");
    let mismatched_package = PACKAGE_MANIFEST.replacen(
        &format!(r#""version": "{current_version}""#),
        &format!(r#""version": "{current_version}-mismatch""#),
        1,
    );
    assert!(matching_manifest_version(CARGO_MANIFEST, &mismatched_package).is_err());
}

#[test]
fn documented_rule_summaries_match_registry() {
    validate_rule_summaries(README, RULES_DOCUMENTATION)
        .expect("public rule summaries must be derived from the registry");
    validate_rule_documentation_rows(RULES_DOCUMENTATION)
        .expect("rule table must match the live registry");
    validate_autofix_documentation(RULES_DOCUMENTATION)
        .expect("autofix documentation must match live rule metadata");
    validate_no_retired_rule_references(&current_documentation_files())
        .expect("current rule documentation must not reference retired identities");
}

#[test]
fn rule_documentation_rejects_default_and_mode_drift() {
    let default_drift = RULES_DOCUMENTATION.replacen("| Plugin | error |", "| Plugin | warn |", 1);
    let mode_drift = RULES_DOCUMENTATION.replacen("| Plugin | error |", "| Always | error |", 1);
    let default_error = validate_rule_documentation_rows(&default_drift).unwrap_err();
    let mode_error = validate_rule_documentation_rows(&mode_drift).unwrap_err();
    assert!(default_error.contains("M001") && default_error.contains("default"));
    assert!(mode_error.contains("M001") && mode_error.contains("mode"));
}

#[test]
fn autofix_documentation_rejects_independent_table_drift() {
    let cases = [
        (
            "code",
            RULES_DOCUMENTATION.replace(
                "| hook-not-executable | H005 |",
                "| hook-not-executable | H006 |",
            ),
        ),
        (
            "missing row",
            RULES_DOCUMENTATION.replace(
                "| hook-not-executable | H005 | `chmod +x` on a directly invoked script |\n",
                "",
            ),
        ),
        (
            "extra non-fixable row",
            RULES_DOCUMENTATION.replace(
                "| hook-not-executable | H005 | `chmod +x` on a directly invoked script |",
                "| hook-not-executable | H005 | `chmod +x` on a directly invoked script |\n| plugin-json-missing | M001 | invalid |",
            ),
        ),
        (
            "duplicate row",
            RULES_DOCUMENTATION.replace(
                "| hook-not-executable | H005 | `chmod +x` on a directly invoked script |",
                "| hook-not-executable | H005 | `chmod +x` on a directly invoked script |\n| hook-not-executable | H005 | duplicate |",
            ),
        ),
        (
            "numerator",
            RULES_DOCUMENTATION.replace("(10 of 294)", "(9 of 294)"),
        ),
        (
            "denominator",
            RULES_DOCUMENTATION.replace("(10 of 294)", "(10 of 293)"),
        ),
    ];
    for (name, documentation) in cases {
        assert!(
            validate_autofix_documentation(&documentation).is_err(),
            "mutation {name} must be rejected"
        );
    }
}

#[test]
fn wrong_documented_prefix_count_fixture_is_rejected() {
    let skill_count = registry_prefix_counts()["S"];
    let wrong_count = README.replacen(
        &format!("| Skills | S | {skill_count} |"),
        &format!("| Skills | S | {} |", skill_count + 1),
        1,
    );
    let error = validate_rule_summaries(&wrong_count, RULES_DOCUMENTATION)
        .expect_err("a stale documented prefix count must fail validation");
    assert!(error.contains("prefix counts"), "unexpected error: {error}");
}

#[test]
fn release_examples_match_package_version_policy() {
    let version = matching_manifest_version(CARGO_MANIFEST, PACKAGE_MANIFEST)
        .expect("manifest versions must agree before examples are checked");
    let files = release_example_files().expect("release example files must be readable");
    validate_release_examples(&version, &files)
        .expect("release examples must match the package version and floating major");
}

#[test]
fn stale_release_example_fixture_is_rejected() {
    let version = matching_manifest_version(CARGO_MANIFEST, PACKAGE_MANIFEST)
        .expect("fixture starts with matching versions");
    let fixture = vec![(
        "fixture.md".to_owned(),
        "- uses: zhupanov/agent-lint@v999\n  with:\n    version: \"999.0.0\"\n".to_owned(),
    )];
    assert!(validate_release_examples(&version, &fixture).is_err());
}
