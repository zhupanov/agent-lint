/// Hard-retired identifiers: `LintRule::from_code_or_name` returns `None`
/// for every entry. A new hard retirement appends its code and name here
/// and nowhere else; every tombstone test iterates this constant.
pub const RETIRED_IDENTIFIERS: &[&str] = &[
    "S012",
    "S013",
    "name-reserved-word",
    "name-has-xml",
    "K001",
    "slack-fallback-mismatch",
    "U003",
    "userconfig-env-missing",
    "I005",
    "instruction-file-structure",
    "CX044",
    "codex-agents-structure",
];

/// Soft-retired identifiers: each resolves through
/// `LintRule::from_code_or_name` but its rule never fires and is absent
/// from `ACTIVE_RULES`. A new soft retirement appends its code and name
/// here and nowhere else.
pub const SOFT_RETIRED_IDENTIFIERS: &[&str] = &[
    "S042",
    "dmi-empty-desc",
    "S045",
    "tools-list-syntax",
    "S049",
    "name-not-gerund",
    "O005",
    "style-name-long",
];
