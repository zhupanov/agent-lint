use crate::diagnostic::DiagnosticCollector;
use regex::Regex;
use std::fs;
use std::path::Path;

/// V22: Docs file references from CLAUDE.md.
/// Every docs/*.md path referenced in the "Canonical sources" section of
/// CLAUDE.md must exist on disk.
///
/// NOTE: The bash script used `awk '/^## Canonical sources/,/^## [^C]/'`
/// which is buggy — it wouldn't stop at sections starting with 'C' (like
/// "## Contributing"). We fix this by stopping at any `## ` heading.
pub fn validate_docs_references(diag: &mut DiagnosticCollector) {
    let claude_md = Path::new("CLAUDE.md");
    if !claude_md.is_file() {
        return;
    }

    let content = match fs::read_to_string(claude_md) {
        Ok(c) => c,
        Err(_) => return,
    };

    // Extract the "Canonical sources" section: start at "## Canonical sources",
    // stop at the next "## " heading (any heading, not just non-C).
    let section = extract_canonical_sources_section(&content);

    let re = Regex::new(r"docs/[a-zA-Z0-9._-]+\.md").unwrap();
    let mut seen = std::collections::HashSet::new();

    for cap in re.find_iter(&section) {
        let doc_path = cap.as_str();
        if seen.insert(doc_path.to_string()) && !Path::new(doc_path).is_file() {
            diag.fail(&format!(
                "docs reference in CLAUDE.md canonical sources not found on disk: {doc_path}"
            ));
        }
    }
}

fn extract_canonical_sources_section(content: &str) -> String {
    let mut in_section = false;
    let mut result = Vec::new();

    for line in content.lines() {
        if line.starts_with("## Canonical sources") {
            in_section = true;
            result.push(line);
            continue;
        }
        if in_section {
            // Stop at any ## heading
            if line.starts_with("## ") {
                break;
            }
            result.push(line);
        }
    }

    result.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_section_stops_at_any_heading() {
        let content = "\
## Canonical sources\n\
- docs/foo.md\n\
- docs/bar.md\n\
## Contributing\n\
This should not be included\n\
";
        let section = extract_canonical_sources_section(content);
        assert!(section.contains("docs/foo.md"));
        assert!(section.contains("docs/bar.md"));
        assert!(!section.contains("Contributing"));
    }

    #[test]
    fn test_extract_section_stops_at_c_heading() {
        // The bash version had a bug here — it would NOT stop at ## Configuration
        let content = "\
## Canonical sources\n\
- docs/foo.md\n\
## Configuration\n\
Should not be here\n\
";
        let section = extract_canonical_sources_section(content);
        assert!(section.contains("docs/foo.md"));
        assert!(!section.contains("Configuration"));
    }
}
