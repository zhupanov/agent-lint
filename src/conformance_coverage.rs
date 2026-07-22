//! Registry-derived conformance coverage floor (test-only module).

#[cfg(test)]
mod tests {
    use crate::rules::ACTIVE_RULES;
    use std::collections::BTreeSet;
    use std::path::Path;

    /// Codes with no conformance manifest coverage yet. Reason-bearing and
    /// shrink-only: when a code gains coverage, its row must be deleted.
    /// Seeded from this test's failure output at adoption; do not add rows.
    const CONFORMANCE_COVERAGE_BASELINE: &[(&str, &str)] = &[
        ("M001", "pre-floor gap, tracked by this issue's baseline"),
        ("M004", "pre-floor gap, tracked by this issue's baseline"),
        ("M005", "pre-floor gap, tracked by this issue's baseline"),
        ("M007", "pre-floor gap, tracked by this issue's baseline"),
        ("M008", "pre-floor gap, tracked by this issue's baseline"),
        ("M009", "pre-floor gap, tracked by this issue's baseline"),
        ("M012", "pre-floor gap, tracked by this issue's baseline"),
        ("M013", "pre-floor gap, tracked by this issue's baseline"),
        ("M014", "pre-floor gap, tracked by this issue's baseline"),
        ("M015", "pre-floor gap, tracked by this issue's baseline"),
        ("M016", "pre-floor gap, tracked by this issue's baseline"),
        ("M017", "pre-floor gap, tracked by this issue's baseline"),
        ("M018", "pre-floor gap, tracked by this issue's baseline"),
        ("M019", "pre-floor gap, tracked by this issue's baseline"),
        ("M020", "pre-floor gap, tracked by this issue's baseline"),
        ("M022", "pre-floor gap, tracked by this issue's baseline"),
        ("H003", "pre-floor gap, tracked by this issue's baseline"),
        ("H006", "pre-floor gap, tracked by this issue's baseline"),
        ("H008", "pre-floor gap, tracked by this issue's baseline"),
        ("H009", "pre-floor gap, tracked by this issue's baseline"),
        ("H010", "pre-floor gap, tracked by this issue's baseline"),
        ("H011", "pre-floor gap, tracked by this issue's baseline"),
        ("H012", "pre-floor gap, tracked by this issue's baseline"),
        ("H013", "pre-floor gap, tracked by this issue's baseline"),
        ("H014", "pre-floor gap, tracked by this issue's baseline"),
        ("H015", "pre-floor gap, tracked by this issue's baseline"),
        ("H016", "pre-floor gap, tracked by this issue's baseline"),
        ("H017", "pre-floor gap, tracked by this issue's baseline"),
        ("H018", "pre-floor gap, tracked by this issue's baseline"),
        ("H019", "pre-floor gap, tracked by this issue's baseline"),
        ("H020", "pre-floor gap, tracked by this issue's baseline"),
        ("H021", "pre-floor gap, tracked by this issue's baseline"),
        ("H022", "pre-floor gap, tracked by this issue's baseline"),
        ("H023", "pre-floor gap, tracked by this issue's baseline"),
        ("H024", "pre-floor gap, tracked by this issue's baseline"),
        ("H025", "pre-floor gap, tracked by this issue's baseline"),
        ("X002", "pre-floor gap, tracked by this issue's baseline"),
        ("X003", "pre-floor gap, tracked by this issue's baseline"),
        ("X004", "pre-floor gap, tracked by this issue's baseline"),
        ("X005", "pre-floor gap, tracked by this issue's baseline"),
        ("S001", "pre-floor gap, tracked by this issue's baseline"),
        ("S002", "pre-floor gap, tracked by this issue's baseline"),
        ("S003", "pre-floor gap, tracked by this issue's baseline"),
        ("S004", "pre-floor gap, tracked by this issue's baseline"),
        ("S005", "pre-floor gap, tracked by this issue's baseline"),
        ("S006", "pre-floor gap, tracked by this issue's baseline"),
        ("S007", "pre-floor gap, tracked by this issue's baseline"),
        ("S008", "pre-floor gap, tracked by this issue's baseline"),
        ("S009", "pre-floor gap, tracked by this issue's baseline"),
        ("S010", "pre-floor gap, tracked by this issue's baseline"),
        ("S011", "pre-floor gap, tracked by this issue's baseline"),
        ("S017", "pre-floor gap, tracked by this issue's baseline"),
        ("S019", "pre-floor gap, tracked by this issue's baseline"),
        ("S020", "pre-floor gap, tracked by this issue's baseline"),
        ("S021", "pre-floor gap, tracked by this issue's baseline"),
        ("S022", "pre-floor gap, tracked by this issue's baseline"),
        ("S024", "pre-floor gap, tracked by this issue's baseline"),
        ("S025", "pre-floor gap, tracked by this issue's baseline"),
        ("S026", "pre-floor gap, tracked by this issue's baseline"),
        ("S027", "pre-floor gap, tracked by this issue's baseline"),
        ("S028", "pre-floor gap, tracked by this issue's baseline"),
        ("S029", "pre-floor gap, tracked by this issue's baseline"),
        ("S033", "pre-floor gap, tracked by this issue's baseline"),
        ("S034", "pre-floor gap, tracked by this issue's baseline"),
        ("S035", "pre-floor gap, tracked by this issue's baseline"),
        ("S037", "pre-floor gap, tracked by this issue's baseline"),
        ("S038", "pre-floor gap, tracked by this issue's baseline"),
        ("S039", "pre-floor gap, tracked by this issue's baseline"),
        ("S041", "pre-floor gap, tracked by this issue's baseline"),
        ("S043", "pre-floor gap, tracked by this issue's baseline"),
        ("S044", "pre-floor gap, tracked by this issue's baseline"),
        ("S046", "pre-floor gap, tracked by this issue's baseline"),
        ("S047", "pre-floor gap, tracked by this issue's baseline"),
        ("S050", "pre-floor gap, tracked by this issue's baseline"),
        ("S051", "pre-floor gap, tracked by this issue's baseline"),
        ("S052", "pre-floor gap, tracked by this issue's baseline"),
        ("S053", "pre-floor gap, tracked by this issue's baseline"),
        ("S054", "pre-floor gap, tracked by this issue's baseline"),
        ("S056", "pre-floor gap, tracked by this issue's baseline"),
        ("S057", "pre-floor gap, tracked by this issue's baseline"),
        ("S062", "pre-floor gap, tracked by this issue's baseline"),
        ("S064", "pre-floor gap, tracked by this issue's baseline"),
        ("S066", "pre-floor gap, tracked by this issue's baseline"),
        ("S069", "pre-floor gap, tracked by this issue's baseline"),
        ("A002", "pre-floor gap, tracked by this issue's baseline"),
        ("A005", "pre-floor gap, tracked by this issue's baseline"),
        ("A008", "pre-floor gap, tracked by this issue's baseline"),
        ("A010", "pre-floor gap, tracked by this issue's baseline"),
        ("A012", "pre-floor gap, tracked by this issue's baseline"),
        ("A013", "pre-floor gap, tracked by this issue's baseline"),
        ("Q001", "pre-floor gap, tracked by this issue's baseline"),
        ("Q003", "pre-floor gap, tracked by this issue's baseline"),
        ("Q004", "pre-floor gap, tracked by this issue's baseline"),
        ("O002", "pre-floor gap, tracked by this issue's baseline"),
        ("O004", "pre-floor gap, tracked by this issue's baseline"),
        ("CX045", "pre-floor gap, tracked by this issue's baseline"),
        ("CX046", "pre-floor gap, tracked by this issue's baseline"),
        ("CX058", "pre-floor gap, tracked by this issue's baseline"),
        (
            "CR-SK-001",
            "pre-floor gap, tracked by this issue's baseline",
        ),
        ("G001", "pre-floor gap, tracked by this issue's baseline"),
        ("G002", "pre-floor gap, tracked by this issue's baseline"),
        ("G003", "pre-floor gap, tracked by this issue's baseline"),
        ("G004", "pre-floor gap, tracked by this issue's baseline"),
        ("E001", "pre-floor gap, tracked by this issue's baseline"),
        ("E002", "pre-floor gap, tracked by this issue's baseline"),
        ("U001", "pre-floor gap, tracked by this issue's baseline"),
        ("D004", "pre-floor gap, tracked by this issue's baseline"),
        ("D005", "pre-floor gap, tracked by this issue's baseline"),
        ("L001", "pre-floor gap, tracked by this issue's baseline"),
        ("L002", "pre-floor gap, tracked by this issue's baseline"),
        ("L003", "pre-floor gap, tracked by this issue's baseline"),
        ("L004", "pre-floor gap, tracked by this issue's baseline"),
        ("L005", "pre-floor gap, tracked by this issue's baseline"),
        ("L006", "pre-floor gap, tracked by this issue's baseline"),
    ];

    fn covered_codes() -> BTreeSet<String> {
        let manifests =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/conformance/manifests");
        let mut covered = BTreeSet::new();
        for entry in std::fs::read_dir(&manifests).expect("manifest dir") {
            let path = entry.expect("manifest entry").path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("manifest read");
            let json: serde_json::Value = serde_json::from_str(&text).expect("manifest json");
            for diagnostic in json["expected_diagnostics"]
                .as_array()
                .into_iter()
                .flatten()
            {
                if let Some(code) = diagnostic["code"].as_str() {
                    covered.insert(code.to_owned());
                }
            }
        }
        covered
    }

    #[test]
    fn registry_codes_have_conformance_coverage() {
        let covered = covered_codes();
        let baseline: BTreeSet<&str> = CONFORMANCE_COVERAGE_BASELINE
            .iter()
            .map(|(code, _)| *code)
            .collect();
        let mut missing = Vec::new();
        let mut stale_baseline = Vec::new();
        for rule in &*ACTIVE_RULES {
            let code = rule.code();
            if code == "X999" {
                continue; // Documented internal sentinel.
            }
            match (covered.contains(code), baseline.contains(code)) {
                (false, false) => missing.push(code),
                (true, true) => stale_baseline.push(code),
                _ => {}
            }
        }
        assert!(
            missing.is_empty(),
            "codes with no conformance coverage and no baseline reason: {missing:?}"
        );
        assert!(
            stale_baseline.is_empty(),
            "covered codes must drop their baseline row: {stale_baseline:?}"
        );
    }
}
