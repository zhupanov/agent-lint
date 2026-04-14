/// Central rule registry for claude-lint.
///
/// Every lint diagnostic has a unique code (e.g., "M001") and human-readable
/// name (e.g., "plugin-json-missing"). Rules are grouped by category prefix.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LintRule {
    // ── Manifest (M) ──────────────────────────────────────────────
    /// M001: .claude-plugin/plugin.json is missing
    PluginJsonMissing,
    /// M002: .claude-plugin/plugin.json is not valid JSON
    PluginJsonInvalid,
    /// M003: plugin.json missing required field (name or version)
    PluginFieldMissing,
    /// M004: plugin.json version is not strict semver
    PluginVersionFormat,
    /// M005: .claude-plugin/marketplace.json is missing
    MarketplaceJsonMissing,
    /// M006: .claude-plugin/marketplace.json is not valid JSON
    MarketplaceJsonInvalid,
    /// M007: marketplace.json missing required field (name or owner.name)
    MarketplaceFieldMissing,
    /// M008: marketplace.json plugins array is empty
    MarketplacePluginsEmpty,
    /// M009: marketplace.json plugin entry has invalid name or source
    MarketplacePluginInvalid,
    /// M010: marketplace.json enriched metadata missing (owner.email or plugin category)
    MarketplaceEnrichedMissing,
    /// M011: plugin.json enriched metadata missing (description, author.email, or keywords)
    PluginEnrichedMissing,

    // ── Hooks (H) ─────────────────────────────────────────────────
    /// H001: hooks/hooks.json is missing
    HooksJsonMissing,
    /// H002: hooks/hooks.json is not valid JSON
    HooksJsonInvalid,
    /// H003: hooks.json missing top-level 'hooks' key
    HooksKeyMissing,
    /// H004: hook command script missing on disk
    HookCommandMissing,
    /// H005: hook command script not executable
    HookNotExecutable,
    /// H006: .claude/settings.json is not valid JSON
    SettingsJsonInvalid,
    /// H007: hooks.json hooks array is empty
    HooksArrayEmpty,

    // ── Skills (S) ────────────────────────────────────────────────
    /// S001: skills/ directory is missing
    SkillsDirMissing,
    /// S002: skills/{name}/ missing SKILL.md
    SkillMdMissing,
    /// S003: no plugin-exported skills found under skills/
    NoExportedSkills,
    /// S004: SKILL.md has malformed frontmatter
    FrontmatterMalformed,
    /// S005: SKILL.md missing required frontmatter field (name or description)
    FrontmatterFieldMissing,
    /// S006: SKILL.md frontmatter name does not match directory name
    FrontmatterNameMismatch,
    /// S007: SKILL.md optional frontmatter field is present but empty
    FrontmatterFieldEmpty,
    /// S008: shared markdown reference missing on disk
    SharedMdMissing,
    /// S009: skill name exceeds 64 characters
    NameTooLong,
    /// S010: skill name contains characters outside [a-z0-9-]
    NameInvalidChars,
    /// S011: skill name starts/ends with hyphen or has consecutive hyphens
    NameBadHyphens,
    /// S012: skill name contains reserved word (anthropic, claude)
    NameReservedWord,
    /// S013: skill name contains XML/HTML tags
    NameHasXml,
    /// S014: skill description exceeds 1024 characters
    DescTooLong,
    /// S015: skill description exceeds 250 characters (listing truncation)
    DescTruncated,
    /// S016: skill description uses first/second person
    DescUsesPerson,
    /// S017: skill description lacks trigger/usage context
    DescNoTrigger,
    /// S018: skill description contains XML/HTML tags
    DescHasXml,
    /// S019: SKILL.md body exceeds 500 lines
    BodyTooLong,
    /// S020: SKILL.md has no content after frontmatter
    BodyEmpty,
    /// S021: consecutive bash code blocks that could be combined
    ConsecutiveBash,
    /// S022: Windows-style backslash paths in skill content
    BackslashPath,
    /// S023: boolean frontmatter field is not true/false
    BoolFieldInvalid,
    /// S024: context field value is not fork
    ContextFieldInvalid,
    /// S025: effort field value is not low/medium/high/max
    EffortFieldInvalid,
    /// S026: shell field value is not bash/powershell
    ShellFieldInvalid,
    /// S027: skill is unreachable (disable-model-invocation: true and user-invocable: false)
    SkillUnreachable,
    /// S028: $ARGUMENTS used in body but argument-hint not set
    ArgsNoHint,
    /// S029: referenced shared .md file itself references other shared .md files
    NestedRefDeep,
    /// S030: files in skill scripts/ not referenced from SKILL.md
    OrphanedSkillFiles,
    /// S031: http:// URL in skill content (not https)
    NonHttpsUrl,
    /// S032: potential hardcoded API key/token/secret
    HardcodedSecret,
    /// S033: skill name uses vague/generic terms
    NameVague,
    /// S034: skill description under 20 characters
    DescTooShort,
    /// S035: compatibility field exceeds 500 characters
    CompatTooLong,
    /// S036: referenced .md file exceeds 100 lines with no headings
    RefNoToc,
    /// S037: SKILL.md body exceeds 300 lines with no file references
    BodyNoRefs,
    /// S038: body contains time-sensitive date/year patterns
    TimeSensitive,
    /// S039: metadata map value is not a string
    MetadataNotString,
    /// S040: allowed-tools lists an unrecognized tool name
    ToolsUnknown,
    /// S041: context: fork set but body has no task instructions
    ForkNoTask,
    /// S042: disable-model-invocation: true with empty/missing description
    DmiEmptyDesc,
    /// S043: Windows-style backslash paths in frontmatter fields
    FrontmatterBackslash,

    // ── Agents (A) ────────────────────────────────────────────────
    /// A001: agents/ directory is missing
    AgentsDirMissing,
    /// A002: agent .md has malformed frontmatter
    AgentFrontmatterMalformed,
    /// A003: agent .md missing required frontmatter field (name or description)
    AgentFieldMissing,
    /// A004: agents/ has no .md files
    NoAgentFiles,
    /// A005: reviewer-templates.md is missing
    TemplateFileMissing,
    /// A006: agent .md missing 'Derived from' marker
    TemplateMarkerMissing,
    /// A007: agent-template count mismatch
    TemplateCountMismatch,
    /// A008: agent description exceeds 1024 characters
    AgentDescLong,
    /// A009: agent description under 20 characters
    AgentDescShort,
    /// A010: agent name contains characters outside [a-z0-9-]
    AgentNameInvalid,

    // ── Hygiene / Scripts (G) ─────────────────────────────────────
    /// G001: SKILL.md uses $PWD/ or hardcoded path instead of ${CLAUDE_PLUGIN_ROOT}/
    PwdInSkill,
    /// G002: script reference missing on disk
    ScriptRefMissing,
    /// G003: script file not executable
    ScriptNotExecutable,
    /// G004: dead script with no structured invocation reference
    DeadScript,
    /// G005: SECURITY.md is missing from repo root
    SecurityMdMissing,
    /// G006: TODO/FIXME/HACK/XXX marker in published skill content
    TodoInSkill,
    /// G007: TODO/FIXME/HACK/XXX marker in agent .md body
    TodoInAgent,

    // ── Email (E) ─────────────────────────────────────────────────
    /// E001: email address is not a valid format
    InvalidEmailFormat,

    // ── User Config (U) ───────────────────────────────────────────
    /// U001: userConfig must be an object
    UserconfigNotObject,
    /// U002: userConfig entry missing or invalid description
    UserconfigDescMissing,
    /// U003: userConfig key has no corresponding env var reference in scripts/
    UserconfigEnvMissing,
    /// U004: userConfig sensitive field must be a boolean
    UserconfigSensitiveType,
    /// U005: userConfig entry missing or invalid title
    UserconfigTitleMissing,
    /// U006: userConfig entry missing or invalid type
    UserconfigTypeMissing,

    // ── Slack (K) ─────────────────────────────────────────────────
    /// K001: Slack fallback variable without corresponding CLAUDE_PLUGIN_OPTION_ reference
    SlackFallbackMismatch,

    // ── Docs (D) ──────────────────────────────────────────────────
    /// D001: docs reference in CLAUDE.md canonical sources not found on disk
    DocsRefMissing,
    /// D002: CLAUDE.md exceeds 500 lines
    ClaudemdTooLarge,
    /// D003: TODO/FIXME/HACK/XXX marker in CLAUDE.md
    TodoInDocs,
}

impl LintRule {
    /// The short code, e.g. `"M001"`.
    pub fn code(self) -> &'static str {
        match self {
            Self::PluginJsonMissing => "M001",
            Self::PluginJsonInvalid => "M002",
            Self::PluginFieldMissing => "M003",
            Self::PluginVersionFormat => "M004",
            Self::MarketplaceJsonMissing => "M005",
            Self::MarketplaceJsonInvalid => "M006",
            Self::MarketplaceFieldMissing => "M007",
            Self::MarketplacePluginsEmpty => "M008",
            Self::MarketplacePluginInvalid => "M009",
            Self::MarketplaceEnrichedMissing => "M010",
            Self::PluginEnrichedMissing => "M011",

            Self::HooksJsonMissing => "H001",
            Self::HooksJsonInvalid => "H002",
            Self::HooksKeyMissing => "H003",
            Self::HookCommandMissing => "H004",
            Self::HookNotExecutable => "H005",
            Self::SettingsJsonInvalid => "H006",
            Self::HooksArrayEmpty => "H007",

            Self::SkillsDirMissing => "S001",
            Self::SkillMdMissing => "S002",
            Self::NoExportedSkills => "S003",
            Self::FrontmatterMalformed => "S004",
            Self::FrontmatterFieldMissing => "S005",
            Self::FrontmatterNameMismatch => "S006",
            Self::FrontmatterFieldEmpty => "S007",
            Self::SharedMdMissing => "S008",
            Self::NameTooLong => "S009",
            Self::NameInvalidChars => "S010",
            Self::NameBadHyphens => "S011",
            Self::NameReservedWord => "S012",
            Self::NameHasXml => "S013",
            Self::DescTooLong => "S014",
            Self::DescTruncated => "S015",
            Self::DescUsesPerson => "S016",
            Self::DescNoTrigger => "S017",
            Self::DescHasXml => "S018",
            Self::BodyTooLong => "S019",
            Self::BodyEmpty => "S020",
            Self::ConsecutiveBash => "S021",
            Self::BackslashPath => "S022",
            Self::BoolFieldInvalid => "S023",
            Self::ContextFieldInvalid => "S024",
            Self::EffortFieldInvalid => "S025",
            Self::ShellFieldInvalid => "S026",
            Self::SkillUnreachable => "S027",
            Self::ArgsNoHint => "S028",
            Self::NestedRefDeep => "S029",
            Self::OrphanedSkillFiles => "S030",
            Self::NonHttpsUrl => "S031",
            Self::HardcodedSecret => "S032",
            Self::NameVague => "S033",
            Self::DescTooShort => "S034",
            Self::CompatTooLong => "S035",
            Self::RefNoToc => "S036",
            Self::BodyNoRefs => "S037",
            Self::TimeSensitive => "S038",
            Self::MetadataNotString => "S039",
            Self::ToolsUnknown => "S040",
            Self::ForkNoTask => "S041",
            Self::DmiEmptyDesc => "S042",
            Self::FrontmatterBackslash => "S043",

            Self::AgentsDirMissing => "A001",
            Self::AgentFrontmatterMalformed => "A002",
            Self::AgentFieldMissing => "A003",
            Self::NoAgentFiles => "A004",
            Self::TemplateFileMissing => "A005",
            Self::TemplateMarkerMissing => "A006",
            Self::TemplateCountMismatch => "A007",
            Self::AgentDescLong => "A008",
            Self::AgentDescShort => "A009",
            Self::AgentNameInvalid => "A010",

            Self::PwdInSkill => "G001",
            Self::ScriptRefMissing => "G002",
            Self::ScriptNotExecutable => "G003",
            Self::DeadScript => "G004",
            Self::SecurityMdMissing => "G005",
            Self::TodoInSkill => "G006",
            Self::TodoInAgent => "G007",

            Self::InvalidEmailFormat => "E001",

            Self::UserconfigNotObject => "U001",
            Self::UserconfigDescMissing => "U002",
            Self::UserconfigEnvMissing => "U003",
            Self::UserconfigSensitiveType => "U004",
            Self::UserconfigTitleMissing => "U005",
            Self::UserconfigTypeMissing => "U006",

            Self::SlackFallbackMismatch => "K001",

            Self::DocsRefMissing => "D001",
            Self::ClaudemdTooLarge => "D002",
            Self::TodoInDocs => "D003",
        }
    }

    /// The human-readable name, e.g. `"plugin-json-missing"`.
    pub fn name(self) -> &'static str {
        match self {
            Self::PluginJsonMissing => "plugin-json-missing",
            Self::PluginJsonInvalid => "plugin-json-invalid",
            Self::PluginFieldMissing => "plugin-field-missing",
            Self::PluginVersionFormat => "plugin-version-format",
            Self::MarketplaceJsonMissing => "marketplace-json-missing",
            Self::MarketplaceJsonInvalid => "marketplace-json-invalid",
            Self::MarketplaceFieldMissing => "marketplace-field-missing",
            Self::MarketplacePluginsEmpty => "marketplace-plugins-empty",
            Self::MarketplacePluginInvalid => "marketplace-plugin-invalid",
            Self::MarketplaceEnrichedMissing => "marketplace-enriched-missing",
            Self::PluginEnrichedMissing => "plugin-enriched-missing",

            Self::HooksJsonMissing => "hooks-json-missing",
            Self::HooksJsonInvalid => "hooks-json-invalid",
            Self::HooksKeyMissing => "hooks-key-missing",
            Self::HookCommandMissing => "hook-command-missing",
            Self::HookNotExecutable => "hook-not-executable",
            Self::SettingsJsonInvalid => "settings-json-invalid",
            Self::HooksArrayEmpty => "hooks-array-empty",

            Self::SkillsDirMissing => "skills-dir-missing",
            Self::SkillMdMissing => "skill-md-missing",
            Self::NoExportedSkills => "no-exported-skills",
            Self::FrontmatterMalformed => "frontmatter-malformed",
            Self::FrontmatterFieldMissing => "frontmatter-field-missing",
            Self::FrontmatterNameMismatch => "frontmatter-name-mismatch",
            Self::FrontmatterFieldEmpty => "frontmatter-field-empty",
            Self::SharedMdMissing => "shared-md-missing",
            Self::NameTooLong => "name-too-long",
            Self::NameInvalidChars => "name-invalid-chars",
            Self::NameBadHyphens => "name-bad-hyphens",
            Self::NameReservedWord => "name-reserved-word",
            Self::NameHasXml => "name-has-xml",
            Self::DescTooLong => "desc-too-long",
            Self::DescTruncated => "desc-truncated",
            Self::DescUsesPerson => "desc-uses-person",
            Self::DescNoTrigger => "desc-no-trigger",
            Self::DescHasXml => "desc-has-xml",
            Self::BodyTooLong => "body-too-long",
            Self::BodyEmpty => "body-empty",
            Self::ConsecutiveBash => "consecutive-bash",
            Self::BackslashPath => "backslash-path",
            Self::BoolFieldInvalid => "bool-field-invalid",
            Self::ContextFieldInvalid => "context-field-invalid",
            Self::EffortFieldInvalid => "effort-field-invalid",
            Self::ShellFieldInvalid => "shell-field-invalid",
            Self::SkillUnreachable => "skill-unreachable",
            Self::ArgsNoHint => "args-no-hint",
            Self::NestedRefDeep => "nested-ref-deep",
            Self::OrphanedSkillFiles => "orphaned-skill-files",
            Self::NonHttpsUrl => "non-https-url",
            Self::HardcodedSecret => "hardcoded-secret",
            Self::NameVague => "name-vague",
            Self::DescTooShort => "desc-too-short",
            Self::CompatTooLong => "compat-too-long",
            Self::RefNoToc => "ref-no-toc",
            Self::BodyNoRefs => "body-no-refs",
            Self::TimeSensitive => "time-sensitive",
            Self::MetadataNotString => "metadata-not-string",
            Self::ToolsUnknown => "tools-unknown",
            Self::ForkNoTask => "fork-no-task",
            Self::DmiEmptyDesc => "dmi-empty-desc",
            Self::FrontmatterBackslash => "frontmatter-backslash",

            Self::AgentsDirMissing => "agents-dir-missing",
            Self::AgentFrontmatterMalformed => "agent-frontmatter-malformed",
            Self::AgentFieldMissing => "agent-field-missing",
            Self::NoAgentFiles => "no-agent-files",
            Self::TemplateFileMissing => "template-file-missing",
            Self::TemplateMarkerMissing => "template-marker-missing",
            Self::TemplateCountMismatch => "template-count-mismatch",
            Self::AgentDescLong => "agent-desc-long",
            Self::AgentDescShort => "agent-desc-short",
            Self::AgentNameInvalid => "agent-name-invalid",

            Self::PwdInSkill => "pwd-in-skill",
            Self::ScriptRefMissing => "script-ref-missing",
            Self::ScriptNotExecutable => "script-not-executable",
            Self::DeadScript => "dead-script",
            Self::SecurityMdMissing => "security-md-missing",
            Self::TodoInSkill => "todo-in-skill",
            Self::TodoInAgent => "todo-in-agent",

            Self::InvalidEmailFormat => "invalid-email-format",

            Self::UserconfigNotObject => "userconfig-not-object",
            Self::UserconfigDescMissing => "userconfig-desc-missing",
            Self::UserconfigEnvMissing => "userconfig-env-missing",
            Self::UserconfigSensitiveType => "userconfig-sensitive-type",
            Self::UserconfigTitleMissing => "userconfig-title-missing",
            Self::UserconfigTypeMissing => "userconfig-type-missing",

            Self::SlackFallbackMismatch => "slack-fallback-mismatch",

            Self::DocsRefMissing => "docs-ref-missing",
            Self::ClaudemdTooLarge => "claudemd-too-large",
            Self::TodoInDocs => "todo-in-docs",
        }
    }

    /// Look up a rule by its code (e.g. `"M001"`) or human-readable name
    /// (e.g. `"plugin-json-missing"`).
    pub fn from_code_or_name(s: &str) -> Option<Self> {
        ALL_RULES
            .iter()
            .find(|r| r.code() == s || r.name() == s)
            .copied()
    }
}

/// Every variant of [`LintRule`], for iteration and exhaustiveness checks.
pub const ALL_RULES: &[LintRule] = &[
    LintRule::PluginJsonMissing,
    LintRule::PluginJsonInvalid,
    LintRule::PluginFieldMissing,
    LintRule::PluginVersionFormat,
    LintRule::MarketplaceJsonMissing,
    LintRule::MarketplaceJsonInvalid,
    LintRule::MarketplaceFieldMissing,
    LintRule::MarketplacePluginsEmpty,
    LintRule::MarketplacePluginInvalid,
    LintRule::MarketplaceEnrichedMissing,
    LintRule::PluginEnrichedMissing,
    LintRule::HooksJsonMissing,
    LintRule::HooksJsonInvalid,
    LintRule::HooksKeyMissing,
    LintRule::HookCommandMissing,
    LintRule::HookNotExecutable,
    LintRule::SettingsJsonInvalid,
    LintRule::HooksArrayEmpty,
    LintRule::SkillsDirMissing,
    LintRule::SkillMdMissing,
    LintRule::NoExportedSkills,
    LintRule::FrontmatterMalformed,
    LintRule::FrontmatterFieldMissing,
    LintRule::FrontmatterNameMismatch,
    LintRule::FrontmatterFieldEmpty,
    LintRule::SharedMdMissing,
    LintRule::NameTooLong,
    LintRule::NameInvalidChars,
    LintRule::NameBadHyphens,
    LintRule::NameReservedWord,
    LintRule::NameHasXml,
    LintRule::DescTooLong,
    LintRule::DescTruncated,
    LintRule::DescUsesPerson,
    LintRule::DescNoTrigger,
    LintRule::DescHasXml,
    LintRule::BodyTooLong,
    LintRule::BodyEmpty,
    LintRule::ConsecutiveBash,
    LintRule::BackslashPath,
    LintRule::BoolFieldInvalid,
    LintRule::ContextFieldInvalid,
    LintRule::EffortFieldInvalid,
    LintRule::ShellFieldInvalid,
    LintRule::SkillUnreachable,
    LintRule::ArgsNoHint,
    LintRule::NestedRefDeep,
    LintRule::OrphanedSkillFiles,
    LintRule::NonHttpsUrl,
    LintRule::HardcodedSecret,
    LintRule::NameVague,
    LintRule::DescTooShort,
    LintRule::CompatTooLong,
    LintRule::RefNoToc,
    LintRule::BodyNoRefs,
    LintRule::TimeSensitive,
    LintRule::MetadataNotString,
    LintRule::ToolsUnknown,
    LintRule::ForkNoTask,
    LintRule::DmiEmptyDesc,
    LintRule::FrontmatterBackslash,
    LintRule::AgentsDirMissing,
    LintRule::AgentFrontmatterMalformed,
    LintRule::AgentFieldMissing,
    LintRule::NoAgentFiles,
    LintRule::TemplateFileMissing,
    LintRule::TemplateMarkerMissing,
    LintRule::TemplateCountMismatch,
    LintRule::AgentDescLong,
    LintRule::AgentDescShort,
    LintRule::AgentNameInvalid,
    LintRule::PwdInSkill,
    LintRule::ScriptRefMissing,
    LintRule::ScriptNotExecutable,
    LintRule::DeadScript,
    LintRule::SecurityMdMissing,
    LintRule::TodoInSkill,
    LintRule::TodoInAgent,
    LintRule::InvalidEmailFormat,
    LintRule::UserconfigNotObject,
    LintRule::UserconfigDescMissing,
    LintRule::UserconfigEnvMissing,
    LintRule::UserconfigSensitiveType,
    LintRule::UserconfigTitleMissing,
    LintRule::UserconfigTypeMissing,
    LintRule::SlackFallbackMismatch,
    LintRule::DocsRefMissing,
    LintRule::ClaudemdTooLarge,
    LintRule::TodoInDocs,
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn all_rules_count_matches_enum() {
        // If a variant is added to LintRule but not to ALL_RULES, code()/name()
        // will still compile (match is exhaustive), but this test will catch it.
        assert_eq!(
            ALL_RULES.len(),
            89,
            "ALL_RULES length must match enum variant count"
        );
    }

    #[test]
    fn no_duplicate_codes() {
        let mut seen = HashSet::new();
        for rule in ALL_RULES {
            assert!(seen.insert(rule.code()), "Duplicate code: {}", rule.code());
        }
    }

    #[test]
    fn no_duplicate_names() {
        let mut seen = HashSet::new();
        for rule in ALL_RULES {
            assert!(seen.insert(rule.name()), "Duplicate name: {}", rule.name());
        }
    }

    #[test]
    fn names_are_max_three_words() {
        for rule in ALL_RULES {
            let word_count = rule.name().split('-').count();
            assert!(
                word_count <= 3,
                "Rule {} name '{}' has {} words (max 3)",
                rule.code(),
                rule.name(),
                word_count
            );
        }
    }

    #[test]
    fn from_code_or_name_lookup() {
        // By code
        assert_eq!(
            LintRule::from_code_or_name("M001"),
            Some(LintRule::PluginJsonMissing)
        );
        // By name
        assert_eq!(
            LintRule::from_code_or_name("plugin-json-missing"),
            Some(LintRule::PluginJsonMissing)
        );
        // Unknown
        assert_eq!(LintRule::from_code_or_name("X999"), None);
        assert_eq!(LintRule::from_code_or_name("nonexistent"), None);
    }

    #[test]
    fn every_rule_round_trips() {
        for rule in ALL_RULES {
            assert_eq!(LintRule::from_code_or_name(rule.code()), Some(*rule));
            assert_eq!(LintRule::from_code_or_name(rule.name()), Some(*rule));
        }
    }
}
