//! Central rule registry for agent-lint.
//!
//! Every lint diagnostic has a unique code (e.g., "M001") and human-readable
//! name (e.g., "plugin-json-missing"). Rules are grouped by category prefix.

use strum::{EnumProperty as StrumEnumProperty, VariantArray as StrumVariantArray};
use strum_macros::{EnumIter, EnumProperty, VariantArray};

/// Compiled-in default severity for a rule. Used as fallback when the user's
/// config does not mention the rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultSeverity {
    /// Rule fires as an error by default.
    Error,
    /// Rule fires as a warning by default (reported but non-blocking).
    Warning,
    /// Rule is silently skipped by default (not reported, not counted).
    Suppressed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumIter, EnumProperty, VariantArray)]
pub enum LintRule {
    // ── Manifest (M) ──────────────────────────────────────────────
    /// M001: .claude-plugin/plugin.json is missing
    #[strum(props(code = "M001", name = "plugin-json-missing"))]
    PluginJsonMissing,
    /// M002: .claude-plugin/plugin.json is not valid JSON
    #[strum(props(code = "M002", name = "plugin-json-invalid"))]
    PluginJsonInvalid,
    /// M003: plugin.json missing required name field
    #[strum(props(code = "M003", name = "plugin-field-missing"))]
    PluginFieldMissing,
    /// M004: plugin.json version is not strict semver
    #[strum(props(code = "M004", name = "plugin-version-format"))]
    PluginVersionFormat,
    /// M005: .claude-plugin/marketplace.json is missing
    #[strum(props(code = "M005", name = "marketplace-json-missing"))]
    MarketplaceJsonMissing,
    /// M006: .claude-plugin/marketplace.json is not valid JSON
    #[strum(props(code = "M006", name = "marketplace-json-invalid"))]
    MarketplaceJsonInvalid,
    /// M007: marketplace.json missing required field (name or owner.name)
    #[strum(props(code = "M007", name = "marketplace-field-missing"))]
    MarketplaceFieldMissing,
    /// M008: marketplace.json plugins array is empty
    #[strum(props(code = "M008", name = "marketplace-plugins-empty"))]
    MarketplacePluginsEmpty,
    /// M009: marketplace plugin entry has an invalid source or effective local root
    #[strum(props(code = "M009", name = "marketplace-plugin-invalid"))]
    MarketplacePluginInvalid,
    /// M010: marketplace.json enriched metadata missing (owner.email or plugin category)
    #[strum(props(code = "M010", name = "marketplace-enriched-missing"))]
    MarketplaceEnrichedMissing,
    /// M011: plugin.json enriched metadata missing (description, author.email, or keywords)
    #[strum(props(code = "M011", name = "plugin-enriched-missing"))]
    PluginEnrichedMissing,
    /// M012: plugin or marketplace component path points inside .claude-plugin/
    #[strum(props(code = "M012", name = "component-path-nested"))]
    ComponentPathNested,
    /// M013: plugin or marketplace component path is absolute, traverses, or lacks ./
    #[strum(props(code = "M013", name = "component-path-unsafe"))]
    ComponentPathUnsafe,
    /// M014: plugin.json author object present but author.name missing/invalid
    #[strum(props(code = "M014", name = "author-name-missing"))]
    AuthorNameMissing,
    /// M015: plugin.json homepage is not a valid http(s) URL
    #[strum(props(code = "M015", name = "homepage-url-invalid"))]
    HomepageUrlInvalid,
    /// M016: plugin.json lspServers entry missing command or extensionToLanguage
    #[strum(props(code = "M016", name = "lsp-server-invalid"))]
    LspServerInvalid,
    /// M017: plugin.json channels entry does not reference a server
    #[strum(props(code = "M017", name = "channel-server-missing"))]
    ChannelServerMissing,
    /// M018: plugin.json omits its optional version field
    #[strum(props(code = "M018", name = "plugin-version-missing"))]
    PluginVersionMissing,
    /// M019: marketplace.json relative string source lacks `./` without pluginRoot
    #[strum(props(code = "M019", name = "marketplace-bare-path"))]
    MarketplaceBarePath,
    /// M020: plugin.json author is present but not an object
    #[strum(props(code = "M020", name = "author-type-invalid"))]
    AuthorTypeInvalid,
    /// M021: marketplace.json marketplace or plugin name is not kebab-case
    #[strum(props(code = "M021", name = "marketplace-name-format"))]
    MarketplaceNameFormat,

    // ── Hooks (H) ─────────────────────────────────────────────────
    /// H001: a declared plugin hook configuration file is missing
    #[strum(props(code = "H001", name = "hooks-json-missing"))]
    HooksJsonMissing,
    /// H002: a discovered plugin hook configuration is not valid JSON
    #[strum(props(code = "H002", name = "hooks-json-invalid"))]
    HooksJsonInvalid,
    /// H003: file-backed hook config missing a top-level 'hooks' key
    #[strum(props(code = "H003", name = "hooks-key-missing"))]
    HooksKeyMissing,
    /// H004: hook command script missing on disk
    #[strum(props(code = "H004", name = "hook-command-missing"))]
    HookCommandMissing,
    /// H005: hook command script not executable
    #[strum(props(code = "H005", name = "hook-not-executable"))]
    HookNotExecutable,
    /// H006: .claude/settings.json is not valid JSON
    #[strum(props(code = "H006", name = "settings-json-invalid"))]
    SettingsJsonInvalid,
    /// H007: syntactically valid plugin hook config has no handler entries
    #[strum(props(code = "H007", name = "hooks-array-empty"))]
    HooksArrayEmpty,
    /// H008: hook event name is not a recognized Claude Code event
    #[strum(props(code = "H008", name = "hook-event-invalid"))]
    HookEventInvalid,
    /// H009: matcher is non-string or present on an event that takes no matcher
    #[strum(props(code = "H009", name = "hook-matcher-invalid"))]
    HookMatcherInvalid,
    /// H010: hook object missing required 'type' field
    #[strum(props(code = "H010", name = "hook-type-missing"))]
    HookTypeMissing,
    /// H011: hook 'type' is not a recognized handler type
    #[strum(props(code = "H011", name = "hook-type-unknown"))]
    HookTypeUnknown,
    /// H012: type: command hook missing 'command'
    #[strum(props(code = "H012", name = "hook-command-required"))]
    HookCommandRequired,
    /// H013: type: prompt or type: agent hook missing 'prompt'
    #[strum(props(code = "H013", name = "hook-prompt-required"))]
    HookPromptRequired,
    /// H014: type: http hook missing 'url'
    #[strum(props(code = "H014", name = "hook-url-required"))]
    HookUrlRequired,
    /// H015: type: mcp_tool hook missing 'server'
    #[strum(props(code = "H015", name = "hook-server-required"))]
    HookServerRequired,
    /// H016: type: mcp_tool hook missing 'tool'
    #[strum(props(code = "H016", name = "hook-tool-required"))]
    HookToolRequired,
    /// H017: hook 'timeout' is not a positive integer
    #[strum(props(code = "H017", name = "hook-timeout-invalid"))]
    HookTimeoutInvalid,
    /// H018: 'async: true' on a non-command hook
    #[strum(props(code = "H018", name = "hook-async-invalid"))]
    HookAsyncInvalid,
    /// H019: 'model' on a hook other than prompt/agent
    #[strum(props(code = "H019", name = "hook-model-invalid"))]
    HookModelInvalid,
    /// H020: hook 'once' is not a boolean
    #[strum(props(code = "H020", name = "hook-once-invalid"))]
    HookOnceInvalid,
    /// H021: hook 'if' is invalid or used outside a tool event
    #[strum(props(code = "H021", name = "hook-if-invalid"))]
    HookIfInvalid,
    /// H022: hook 'shell' value is not bash/powershell
    #[strum(props(code = "H022", name = "hook-shell-invalid"))]
    HookShellInvalid,
    /// H023: dangerous command pattern in hook command
    #[strum(props(code = "H023", name = "hook-command-dangerous"))]
    HookCommandDangerous,
    /// H024: http hook headers interpolate $VAR without allowedEnvVars
    #[strum(props(code = "H024", name = "hook-headers-interpolated"))]
    HookHeadersInterpolated,
    /// H025: .claude/settings.local.json is not valid JSON
    #[strum(props(code = "H025", name = "settings-local-invalid"))]
    SettingsLocalInvalid,
    /// H026: hook configuration does not match the documented nesting
    #[strum(props(code = "H026", name = "hook-config-malformed"))]
    HookConfigMalformed,

    // ── Markdown structure (X) ────────────────────────────────────
    /// X001: skill/agent frontmatter is not valid YAML
    #[strum(props(code = "X001", name = "frontmatter-yaml-invalid"))]
    FrontmatterYamlInvalid,
    /// X002: unclosed code fence in a linted markdown file
    #[strum(props(code = "X002", name = "unclosed-code-fence"))]
    UnclosedCodeFence,
    /// X003: unclosed XML tag in markdown body
    #[strum(props(code = "X003", name = "xml-tag-unclosed"))]
    XmlTagUnclosed,
    /// X004: mismatched closing XML tag in markdown body
    #[strum(props(code = "X004", name = "xml-tag-mismatched"))]
    XmlTagMismatched,
    /// X005: closing XML tag with no opening tag
    #[strum(props(code = "X005", name = "xml-tag-orphan"))]
    XmlTagOrphan,

    // ── Skills (S) ────────────────────────────────────────────────
    /// S001: skills/ directory is missing (deprecated — no longer fires;
    /// retained so existing config identifiers keep parsing)
    #[strum(props(code = "S001", name = "skills-dir-missing"))]
    SkillsDirMissing,
    /// S002: skills/{name}/ missing SKILL.md
    #[strum(props(code = "S002", name = "skill-md-missing"))]
    SkillMdMissing,
    /// S003: no plugin-exported skills found under skills/
    #[strum(props(code = "S003", name = "no-exported-skills"))]
    NoExportedSkills,
    /// S004: SKILL.md has malformed frontmatter
    #[strum(props(code = "S004", name = "frontmatter-malformed"))]
    FrontmatterMalformed,
    /// S005: SKILL.md required frontmatter field is missing or not a non-empty string
    #[strum(props(code = "S005", name = "frontmatter-field-missing"))]
    FrontmatterFieldMissing,
    /// S006: SKILL.md frontmatter name does not match directory name
    #[strum(props(code = "S006", name = "frontmatter-name-mismatch"))]
    FrontmatterNameMismatch,
    /// S007: SKILL.md optional frontmatter field is present but empty
    #[strum(props(code = "S007", name = "frontmatter-field-empty"))]
    FrontmatterFieldEmpty,
    /// S008: shared markdown reference missing on disk
    #[strum(props(code = "S008", name = "shared-md-missing"))]
    SharedMdMissing,
    /// S009: skill name exceeds 64 characters
    #[strum(props(code = "S009", name = "name-too-long"))]
    NameTooLong,
    /// S010: skill name contains characters outside [a-z0-9-]
    #[strum(props(code = "S010", name = "name-invalid-chars"))]
    NameInvalidChars,
    /// S011: skill name starts/ends with hyphen or has consecutive hyphens
    #[strum(props(code = "S011", name = "name-bad-hyphens"))]
    NameBadHyphens,
    /// S014: skill description exceeds 1024 characters
    #[strum(props(code = "S014", name = "desc-too-long"))]
    DescTooLong,
    /// S015: combined skill listing entry exceeds its configured cap
    #[strum(props(code = "S015", name = "desc-truncated"))]
    DescTruncated,
    /// S016: skill description uses first/second person
    #[strum(props(code = "S016", name = "desc-uses-person"))]
    DescUsesPerson,
    /// S017: skill description lacks trigger/usage context
    #[strum(props(code = "S017", name = "desc-no-trigger"))]
    DescNoTrigger,
    /// S018: skill description contains XML/HTML tags
    #[strum(props(code = "S018", name = "desc-has-xml"))]
    DescHasXml,
    /// S019: SKILL.md body exceeds 500 lines
    #[strum(props(code = "S019", name = "body-too-long"))]
    BodyTooLong,
    /// S020: SKILL.md has no content after frontmatter
    #[strum(props(code = "S020", name = "body-empty"))]
    BodyEmpty,
    /// S021: consecutive bash code blocks that could be combined
    #[strum(props(code = "S021", name = "consecutive-bash"))]
    ConsecutiveBash,
    /// S022: Windows-style backslash paths in skill content
    #[strum(props(code = "S022", name = "backslash-path"))]
    BackslashPath,
    /// S023: boolean frontmatter field is not true/false
    #[strum(props(code = "S023", name = "bool-field-invalid"))]
    BoolFieldInvalid,
    /// S024: context field value is not fork
    #[strum(props(code = "S024", name = "context-field-invalid"))]
    ContextFieldInvalid,
    /// S025: effort field value is not low/medium/high/xhigh/max
    #[strum(props(code = "S025", name = "effort-field-invalid"))]
    EffortFieldInvalid,
    /// S026: shell field value is not bash/powershell
    #[strum(props(code = "S026", name = "shell-field-invalid"))]
    ShellFieldInvalid,
    /// S027: skill is unreachable (disable-model-invocation: true and user-invocable: false)
    #[strum(props(code = "S027", name = "skill-unreachable"))]
    SkillUnreachable,
    /// S028: $ARGUMENTS used in body but argument-hint not set
    #[strum(props(code = "S028", name = "args-no-hint"))]
    ArgsNoHint,
    /// S029: referenced shared .md file itself references other shared .md files
    #[strum(props(code = "S029", name = "nested-ref-deep"))]
    NestedRefDeep,
    /// S030: files in skill scripts/ not referenced from any skill-local .md
    #[strum(props(code = "S030", name = "orphaned-skill-files"))]
    OrphanedSkillFiles,
    /// S031: http:// URL in skill content (not https)
    #[strum(props(code = "S031", name = "non-https-url"))]
    NonHttpsUrl,
    /// S032: potential hardcoded API key/token/secret
    #[strum(props(code = "S032", name = "hardcoded-secret"))]
    HardcodedSecret,
    /// S033: published plugin skill name is an exact domainless label
    #[strum(props(code = "S033", name = "name-vague"))]
    NameVague,
    /// S034: skill description under 20 characters
    #[strum(props(code = "S034", name = "desc-too-short"))]
    DescTooShort,
    /// S035: compatibility field exceeds 500 characters
    #[strum(props(code = "S035", name = "compat-too-long"))]
    CompatTooLong,
    /// S036: referenced .md file exceeds 100 lines with no ATX headings
    #[strum(props(code = "S036", name = "ref-no-toc"))]
    RefNoToc,
    /// S037: SKILL.md body exceeds 300 lines with no file references
    #[strum(props(code = "S037", name = "body-no-refs"))]
    BodyNoRefs,
    /// S038: body contains time-sensitive date/year patterns
    #[strum(props(code = "S038", name = "time-sensitive"))]
    TimeSensitive,
    /// S039: metadata map value is not a string
    #[strum(props(code = "S039", name = "metadata-not-string"))]
    MetadataNotString,
    /// S040: allowed-tools lists an unrecognized tool name
    #[strum(props(code = "S040", name = "tools-unknown"))]
    ToolsUnknown,
    /// S041: context: fork set but body has no task instructions
    #[strum(props(code = "S041", name = "fork-no-task"))]
    ForkNoTask,
    /// S042: disable-model-invocation: true with empty/missing description
    #[strum(props(code = "S042", name = "dmi-empty-desc"))]
    DmiEmptyDesc,
    /// S043: Windows-style backslash paths in frontmatter fields
    #[strum(props(code = "S043", name = "frontmatter-backslash"))]
    FrontmatterBackslash,
    /// S044: MCP tool reference without server prefix
    #[strum(props(code = "S044", name = "mcp-tool-unqualified"))]
    McpToolUnqualified,
    /// S045: allowed-tools uses YAML list syntax instead of comma-separated scalar
    #[strum(props(code = "S045", name = "tools-list-syntax"))]
    ToolsListSyntax,
    /// S046: Long skill body lacks workflow structure
    #[strum(props(code = "S046", name = "body-no-workflow"))]
    BodyNoWorkflow,
    /// S047: Long skill body lacks examples or templates
    #[strum(props(code = "S047", name = "body-no-examples"))]
    BodyNoExamples,
    /// S048: non-descriptive reference file name in skill directory
    #[strum(props(code = "S048", name = "ref-name-generic"))]
    RefNameGeneric,
    /// S049: skill name not in gerund form (deprecated — no longer fires;
    /// retained so existing config identifiers keep parsing)
    #[strum(props(code = "S049", name = "name-not-gerund"))]
    NameNotGerund,
    /// S050: skill description content is too vague/generic
    #[strum(props(code = "S050", name = "desc-vague-content"))]
    DescVagueContent,
    /// S051: script-backed skill lacks dependency/package notes
    #[strum(props(code = "S051", name = "script-deps-missing"))]
    ScriptDepsMissing,
    /// S052: script-backed skill lacks verification step
    #[strum(props(code = "S052", name = "script-verify-missing"))]
    ScriptVerifyMissing,
    /// S053: terminology inconsistency — 3+ synonym variants used
    #[strum(props(code = "S053", name = "terminology-inconsistent"))]
    TerminologyInconsistent,
    /// S054: skill description keywords not reflected in body
    #[strum(props(code = "S054", name = "desc-body-misalign"))]
    DescBodyMisalign,
    /// S055: script file lacks error handling patterns
    #[strum(props(code = "S055", name = "script-errhand-missing"))]
    ScriptErrhandMissing,
    /// S056: body lists alternatives without stating a default
    #[strum(props(code = "S056", name = "body-no-default"))]
    BodyNoDefault,
    /// S057: undocumented magic number in code block
    #[strum(props(code = "S057", name = "magic-number-undoc"))]
    MagicNumberUndoc,
    /// S058: Skill tool allowed without a clear invocation step
    #[strum(props(code = "S058", name = "skill-invoke-missing"))]
    SkillInvokeMissing,
    /// S059: prompt invocation flag is not accepted by its shipped script
    #[strum(props(code = "S059", name = "skill-flag-mismatch"))]
    SkillFlagMismatch,
    /// S060: awk positional field appears in a skill shell fence
    #[strum(props(code = "S060", name = "awk-field-ref"))]
    AwkFieldRef,
    /// S061: grep-family probe in a skill shell fence is unbounded
    #[strum(props(code = "S061", name = "unsafe-grep-probe"))]
    UnsafeGrepProbe,
    /// S062: always-loaded skill prompt closure exceeds configured budget
    #[strum(props(code = "S062", name = "skill-closure-large"))]
    SkillClosureLarge,
    /// S063: model field value is not a recognized alias or model ID
    #[strum(props(code = "S063", name = "model-invalid"))]
    ModelInvalid,
    /// S064: agent field present without context: fork
    #[strum(props(code = "S064", name = "agent-no-fork"))]
    AgentNoFork,
    /// S065: agent value is not a built-in or existing custom agent
    #[strum(props(code = "S065", name = "agent-unknown"))]
    AgentUnknown,
    /// S066: side-effect-named skill without disable-model-invocation: true
    #[strum(props(code = "S066", name = "side-effect-auto"))]
    SideEffectAuto,
    /// S067: allowed-tools lists unscoped Bash (suggest Bash(…)-style scoping)
    #[strum(props(code = "S067", name = "bash-unscoped"))]
    BashUnscoped,
    /// S068: more than 3 dynamic context injections in skill body
    #[strum(props(code = "S068", name = "injection-overflow"))]
    InjectionOverflow,
    /// S069: argument-hint set but body never references $ARGUMENTS
    #[strum(props(code = "S069", name = "hint-no-args"))]
    HintNoArgs,
    /// S070: unknown skill frontmatter field
    #[strum(props(code = "S070", name = "unknown-fm-field"))]
    UnknownFmField,
    /// S071: paths field present but empty
    #[strum(props(code = "S071", name = "paths-empty"))]
    PathsEmpty,
    /// S072: skill directory exceeds 8MB (platform upload limit)
    #[strum(props(code = "S072", name = "skill-dir-oversized"))]
    SkillDirOversized,
    /// S073: skill-relative `.md` link nested deeper than one directory level
    #[strum(props(code = "S073", name = "skill-ref-nested"))]
    SkillRefNested,
    /// S074: skill routing descriptions overlap within a shared namespace
    #[strum(props(code = "S074", name = "skill-desc-overlap"))]
    SkillDescOverlap,

    // ── Agents (A) ────────────────────────────────────────────────
    /// A001: a manifest-declared plugin agent path is missing. The implicit
    /// default `agents/` directory is optional, so its absence is never reported;
    /// only an explicit plugin.json `agents` path that does not exist fires.
    #[strum(props(code = "A001", name = "agents-dir-missing"))]
    AgentsDirMissing,
    /// A002: agent .md has malformed frontmatter
    #[strum(props(code = "A002", name = "agent-frontmatter-malformed"))]
    AgentFrontmatterMalformed,
    /// A003: agent .md missing required frontmatter field (name or description)
    #[strum(props(code = "A003", name = "agent-field-missing"))]
    AgentFieldMissing,
    /// A004: a present plugin agent root (default `agents/` or a manifest-declared
    /// path) holds no agent .md files after recursive discovery. An all-excluded
    /// root stays silent; an absent root reports nothing (A001 owns declared
    /// absence).
    #[strum(props(code = "A004", name = "no-agent-files"))]
    NoAgentFiles,
    /// A005: reviewer-templates.md is missing
    #[strum(props(code = "A005", name = "template-file-missing"))]
    TemplateFileMissing,
    /// A006: agent .md missing 'Derived from' marker
    #[strum(props(code = "A006", name = "template-marker-missing"))]
    TemplateMarkerMissing,
    /// A007: agent-template count mismatch
    #[strum(props(code = "A007", name = "template-count-mismatch"))]
    TemplateCountMismatch,
    /// A008: agent description exceeds 1024 characters
    #[strum(props(code = "A008", name = "agent-desc-long"))]
    AgentDescLong,
    /// A009: agent description under 20 characters
    #[strum(props(code = "A009", name = "agent-desc-short"))]
    AgentDescShort,
    /// A010: agent name contains characters outside [a-z0-9-]
    #[strum(props(code = "A010", name = "agent-name-invalid"))]
    AgentNameInvalid,
    /// A011: agent description too similar to agent name
    #[strum(props(code = "A011", name = "agent-desc-redundant"))]
    AgentDescRedundant,
    /// A012: agent prompt asks to read evidence without the Read tool
    #[strum(props(code = "A012", name = "agent-read-mismatch"))]
    AgentReadMismatch,
    /// A013: machine-only agent output lacks fail-closed evidence handling
    #[strum(props(code = "A013", name = "agent-output-unsafe"))]
    AgentOutputUnsafe,
    /// A014: agent `model` is not a recognized Claude Code model
    #[strum(props(code = "A014", name = "agent-model-invalid"))]
    AgentModelInvalid,
    /// A015: agent `permissionMode` is not one of the allowed enum values
    #[strum(props(code = "A015", name = "agent-permission-invalid"))]
    AgentPermissionInvalid,
    /// A016: agent `skills` entry does not exist on disk
    #[strum(props(code = "A016", name = "agent-skill-missing"))]
    AgentSkillMissing,
    /// A017: a tool appears in both `tools` and `disallowedTools`
    #[strum(props(code = "A017", name = "agent-tools-overlap"))]
    AgentToolsOverlap,
    /// A018: agent `memory` is not `user`/`project`/`local`
    #[strum(props(code = "A018", name = "agent-memory-invalid"))]
    AgentMemoryInvalid,
    /// A019: agent `tools` lists an unrecognized tool name
    #[strum(props(code = "A019", name = "agent-tools-unknown"))]
    AgentToolsUnknown,
    /// A020: agent `disallowedTools` lists an unrecognized tool name
    #[strum(props(code = "A020", name = "agent-disallowed-unknown"))]
    AgentDisallowedUnknown,
    /// A021: agent `permissionMode: bypassPermissions` disables safety checks
    #[strum(props(code = "A021", name = "agent-bypass-permissions"))]
    AgentBypassPermissions,
    /// A022: agent `skills` entry is not kebab-case
    #[strum(props(code = "A022", name = "agent-skill-kebab"))]
    AgentSkillKebab,
    /// A023: agent `effort` is not `low`/`medium`/`high`/`xhigh`/`max`
    #[strum(props(code = "A023", name = "agent-effort-invalid"))]
    AgentEffortInvalid,
    /// A024: agent `isolation` is not `worktree` or `remote`
    #[strum(props(code = "A024", name = "agent-isolation-invalid"))]
    AgentIsolationInvalid,
    /// A025: agent `background` is not a boolean
    #[strum(props(code = "A025", name = "agent-background-invalid"))]
    AgentBackgroundInvalid,
    /// A026: agent `maxTurns` is not a positive integer
    #[strum(props(code = "A026", name = "agent-maxturns-invalid"))]
    AgentMaxturnsInvalid,
    /// A027: unrecognized agent frontmatter field (possible typo)
    #[strum(props(code = "A027", name = "agent-field-unknown"))]
    AgentFieldUnknown,
    /// A028: agent frontmatter uses a field unsupported for plugin agents
    #[strum(props(code = "A028", name = "agent-field-unsupported"))]
    AgentFieldUnsupported,
    /// A029: tool-using agent has no explicit stop control or failure outcome
    #[strum(props(code = "A029", name = "agent-stop-missing"))]
    AgentStopMissing,
    /// A030: agent routing descriptions overlap within a shared namespace
    #[strum(props(code = "A030", name = "agent-desc-overlap"))]
    AgentDescOverlap,

    // ── Prompt content (Q) ───────────────────────────────────────
    /// Q001: generic filler instruction that provides no actionable guidance
    #[strum(props(code = "Q001", name = "prompt-generic-filler"))]
    PromptGenericFiller,
    /// Q002: operative style negative without a nearby positive alternative
    #[strum(props(code = "Q002", name = "prompt-negative-only"))]
    PromptNegativeOnly,
    /// Q003: weak language inside a critical or important section
    #[strum(props(code = "Q003", name = "prompt-weak-critical"))]
    PromptWeakCritical,
    /// Q004: CLAUDE.md substantially duplicates README.md
    #[strum(props(code = "Q004", name = "claude-readme-duplicate"))]
    ClaudeReadmeDuplicate,
    /// Q005: operative retry or continuation instruction has no bound or fallback
    #[strum(props(code = "Q005", name = "prompt-unbounded-retry"))]
    PromptUnboundedRetry,
    /// Q006: mechanically incompatible operative output instructions in one response scope
    #[strum(props(code = "Q006", name = "prompt-output-conflict"))]
    PromptOutputConflict,

    // ── Claude configuration (R/O/T) ─────────────────────────────
    /// R001: .claude/rules frontmatter paths contains an invalid glob
    #[strum(props(code = "R001", name = "rules-glob-invalid"))]
    RulesGlobInvalid,
    /// R002: .claude/rules frontmatter contains an unrecognized field
    #[strum(props(code = "R002", name = "rules-field-unknown"))]
    RulesFieldUnknown,
    /// O001: output style description is missing or blank
    #[strum(props(code = "O001", name = "style-description-missing"))]
    OutputStyleDescriptionMissing,
    /// O002: output style keep-coding-instructions is not a boolean
    #[strum(props(code = "O002", name = "style-instructions-invalid"))]
    OutputStyleKeepCodingInstructionsInvalid,
    /// O003: output style frontmatter contains an unrecognized field
    #[strum(props(code = "O003", name = "style-field-unknown"))]
    OutputStyleFieldUnknown,
    /// O004: output style has no body after frontmatter
    #[strum(props(code = "O004", name = "style-body-empty"))]
    OutputStyleBodyEmpty,
    /// O005: output style name exceeds 64 characters
    #[strum(props(code = "O005", name = "style-name-long"))]
    OutputStyleNameTooLong,
    /// O006: output style frontmatter is missing or invalid YAML
    #[strum(props(code = "O006", name = "style-frontmatter-invalid"))]
    OutputStyleFrontmatterInvalid,
    /// T001: settings prUrlTemplate is not a usable template string
    #[strum(props(code = "T001", name = "pr-template-invalid"))]
    SettingsPrUrlTemplateInvalid,
    /// T002: repository settings channelsEnabled is ignored by Claude Code
    #[strum(props(code = "T002", name = "channels-enabled-unsupported"))]
    SettingsChannelsEnabledInvalid,

    // ── Shared instruction files (I) ──────────────────────────────
    /// I001: an AGENTS.md file is empty or whitespace-only
    #[strum(props(code = "I001", name = "instruction-file-empty"))]
    InstructionFileEmpty,
    /// I002: an AGENTS.md file contains a potential hardcoded secret
    #[strum(props(code = "I002", name = "instruction-file-secret"))]
    InstructionFileSecret,
    /// I003: an AGENTS.md file references a missing inline-code path
    #[strum(props(code = "I003", name = "instruction-file-path"))]
    InstructionFilePathMissing,
    /// I004: an AGENTS.md file contains only generic guidance
    #[strum(props(code = "I004", name = "instruction-file-generic"))]
    InstructionFileGenericGuidance,

    // ── Codex configuration (CX) ─────────────────────────────────
    /// CX001: .codex/config.toml is not valid TOML
    #[strum(props(code = "CX001", name = "codex-toml-invalid"))]
    CodexTomlInvalid,
    /// CX002: project_doc_max_bytes is outside the supported range
    #[strum(props(code = "CX002", name = "codex-doc-bytes"))]
    CodexProjectDocMaxBytes,
    /// CX003: project_doc_fallback_filenames is invalid
    #[strum(props(code = "CX003", name = "codex-doc-names"))]
    CodexProjectDocFallbackNames,
    /// CX004: unknown Codex configuration key
    #[strum(props(code = "CX004", name = "codex-key-unknown"))]
    CodexUnknownNestedKey,
    /// CX005: approval_policy is invalid
    #[strum(props(code = "CX005", name = "codex-approval-policy"))]
    CodexApprovalPolicy,
    /// CX006: sandbox_mode is invalid
    #[strum(props(code = "CX006", name = "codex-sandbox-mode"))]
    CodexSandboxMode,
    /// CX007: model_reasoning_effort is invalid
    #[strum(props(code = "CX007", name = "codex-reasoning-effort"))]
    CodexReasoningEffort,
    /// CX008: model_verbosity is invalid
    #[strum(props(code = "CX008", name = "codex-model-verbosity"))]
    CodexModelVerbosity,
    /// CX009: personality is invalid
    #[strum(props(code = "CX009", name = "codex-personality"))]
    CodexPersonality,
    /// CX011: shell_environment_policy.inherit is invalid
    #[strum(props(code = "CX011", name = "codex-shell-inherit"))]
    CodexShellEnvironmentInherit,
    /// CX012: an MCP server lacks a command or URL
    #[strum(props(code = "CX012", name = "codex-mcp-transport"))]
    CodexMcpServerTransport,
    /// CX013: an MCP configuration contains a hardcoded secret
    #[strum(props(code = "CX013", name = "codex-secret-literal"))]
    CodexHardcodedSecret,
    /// CX014: cli_auth_credentials_store is invalid
    #[strum(props(code = "CX014", name = "codex-cli-credentials"))]
    CodexCliCredentialsStore,
    /// CX015: sandbox_workspace_write has an invalid field type
    #[strum(props(code = "CX015", name = "codex-workspace-write"))]
    CodexWorkspaceWrite,
    /// CX016: model is not a string
    #[strum(props(code = "CX016", name = "codex-model-type"))]
    CodexModelType,
    /// CX017: model_provider is not a string
    #[strum(props(code = "CX017", name = "codex-provider-type"))]
    CodexModelProviderType,
    /// CX018: model_reasoning_summary is invalid
    #[strum(props(code = "CX018", name = "codex-reasoning-summary"))]
    CodexReasoningSummary,
    /// CX019: history is not a TOML table
    #[strum(props(code = "CX019", name = "codex-history-type"))]
    CodexHistoryType,
    /// CX020: tui is not a TOML table
    #[strum(props(code = "CX020", name = "codex-tui-type"))]
    CodexTuiType,
    /// CX021: file_opener is not a string
    #[strum(props(code = "CX021", name = "codex-opener-type"))]
    CodexFileOpenerType,
    /// CX022: mcp_oauth_credentials_store is invalid
    #[strum(props(code = "CX022", name = "codex-mcp-credentials"))]
    CodexMcpCredentialsStore,
    /// CX023: model_context_window is not positive
    #[strum(props(code = "CX023", name = "codex-context-window"))]
    CodexContextWindow,
    /// CX024: model_auto_compact_token_limit is not positive
    #[strum(props(code = "CX024", name = "codex-compact-limit"))]
    CodexAutoCompactLimit,
    /// CX025: approval_policy table has an unknown field
    #[strum(props(code = "CX025", name = "codex-approval-field"))]
    CodexApprovalPolicyField,
    /// CX026: approvals_reviewer is invalid
    #[strum(props(code = "CX026", name = "codex-approval-reviewer"))]
    CodexApprovalsReviewer,
    /// CX027: service_tier is not a string
    #[strum(props(code = "CX027", name = "codex-service-tier-type"))]
    CodexServiceTier,
    /// CX028: inline MCP bearer_token is forbidden
    #[strum(props(code = "CX028", name = "codex-bearer-token"))]
    CodexInlineBearerToken,
    /// CX029: agents.max_threads is not a positive integer
    #[strum(props(code = "CX029", name = "codex-agent-threads"))]
    CodexAgentThreads,
    /// CX030: app default_tools_approval_mode is invalid
    #[strum(props(code = "CX030", name = "codex-app-approval"))]
    CodexAppApprovalMode,
    /// CX031: skills is not a TOML table
    #[strum(props(code = "CX031", name = "codex-skills-type"))]
    CodexSkillsType,
    /// CX032: profile is not a string
    #[strum(props(code = "CX032", name = "codex-profile-type"))]
    CodexProfileType,
    /// CX033: unknown top-level Codex configuration key
    #[strum(props(code = "CX033", name = "codex-top-key"))]
    CodexTopLevelKey,
    /// CX034: unknown Codex feature flag
    #[strum(props(code = "CX034", name = "codex-feature-key"))]
    CodexFeatureKey,
    /// CX035: unknown permissions.network field
    #[strum(props(code = "CX035", name = "codex-network-field"))]
    CodexNetworkPermissionField,
    /// CX036: windows.sandbox is invalid
    #[strum(props(code = "CX036", name = "codex-windows-sandbox"))]
    CodexWindowsSandbox,
    /// CX039: AGENTS.md exceeds Codex's hard size limit
    #[strum(props(code = "CX039", name = "codex-agents-large"))]
    CodexAgentsTooLarge,
    /// CX040: AGENTS.md exceeds the configured Codex document budget
    #[strum(props(code = "CX040", name = "codex-agents-limit"))]
    CodexAgentsDocLimit,
    /// CX042: AGENTS.override.md is tracked by Git
    #[strum(props(code = "CX042", name = "codex-agents-override"))]
    CodexAgentsOverrideTracked,
    /// CX045: AGENTS.md explicitly contradicts a Codex config value
    #[strum(props(code = "CX045", name = "codex-agents-conflict"))]
    CodexAgentsConfigConflict,
    /// CX046: a Codex plugin manifest is not at the repository root
    #[strum(props(code = "CX046", name = "codex-plugin-path"))]
    CodexPluginManifestPath,
    /// CX047: .codex-plugin/plugin.json is not valid JSON
    #[strum(props(code = "CX047", name = "codex-plugin-invalid"))]
    CodexPluginManifestInvalid,
    /// CX048: Codex plugin manifest name is missing or blank
    #[strum(props(code = "CX048", name = "codex-name-missing"))]
    CodexPluginNameMissing,
    /// CX049: Codex plugin manifest name contains invalid characters
    #[strum(props(code = "CX049", name = "codex-name-invalid"))]
    CodexPluginNameInvalid,
    /// CX050: Codex plugin component path lacks a ./ prefix
    #[strum(props(code = "CX050", name = "codex-path-prefix"))]
    CodexPluginPathPrefix,
    /// CX051: Codex plugin component path contains traversal
    #[strum(props(code = "CX051", name = "codex-path-traversal"))]
    CodexPluginPathTraversal,
    /// CX052: Codex plugin component path is a bare ./
    #[strum(props(code = "CX052", name = "codex-path-bare"))]
    CodexPluginPathBare,
    /// CX053: Codex plugin has too many default prompts
    #[strum(props(code = "CX053", name = "codex-prompt-count"))]
    CodexPluginDefaultPromptCount,
    /// CX054: Codex plugin default prompt exceeds Codex's character limit
    #[strum(props(code = "CX054", name = "codex-prompt-length"))]
    CodexPluginDefaultPromptLength,
    /// CX055: Codex plugin default prompt is empty after whitespace normalization
    #[strum(props(code = "CX055", name = "codex-prompt-empty"))]
    CodexPluginDefaultPromptEmpty,
    /// CX056: Codex plugin interface URL is not HTTP(S)
    #[strum(props(code = "CX056", name = "codex-plugin-url"))]
    CodexPluginInterfaceUrl,
    /// CX057: Codex plugin interface asset path is unsafe
    #[strum(props(code = "CX057", name = "codex-plugin-asset"))]
    CodexPluginInterfaceAssetPath,
    /// CX058: Codex plugin manifest uses the unsupported hooks field
    #[strum(props(code = "CX058", name = "codex-plugin-hooks"))]
    CodexPluginHooksUnsupported,
    /// CX059: Codex plugin manifest description is missing or blank
    #[strum(props(code = "CX059", name = "codex-plugin-description"))]
    CodexPluginDescriptionMissing,
    /// CX060: a Codex skill uses Claude-only frontmatter
    #[strum(props(code = "CX060", name = "codex-skill-frontmatter"))]
    CodexSkillUnsupportedFrontmatter,
    /// CX061: approval_policy granular form has an invalid shape
    #[strum(props(code = "CX061", name = "codex-approval-shape"))]
    CodexApprovalPolicyShape,
    /// CX062: a structured Codex configuration container is not a table
    #[strum(props(code = "CX062", name = "codex-config-container-type"))]
    CodexConfigContainerType,

    // ── Cursor configuration (CU / CR) ───────────────────────────
    /// CU001: Cursor rule file has no instructions
    #[strum(props(code = "CU001", name = "cursor-rule-empty"))]
    CursorRuleEmpty,
    /// CU002: Cursor .mdc rule lacks YAML frontmatter
    #[strum(props(code = "CU002", name = "cursor-frontmatter-missing"))]
    CursorRuleFrontmatterMissing,
    /// CU003: Cursor rule frontmatter is invalid YAML
    #[strum(props(code = "CU003", name = "cursor-frontmatter-invalid"))]
    CursorRuleFrontmatterInvalid,
    /// CU004: Cursor rule globs field contains an invalid pattern
    #[strum(props(code = "CU004", name = "cursor-glob-invalid"))]
    CursorRuleGlobInvalid,
    /// CU005: Cursor rule frontmatter contains an unknown field
    #[strum(props(code = "CU005", name = "cursor-field-unknown"))]
    CursorRuleFieldUnknown,
    /// CU006: legacy .cursorrules file is present
    #[strum(props(code = "CU006", name = "cursor-legacy-rules"))]
    CursorLegacyRules,
    /// CU007: alwaysApply rule also declares globs
    #[strum(props(code = "CU007", name = "cursor-always-globs"))]
    CursorAlwaysApplyGlobs,
    /// CU008: alwaysApply is not a boolean
    #[strum(props(code = "CU008", name = "cursor-always-invalid"))]
    CursorAlwaysApplyInvalid,
    /// CU009: agent-requested Cursor rule lacks a description
    #[strum(props(code = "CU009", name = "cursor-description-missing"))]
    CursorRuleDescriptionMissing,
    /// CU010: .cursor/hooks.json has an invalid schema
    #[strum(props(code = "CU010", name = "cursor-hooks-invalid"))]
    CursorHooksSchemaInvalid,
    /// CU011: Cursor hook event is unknown
    #[strum(props(code = "CU011", name = "cursor-event-unknown"))]
    CursorHookEventUnknown,
    /// CU012: Cursor hook entry lacks a command
    #[strum(props(code = "CU012", name = "cursor-command-missing"))]
    CursorHookCommandMissing,
    /// CU013: Cursor hook type is invalid
    #[strum(props(code = "CU013", name = "cursor-type-invalid"))]
    CursorHookTypeInvalid,
    /// CU014: Cursor subagent frontmatter is invalid
    #[strum(props(code = "CU014", name = "cursor-agent-invalid"))]
    CursorAgentFrontmatterInvalid,
    /// CU015: Cursor subagent has no body
    #[strum(props(code = "CU015", name = "cursor-body-empty"))]
    CursorAgentBodyEmpty,
    /// CU016: .cursor/environment.json has an invalid schema
    #[strum(props(code = "CU016", name = "cursor-environment-invalid"))]
    CursorEnvironmentInvalid,
    /// CU017: Cursor hook entry field type is invalid
    #[strum(props(code = "CU017", name = "cursor-hook-invalid"))]
    CursorHookFieldTypeInvalid,
    /// CU018: Cursor prompt hook lacks prompt
    #[strum(props(code = "CU018", name = "cursor-prompt-missing"))]
    CursorPromptHookPromptMissing,
    /// CU019: Cursor prompt hook model is not a string
    #[strum(props(code = "CU019", name = "cursor-model-invalid"))]
    CursorPromptHookModelInvalid,
    /// CU020: Cursor project rule has an unsupported extension
    #[strum(props(code = "CU020", name = "cursor-rule-extension"))]
    CursorRuleExtension,
    /// CR-SK-001: Cursor skill uses unsupported frontmatter
    #[strum(props(code = "CR-SK-001", name = "cursor-skill-unsupported"))]
    CursorSkillFieldUnsupported,

    // ── Hygiene / Scripts (G) ─────────────────────────────────────
    /// G001: bundled plugin asset uses $PWD instead of ${CLAUDE_PLUGIN_ROOT}
    #[strum(props(code = "G001", name = "pwd-in-skill"))]
    PwdInSkill,
    /// G002: script reference missing on disk
    #[strum(props(code = "G002", name = "script-ref-missing"))]
    ScriptRefMissing,
    /// G003: script file not executable
    #[strum(props(code = "G003", name = "script-not-executable"))]
    ScriptNotExecutable,
    /// G004: dead script with no structured invocation reference
    #[strum(props(code = "G004", name = "dead-script"))]
    DeadScript,
    /// G005: no repository-local SECURITY.md security policy in a
    /// GitHub-supported location (repository root, `.github/`, or `docs/`)
    #[strum(props(code = "G005", name = "security-policy-missing"))]
    SecurityMdMissing,
    /// G006: TODO/FIXME/HACK/XXX marker in published skill content
    #[strum(props(code = "G006", name = "todo-in-skill"))]
    TodoInSkill,
    /// G007: TODO/FIXME/HACK/XXX marker in agent .md body
    #[strum(props(code = "G007", name = "todo-in-agent"))]
    TodoInAgent,
    /// G008: GitHub body or release notes are passed inline
    #[strum(props(code = "G008", name = "gh-inline-body"))]
    GhInlineBody,
    /// G009: Bash global substitution uses an unsafe variable replacement
    #[strum(props(code = "G009", name = "bash-replacement-unsafe"))]
    BashReplacementUnsafe,
    /// G010: shipped shell uses syntax unavailable in Bash 3.2
    #[strum(props(code = "G010", name = "bash32-incompatible"))]
    Bash32Incompatible,
    /// G011: dynamic awk regex contains non-ASCII text
    #[strum(props(code = "G011", name = "awk-regex-nonascii"))]
    AwkRegexNonascii,
    /// G012: skill contains a machine-specific or ambiguous runtime path
    #[strum(props(code = "G012", name = "hardcoded-machine-path"))]
    HardcodedMachinePath,

    // ── Email (E) ─────────────────────────────────────────────────
    /// E001: email address does not meet the contact-metadata quality convention
    #[strum(props(code = "E001", name = "invalid-email-format"))]
    InvalidEmailFormat,
    /// E002: email address metadata has a non-string JSON type
    #[strum(props(code = "E002", name = "email-type-invalid"))]
    EmailTypeInvalid,

    // ── User Config (U) ───────────────────────────────────────────
    /// U001: userConfig must be an object
    #[strum(props(code = "U001", name = "userconfig-not-object"))]
    UserconfigNotObject,
    /// U002: userConfig entry missing or invalid description
    #[strum(props(code = "U002", name = "userconfig-desc-missing"))]
    UserconfigDescMissing,
    /// U004: userConfig sensitive field must be a boolean
    #[strum(props(code = "U004", name = "userconfig-sensitive-type"))]
    UserconfigSensitiveType,
    /// U005: userConfig entry missing or invalid title
    #[strum(props(code = "U005", name = "userconfig-title-missing"))]
    UserconfigTitleMissing,
    /// U006: userConfig entry missing or invalid type
    #[strum(props(code = "U006", name = "userconfig-type-missing"))]
    UserconfigTypeMissing,
    /// U007: userConfig key is not a valid identifier
    #[strum(props(code = "U007", name = "userconfig-key-invalid"))]
    UserconfigKeyInvalid,
    /// U008: userConfig option entry is not an object or has an invalid optional field
    #[strum(props(code = "U008", name = "userconfig-option-invalid"))]
    UserconfigOptionInvalid,
    /// U009: userConfig option `default` ships a secret (sensitive option or secret-shaped literal)
    #[strum(props(code = "U009", name = "userconfig-default-secret"))]
    UserconfigDefaultSecret,

    // ── Docs (D) ──────────────────────────────────────────────────
    /// D001: docs reference in CLAUDE.md canonical sources not found on disk
    #[strum(props(code = "D001", name = "docs-ref-missing"))]
    DocsRefMissing,
    /// D002: CLAUDE.md exceeds 500 lines
    #[strum(props(code = "D002", name = "claudemd-too-large"))]
    ClaudemdTooLarge,
    /// D003: TODO/FIXME/HACK/XXX marker in CLAUDE.md
    #[strum(props(code = "D003", name = "todo-in-docs"))]
    TodoInDocs,
    /// D004: repository-local CLAUDE.md import closure exceeds configured budget
    #[strum(props(code = "D004", name = "claude-import-large"))]
    ClaudeImportLarge,
    /// D005: inline-code repository path does not exist
    #[strum(props(code = "D005", name = "inline-path-missing"))]
    InlinePathMissing,

    // ── MCP configuration (P) ──────────────────────────────────────
    /// P001: MCP configuration is not valid JSON
    #[strum(props(code = "P001", name = "mcp-json-invalid"))]
    McpJsonInvalid,
    /// P009: stdio MCP server is missing its command
    #[strum(props(code = "P009", name = "mcp-stdio-command"))]
    McpStdioCommandMissing,
    /// P010: remote MCP server URL is missing or invalid for its transport
    #[strum(props(code = "P010", name = "mcp-http-url"))]
    McpHttpUrlMissing,
    /// P011: MCP server type is not supported
    #[strum(props(code = "P011", name = "mcp-type-invalid"))]
    McpTypeInvalid,
    /// P012: SSE transport is deprecated
    #[strum(props(code = "P012", name = "mcp-sse-deprecated"))]
    McpSseDeprecated,
    /// P017: non-local HTTP or WebSocket MCP URL is insecure
    #[strum(props(code = "P017", name = "mcp-insecure-url"))]
    McpUrlNotHttps,
    /// P018: MCP environment contains a literal secret
    #[strum(props(code = "P018", name = "mcp-env-secret"))]
    McpEnvSecretLiteral,
    /// P019: MCP command contains a dangerous shell pattern
    #[strum(props(code = "P019", name = "mcp-command-dangerous"))]
    McpCommandDangerous,
    /// P022: MCP args is not an array of strings
    #[strum(props(code = "P022", name = "mcp-args-invalid"))]
    McpArgsInvalid,
    /// P023: mcpServers has duplicate server names
    #[strum(props(code = "P023", name = "mcp-duplicate-server"))]
    McpDuplicateServer,
    /// P024: MCP server configuration is empty
    #[strum(props(code = "P024", name = "mcp-server-empty"))]
    McpServerEmpty,
    /// P025: MCP alwaysLoad is not a boolean
    #[strum(props(code = "P025", name = "mcp-alwaysload-invalid"))]
    McpAlwaysLoadInvalid,
    /// P026: MCP server name is reserved by Claude Code's built-in list
    #[strum(props(code = "P026", name = "mcp-server-reserved"))]
    McpServerReserved,
    /// P027: MCP document or server entry has an invalid structure
    #[strum(props(code = "P027", name = "mcp-structure-invalid"))]
    McpStructureInvalid,

    // ── Link/import integrity (L) ────────────────────────────────
    /// L001: repository-relative @import target is missing or unreadable
    #[strum(props(code = "L001", name = "import-path-missing"))]
    ImportPathMissing,
    /// L002: circular repository-local @import chain detected
    #[strum(props(code = "L002", name = "circular-import"))]
    CircularImport,
    /// L003: repository-local @import chain depth exceeds 5 hops
    #[strum(props(code = "L003", name = "import-depth-exceeded"))]
    ImportDepthExceeded,
    /// L004: duplicate normalized direct @import edge
    #[strum(props(code = "L004", name = "duplicate-import"))]
    DuplicateImport,
    /// L005: broken relative markdown link target
    #[strum(props(code = "L005", name = "broken-markdown-link"))]
    BrokenMarkdownLink,
    /// L006: npm run script not defined in package.json
    #[strum(props(code = "L006", name = "npm-script-missing"))]
    NpmScriptMissing,
}

impl LintRule {
    /// The short code, e.g. `"M001"`.
    pub fn code(self) -> &'static str {
        self.get_str("code")
            .expect("every LintRule variant defines its code metadata")
    }

    /// The human-readable name, e.g. `"plugin-json-missing"`.
    pub fn name(self) -> &'static str {
        self.get_str("name")
            .expect("every LintRule variant defines its name metadata")
    }

    /// Whether this rule is a "too-long" length-cap rule, excluded from
    /// pedantic error promotion.
    pub fn is_too_long(self) -> bool {
        matches!(
            self,
            Self::NameTooLong | Self::DescTooLong | Self::BodyTooLong | Self::CompatTooLong
        )
    }

    /// Look up a rule by its code (e.g. `"M001"`) or human-readable name
    /// (e.g. `"plugin-json-missing"`).
    pub fn from_code_or_name(s: &str) -> Option<Self> {
        let migrated = match s {
            "channels-enabled-invalid" => Some(Self::SettingsChannelsEnabledInvalid),
            "CX037" | "codex-agents-empty" => Some(Self::InstructionFileEmpty),
            "CX038" | "codex-agents-secret" => Some(Self::InstructionFileSecret),
            "CX041" | "codex-agents-path" => Some(Self::InstructionFilePathMissing),
            "CX043" | "codex-agents-generic" => Some(Self::InstructionFileGenericGuidance),
            _ => None,
        };
        if migrated.is_some() {
            return migrated;
        }
        ALL_RULES
            .iter()
            .find(|r| r.code() == s || r.name() == s)
            .copied()
    }

    /// Whether this rule's violations can be automatically fixed by `--autofix`.
    /// Only purely mechanical, unambiguous fixes are classified as auto-fixable.
    pub fn is_autofixable(self) -> bool {
        matches!(
            self,
            Self::HookNotExecutable
                | Self::ScriptNotExecutable
                | Self::FrontmatterNameMismatch
                | Self::FrontmatterFieldEmpty
                | Self::DescHasXml
                | Self::ConsecutiveBash
                | Self::BackslashPath
                | Self::NonHttpsUrl
                | Self::FrontmatterBackslash
                | Self::ToolsListSyntax
                | Self::PwdInSkill
        )
    }

    /// Compiled-in default severity. Rules not mentioned in the user's config
    /// fall back to this. Style, quality, and niche rules default to
    /// `Suppressed`; structural and correctness rules default to `Error`.
    pub fn default_severity(self) -> DefaultSeverity {
        match self {
            // ── Default-suppressed ──────────────────────────────────
            Self::NameNotGerund | Self::BodyNoExamples | Self::BodyTooLong
                => DefaultSeverity::Suppressed,

            // ── Default-warning: enriched metadata ───────────────────
            Self::MarketplaceEnrichedMissing | Self::PluginEnrichedMissing |

            // ── Default-warning: hook schema advisories ──────────────
            Self::HookIfInvalid | Self::HookShellInvalid |
            Self::HookCommandDangerous | Self::HookHeadersInterpolated |

            // ── Default-warning: optional manifest sections ──────────
            Self::AuthorNameMissing | Self::HomepageUrlInvalid |
            Self::ChannelServerMissing | Self::PluginVersionMissing |

            // ── Default-warning: marketplace entry advisories ────────
            Self::MarketplaceBarePath | Self::MarketplaceNameFormat |

            // ── Default-warning: optional manifest files ────────────
            Self::MarketplaceJsonMissing | Self::MarketplacePluginsEmpty |

            // ── Default-warning: style / quality (skills) ────────────
            Self::DescTruncated | Self::ConsecutiveBash |
            Self::NameVague | Self::DescTooShort | Self::BodyNoRefs |
            Self::DescUsesPerson | Self::DescNoTrigger |
            Self::ForkNoTask | Self::BodyNoWorkflow | Self::RefNameGeneric |
            Self::DescVagueContent | Self::ScriptDepsMissing |
            Self::ScriptVerifyMissing | Self::TerminologyInconsistent |
            Self::DescBodyMisalign | Self::ScriptErrhandMissing |
            Self::BodyNoDefault | Self::MagicNumberUndoc |
            Self::SkillClosureLarge | Self::PromptGenericFiller |
            Self::PromptOutputConflict | Self::ClaudeReadmeDuplicate |

            // ── Default-warning: niche (skills) ──────────────────────
            Self::NestedRefDeep | Self::CompatTooLong | Self::RefNoToc |
            Self::TimeSensitive | Self::ToolsUnknown |
            Self::McpToolUnqualified | Self::ToolsListSyntax |
            Self::SideEffectAuto | Self::BashUnscoped |
            Self::InjectionOverflow | Self::HintNoArgs |
            Self::UnknownFmField | Self::PathsEmpty |

            // ── Default-warning: template rules (agents) ─────────────
            Self::TemplateFileMissing | Self::TemplateMarkerMissing |
            Self::TemplateCountMismatch |

            // ── Default-warning: agent field-value (advisory) ────────
            Self::AgentBypassPermissions | Self::AgentSkillKebab |
            Self::AgentBackgroundInvalid | Self::AgentFieldUnknown |
            Self::AgentFieldUnsupported | Self::AgentStopMissing |
            Self::AgentDescOverlap |

            // ── Default-warning: skill routing overlap ───────────────
            Self::SkillDescOverlap |

            // ── Default-warning: Claude configuration (advisory) ──
            Self::RulesFieldUnknown | Self::OutputStyleDescriptionMissing |
            Self::OutputStyleFieldUnknown | Self::OutputStyleBodyEmpty |
            Self::OutputStyleNameTooLong | Self::SettingsPrUrlTemplateInvalid |
            Self::SettingsChannelsEnabledInvalid |
            Self::CodexUnknownNestedKey | Self::CodexModelType |
            Self::CodexReasoningSummary | Self::CodexHistoryType |
            Self::CodexTuiType | Self::CodexFileOpenerType |
            Self::CodexContextWindow | Self::CodexAutoCompactLimit |
            Self::CodexApprovalPolicyField | Self::CodexApprovalsReviewer |
            Self::CodexSkillsType | Self::CodexProfileType |
            Self::CodexTopLevelKey | Self::CodexFeatureKey |
            Self::CodexNetworkPermissionField |
            Self::CodexAgentsTooLarge | Self::CodexAgentsDocLimit |
            Self::InstructionFilePathMissing | Self::InstructionFileGenericGuidance |
            Self::CodexAgentsOverrideTracked |
            Self::CodexPluginDefaultPromptCount | Self::CodexPluginDefaultPromptLength |
            Self::CodexPluginDefaultPromptEmpty | Self::CodexPluginInterfaceUrl |
            Self::CodexPluginHooksUnsupported | Self::CodexPluginDescriptionMissing |
            Self::CodexSkillUnsupportedFrontmatter |
            Self::CursorRuleFrontmatterMissing | Self::CursorRuleFieldUnknown |
            Self::CursorLegacyRules | Self::CursorAlwaysApplyGlobs |
            Self::CursorRuleDescriptionMissing | Self::CursorHookEventUnknown |
            Self::CursorHookFieldTypeInvalid |
            Self::CursorAgentBodyEmpty | Self::CursorSkillFieldUnsupported |
            Self::CursorRuleExtension |

            // ── Default-warning: userConfig security convention ─────
            Self::UserconfigDefaultSecret |

            // ── Default-warning: contact metadata ───────────────────
            Self::InvalidEmailFormat |

            // ── Default-warning: hygiene ─────────────────────────────
            Self::DeadScript | Self::SecurityMdMissing | Self::TodoInSkill | Self::TodoInAgent |
            Self::GhInlineBody |

            // ── Default-warning: docs ────────────────────────────────
            Self::ClaudemdTooLarge | Self::TodoInDocs |
            Self::ClaudeImportLarge | Self::InlinePathMissing
            | Self::McpSseDeprecated | Self::McpEnvSecretLiteral
            | Self::McpCommandDangerous | Self::McpAlwaysLoadInvalid |

            // ── Default-warning: markdown structure ──────────────────
            Self::XmlTagUnclosed | Self::XmlTagMismatched | Self::XmlTagOrphan |
            Self::SkillDirOversized |

            // ── Default-warning: link/import integrity ───────────────
            Self::DuplicateImport | Self::BrokenMarkdownLink |
            Self::HardcodedMachinePath |
            Self::NpmScriptMissing
                => DefaultSeverity::Warning,

            // Everything else defaults to error.
            _ => DefaultSeverity::Error,
        }
    }
}

/// Every variant of [`LintRule`], derived from the enum declaration.
pub const ALL_RULES: &[LintRule] = LintRule::VARIANTS;

#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;
    use std::collections::{HashMap, HashSet};
    use strum::IntoEnumIterator;

    #[derive(Debug)]
    struct DocumentedRule {
        line_number: usize,
        name: Option<String>,
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

    fn documented_rule_rows(documentation: &str) -> HashMap<String, DocumentedRule> {
        let mut rules = HashMap::new();
        let rule_row =
            Regex::new(r"^\|\s*(.*?)\s*\|\s*(.*?)\s*\|").expect("rule row regex is valid");

        for (line_number, line) in documentation.lines().enumerate() {
            let Some(captures) = rule_row.captures(line) else {
                continue;
            };
            let code_cell = captures.get(1).expect("rule code capture exists").as_str();
            let name_cell = captures.get(2).expect("rule name capture exists").as_str();

            let (first_code, last_code) = if let Some((first, last)) = code_cell.split_once('–') {
                (first, Some(last))
            } else if let Some((first, last)) = code_cell.split_once("--") {
                (first, Some(last))
            } else {
                (code_cell, None)
            };
            let Some((prefix, first_number)) = rule_code_parts(first_code) else {
                continue;
            };
            let last_number = match last_code {
                Some(last_code) => {
                    let (last_prefix, last_number) =
                        rule_code_parts(last_code).unwrap_or_else(|| {
                            panic!(
                                "docs/rules.md:{}: invalid rule code range {code_cell}",
                                line_number + 1
                            )
                        });
                    assert_eq!(
                        prefix,
                        last_prefix,
                        "docs/rules.md:{}: rule code range {code_cell} changes prefix",
                        line_number + 1
                    );
                    assert!(
                        first_number <= last_number,
                        "docs/rules.md:{}: rule code range {code_cell} is descending",
                        line_number + 1
                    );
                    last_number
                }
                None => first_number,
            };

            let name = if first_number == last_number {
                Some(
                    name_cell
                        .strip_prefix('`')
                        .and_then(|name| name.strip_suffix('`'))
                        .unwrap_or_else(|| {
                            panic!(
                                "docs/rules.md:{}: rule row for {first_code} must use a backticked name",
                                line_number + 1
                            )
                        })
                        .to_owned(),
                )
            } else {
                None
            };

            for number in first_number..=last_number {
                let code = format!("{prefix}{number:03}");
                assert!(
                    rules
                        .insert(
                            code.clone(),
                            DocumentedRule {
                                line_number: line_number + 1,
                                name: name.clone(),
                            },
                        )
                        .is_none(),
                    "docs/rules.md:{}: duplicate documented rule code {code}",
                    line_number + 1
                );
            }
        }

        rules
    }

    #[test]
    fn all_rules_count_matches_enum() {
        let iterated: Vec<_> = LintRule::iter().collect();
        assert_eq!(ALL_RULES, iterated);
        assert_eq!(
            ALL_RULES.len(),
            298,
            "every enum variant must be registered"
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
                word_count <= 3
                    || matches!(
                        rule,
                        LintRule::CodexServiceTier | LintRule::CodexConfigContainerType
                    ),
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
        assert_eq!(LintRule::from_code_or_name("CX010"), None);
        assert_eq!(LintRule::from_code_or_name("codex-access-ack"), None);
        assert_eq!(
            LintRule::from_code_or_name("channels-enabled-invalid"),
            Some(LintRule::SettingsChannelsEnabledInvalid)
        );
    }

    #[test]
    fn migrated_codex_agents_identifiers_resolve_to_shared_rules() {
        for (identifier, expected) in [
            ("CX037", LintRule::InstructionFileEmpty),
            ("codex-agents-secret", LintRule::InstructionFileSecret),
            ("CX041", LintRule::InstructionFilePathMissing),
            (
                "codex-agents-generic",
                LintRule::InstructionFileGenericGuidance,
            ),
            ("CX043", LintRule::InstructionFileGenericGuidance),
        ] {
            assert_eq!(LintRule::from_code_or_name(identifier), Some(expected));
        }
        for retired in [
            "I005",
            "instruction-file-structure",
            "CX044",
            "codex-agents-structure",
        ] {
            assert_eq!(
                LintRule::from_code_or_name(retired),
                None,
                "{retired} must not resolve after I005 removal"
            );
        }
    }

    #[test]
    fn every_rule_round_trips() {
        for rule in ALL_RULES {
            assert_eq!(LintRule::from_code_or_name(rule.code()), Some(*rule));
            assert_eq!(LintRule::from_code_or_name(rule.name()), Some(*rule));
        }
    }

    #[test]
    fn rules_documentation_matches_registry() {
        let documented = documented_rule_rows(include_str!("../docs/rules.md"));

        assert_eq!(
            documented.len(),
            ALL_RULES.len(),
            "docs/rules.md must document every registered rule code"
        );

        for rule in ALL_RULES {
            let documented_rule = documented.get(rule.code()).unwrap_or_else(|| {
                panic!(
                    "docs/rules.md is missing {} ({}) from the rule registry",
                    rule.code(),
                    rule.name()
                )
            });
            if let Some(documented_name) = &documented_rule.name {
                assert_eq!(
                    documented_name,
                    rule.name(),
                    "docs/rules.md documents {} with the wrong rule name",
                    rule.code()
                );
            }
        }

        for (code, documented_rule) in documented {
            let rule = LintRule::from_code_or_name(&code).unwrap_or_else(|| {
                panic!(
                    "docs/rules.md:{} documents {code}, but it has no implementation",
                    documented_rule.line_number
                )
            });
            if let Some(documented_name) = documented_rule.name {
                assert_eq!(
                    rule.name(),
                    documented_name,
                    "docs/rules.md:{} documents {code} with `{documented_name}`, but the registry names it `{}`",
                    documented_rule.line_number,
                    rule.name()
                );
            }
        }
    }

    #[test]
    fn default_suppressed_count() {
        let suppressed: Vec<_> = ALL_RULES
            .iter()
            .filter(|r| r.default_severity() == DefaultSeverity::Suppressed)
            .collect();
        assert_eq!(
            suppressed.len(),
            3,
            "Expected 3 default-suppressed rules, got {}",
            suppressed.len()
        );
    }

    #[test]
    fn default_warning_count() {
        let warnings: Vec<_> = ALL_RULES
            .iter()
            .filter(|r| r.default_severity() == DefaultSeverity::Warning)
            .collect();
        assert_eq!(
            warnings.len(),
            127,
            "Expected 127 default-warning rules, got {}",
            warnings.len()
        );
    }

    #[test]
    fn codex_default_severities_match_the_schema_contract() {
        for rule in [
            LintRule::CodexWindowsSandbox,
            LintRule::CodexServiceTier,
            LintRule::CodexAgentThreads,
            LintRule::CodexApprovalPolicyShape,
            LintRule::CodexConfigContainerType,
            LintRule::CodexAgentsConfigConflict,
            LintRule::PromptNegativeOnly,
            LintRule::PromptWeakCritical,
            LintRule::PromptUnboundedRetry,
            LintRule::CursorPromptHookModelInvalid,
            LintRule::CursorPromptHookPromptMissing,
            LintRule::SkillRefNested,
        ] {
            assert_eq!(rule.default_severity(), DefaultSeverity::Error, "{rule:?}");
        }
        assert_eq!(
            LintRule::CodexNetworkPermissionField.default_severity(),
            DefaultSeverity::Warning
        );
        assert_eq!(
            LintRule::InstructionFileGenericGuidance.default_severity(),
            DefaultSeverity::Warning
        );
    }

    #[test]
    fn issue_328_s041_is_a_default_warning() {
        assert_eq!(
            LintRule::ForkNoTask.default_severity(),
            DefaultSeverity::Warning
        );
    }

    #[test]
    fn issue_360_q004_is_a_default_warning() {
        assert_eq!(
            LintRule::ClaudeReadmeDuplicate.default_severity(),
            DefaultSeverity::Warning
        );
    }

    #[test]
    fn issue_162_portability_rules_are_default_errors() {
        for rule in [LintRule::Bash32Incompatible, LintRule::AwkRegexNonascii] {
            assert_eq!(rule.default_severity(), DefaultSeverity::Error, "{rule:?}");
        }
    }

    #[test]
    fn is_too_long_matches_exactly_four() {
        let too_long: Vec<_> = ALL_RULES.iter().filter(|r| r.is_too_long()).collect();
        assert_eq!(
            too_long.len(),
            4,
            "Expected 4 too-long rules, got {}",
            too_long.len()
        );
    }

    #[test]
    fn is_too_long_correct_rules() {
        assert!(LintRule::NameTooLong.is_too_long());
        assert!(LintRule::DescTooLong.is_too_long());
        assert!(LintRule::BodyTooLong.is_too_long());
        assert!(LintRule::CompatTooLong.is_too_long());
        // Verify some non-too-long rules
        assert!(!LintRule::DescTruncated.is_too_long());
        assert!(!LintRule::AgentDescLong.is_too_long());
        assert!(!LintRule::ClaudemdTooLarge.is_too_long());
        assert!(!LintRule::PluginJsonMissing.is_too_long());
    }

    #[test]
    fn autofixable_count() {
        let fixable: Vec<_> = ALL_RULES.iter().filter(|r| r.is_autofixable()).collect();
        assert_eq!(
            fixable.len(),
            11,
            "Expected 11 auto-fixable rules, got {}",
            fixable.len()
        );
    }

    #[test]
    fn issue_319_s016_s017_are_default_warnings() {
        assert_eq!(
            LintRule::DescUsesPerson.default_severity(),
            DefaultSeverity::Warning
        );
        assert_eq!(
            LintRule::DescNoTrigger.default_severity(),
            DefaultSeverity::Warning
        );
        assert_eq!(
            LintRule::DescHasXml.default_severity(),
            DefaultSeverity::Error
        );
    }

    #[test]
    fn default_error_count() {
        let errors: Vec<_> = ALL_RULES
            .iter()
            .filter(|r| r.default_severity() == DefaultSeverity::Error)
            .collect();
        assert_eq!(
            errors.len(),
            168,
            "Expected 168 default-error rules, got {}",
            errors.len()
        );
    }

    #[test]
    fn cursor_hook_rules_are_not_autofixable() {
        for rule in [
            LintRule::CursorHooksSchemaInvalid,
            LintRule::CursorHookEventUnknown,
            LintRule::CursorHookCommandMissing,
            LintRule::CursorHookTypeInvalid,
            LintRule::CursorHookFieldTypeInvalid,
            LintRule::CursorPromptHookPromptMissing,
            LintRule::CursorPromptHookModelInvalid,
        ] {
            assert!(
                !rule.is_autofixable(),
                "{rule:?} must remain diagnostic-only"
            );
        }
    }
}
