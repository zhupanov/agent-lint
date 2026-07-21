use crate::rules::ALL_RULES;
use regex::Regex;
use std::collections::BTreeMap;

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
