//! Internal YAML adapter.
//!
//! Parser configuration and parser-specific error handling stay here so
//! validators depend on Agent Lint's YAML boundary rather than `noyalib`.

pub(crate) use noyalib::{Mapping, Value};

use noyalib::{DuplicateKeyPolicy, Error, ParserConfig, from_str_with_config};

fn parser_config() -> ParserConfig {
    // serde_yaml rejected duplicate mapping keys. Preserve that safety
    // guarantee explicitly instead of inheriting noyalib's YAML 1.2 default
    // of keeping the last occurrence.
    ParserConfig::new().duplicate_key_policy(DuplicateKeyPolicy::Error)
}

/// Parse one YAML document using Agent Lint's compatibility policy.
pub(crate) fn parse(source: &str) -> Result<Value, Error> {
    from_str_with_config(source, &parser_config())
}

/// Return the parser-reported 1-based source line when it is available.
pub(crate) fn error_line(error: &Error) -> Option<usize> {
    error.location().map(|location| location.line())
}

/// Return the parser-reported 1-based source column when it is available.
pub(crate) fn error_column(error: &Error) -> Option<usize> {
    error.location().map(|location| location.column())
}

#[cfg(test)]
mod tests {
    use super::*;
    use noyalib::ErrorKind;

    fn mapping(source: &str) -> Mapping {
        match parse(source).unwrap() {
            Value::Mapping(mapping) => mapping,
            value => panic!("expected mapping, got {value:?}"),
        }
    }

    #[test]
    fn yaml_12_scalars_and_collections_preserve_expected_types() {
        let value = mapping(
            "boolean: true\nlegacy_boolean: yes\nnull_word: null\nnull_tilde: ~\ninteger: 42\nfloat: 1.5\noctal: 0o644\nlegacy_octal: 0644\nquoted: 'yes'\nitems: [one, two]\n",
        );

        assert_eq!(value.get("boolean").and_then(Value::as_bool), Some(true));
        assert_eq!(
            value.get("legacy_boolean").and_then(Value::as_str),
            Some("yes")
        );
        assert!(value.get("null_word").is_some_and(Value::is_null));
        assert!(value.get("null_tilde").is_some_and(Value::is_null));
        assert_eq!(value.get("integer").and_then(Value::as_i64), Some(42));
        assert_eq!(value.get("float").and_then(Value::as_f64), Some(1.5));
        assert_eq!(value.get("octal").and_then(Value::as_i64), Some(420));
        assert_eq!(value.get("legacy_octal").and_then(Value::as_i64), Some(644));
        assert_eq!(value.get("quoted").and_then(Value::as_str), Some("yes"));
        assert_eq!(
            value
                .get("items")
                .and_then(Value::as_sequence)
                .map(|items| items.len()),
            Some(2)
        );
    }

    #[test]
    fn aliases_and_merge_keys_are_resolved() {
        let value = mapping("base: &base\n  name: example\ncopy:\n  <<: *base\n");
        let copy = value.get("copy").and_then(Value::as_mapping).unwrap();

        assert_eq!(copy.get("name").and_then(Value::as_str), Some("example"));
    }

    #[test]
    fn tags_are_preserved_for_callers_to_validate_or_reject_by_shape() {
        let value = mapping("value: !custom example\n");
        assert!(matches!(value.get("value"), Some(Value::Tagged(_))));
    }

    #[test]
    fn duplicate_keys_are_rejected() {
        let error = parse("name: first\nname: second\n").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::DuplicateKey);
    }

    #[test]
    fn syntax_errors_retain_their_source_line() {
        let error = parse("name: valid\n\tinvalid: indentation\n").unwrap_err();
        assert_eq!(error_line(&error), Some(2));
    }
}
