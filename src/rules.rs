//! Central rule registry for agent-lint.
//!
//! Every lint diagnostic has a unique code (e.g., "M001") and human-readable
//! name (e.g., "plugin-json-missing"). Rules are grouped by category prefix.

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
    /// M012: plugin component lives inside or is declared inside .claude-plugin/
    ComponentPathNested,
    /// M013: plugin.json component path is absolute or uses '..' traversal
    ComponentPathUnsafe,
    /// M014: plugin.json author object present but author.name missing/invalid
    AuthorNameMissing,
    /// M015: plugin.json homepage is not a valid http(s) URL
    HomepageUrlInvalid,
    /// M016: plugin.json lspServers entry missing command or extensionToLanguage
    LspServerInvalid,
    /// M017: plugin.json channels entry does not reference a server
    ChannelServerMissing,

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
    /// H007: hooks.json hooks collection is empty
    HooksArrayEmpty,
    /// H008: hook event name is not a recognized Claude Code event
    HookEventInvalid,
    /// H009: matcher present on an event that takes no matcher
    HookMatcherInvalid,
    /// H010: hook object missing required 'type' field
    HookTypeMissing,
    /// H011: hook 'type' is not a recognized handler type
    HookTypeUnknown,
    /// H012: type: command hook missing 'command'
    HookCommandRequired,
    /// H013: type: prompt or type: agent hook missing 'prompt'
    HookPromptRequired,
    /// H014: type: http hook missing 'url'
    HookUrlRequired,
    /// H015: type: mcp_tool hook missing 'server'
    HookServerRequired,
    /// H016: type: mcp_tool hook missing 'tool'
    HookToolRequired,
    /// H017: hook 'timeout' is not a positive integer
    HookTimeoutInvalid,
    /// H018: 'async: true' on a non-command hook
    HookAsyncInvalid,
    /// H019: 'model' on a hook other than prompt/agent
    HookModelInvalid,
    /// H020: hook 'once' is not a boolean
    HookOnceInvalid,
    /// H021: hook 'if' is invalid or used outside a tool event
    HookIfInvalid,
    /// H022: hook 'shell' value is not bash/powershell
    HookShellInvalid,
    /// H023: dangerous command pattern in hook command
    HookCommandDangerous,
    /// H024: http hook headers interpolate $VAR without allowedEnvVars
    HookHeadersInterpolated,
    /// H025: .claude/settings.local.json is not valid JSON
    SettingsLocalInvalid,

    // ── Markdown structure (X) ────────────────────────────────────
    /// X001: skill/agent frontmatter is not valid YAML
    FrontmatterYamlInvalid,
    /// X002: unclosed code fence in a linted markdown file
    UnclosedCodeFence,
    /// X003: unclosed XML tag in markdown body
    XmlTagUnclosed,
    /// X004: mismatched closing XML tag in markdown body
    XmlTagMismatched,
    /// X005: closing XML tag with no opening tag
    XmlTagOrphan,

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
    /// S025: effort field value is not low/medium/high/xhigh/max
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
    /// S044: MCP tool reference without server prefix
    McpToolUnqualified,
    /// S045: allowed-tools uses YAML list syntax instead of comma-separated scalar
    ToolsListSyntax,
    /// S046: Long skill body lacks workflow structure
    BodyNoWorkflow,
    /// S047: Long skill body lacks examples or templates
    BodyNoExamples,
    /// S048: non-descriptive reference file name in skill directory
    RefNameGeneric,
    /// S049: skill name not in gerund form
    NameNotGerund,
    /// S050: skill description content is too vague/generic
    DescVagueContent,
    /// S051: script-backed skill lacks dependency/package notes
    ScriptDepsMissing,
    /// S052: script-backed skill lacks verification step
    ScriptVerifyMissing,
    /// S053: terminology inconsistency — 3+ synonym variants used
    TerminologyInconsistent,
    /// S054: skill description keywords not reflected in body
    DescBodyMisalign,
    /// S055: script file lacks error handling patterns
    ScriptErrhandMissing,
    /// S056: body lists alternatives without stating a default
    BodyNoDefault,
    /// S057: undocumented magic number in code block
    MagicNumberUndoc,
    /// S058: Skill tool allowed without a clear invocation step
    SkillInvokeMissing,
    /// S059: prompt invocation flag is not accepted by its shipped script
    SkillFlagMismatch,
    /// S060: awk positional field appears in a skill shell fence
    AwkFieldRef,
    /// S061: grep-family probe in a skill shell fence is unbounded
    UnsafeGrepProbe,
    /// S062: always-loaded skill prompt closure exceeds configured budget
    SkillClosureLarge,
    /// S063: model field value is not a recognized alias or model ID
    ModelInvalid,
    /// S064: agent field present without context: fork
    AgentNoFork,
    /// S065: agent value is not a built-in or existing custom agent
    AgentUnknown,
    /// S066: side-effect-named skill without disable-model-invocation: true
    SideEffectAuto,
    /// S067: allowed-tools lists unscoped Bash (suggest Bash(…)-style scoping)
    BashUnscoped,
    /// S068: more than 3 dynamic context injections in skill body
    InjectionOverflow,
    /// S069: argument-hint set but body never references $ARGUMENTS
    HintNoArgs,
    /// S070: unknown skill frontmatter field
    UnknownFmField,
    /// S071: paths field present but empty
    PathsEmpty,
    /// S072: skill directory exceeds 8MB (platform upload limit)
    SkillDirOversized,
    /// S073: skill file reference nested deeper than one level
    SkillRefNested,

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
    /// A011: agent description too similar to agent name
    AgentDescRedundant,
    /// A012: agent prompt asks to read evidence without the Read tool
    AgentReadMismatch,
    /// A013: machine-only agent output lacks fail-closed evidence handling
    AgentOutputUnsafe,
    /// A014: agent `model` is not a recognized Claude Code model
    AgentModelInvalid,
    /// A015: agent `permissionMode` is not one of the allowed enum values
    AgentPermissionInvalid,
    /// A016: agent `skills` entry does not exist on disk
    AgentSkillMissing,
    /// A017: a tool appears in both `tools` and `disallowedTools`
    AgentToolsOverlap,
    /// A018: agent `memory` is not `user`/`project`/`local`
    AgentMemoryInvalid,
    /// A019: agent `tools` lists an unrecognized tool name
    AgentToolsUnknown,
    /// A020: agent `disallowedTools` lists an unrecognized tool name
    AgentDisallowedUnknown,
    /// A021: agent `permissionMode: bypassPermissions` disables safety checks
    AgentBypassPermissions,
    /// A022: agent `skills` entry is not kebab-case
    AgentSkillKebab,
    /// A023: agent `effort` is not `low`/`medium`/`high`/`xhigh`/`max`
    AgentEffortInvalid,
    /// A024: agent `isolation` is not `worktree`
    AgentIsolationInvalid,
    /// A025: agent `background` is not a boolean
    AgentBackgroundInvalid,
    /// A026: agent `maxTurns` is not a positive integer
    AgentMaxturnsInvalid,
    /// A027: unrecognized agent frontmatter field (possible typo)
    AgentFieldUnknown,
    /// A028: agent frontmatter uses a field unsupported for plugin agents
    AgentFieldUnsupported,

    // ── Prompt content (Q) ───────────────────────────────────────
    /// Q001: generic filler instruction that provides no actionable guidance
    PromptGenericFiller,
    /// Q002: negative instruction without a nearby positive alternative
    PromptNegativeOnly,
    /// Q003: weak language inside a critical or important section
    PromptWeakCritical,
    /// Q004: CLAUDE.md substantially duplicates README.md
    ClaudeReadmeDuplicate,

    // ── Claude configuration (R/O/T) ─────────────────────────────
    /// R001: .claude/rules frontmatter paths contains an invalid glob
    RulesGlobInvalid,
    /// R002: .claude/rules frontmatter contains an unrecognized field
    RulesFieldUnknown,
    /// O001: output style description is missing or blank
    OutputStyleDescriptionMissing,
    /// O002: output style keep-coding-instructions is not a boolean
    OutputStyleKeepCodingInstructionsInvalid,
    /// O003: output style frontmatter contains an unrecognized field
    OutputStyleFieldUnknown,
    /// O004: output style has no body after frontmatter
    OutputStyleBodyEmpty,
    /// O005: output style name exceeds 64 characters
    OutputStyleNameTooLong,
    /// O006: output style frontmatter is missing or invalid YAML
    OutputStyleFrontmatterInvalid,
    /// T001: settings prUrlTemplate is not a usable template string
    SettingsPrUrlTemplateInvalid,
    /// T002: settings channelsEnabled is not a boolean
    SettingsChannelsEnabledInvalid,

    // ── Shared instruction files (I) ──────────────────────────────
    /// I001: an AGENTS.md file is empty or whitespace-only
    InstructionFileEmpty,
    /// I002: an AGENTS.md file contains a potential hardcoded secret
    InstructionFileSecret,
    /// I003: an AGENTS.md file references a missing inline-code path
    InstructionFilePathMissing,
    /// I004: an AGENTS.md file contains only generic guidance
    InstructionFileGenericGuidance,
    /// I005: an AGENTS.md file lacks project-specific structure
    InstructionFileMissingStructure,

    // ── Codex configuration (CX) ─────────────────────────────────
    /// CX001: .codex/config.toml is not valid TOML
    CodexTomlInvalid,
    /// CX002: project_doc_max_bytes is outside the supported range
    CodexProjectDocMaxBytes,
    /// CX003: project_doc_fallback_filenames is invalid
    CodexProjectDocFallbackNames,
    /// CX004: unknown Codex configuration key
    CodexUnknownNestedKey,
    /// CX005: approval_policy is invalid
    CodexApprovalPolicy,
    /// CX006: sandbox_mode is invalid
    CodexSandboxMode,
    /// CX007: model_reasoning_effort is invalid
    CodexReasoningEffort,
    /// CX008: model_verbosity is invalid
    CodexModelVerbosity,
    /// CX009: personality is invalid
    CodexPersonality,
    /// CX010: danger-full-access acknowledgement is missing
    CodexFullAccessAcknowledgment,
    /// CX011: shell_environment_policy.inherit is invalid
    CodexShellEnvironmentInherit,
    /// CX012: an MCP server lacks a command or URL
    CodexMcpServerTransport,
    /// CX013: an MCP configuration contains a hardcoded secret
    CodexHardcodedSecret,
    /// CX014: cli_auth_credentials_store is invalid
    CodexCliCredentialsStore,
    /// CX015: sandbox_workspace_write.mode is invalid
    CodexWorkspaceWriteMode,
    /// CX016: model is not a string
    CodexModelType,
    /// CX017: model_provider is not a string
    CodexModelProviderType,
    /// CX018: model_reasoning_summary is invalid
    CodexReasoningSummary,
    /// CX019: history is not a TOML table
    CodexHistoryType,
    /// CX020: tui is not a TOML table
    CodexTuiType,
    /// CX021: file_opener is not a string
    CodexFileOpenerType,
    /// CX022: mcp_oauth_credentials_store is invalid
    CodexMcpCredentialsStore,
    /// CX023: model_context_window is not positive
    CodexContextWindow,
    /// CX024: model_auto_compact_token_limit is not positive
    CodexAutoCompactLimit,
    /// CX025: approval_policy table has an unknown field
    CodexApprovalPolicyField,
    /// CX026: approvals_reviewer is invalid
    CodexApprovalsReviewer,
    /// CX027: service_tier is invalid
    CodexServiceTier,
    /// CX028: inline MCP bearer_token is forbidden
    CodexInlineBearerToken,
    /// CX029: agents.max_threads conflicts with multi_agent_v2
    CodexMultiAgentThreadLimit,
    /// CX030: app default_tools_approval_mode is invalid
    CodexAppApprovalMode,
    /// CX031: skills is not a TOML table
    CodexSkillsType,
    /// CX032: profile is not a string
    CodexProfileType,
    /// CX033: unknown top-level Codex configuration key
    CodexTopLevelKey,
    /// CX034: unknown Codex feature flag
    CodexFeatureKey,
    /// CX035: unknown permissions.network field
    CodexNetworkPermissionField,
    /// CX036: windows.sandbox is invalid
    CodexWindowsSandbox,
    /// CX039: AGENTS.md exceeds Codex's hard size limit
    CodexAgentsTooLarge,
    /// CX040: AGENTS.md exceeds the configured Codex document budget
    CodexAgentsDocLimit,
    /// CX042: AGENTS.override.md is tracked by Git
    CodexAgentsOverrideTracked,
    /// CX045: AGENTS.md explicitly contradicts a Codex config value
    CodexAgentsConfigConflict,
    /// CX046: a Codex plugin manifest is not at the repository root
    CodexPluginManifestPath,
    /// CX047: .codex-plugin/plugin.json is not valid JSON
    CodexPluginManifestInvalid,
    /// CX048: Codex plugin manifest name is missing or blank
    CodexPluginNameMissing,
    /// CX049: Codex plugin manifest name contains invalid characters
    CodexPluginNameInvalid,
    /// CX050: Codex plugin component path lacks a ./ prefix
    CodexPluginPathPrefix,
    /// CX051: Codex plugin component path contains traversal
    CodexPluginPathTraversal,
    /// CX052: Codex plugin component path is a bare ./
    CodexPluginPathBare,
    /// CX053: Codex plugin has too many default prompts
    CodexPluginDefaultPromptCount,
    /// CX054: Codex plugin default prompt exceeds Codex's character limit
    CodexPluginDefaultPromptLength,
    /// CX055: Codex plugin default prompt is empty after whitespace normalization
    CodexPluginDefaultPromptEmpty,
    /// CX056: Codex plugin interface URL is not HTTP(S)
    CodexPluginInterfaceUrl,
    /// CX057: Codex plugin interface asset path is unsafe
    CodexPluginInterfaceAssetPath,
    /// CX058: Codex plugin manifest uses the unsupported hooks field
    CodexPluginHooksUnsupported,
    /// CX059: Codex plugin manifest description is missing or blank
    CodexPluginDescriptionMissing,
    /// CX060: a Codex skill uses Claude-only frontmatter
    CodexSkillUnsupportedFrontmatter,

    // ── Cursor configuration (CU / CR) ───────────────────────────
    /// CU001: Cursor rule file has no instructions
    CursorRuleEmpty,
    /// CU002: Cursor .mdc rule lacks YAML frontmatter
    CursorRuleFrontmatterMissing,
    /// CU003: Cursor rule frontmatter is invalid YAML
    CursorRuleFrontmatterInvalid,
    /// CU004: Cursor rule globs field contains an invalid pattern
    CursorRuleGlobInvalid,
    /// CU005: Cursor rule frontmatter contains an unknown field
    CursorRuleFieldUnknown,
    /// CU006: legacy .cursorrules file is present
    CursorLegacyRules,
    /// CU007: alwaysApply rule also declares globs
    CursorAlwaysApplyGlobs,
    /// CU008: alwaysApply is not a boolean
    CursorAlwaysApplyInvalid,
    /// CU009: agent-requested Cursor rule lacks a description
    CursorRuleDescriptionMissing,
    /// CU010: .cursor/hooks.json has an invalid schema
    CursorHooksSchemaInvalid,
    /// CU011: Cursor hook event is unknown
    CursorHookEventUnknown,
    /// CU012: Cursor hook entry lacks a command
    CursorHookCommandMissing,
    /// CU013: Cursor hook type is invalid
    CursorHookTypeInvalid,
    /// CU014: Cursor subagent frontmatter is invalid
    CursorAgentFrontmatterInvalid,
    /// CU015: Cursor subagent has no body
    CursorAgentBodyEmpty,
    /// CU016: .cursor/environment.json has an invalid schema
    CursorEnvironmentInvalid,
    /// CU017: Cursor hook entry field type is invalid
    CursorHookFieldTypeInvalid,
    /// CU018: Cursor prompt hook lacks prompt
    CursorPromptHookPromptMissing,
    /// CU019: Cursor prompt hook model is not a string
    CursorPromptHookModelInvalid,
    /// CR-SK-001: Cursor skill uses unsupported frontmatter
    CursorSkillFieldUnsupported,

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
    /// G008: GitHub body or release notes are passed inline
    GhInlineBody,
    /// G009: Bash global substitution uses an unsafe variable replacement
    BashReplacementUnsafe,
    /// G010: shipped shell uses syntax unavailable in Bash 3.2
    Bash32Incompatible,
    /// G011: dynamic awk regex contains non-ASCII text
    AwkRegexNonascii,

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
    /// U007: userConfig key is not a valid identifier
    UserconfigKeyInvalid,

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
    /// D004: CLAUDE.md import closure exceeds configured budget
    ClaudeImportLarge,
    /// D005: inline-code repository path does not exist
    InlinePathMissing,

    // ── MCP configuration (P) ──────────────────────────────────────
    /// P001: MCP configuration is not valid JSON
    McpJsonInvalid,
    /// P009: stdio MCP server is missing its command
    McpStdioCommandMissing,
    /// P010: HTTP/SSE MCP server is missing its URL
    McpHttpUrlMissing,
    /// P011: MCP server type is not supported
    McpTypeInvalid,
    /// P012: SSE transport is deprecated
    McpSseDeprecated,
    /// P017: non-local HTTP MCP URL is not HTTPS
    McpUrlNotHttps,
    /// P018: MCP environment contains a literal secret
    McpEnvSecretLiteral,
    /// P019: MCP command contains a dangerous shell pattern
    McpCommandDangerous,
    /// P022: MCP args is not an array of strings
    McpArgsInvalid,
    /// P023: mcpServers has duplicate server names
    McpDuplicateServer,
    /// P024: MCP server configuration is empty
    McpServerEmpty,
    /// P025: MCP alwaysLoad is not a boolean
    McpAlwaysLoadInvalid,
    /// P026: MCP server name is reserved by Claude Code
    McpServerReserved,

    // ── Link/import integrity (L) ────────────────────────────────
    /// L001: @import target markdown file does not exist
    ImportPathMissing,
    /// L002: circular @import chain detected
    CircularImport,
    /// L003: @import chain depth exceeds 5 hops
    ImportDepthExceeded,
    /// L004: duplicate @import of the same file
    DuplicateImport,
    /// L005: broken relative markdown link target
    BrokenMarkdownLink,
    /// L006: npm run script not defined in package.json
    NpmScriptMissing,
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
            Self::ComponentPathNested => "M012",
            Self::ComponentPathUnsafe => "M013",
            Self::AuthorNameMissing => "M014",
            Self::HomepageUrlInvalid => "M015",
            Self::LspServerInvalid => "M016",
            Self::ChannelServerMissing => "M017",

            Self::HooksJsonMissing => "H001",
            Self::HooksJsonInvalid => "H002",
            Self::HooksKeyMissing => "H003",
            Self::HookCommandMissing => "H004",
            Self::HookNotExecutable => "H005",
            Self::SettingsJsonInvalid => "H006",
            Self::HooksArrayEmpty => "H007",
            Self::HookEventInvalid => "H008",
            Self::HookMatcherInvalid => "H009",
            Self::HookTypeMissing => "H010",
            Self::HookTypeUnknown => "H011",
            Self::HookCommandRequired => "H012",
            Self::HookPromptRequired => "H013",
            Self::HookUrlRequired => "H014",
            Self::HookServerRequired => "H015",
            Self::HookToolRequired => "H016",
            Self::HookTimeoutInvalid => "H017",
            Self::HookAsyncInvalid => "H018",
            Self::HookModelInvalid => "H019",
            Self::HookOnceInvalid => "H020",
            Self::HookIfInvalid => "H021",
            Self::HookShellInvalid => "H022",
            Self::HookCommandDangerous => "H023",
            Self::HookHeadersInterpolated => "H024",
            Self::SettingsLocalInvalid => "H025",

            Self::FrontmatterYamlInvalid => "X001",
            Self::UnclosedCodeFence => "X002",
            Self::XmlTagUnclosed => "X003",
            Self::XmlTagMismatched => "X004",
            Self::XmlTagOrphan => "X005",

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
            Self::McpToolUnqualified => "S044",
            Self::ToolsListSyntax => "S045",
            Self::BodyNoWorkflow => "S046",
            Self::BodyNoExamples => "S047",
            Self::RefNameGeneric => "S048",
            Self::NameNotGerund => "S049",
            Self::DescVagueContent => "S050",
            Self::ScriptDepsMissing => "S051",
            Self::ScriptVerifyMissing => "S052",
            Self::TerminologyInconsistent => "S053",
            Self::DescBodyMisalign => "S054",
            Self::ScriptErrhandMissing => "S055",
            Self::BodyNoDefault => "S056",
            Self::MagicNumberUndoc => "S057",
            Self::SkillInvokeMissing => "S058",
            Self::SkillFlagMismatch => "S059",
            Self::AwkFieldRef => "S060",
            Self::UnsafeGrepProbe => "S061",
            Self::SkillClosureLarge => "S062",
            Self::ModelInvalid => "S063",
            Self::AgentNoFork => "S064",
            Self::AgentUnknown => "S065",
            Self::SideEffectAuto => "S066",
            Self::BashUnscoped => "S067",
            Self::InjectionOverflow => "S068",
            Self::HintNoArgs => "S069",
            Self::UnknownFmField => "S070",
            Self::PathsEmpty => "S071",
            Self::SkillDirOversized => "S072",
            Self::SkillRefNested => "S073",

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
            Self::AgentDescRedundant => "A011",
            Self::AgentReadMismatch => "A012",
            Self::AgentOutputUnsafe => "A013",
            Self::AgentModelInvalid => "A014",
            Self::AgentPermissionInvalid => "A015",
            Self::AgentSkillMissing => "A016",
            Self::AgentToolsOverlap => "A017",
            Self::AgentMemoryInvalid => "A018",
            Self::AgentToolsUnknown => "A019",
            Self::AgentDisallowedUnknown => "A020",
            Self::AgentBypassPermissions => "A021",
            Self::AgentSkillKebab => "A022",
            Self::AgentEffortInvalid => "A023",
            Self::AgentIsolationInvalid => "A024",
            Self::AgentBackgroundInvalid => "A025",
            Self::AgentMaxturnsInvalid => "A026",
            Self::AgentFieldUnknown => "A027",
            Self::AgentFieldUnsupported => "A028",

            Self::PromptGenericFiller => "Q001",
            Self::PromptNegativeOnly => "Q002",
            Self::PromptWeakCritical => "Q003",
            Self::ClaudeReadmeDuplicate => "Q004",

            Self::RulesGlobInvalid => "R001",
            Self::RulesFieldUnknown => "R002",
            Self::OutputStyleDescriptionMissing => "O001",
            Self::OutputStyleKeepCodingInstructionsInvalid => "O002",
            Self::OutputStyleFieldUnknown => "O003",
            Self::OutputStyleBodyEmpty => "O004",
            Self::OutputStyleNameTooLong => "O005",
            Self::OutputStyleFrontmatterInvalid => "O006",
            Self::SettingsPrUrlTemplateInvalid => "T001",
            Self::SettingsChannelsEnabledInvalid => "T002",

            Self::InstructionFileEmpty => "I001",
            Self::InstructionFileSecret => "I002",
            Self::InstructionFilePathMissing => "I003",
            Self::InstructionFileGenericGuidance => "I004",
            Self::InstructionFileMissingStructure => "I005",

            Self::CodexTomlInvalid => "CX001",
            Self::CodexProjectDocMaxBytes => "CX002",
            Self::CodexProjectDocFallbackNames => "CX003",
            Self::CodexUnknownNestedKey => "CX004",
            Self::CodexApprovalPolicy => "CX005",
            Self::CodexSandboxMode => "CX006",
            Self::CodexReasoningEffort => "CX007",
            Self::CodexModelVerbosity => "CX008",
            Self::CodexPersonality => "CX009",
            Self::CodexFullAccessAcknowledgment => "CX010",
            Self::CodexShellEnvironmentInherit => "CX011",
            Self::CodexMcpServerTransport => "CX012",
            Self::CodexHardcodedSecret => "CX013",
            Self::CodexCliCredentialsStore => "CX014",
            Self::CodexWorkspaceWriteMode => "CX015",
            Self::CodexModelType => "CX016",
            Self::CodexModelProviderType => "CX017",
            Self::CodexReasoningSummary => "CX018",
            Self::CodexHistoryType => "CX019",
            Self::CodexTuiType => "CX020",
            Self::CodexFileOpenerType => "CX021",
            Self::CodexMcpCredentialsStore => "CX022",
            Self::CodexContextWindow => "CX023",
            Self::CodexAutoCompactLimit => "CX024",
            Self::CodexApprovalPolicyField => "CX025",
            Self::CodexApprovalsReviewer => "CX026",
            Self::CodexServiceTier => "CX027",
            Self::CodexInlineBearerToken => "CX028",
            Self::CodexMultiAgentThreadLimit => "CX029",
            Self::CodexAppApprovalMode => "CX030",
            Self::CodexSkillsType => "CX031",
            Self::CodexProfileType => "CX032",
            Self::CodexTopLevelKey => "CX033",
            Self::CodexFeatureKey => "CX034",
            Self::CodexNetworkPermissionField => "CX035",
            Self::CodexWindowsSandbox => "CX036",
            Self::CodexAgentsTooLarge => "CX039",
            Self::CodexAgentsDocLimit => "CX040",
            Self::CodexAgentsOverrideTracked => "CX042",
            Self::CodexAgentsConfigConflict => "CX045",
            Self::CodexPluginManifestPath => "CX046",
            Self::CodexPluginManifestInvalid => "CX047",
            Self::CodexPluginNameMissing => "CX048",
            Self::CodexPluginNameInvalid => "CX049",
            Self::CodexPluginPathPrefix => "CX050",
            Self::CodexPluginPathTraversal => "CX051",
            Self::CodexPluginPathBare => "CX052",
            Self::CodexPluginDefaultPromptCount => "CX053",
            Self::CodexPluginDefaultPromptLength => "CX054",
            Self::CodexPluginDefaultPromptEmpty => "CX055",
            Self::CodexPluginInterfaceUrl => "CX056",
            Self::CodexPluginInterfaceAssetPath => "CX057",
            Self::CodexPluginHooksUnsupported => "CX058",
            Self::CodexPluginDescriptionMissing => "CX059",
            Self::CodexSkillUnsupportedFrontmatter => "CX060",

            Self::CursorRuleEmpty => "CU001",
            Self::CursorRuleFrontmatterMissing => "CU002",
            Self::CursorRuleFrontmatterInvalid => "CU003",
            Self::CursorRuleGlobInvalid => "CU004",
            Self::CursorRuleFieldUnknown => "CU005",
            Self::CursorLegacyRules => "CU006",
            Self::CursorAlwaysApplyGlobs => "CU007",
            Self::CursorAlwaysApplyInvalid => "CU008",
            Self::CursorRuleDescriptionMissing => "CU009",
            Self::CursorHooksSchemaInvalid => "CU010",
            Self::CursorHookEventUnknown => "CU011",
            Self::CursorHookCommandMissing => "CU012",
            Self::CursorHookTypeInvalid => "CU013",
            Self::CursorAgentFrontmatterInvalid => "CU014",
            Self::CursorAgentBodyEmpty => "CU015",
            Self::CursorEnvironmentInvalid => "CU016",
            Self::CursorHookFieldTypeInvalid => "CU017",
            Self::CursorPromptHookPromptMissing => "CU018",
            Self::CursorPromptHookModelInvalid => "CU019",
            Self::CursorSkillFieldUnsupported => "CR-SK-001",

            Self::PwdInSkill => "G001",
            Self::ScriptRefMissing => "G002",
            Self::ScriptNotExecutable => "G003",
            Self::DeadScript => "G004",
            Self::SecurityMdMissing => "G005",
            Self::TodoInSkill => "G006",
            Self::TodoInAgent => "G007",
            Self::GhInlineBody => "G008",
            Self::BashReplacementUnsafe => "G009",
            Self::Bash32Incompatible => "G010",
            Self::AwkRegexNonascii => "G011",

            Self::InvalidEmailFormat => "E001",

            Self::UserconfigNotObject => "U001",
            Self::UserconfigDescMissing => "U002",
            Self::UserconfigEnvMissing => "U003",
            Self::UserconfigSensitiveType => "U004",
            Self::UserconfigTitleMissing => "U005",
            Self::UserconfigTypeMissing => "U006",
            Self::UserconfigKeyInvalid => "U007",

            Self::SlackFallbackMismatch => "K001",

            Self::DocsRefMissing => "D001",
            Self::ClaudemdTooLarge => "D002",
            Self::TodoInDocs => "D003",
            Self::ClaudeImportLarge => "D004",
            Self::InlinePathMissing => "D005",

            Self::McpJsonInvalid => "P001",
            Self::McpStdioCommandMissing => "P009",
            Self::McpHttpUrlMissing => "P010",
            Self::McpTypeInvalid => "P011",
            Self::McpSseDeprecated => "P012",
            Self::McpUrlNotHttps => "P017",
            Self::McpEnvSecretLiteral => "P018",
            Self::McpCommandDangerous => "P019",
            Self::McpArgsInvalid => "P022",
            Self::McpDuplicateServer => "P023",
            Self::McpServerEmpty => "P024",
            Self::McpAlwaysLoadInvalid => "P025",
            Self::McpServerReserved => "P026",

            Self::ImportPathMissing => "L001",
            Self::CircularImport => "L002",
            Self::ImportDepthExceeded => "L003",
            Self::DuplicateImport => "L004",
            Self::BrokenMarkdownLink => "L005",
            Self::NpmScriptMissing => "L006",
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
            Self::ComponentPathNested => "component-path-nested",
            Self::ComponentPathUnsafe => "component-path-unsafe",
            Self::AuthorNameMissing => "author-name-missing",
            Self::HomepageUrlInvalid => "homepage-url-invalid",
            Self::LspServerInvalid => "lsp-server-invalid",
            Self::ChannelServerMissing => "channel-server-missing",

            Self::HooksJsonMissing => "hooks-json-missing",
            Self::HooksJsonInvalid => "hooks-json-invalid",
            Self::HooksKeyMissing => "hooks-key-missing",
            Self::HookCommandMissing => "hook-command-missing",
            Self::HookNotExecutable => "hook-not-executable",
            Self::SettingsJsonInvalid => "settings-json-invalid",
            Self::HooksArrayEmpty => "hooks-array-empty",
            Self::HookEventInvalid => "hook-event-invalid",
            Self::HookMatcherInvalid => "hook-matcher-invalid",
            Self::HookTypeMissing => "hook-type-missing",
            Self::HookTypeUnknown => "hook-type-unknown",
            Self::HookCommandRequired => "hook-command-required",
            Self::HookPromptRequired => "hook-prompt-required",
            Self::HookUrlRequired => "hook-url-required",
            Self::HookServerRequired => "hook-server-required",
            Self::HookToolRequired => "hook-tool-required",
            Self::HookTimeoutInvalid => "hook-timeout-invalid",
            Self::HookAsyncInvalid => "hook-async-invalid",
            Self::HookModelInvalid => "hook-model-invalid",
            Self::HookOnceInvalid => "hook-once-invalid",
            Self::HookIfInvalid => "hook-if-invalid",
            Self::HookShellInvalid => "hook-shell-invalid",
            Self::HookCommandDangerous => "hook-command-dangerous",
            Self::HookHeadersInterpolated => "hook-headers-interpolated",
            Self::SettingsLocalInvalid => "settings-local-invalid",

            Self::FrontmatterYamlInvalid => "frontmatter-yaml-invalid",
            Self::UnclosedCodeFence => "unclosed-code-fence",
            Self::XmlTagUnclosed => "xml-tag-unclosed",
            Self::XmlTagMismatched => "xml-tag-mismatched",
            Self::XmlTagOrphan => "xml-tag-orphan",

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
            Self::McpToolUnqualified => "mcp-tool-unqualified",
            Self::ToolsListSyntax => "tools-list-syntax",
            Self::BodyNoWorkflow => "body-no-workflow",
            Self::BodyNoExamples => "body-no-examples",
            Self::RefNameGeneric => "ref-name-generic",
            Self::NameNotGerund => "name-not-gerund",
            Self::DescVagueContent => "desc-vague-content",
            Self::ScriptDepsMissing => "script-deps-missing",
            Self::ScriptVerifyMissing => "script-verify-missing",
            Self::TerminologyInconsistent => "terminology-inconsistent",
            Self::DescBodyMisalign => "desc-body-misalign",
            Self::ScriptErrhandMissing => "script-errhand-missing",
            Self::BodyNoDefault => "body-no-default",
            Self::MagicNumberUndoc => "magic-number-undoc",
            Self::SkillInvokeMissing => "skill-invoke-missing",
            Self::SkillFlagMismatch => "skill-flag-mismatch",
            Self::AwkFieldRef => "awk-field-ref",
            Self::UnsafeGrepProbe => "unsafe-grep-probe",
            Self::SkillClosureLarge => "skill-closure-large",
            Self::ModelInvalid => "model-invalid",
            Self::AgentNoFork => "agent-no-fork",
            Self::AgentUnknown => "agent-unknown",
            Self::SideEffectAuto => "side-effect-auto",
            Self::BashUnscoped => "bash-unscoped",
            Self::InjectionOverflow => "injection-overflow",
            Self::HintNoArgs => "hint-no-args",
            Self::UnknownFmField => "unknown-fm-field",
            Self::PathsEmpty => "paths-empty",
            Self::SkillDirOversized => "skill-dir-oversized",
            Self::SkillRefNested => "skill-ref-nested",

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
            Self::AgentDescRedundant => "agent-desc-redundant",
            Self::AgentReadMismatch => "agent-read-mismatch",
            Self::AgentOutputUnsafe => "agent-output-unsafe",
            Self::AgentModelInvalid => "agent-model-invalid",
            Self::AgentPermissionInvalid => "agent-permission-invalid",
            Self::AgentSkillMissing => "agent-skill-missing",
            Self::AgentToolsOverlap => "agent-tools-overlap",
            Self::AgentMemoryInvalid => "agent-memory-invalid",
            Self::AgentToolsUnknown => "agent-tools-unknown",
            Self::AgentDisallowedUnknown => "agent-disallowed-unknown",
            Self::AgentBypassPermissions => "agent-bypass-permissions",
            Self::AgentSkillKebab => "agent-skill-kebab",
            Self::AgentEffortInvalid => "agent-effort-invalid",
            Self::AgentIsolationInvalid => "agent-isolation-invalid",
            Self::AgentBackgroundInvalid => "agent-background-invalid",
            Self::AgentMaxturnsInvalid => "agent-maxturns-invalid",
            Self::AgentFieldUnknown => "agent-field-unknown",
            Self::AgentFieldUnsupported => "agent-field-unsupported",

            Self::PromptGenericFiller => "prompt-generic-filler",
            Self::PromptNegativeOnly => "prompt-negative-only",
            Self::PromptWeakCritical => "prompt-weak-critical",
            Self::ClaudeReadmeDuplicate => "claude-readme-duplicate",

            Self::RulesGlobInvalid => "rules-glob-invalid",
            Self::RulesFieldUnknown => "rules-field-unknown",
            Self::OutputStyleDescriptionMissing => "style-description-missing",
            Self::OutputStyleKeepCodingInstructionsInvalid => "style-instructions-invalid",
            Self::OutputStyleFieldUnknown => "style-field-unknown",
            Self::OutputStyleBodyEmpty => "style-body-empty",
            Self::OutputStyleNameTooLong => "style-name-long",
            Self::OutputStyleFrontmatterInvalid => "style-frontmatter-invalid",
            Self::SettingsPrUrlTemplateInvalid => "pr-template-invalid",
            Self::SettingsChannelsEnabledInvalid => "channels-enabled-invalid",

            Self::InstructionFileEmpty => "instruction-file-empty",
            Self::InstructionFileSecret => "instruction-file-secret",
            Self::InstructionFilePathMissing => "instruction-file-path",
            Self::InstructionFileGenericGuidance => "instruction-file-generic",
            Self::InstructionFileMissingStructure => "instruction-file-structure",

            Self::CodexTomlInvalid => "codex-toml-invalid",
            Self::CodexProjectDocMaxBytes => "codex-doc-bytes",
            Self::CodexProjectDocFallbackNames => "codex-doc-names",
            Self::CodexUnknownNestedKey => "codex-key-unknown",
            Self::CodexApprovalPolicy => "codex-approval-policy",
            Self::CodexSandboxMode => "codex-sandbox-mode",
            Self::CodexReasoningEffort => "codex-reasoning-effort",
            Self::CodexModelVerbosity => "codex-model-verbosity",
            Self::CodexPersonality => "codex-personality",
            Self::CodexFullAccessAcknowledgment => "codex-access-ack",
            Self::CodexShellEnvironmentInherit => "codex-shell-inherit",
            Self::CodexMcpServerTransport => "codex-mcp-transport",
            Self::CodexHardcodedSecret => "codex-secret-literal",
            Self::CodexCliCredentialsStore => "codex-cli-credentials",
            Self::CodexWorkspaceWriteMode => "codex-write-mode",
            Self::CodexModelType => "codex-model-type",
            Self::CodexModelProviderType => "codex-provider-type",
            Self::CodexReasoningSummary => "codex-reasoning-summary",
            Self::CodexHistoryType => "codex-history-type",
            Self::CodexTuiType => "codex-tui-type",
            Self::CodexFileOpenerType => "codex-opener-type",
            Self::CodexMcpCredentialsStore => "codex-mcp-credentials",
            Self::CodexContextWindow => "codex-context-window",
            Self::CodexAutoCompactLimit => "codex-compact-limit",
            Self::CodexApprovalPolicyField => "codex-approval-field",
            Self::CodexApprovalsReviewer => "codex-approval-reviewer",
            Self::CodexServiceTier => "codex-service-tier",
            Self::CodexInlineBearerToken => "codex-bearer-token",
            Self::CodexMultiAgentThreadLimit => "codex-agent-threads",
            Self::CodexAppApprovalMode => "codex-app-approval",
            Self::CodexSkillsType => "codex-skills-type",
            Self::CodexProfileType => "codex-profile-type",
            Self::CodexTopLevelKey => "codex-top-key",
            Self::CodexFeatureKey => "codex-feature-key",
            Self::CodexNetworkPermissionField => "codex-network-field",
            Self::CodexWindowsSandbox => "codex-windows-sandbox",
            Self::CodexAgentsTooLarge => "codex-agents-large",
            Self::CodexAgentsDocLimit => "codex-agents-limit",
            Self::CodexAgentsOverrideTracked => "codex-agents-override",
            Self::CodexAgentsConfigConflict => "codex-agents-conflict",
            Self::CodexPluginManifestPath => "codex-plugin-path",
            Self::CodexPluginManifestInvalid => "codex-plugin-invalid",
            Self::CodexPluginNameMissing => "codex-name-missing",
            Self::CodexPluginNameInvalid => "codex-name-invalid",
            Self::CodexPluginPathPrefix => "codex-path-prefix",
            Self::CodexPluginPathTraversal => "codex-path-traversal",
            Self::CodexPluginPathBare => "codex-path-bare",
            Self::CodexPluginDefaultPromptCount => "codex-prompt-count",
            Self::CodexPluginDefaultPromptLength => "codex-prompt-length",
            Self::CodexPluginDefaultPromptEmpty => "codex-prompt-empty",
            Self::CodexPluginInterfaceUrl => "codex-plugin-url",
            Self::CodexPluginInterfaceAssetPath => "codex-plugin-asset",
            Self::CodexPluginHooksUnsupported => "codex-plugin-hooks",
            Self::CodexPluginDescriptionMissing => "codex-plugin-description",
            Self::CodexSkillUnsupportedFrontmatter => "codex-skill-frontmatter",

            Self::CursorRuleEmpty => "cursor-rule-empty",
            Self::CursorRuleFrontmatterMissing => "cursor-frontmatter-missing",
            Self::CursorRuleFrontmatterInvalid => "cursor-frontmatter-invalid",
            Self::CursorRuleGlobInvalid => "cursor-glob-invalid",
            Self::CursorRuleFieldUnknown => "cursor-field-unknown",
            Self::CursorLegacyRules => "cursor-legacy-rules",
            Self::CursorAlwaysApplyGlobs => "cursor-always-globs",
            Self::CursorAlwaysApplyInvalid => "cursor-always-invalid",
            Self::CursorRuleDescriptionMissing => "cursor-description-missing",
            Self::CursorHooksSchemaInvalid => "cursor-hooks-invalid",
            Self::CursorHookEventUnknown => "cursor-event-unknown",
            Self::CursorHookCommandMissing => "cursor-command-missing",
            Self::CursorHookTypeInvalid => "cursor-type-invalid",
            Self::CursorAgentFrontmatterInvalid => "cursor-agent-invalid",
            Self::CursorAgentBodyEmpty => "cursor-body-empty",
            Self::CursorEnvironmentInvalid => "cursor-environment-invalid",
            Self::CursorHookFieldTypeInvalid => "cursor-hook-invalid",
            Self::CursorPromptHookPromptMissing => "cursor-prompt-missing",
            Self::CursorPromptHookModelInvalid => "cursor-model-invalid",
            Self::CursorSkillFieldUnsupported => "cursor-skill-unsupported",

            Self::PwdInSkill => "pwd-in-skill",
            Self::ScriptRefMissing => "script-ref-missing",
            Self::ScriptNotExecutable => "script-not-executable",
            Self::DeadScript => "dead-script",
            Self::SecurityMdMissing => "security-md-missing",
            Self::TodoInSkill => "todo-in-skill",
            Self::TodoInAgent => "todo-in-agent",
            Self::GhInlineBody => "gh-inline-body",
            Self::BashReplacementUnsafe => "bash-replacement-unsafe",
            Self::Bash32Incompatible => "bash32-incompatible",
            Self::AwkRegexNonascii => "awk-regex-nonascii",

            Self::InvalidEmailFormat => "invalid-email-format",

            Self::UserconfigNotObject => "userconfig-not-object",
            Self::UserconfigDescMissing => "userconfig-desc-missing",
            Self::UserconfigEnvMissing => "userconfig-env-missing",
            Self::UserconfigSensitiveType => "userconfig-sensitive-type",
            Self::UserconfigTitleMissing => "userconfig-title-missing",
            Self::UserconfigTypeMissing => "userconfig-type-missing",
            Self::UserconfigKeyInvalid => "userconfig-key-invalid",

            Self::SlackFallbackMismatch => "slack-fallback-mismatch",

            Self::DocsRefMissing => "docs-ref-missing",
            Self::ClaudemdTooLarge => "claudemd-too-large",
            Self::TodoInDocs => "todo-in-docs",
            Self::ClaudeImportLarge => "claude-import-large",
            Self::InlinePathMissing => "inline-path-missing",

            Self::McpJsonInvalid => "mcp-json-invalid",
            Self::McpStdioCommandMissing => "mcp-stdio-command",
            Self::McpHttpUrlMissing => "mcp-http-url",
            Self::McpTypeInvalid => "mcp-type-invalid",
            Self::McpSseDeprecated => "mcp-sse-deprecated",
            Self::McpUrlNotHttps => "mcp-insecure-url",
            Self::McpEnvSecretLiteral => "mcp-env-secret",
            Self::McpCommandDangerous => "mcp-command-dangerous",
            Self::McpArgsInvalid => "mcp-args-invalid",
            Self::McpDuplicateServer => "mcp-duplicate-server",
            Self::McpServerEmpty => "mcp-server-empty",
            Self::McpAlwaysLoadInvalid => "mcp-alwaysload-invalid",
            Self::McpServerReserved => "mcp-server-reserved",

            Self::ImportPathMissing => "import-path-missing",
            Self::CircularImport => "circular-import",
            Self::ImportDepthExceeded => "import-depth-exceeded",
            Self::DuplicateImport => "duplicate-import",
            Self::BrokenMarkdownLink => "broken-markdown-link",
            Self::NpmScriptMissing => "npm-script-missing",
        }
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
            "CX037" | "codex-agents-empty" => Some(Self::InstructionFileEmpty),
            "CX038" | "codex-agents-secret" => Some(Self::InstructionFileSecret),
            "CX041" | "codex-agents-path" => Some(Self::InstructionFilePathMissing),
            "CX043" | "codex-agents-generic" => Some(Self::InstructionFileGenericGuidance),
            "CX044" | "codex-agents-structure" => Some(Self::InstructionFileMissingStructure),
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
                | Self::NameHasXml
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
            Self::NameNotGerund | Self::BodyNoExamples |
            Self::BodyTooLong | Self::Bash32Incompatible |
            Self::AwkRegexNonascii | Self::CodexNetworkPermissionField |
            Self::CodexWindowsSandbox | Self::InstructionFileGenericGuidance |
            Self::InstructionFileMissingStructure | Self::CodexAgentsConfigConflict |
            Self::PromptNegativeOnly | Self::PromptWeakCritical |
            Self::ClaudeReadmeDuplicate | Self::CursorPromptHookModelInvalid |
            Self::SkillRefNested
                => DefaultSeverity::Suppressed,

            // ── Default-warning: enriched metadata ───────────────────
            Self::MarketplaceEnrichedMissing | Self::PluginEnrichedMissing |

            // ── Default-warning: hook schema advisories ──────────────
            Self::HookIfInvalid | Self::HookShellInvalid |
            Self::HookCommandDangerous | Self::HookHeadersInterpolated |

            // ── Default-warning: optional manifest sections ──────────
            Self::AuthorNameMissing | Self::HomepageUrlInvalid |
            Self::ChannelServerMissing |

            // ── Default-warning: style / quality (skills) ────────────
            Self::DescTruncated | Self::ConsecutiveBash |
            Self::NameVague | Self::DescTooShort | Self::BodyNoRefs |
            Self::BodyNoWorkflow | Self::RefNameGeneric |
            Self::DescVagueContent | Self::ScriptDepsMissing |
            Self::ScriptVerifyMissing | Self::TerminologyInconsistent |
            Self::DescBodyMisalign | Self::ScriptErrhandMissing |
            Self::BodyNoDefault | Self::MagicNumberUndoc |
            Self::SkillClosureLarge | Self::PromptGenericFiller |

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
            Self::AgentFieldUnsupported |

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
            Self::CodexServiceTier | Self::CodexSkillsType | Self::CodexProfileType |
            Self::CodexTopLevelKey | Self::CodexFeatureKey |
            Self::CodexAgentsTooLarge | Self::CodexAgentsDocLimit |
            Self::InstructionFilePathMissing | Self::CodexAgentsOverrideTracked |
            Self::CodexPluginDefaultPromptCount | Self::CodexPluginDefaultPromptLength |
            Self::CodexPluginDefaultPromptEmpty | Self::CodexPluginInterfaceUrl |
            Self::CodexPluginHooksUnsupported | Self::CodexPluginDescriptionMissing |
            Self::CodexSkillUnsupportedFrontmatter |
            Self::CursorRuleFrontmatterMissing | Self::CursorRuleFieldUnknown |
            Self::CursorLegacyRules | Self::CursorAlwaysApplyGlobs |
            Self::CursorRuleDescriptionMissing | Self::CursorHookEventUnknown |
            Self::CursorHookFieldTypeInvalid | Self::CursorPromptHookPromptMissing |
            Self::CursorAgentBodyEmpty | Self::CursorSkillFieldUnsupported |

            // ── Default-warning: user config ─────────────────────────
            Self::UserconfigKeyInvalid |

            // ── Default-warning: hygiene ─────────────────────────────
            Self::SecurityMdMissing | Self::TodoInSkill | Self::TodoInAgent |
            Self::GhInlineBody |

            // ── Default-warning: Slack ───────────────────────────────
            Self::SlackFallbackMismatch |

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
            Self::NpmScriptMissing
                => DefaultSeverity::Warning,

            // Everything else defaults to error.
            _ => DefaultSeverity::Error,
        }
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
    LintRule::ComponentPathNested,
    LintRule::ComponentPathUnsafe,
    LintRule::AuthorNameMissing,
    LintRule::HomepageUrlInvalid,
    LintRule::LspServerInvalid,
    LintRule::ChannelServerMissing,
    LintRule::HooksJsonMissing,
    LintRule::HooksJsonInvalid,
    LintRule::HooksKeyMissing,
    LintRule::HookCommandMissing,
    LintRule::HookNotExecutable,
    LintRule::SettingsJsonInvalid,
    LintRule::HooksArrayEmpty,
    LintRule::HookEventInvalid,
    LintRule::HookMatcherInvalid,
    LintRule::HookTypeMissing,
    LintRule::HookTypeUnknown,
    LintRule::HookCommandRequired,
    LintRule::HookPromptRequired,
    LintRule::HookUrlRequired,
    LintRule::HookServerRequired,
    LintRule::HookToolRequired,
    LintRule::HookTimeoutInvalid,
    LintRule::HookAsyncInvalid,
    LintRule::HookModelInvalid,
    LintRule::HookOnceInvalid,
    LintRule::HookIfInvalid,
    LintRule::HookShellInvalid,
    LintRule::HookCommandDangerous,
    LintRule::HookHeadersInterpolated,
    LintRule::SettingsLocalInvalid,
    LintRule::FrontmatterYamlInvalid,
    LintRule::UnclosedCodeFence,
    LintRule::XmlTagUnclosed,
    LintRule::XmlTagMismatched,
    LintRule::XmlTagOrphan,
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
    LintRule::McpToolUnqualified,
    LintRule::ToolsListSyntax,
    LintRule::BodyNoWorkflow,
    LintRule::BodyNoExamples,
    LintRule::RefNameGeneric,
    LintRule::NameNotGerund,
    LintRule::DescVagueContent,
    LintRule::ScriptDepsMissing,
    LintRule::ScriptVerifyMissing,
    LintRule::TerminologyInconsistent,
    LintRule::DescBodyMisalign,
    LintRule::ScriptErrhandMissing,
    LintRule::BodyNoDefault,
    LintRule::MagicNumberUndoc,
    LintRule::SkillInvokeMissing,
    LintRule::SkillFlagMismatch,
    LintRule::AwkFieldRef,
    LintRule::UnsafeGrepProbe,
    LintRule::SkillClosureLarge,
    LintRule::ModelInvalid,
    LintRule::AgentNoFork,
    LintRule::AgentUnknown,
    LintRule::SideEffectAuto,
    LintRule::BashUnscoped,
    LintRule::InjectionOverflow,
    LintRule::HintNoArgs,
    LintRule::UnknownFmField,
    LintRule::PathsEmpty,
    LintRule::SkillDirOversized,
    LintRule::SkillRefNested,
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
    LintRule::AgentDescRedundant,
    LintRule::AgentReadMismatch,
    LintRule::AgentOutputUnsafe,
    LintRule::AgentModelInvalid,
    LintRule::AgentPermissionInvalid,
    LintRule::AgentSkillMissing,
    LintRule::AgentToolsOverlap,
    LintRule::AgentMemoryInvalid,
    LintRule::AgentToolsUnknown,
    LintRule::AgentDisallowedUnknown,
    LintRule::AgentBypassPermissions,
    LintRule::AgentSkillKebab,
    LintRule::AgentEffortInvalid,
    LintRule::AgentIsolationInvalid,
    LintRule::AgentBackgroundInvalid,
    LintRule::AgentMaxturnsInvalid,
    LintRule::AgentFieldUnknown,
    LintRule::AgentFieldUnsupported,
    LintRule::PromptGenericFiller,
    LintRule::PromptNegativeOnly,
    LintRule::PromptWeakCritical,
    LintRule::ClaudeReadmeDuplicate,
    LintRule::RulesGlobInvalid,
    LintRule::RulesFieldUnknown,
    LintRule::OutputStyleDescriptionMissing,
    LintRule::OutputStyleKeepCodingInstructionsInvalid,
    LintRule::OutputStyleFieldUnknown,
    LintRule::OutputStyleBodyEmpty,
    LintRule::OutputStyleNameTooLong,
    LintRule::OutputStyleFrontmatterInvalid,
    LintRule::SettingsPrUrlTemplateInvalid,
    LintRule::SettingsChannelsEnabledInvalid,
    LintRule::InstructionFileEmpty,
    LintRule::InstructionFileSecret,
    LintRule::InstructionFilePathMissing,
    LintRule::InstructionFileGenericGuidance,
    LintRule::InstructionFileMissingStructure,
    LintRule::CodexTomlInvalid,
    LintRule::CodexProjectDocMaxBytes,
    LintRule::CodexProjectDocFallbackNames,
    LintRule::CodexUnknownNestedKey,
    LintRule::CodexApprovalPolicy,
    LintRule::CodexSandboxMode,
    LintRule::CodexReasoningEffort,
    LintRule::CodexModelVerbosity,
    LintRule::CodexPersonality,
    LintRule::CodexFullAccessAcknowledgment,
    LintRule::CodexShellEnvironmentInherit,
    LintRule::CodexMcpServerTransport,
    LintRule::CodexHardcodedSecret,
    LintRule::CodexCliCredentialsStore,
    LintRule::CodexWorkspaceWriteMode,
    LintRule::CodexModelType,
    LintRule::CodexModelProviderType,
    LintRule::CodexReasoningSummary,
    LintRule::CodexHistoryType,
    LintRule::CodexTuiType,
    LintRule::CodexFileOpenerType,
    LintRule::CodexMcpCredentialsStore,
    LintRule::CodexContextWindow,
    LintRule::CodexAutoCompactLimit,
    LintRule::CodexApprovalPolicyField,
    LintRule::CodexApprovalsReviewer,
    LintRule::CodexServiceTier,
    LintRule::CodexInlineBearerToken,
    LintRule::CodexMultiAgentThreadLimit,
    LintRule::CodexAppApprovalMode,
    LintRule::CodexSkillsType,
    LintRule::CodexProfileType,
    LintRule::CodexTopLevelKey,
    LintRule::CodexFeatureKey,
    LintRule::CodexNetworkPermissionField,
    LintRule::CodexWindowsSandbox,
    LintRule::CodexAgentsTooLarge,
    LintRule::CodexAgentsDocLimit,
    LintRule::CodexAgentsOverrideTracked,
    LintRule::CodexAgentsConfigConflict,
    LintRule::CodexPluginManifestPath,
    LintRule::CodexPluginManifestInvalid,
    LintRule::CodexPluginNameMissing,
    LintRule::CodexPluginNameInvalid,
    LintRule::CodexPluginPathPrefix,
    LintRule::CodexPluginPathTraversal,
    LintRule::CodexPluginPathBare,
    LintRule::CodexPluginDefaultPromptCount,
    LintRule::CodexPluginDefaultPromptLength,
    LintRule::CodexPluginDefaultPromptEmpty,
    LintRule::CodexPluginInterfaceUrl,
    LintRule::CodexPluginInterfaceAssetPath,
    LintRule::CodexPluginHooksUnsupported,
    LintRule::CodexPluginDescriptionMissing,
    LintRule::CodexSkillUnsupportedFrontmatter,
    LintRule::CursorRuleEmpty,
    LintRule::CursorRuleFrontmatterMissing,
    LintRule::CursorRuleFrontmatterInvalid,
    LintRule::CursorRuleGlobInvalid,
    LintRule::CursorRuleFieldUnknown,
    LintRule::CursorLegacyRules,
    LintRule::CursorAlwaysApplyGlobs,
    LintRule::CursorAlwaysApplyInvalid,
    LintRule::CursorRuleDescriptionMissing,
    LintRule::CursorHooksSchemaInvalid,
    LintRule::CursorHookEventUnknown,
    LintRule::CursorHookCommandMissing,
    LintRule::CursorHookTypeInvalid,
    LintRule::CursorAgentFrontmatterInvalid,
    LintRule::CursorAgentBodyEmpty,
    LintRule::CursorEnvironmentInvalid,
    LintRule::CursorHookFieldTypeInvalid,
    LintRule::CursorPromptHookPromptMissing,
    LintRule::CursorPromptHookModelInvalid,
    LintRule::CursorSkillFieldUnsupported,
    LintRule::PwdInSkill,
    LintRule::ScriptRefMissing,
    LintRule::ScriptNotExecutable,
    LintRule::DeadScript,
    LintRule::SecurityMdMissing,
    LintRule::TodoInSkill,
    LintRule::TodoInAgent,
    LintRule::GhInlineBody,
    LintRule::BashReplacementUnsafe,
    LintRule::Bash32Incompatible,
    LintRule::AwkRegexNonascii,
    LintRule::InvalidEmailFormat,
    LintRule::UserconfigNotObject,
    LintRule::UserconfigDescMissing,
    LintRule::UserconfigEnvMissing,
    LintRule::UserconfigSensitiveType,
    LintRule::UserconfigTitleMissing,
    LintRule::UserconfigTypeMissing,
    LintRule::UserconfigKeyInvalid,
    LintRule::SlackFallbackMismatch,
    LintRule::DocsRefMissing,
    LintRule::ClaudemdTooLarge,
    LintRule::TodoInDocs,
    LintRule::ClaudeImportLarge,
    LintRule::InlinePathMissing,
    LintRule::McpJsonInvalid,
    LintRule::McpStdioCommandMissing,
    LintRule::McpHttpUrlMissing,
    LintRule::McpTypeInvalid,
    LintRule::McpSseDeprecated,
    LintRule::McpUrlNotHttps,
    LintRule::McpEnvSecretLiteral,
    LintRule::McpCommandDangerous,
    LintRule::McpArgsInvalid,
    LintRule::McpDuplicateServer,
    LintRule::McpServerEmpty,
    LintRule::McpAlwaysLoadInvalid,
    LintRule::McpServerReserved,
    LintRule::ImportPathMissing,
    LintRule::CircularImport,
    LintRule::ImportDepthExceeded,
    LintRule::DuplicateImport,
    LintRule::BrokenMarkdownLink,
    LintRule::NpmScriptMissing,
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
            286,
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
    fn migrated_codex_agents_identifiers_resolve_to_shared_rules() {
        for (identifier, expected) in [
            ("CX037", LintRule::InstructionFileEmpty),
            ("codex-agents-secret", LintRule::InstructionFileSecret),
            ("CX041", LintRule::InstructionFilePathMissing),
            (
                "codex-agents-generic",
                LintRule::InstructionFileGenericGuidance,
            ),
            ("CX044", LintRule::InstructionFileMissingStructure),
        ] {
            assert_eq!(LintRule::from_code_or_name(identifier), Some(expected));
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
    fn default_suppressed_count() {
        let suppressed: Vec<_> = ALL_RULES
            .iter()
            .filter(|r| r.default_severity() == DefaultSeverity::Suppressed)
            .collect();
        assert_eq!(
            suppressed.len(),
            15,
            "Expected 15 default-suppressed rules, got {}",
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
            111,
            "Expected 111 default-warning rules, got {}",
            warnings.len()
        );
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
            12,
            "Expected 12 auto-fixable rules, got {}",
            fixable.len()
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
            160,
            "Expected 160 default-error rules, got {}",
            errors.len()
        );
    }
}
