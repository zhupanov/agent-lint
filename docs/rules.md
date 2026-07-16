# Lint Rules Reference

Agent Lint ships 225 rules across 14 categories. Every rule has a unique
code (e.g., `M001`) and a human-readable name (e.g., `plugin-json-missing`).
Either form can be used in `agent-lint.toml` to configure rule severity.

**Default column key:**

- **error** -- rule fires as an error by default
- **warn** -- rule fires as a warning by default (non-blocking)
- **suppressed** -- rule is silently skipped by default (enable via `[lint] error`)

**Strictness modes** (`--pedantic` / `--all`) override these defaults.
`--pedantic` promotes warnings (both `warn`-listed and default-warning
rules) to errors, except for suppressed rules and too-long rules
(`name-too-long`, `desc-too-long`, `body-too-long`, `compat-too-long`).
`--all` forces
every rule to error regardless of config. See
[configuration](configuration.md) for details.

**Mode column key:**

- **Plugin** -- runs only when `.claude-plugin/` is present
- **Always** -- runs in both Basic (`.claude/` or MCP configuration) and Plugin modes

## Manifest Rules (M)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| M001 | `plugin-json-missing` | `.claude-plugin/plugin.json` is missing | Plugin | error |
| M002 | `plugin-json-invalid` | `plugin.json` is not valid JSON | Plugin | error |
| M003 | `plugin-field-missing` | `plugin.json` missing required field (`name` or `version`). A `name` that is absent, empty, or whitespace-only all count as missing | Plugin | error |
| M004 | `plugin-version-format` | `plugin.json` version is not strict `MAJOR.MINOR.PATCH` semver | Plugin | error |
| M005 | `marketplace-json-missing` | `marketplace.json` is missing | Plugin | error |
| M006 | `marketplace-json-invalid` | `marketplace.json` is not valid JSON | Plugin | error |
| M007 | `marketplace-field-missing` | `marketplace.json` missing required field (`name` or `owner.name`) | Plugin | error |
| M008 | `marketplace-plugins-empty` | `marketplace.json` plugins array is empty | Plugin | error |
| M009 | `marketplace-plugin-invalid` | `marketplace.json` plugin entry has invalid `name` or `source` | Plugin | error |
| M010 | `marketplace-enriched-missing` | `marketplace.json` missing `owner.email` or plugin `category` | Plugin | warn |
| M011 | `plugin-enriched-missing` | `plugin.json` missing `description`, `author.email`, or `keywords` | Plugin | warn |
| M012 | `component-path-nested` | A component (`commands`/`agents`/`skills`/`hooks`) lives inside `.claude-plugin/`, or a manifest path points there | Plugin | error |
| M013 | `component-path-unsafe` | Manifest component path is absolute (`/…`, `C:\…`) or uses `..` traversal | Plugin | error |
| M014 | `author-name-missing` | `plugin.json` `author` object present but `author.name` is missing or not a non-empty string | Plugin | warn |
| M015 | `homepage-url-invalid` | `plugin.json` `homepage` is present but is not a valid http(s) URL | Plugin | warn |
| M016 | `lsp-server-invalid` | `plugin.json` `lspServers` entry missing `command` or `extensionToLanguage` | Plugin | error |
| M017 | `channel-server-missing` | `plugin.json` `channels` entry does not reference a `server` | Plugin | warn |

## Hooks Rules (H)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| H001 | `hooks-json-missing` | `hooks/hooks.json` is missing | Plugin | error |
| H002 | `hooks-json-invalid` | `hooks/hooks.json` is not valid JSON | Plugin | error |
| H003 | `hooks-key-missing` | `hooks.json` missing top-level `hooks` key | Plugin | error |
| H004 | `hook-command-missing` | Hook command script missing on disk | Always | error |
| H005 | `hook-not-executable` | Hook command script not executable | Always | error |
| H006 | `settings-json-invalid` | `.claude/settings.json` is not valid JSON | Always | error |
| H007 | `hooks-array-empty` | `hooks.json` has empty `hooks` array | Plugin | error |
| H008 | `hook-event-invalid` | Hook event name is not a recognized Claude Code event | Always | error |
| H009 | `hook-matcher-invalid` | `matcher` present on an event that takes no matcher | Always | error |
| H010 | `hook-type-missing` | Hook object missing required `type` field | Always | error |
| H011 | `hook-type-unknown` | Hook `type` is not `command`/`prompt`/`agent`/`http`/`mcp_tool` | Always | error |
| H012 | `hook-command-required` | `type: command` hook missing `command` | Always | error |
| H013 | `hook-prompt-required` | `type: prompt` or `type: agent` hook missing `prompt` | Always | error |
| H014 | `hook-url-required` | `type: http` hook missing `url` | Always | error |
| H015 | `hook-server-required` | `type: mcp_tool` hook missing `server` | Always | error |
| H016 | `hook-tool-required` | `type: mcp_tool` hook missing `tool` | Always | error |
| H017 | `hook-timeout-invalid` | Hook `timeout` is not a positive integer | Always | error |
| H018 | `hook-async-invalid` | `async: true` on a non-`command` hook | Always | error |
| H019 | `hook-model-invalid` | `model` on a hook other than `prompt`/`agent` | Always | error |
| H020 | `hook-once-invalid` | Hook `once` is not a boolean | Always | error |
| H021 | `hook-if-invalid` | Hook `if` is not a non-empty string or is used outside a tool event | Always | warn |
| H022 | `hook-shell-invalid` | Hook `shell` is not `bash`/`powershell` | Always | warn |
| H023 | `hook-command-dangerous` | Dangerous command pattern in hook command (`rm -rf`, `git reset --hard`, `curl \| sh`, ...) | Always | warn |
| H024 | `hook-headers-interpolated` | HTTP hook headers interpolate `$VAR` without `allowedEnvVars` | Always | warn |
| H025 | `settings-local-invalid` | `.claude/settings.local.json` is not valid JSON | Always | error |

### Hook schema validation (H008--H024)

H008--H024 share one hook-object validation engine, applied to
`hooks/hooks.json`, `.claude/settings.json`, and `.claude/settings.local.json`.
The engine walks the event-keyed shape:

```json
{"hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "..."}]}]}}
```

`hooks` object -> event name key -> matcher groups -> each group's nested
`hooks` array -> hook objects. A file whose `hooks` key is a flat array carries
no event context, so the schema engine skips it; only H001--H007 apply there.

The valid event list and handler-type table live in
`src/validators/hook_schema.rs` and track the
[Claude Code hooks reference](https://code.claude.com/docs/en/hooks.md);
expect them to change with Claude Code releases.

H009 uses an explicit list of the events the hooks reference marks "no matcher
support": `UserPromptSubmit`, `PostToolBatch`, `Stop`, `TeammateIdle`,
`TaskCreated`, `TaskCompleted`, `CwdChanged`, `MessageDisplay`,
`WorktreeCreate`, and `WorktreeRemove`. Every other event filters on some
documented field -- not just the tool events, but also `SessionStart` (how the
session started), `SessionEnd` (exit reason), `PreCompact`/`PostCompact`
(`manual`/`auto`), `SubagentStop` (agent type), and `InstructionsLoaded` (load
reason) -- so a blanket "non-tool event" check would flag valid configs.

Hook `hooks:` keys in skill and agent frontmatter are not yet validated; that
requires structured YAML frontmatter parsing.

## Skills Rules (S)

### Structure and Frontmatter (S001--S008)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| S001 | `skills-dir-missing` | `skills/` directory is missing (deprecated — no longer fires) | Plugin | error |
| S002 | `skill-md-missing` | `skills/{name}/` missing `SKILL.md` | Plugin | error |
| S003 | `no-exported-skills` | No plugin-exported skills found under `skills/` | Plugin | error |
| S004 | `frontmatter-malformed` | `SKILL.md` has malformed frontmatter (must start/end with `---`) | Always | error |
| S005 | `frontmatter-field-missing` | `SKILL.md` missing required field (`name` or `description`) | Always | error |
| S006 | `frontmatter-name-mismatch` | Frontmatter `name` does not match directory name | Plugin | error |
| S007 | `frontmatter-field-empty` | Optional frontmatter field present but empty | Always | error |
| S008 | `shared-md-missing` | Shared markdown reference missing on disk | Plugin | error |

### Name Validation (S009--S013, S033, S049)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| S009 | `name-too-long` | Skill name exceeds 64 characters | Always | error |
| S010 | `name-invalid-chars` | Skill name contains characters outside `[a-z0-9-]` | Always | error |
| S011 | `name-bad-hyphens` | Skill name starts/ends with hyphen or has consecutive hyphens | Always | error |
| S012 | `name-reserved-word` | Skill name contains reserved word (`anthropic` or `claude`) | Always | error |
| S013 | `name-has-xml` | Skill name contains XML/HTML tags | Always | error |
| S033 | `name-vague` | Skill name is too vague/generic (`helper`, `utils`, `tools`, etc.) | Plugin | warn |
| S049 | `name-not-gerund` | Skill name not in gerund (verb+ing) form | Plugin | suppressed |

### Description Validation (S014--S018, S034, S050)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| S014 | `desc-too-long` | Skill description exceeds 1024 characters | Always | error |
| S015 | `desc-truncated` | Skill description exceeds the configurable listing threshold (250 by default) | Always | warn |
| S016 | `desc-uses-person` | Skill description uses first/second person | Plugin | error |
| S017 | `desc-no-trigger` | Skill description lacks trigger context (e.g., "Use when...") | Plugin | error |
| S018 | `desc-has-xml` | Skill description contains XML/HTML tags | Always | error |
| S034 | `desc-too-short` | Skill description under 20 characters | Always | warn |
| S050 | `desc-vague-content` | Skill description content is too vague/generic | Plugin | warn |

### Body Content (S019--S022, S037--S038, S041, S046--S047, S051--S053, S055--S057)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| S019 | `body-too-long` | `SKILL.md` body exceeds 500 lines | Always | suppressed |
| S020 | `body-empty` | `SKILL.md` has no content after frontmatter | Always | error |
| S021 | `consecutive-bash` | Consecutive bash code blocks, including reference-file blocks separated by short breadcrumbs/comments, that could be combined | Always | warn |
| S022 | `backslash-path` | Windows-style backslash paths in skill content | Always | error |
| S037 | `body-no-refs` | Body exceeds 300 lines with no file references | Plugin | warn |
| S038 | `time-sensitive` | Body contains time-sensitive date/year patterns | Plugin | warn |
| S041 | `fork-no-task` | `context: fork` set but body lacks task instructions | Always | error |
| S046 | `body-no-workflow` | Body exceeds 300 lines with no workflow structure | Plugin | warn |
| S047 | `body-no-examples` | Body exceeds 200 lines with no examples or templates | Plugin | suppressed |
| S051 | `script-deps-missing` | Script-backed skill lacks dependency/package documentation | Plugin | warn |
| S052 | `script-verify-missing` | Script-backed skill lacks verification/validation steps | Plugin | warn |
| S053 | `terminology-inconsistent` | Uses 3+ variants from the same synonym group | Plugin | warn |
| S055 | `script-errhand-missing` | Script file lacks error handling patterns (`set -e`/`trap` for shell, `try`/`except` for Python) | Plugin | warn |
| S056 | `body-no-default` | Body lists alternatives without stating a default recommendation | Plugin | warn |
| S057 | `magic-number-undoc` | Undocumented magic number in code block (no justification comment) | Plugin | warn |

### Prompt and Invocation Contracts (S058--S062)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| S058 | `skill-invoke-missing` | `allowed-tools` includes `Skill` without a clear Skill tool invocation step, or uses ambiguous `Invoke /name` prose | Always | error |
| S059 | `skill-flag-mismatch` | A flag in a fenced shipped-script invocation is not accepted by that script; forwarding scripts are skipped | Always | error |
| S060 | `awk-field-ref` | Awk positional fields such as `$0` or `$1` appear inside a `SKILL.md` shell fence | Always | error |
| S061 | `unsafe-grep-probe` | A shell fence contains unbounded grep-family input, bare top-level `grep`, or a parent-directory ascent | Always | error |
| S062 | `skill-closure-large` | Transitive always-loaded skill prompt closure exceeds `skill-closure-max-lines` | Always | warn |

### Frontmatter Field Types (S023--S027, S063--S066, S070--S071)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| S023 | `bool-field-invalid` | Boolean fields (`user-invocable`, `disable-model-invocation`) must be `true`/`false` | Always | error |
| S024 | `context-field-invalid` | `context` field must be `fork` (if present) | Always | error |
| S025 | `effort-field-invalid` | `effort` field must be `low`/`medium`/`high`/`xhigh`/`max` (if present) | Always | error |
| S026 | `shell-field-invalid` | `shell` field must be `bash`/`powershell` (if present) | Always | error |
| S027 | `skill-unreachable` | Skill unreachable: `disable-model-invocation: true` AND `user-invocable: false` | Always | error |
| S063 | `model-invalid` | `model` must be a recognized alias (`sonnet`/`opus`/`haiku`/`inherit`/…) or `claude-…` ID | Always | error |
| S064 | `agent-no-fork` | `agent` is set without `context: fork` | Always | error |
| S065 | `agent-unknown` | `agent` is not a built-in (`Explore`/`Plan`/`general-purpose`) or existing custom agent | Always | error |
| S066 | `side-effect-auto` | Side-effect-named skill lacks `disable-model-invocation: true` | Always | warn |
| S070 | `unknown-fm-field` | Unknown skill frontmatter field (typo catcher) | Always | warn |
| S071 | `paths-empty` | `paths` field is present but empty | Always | warn |

### Extended Frontmatter (S035, S039--S040, S042--S045, S067)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| S035 | `compat-too-long` | `compatibility` field exceeds 500 characters | Always | warn |
| S039 | `metadata-not-string` | Metadata map values must be strings | Always | error |
| S040 | `tools-unknown` | `allowed-tools` lists unrecognized tool name | Always | warn |
| S042 | `dmi-empty-desc` | `disable-model-invocation: true` with empty/missing description | Always | error |
| S043 | `frontmatter-backslash` | Windows-style backslash paths in frontmatter fields | Always | error |
| S044 | `mcp-tool-unqualified` | MCP tool reference without server prefix | Always | warn |
| S045 | `tools-list-syntax` | `allowed-tools` uses YAML list syntax instead of comma-separated scalar | Always | warn |
| S067 | `bash-unscoped` | `allowed-tools` lists unscoped `Bash` (prefer `Bash(…)` scoping) | Always | warn |

### Cross-Field and Structural (S028--S032, S036, S048, S054, S068--S069)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| S028 | `args-no-hint` | Body uses `$ARGUMENTS` but frontmatter has no `argument-hint` field | Always | error |
| S029 | `nested-ref-deep` | Referenced shared `.md` itself references other shared `.md` files | Plugin | warn |
| S030 | `orphaned-skill-files` | Files in skill `scripts/` not referenced from `SKILL.md` | Always | error |
| S031 | `non-https-url` | Non-HTTPS URL (`http://`) found in skill content | Always | error |
| S032 | `hardcoded-secret` | Potential hardcoded secret/API key detected | Always | error |
| S036 | `ref-no-toc` | Referenced `.md` file exceeds 100 lines with no `##` headings | Plugin | warn |
| S048 | `ref-name-generic` | Non-descriptive reference file name in skill directory | Always | warn |
| S054 | `desc-body-misalign` | Skill description keywords not reflected in body | Plugin | warn |
| S068 | `injection-overflow` | More than 3 dynamic context injections (`!`…``) in skill body | Always | warn |
| S069 | `hint-no-args` | `argument-hint` set but body never references `$ARGUMENTS` | Always | warn |

## Agent Rules (A)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| A001 | `agents-dir-missing` | `agents/` directory is missing | Plugin | error |
| A002 | `agent-frontmatter-malformed` | Agent `.md` has malformed frontmatter | Always | error |
| A003 | `agent-field-missing` | Agent `.md` missing required field (`name` or `description`) | Always | error |
| A004 | `no-agent-files` | `agents/` has no `.md` files | Plugin | error |
| A005 | `template-file-missing` | `skills/shared/reviewer-templates.md` is missing | Plugin | warn |
| A006 | `template-marker-missing` | Agent `.md` missing "Derived from" marker | Plugin | warn |
| A007 | `template-count-mismatch` | Agent-template count mismatch | Plugin | warn |
| A008 | `agent-desc-long` | Agent description exceeds 1024 characters | Always | error |
| A009 | `agent-desc-short` | Agent description under 20 characters | Always | error |
| A010 | `agent-name-invalid` | Agent name contains characters outside `[a-z0-9-]` | Always | error |
| A011 | `agent-desc-redundant` | Agent description too similar to agent name | Always | error |
| A012 | `agent-read-mismatch` | Explicit agent tools omit `Read` while its prompt instructs reading file-backed evidence | Always | error |
| A013 | `agent-output-unsafe` | Machine-only evidence output lacks both an unreadable-evidence outcome and never-invent language | Always | error |
| A014 | `agent-model-invalid` | Agent `model` is not a recognized Claude Code model | Always | error |
| A015 | `agent-permission-invalid` | Agent `permissionMode` is not one of the allowed enum values | Always | error |
| A016 | `agent-skill-missing` | Agent `skills` entry has no matching `SKILL.md` on disk | Always | error |
| A017 | `agent-tools-overlap` | A tool appears in both `tools` and `disallowedTools` | Always | error |
| A018 | `agent-memory-invalid` | Agent `memory` is not `user`/`project`/`local` | Always | error |
| A019 | `agent-tools-unknown` | Agent `tools` lists an unrecognized tool name | Always | error |
| A020 | `agent-disallowed-unknown` | Agent `disallowedTools` lists an unrecognized tool name | Always | error |
| A021 | `agent-bypass-permissions` | Agent `permissionMode: bypassPermissions` disables safety checks | Always | warn |
| A022 | `agent-skill-kebab` | Agent `skills` entry is not kebab-case | Always | warn |
| A023 | `agent-effort-invalid` | Agent `effort` is not `low`/`medium`/`high`/`xhigh`/`max` | Always | error |
| A024 | `agent-isolation-invalid` | Agent `isolation` is not `worktree` | Always | error |
| A025 | `agent-background-invalid` | Agent `background` is not a boolean | Always | warn |
| A026 | `agent-maxturns-invalid` | Agent `maxTurns` is not a positive integer | Always | error |
| A027 | `agent-field-unknown` | Unrecognized agent frontmatter field (possible typo) | Always | warn |
| A028 | `agent-field-unsupported` | Agent frontmatter uses `hooks`, `mcpServers`, or `permissionMode`, which are unsupported for plugin agents | Plugin | warn |

> **Agent field-value rules (A014-A027).** These spec-grounded checks run on
> agent frontmatter in both `agents/` (Plugin mode) and `.claude/agents/`
> (Basic mode). They catch typos and invalid enum values (e.g. `model: sonet`,
> `permissionMode: yolo`, `tools: [Bsh]`, dangling `skills:` references) with
> near-zero false-positive risk. The larch-specific template rules A005-A007
> remain Plugin-only. The known-tool list is shared with S040
> (`skill allowed-tools`); `mcp__<server>__<tool>` names are accepted.

## Claude Configuration Rules (R/O/T)

These optional rules scan `.claude/rules/`, `.claude/output-styles/`, and
`.claude/settings.json` / `.claude/settings.local.json` in both Basic and
Plugin modes. They are silent when the corresponding directories or files do
not exist.

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| R001 | `rules-glob-invalid` | A `.claude/rules/` frontmatter `paths` glob is invalid | Always | error |
| R002 | `rules-field-unknown` | `.claude/rules/` frontmatter contains an unknown field | Always | warn |
| O001 | `style-description-missing` | Output-style `description` is missing or whitespace-only | Always | warn |
| O002 | `style-instructions-invalid` | Output-style `keep-coding-instructions` is not a YAML boolean | Always | error |
| O003 | `style-field-unknown` | Output-style frontmatter contains an unknown field | Always | warn |
| O004 | `style-body-empty` | Output style has no non-whitespace body after frontmatter | Always | warn |
| O005 | `style-name-long` | Output-style `name` exceeds 64 characters | Always | warn |
| O006 | `style-frontmatter-invalid` | Output-style frontmatter is missing or invalid YAML | Always | error |
| T001 | `pr-template-invalid` | `prUrlTemplate` is not a non-empty string with a documented placeholder | Always | warn |
| T002 | `channels-enabled-invalid` | `channelsEnabled` is not a boolean | Always | warn |

## Codex Configuration Rules (CX)

These optional rules validate a project-local `.codex/config.toml` in Basic
and Plugin modes. The allowlists were verified against the live Codex schema
on 2026-07-16. Codex's legacy `approvalMode` and `fullAutoErrorMode` keys are
not registered as standalone rules because they are absent from the current
schema and are covered by the unknown-key rules.

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| CX001 | `codex-toml-invalid` | `.codex/config.toml` is invalid TOML | Always | error |
| CX002 | `codex-doc-bytes` | `project_doc_max_bytes` is not 1–65536 | Always | error |
| CX003 | `codex-doc-names` | project documentation fallback names are invalid | Always | error |
| CX004 | `codex-key-unknown` | Unknown supported nested configuration key | Always | warn |
| CX005–CX009 | — | Invalid core approval, sandbox, model, or personality enum | Always | error |
| CX010 | `codex-access-ack` | Full sandbox access lacks its explicit acknowledgement | Always | error |
| CX011 | `codex-shell-inherit` | Invalid shell environment inheritance mode | Always | error |
| CX012 | `codex-mcp-transport` | MCP server has neither a command nor URL | Always | error |
| CX013 | `codex-secret-literal` | MCP configuration contains a hardcoded secret | Always | error |
| CX014–CX015 | — | Invalid credential-store or workspace-write mode | Always | error |
| CX016–CX024 | — | Invalid Codex scalar/table type or model token limit | Always | warn/error |
| CX025–CX027 | — | Unknown granular approval field or invalid reviewer/tier | Always | warn |
| CX028 | `codex-bearer-token` | Inline MCP bearer token is forbidden | Always | error |
| CX029 | `codex-agent-threads` | `agents.max_threads` conflicts with `multi_agent_v2` | Always | error |
| CX030–CX032 | — | Invalid app approval mode, skills table, or profile type | Always | error/warn |
| CX033 | `codex-top-key` | Unknown top-level Codex key | Always | warn |
| CX034 | `codex-feature-key` | Unknown Codex feature flag | Always | warn |
| CX035 | `codex-network-field` | Unknown `permissions.network` field | Always | suppressed |
| CX036 | `codex-windows-sandbox` | Invalid Windows sandbox mode | Always | suppressed |

## Hygiene / Scripts Rules (G)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| G001 | `pwd-in-skill` | `SKILL.md` uses `$PWD/` or hardcoded path instead of `${CLAUDE_PLUGIN_ROOT}/` | Plugin | error |
| G002 | `script-ref-missing` | Script reference missing on disk | Always | error |
| G003 | `script-not-executable` | Script file not executable | Always | error |
| G004 | `dead-script` | Dead script with no structured invocation reference | Plugin | error |
| G005 | `security-md-missing` | `SECURITY.md` is missing from repo root | Plugin | warn |
| G006 | `todo-in-skill` | `TODO`/`FIXME`/`HACK`/`XXX` marker in published skill body | Plugin | warn |
| G007 | `todo-in-agent` | `TODO`/`FIXME`/`HACK`/`XXX` marker in agent `.md` body | Plugin | warn |
| G008 | `gh-inline-body` | Shipped script passes a GitHub body or release notes inline instead of using a file-backed option | Always | warn |
| G009 | `bash-replacement-unsafe` | Bash global substitution uses a variable replacement that can reinterpret `&` | Always | error |
| G010 | `bash32-incompatible` | Shipped shell uses syntax unavailable in macOS Bash 3.2 | Always | suppressed |
| G011 | `awk-regex-nonascii` | Dynamic awk regex contains non-ASCII text with implementation-dependent behavior | Always | suppressed |

## Email Rules (E)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| E001 | `invalid-email-format` | Email address is not a valid format | Plugin | error |

## User Config Rules (U)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| U001 | `userconfig-not-object` | `userConfig` in `.claude/settings.json` must be an object | Plugin | error |
| U002 | `userconfig-desc-missing` | `userConfig` entry missing or invalid description | Plugin | error |
| U003 | `userconfig-env-missing` | `userConfig` key has no corresponding env var reference in `scripts/` | Plugin | error |
| U004 | `userconfig-sensitive-type` | `userConfig` `sensitive` field must be a boolean | Plugin | error |
| U005 | `userconfig-title-missing` | `userConfig` entry missing or invalid title | Plugin | error |
| U006 | `userconfig-type-missing` | `userConfig` entry missing or invalid type | Plugin | error |
| U007 | `userconfig-key-invalid` | `userConfig` key is not a valid identifier (letters, digits, `_`, `-`, `.`; must start with a letter or `_`) | Plugin | warn |

## MCP Configuration Rules (P)

MCP configuration is validated in root and nested `*.mcp.json` files and in
`.claude/settings.json` / `.claude/settings.local.json` when those files are
present. These rules run in both Basic and Plugin modes.

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| P001 | `mcp-json-invalid` | MCP configuration is not valid JSON | Always | error |
| P009 | `mcp-stdio-command` | `stdio` server (including omitted type) has no non-empty `command` | Always | error |
| P010 | `mcp-http-url` | `http` or `sse` server has no non-empty `url` | Always | error |
| P011 | `mcp-type-invalid` | Server `type` is not `stdio`, `http`, or `sse` | Always | error |
| P012 | `mcp-sse-deprecated` | `sse` transport is deprecated; use Streamable HTTP | Always | warn |
| P017 | `mcp-insecure-url` | Non-local `http://` server URL is not HTTPS | Always | error |
| P018 | `mcp-env-secret` | Secret-like environment variable contains a literal plaintext value | Always | warn |
| P019 | `mcp-command-dangerous` | Server command contains a dangerous shell pattern | Always | warn |
| P022 | `mcp-args-invalid` | `args` is not an array of strings | Always | error |
| P023 | `mcp-duplicate-server` | `mcpServers` contains a duplicate server name | Always | error |
| P024 | `mcp-server-empty` | Server configuration is an empty object | Always | error |
| P025 | `mcp-alwaysload-invalid` | `alwaysLoad` is not a boolean | Always | warn |
| P026 | `mcp-server-reserved` | Server name is reserved by Claude Code | Always | error |

## Slack Rules (K)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| K001 | `slack-fallback-mismatch` | Slack fallback variable without corresponding `CLAUDE_PLUGIN_OPTION_` reference | Plugin | warn |

## Docs Rules (D)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| D001 | `docs-ref-missing` | Docs reference in `CLAUDE.md` not found on disk | Plugin | error |
| D002 | `claudemd-too-large` | `CLAUDE.md` exceeds 500 lines | Plugin | warn |
| D003 | `todo-in-docs` | `TODO`/`FIXME`/`HACK`/`XXX` marker in `CLAUDE.md` (outside code fences) | Plugin | warn |
| D004 | `claude-import-large` | Recursive `CLAUDE.md` `@`-import closure exceeds a configured per-file or total line budget | Always | warn |
| D005 | `inline-path-missing` | Path-shaped inline-code pointer in a configured instruction file is dead or escapes the repository | Always | warn |

## Auto-Fixable Rules

When `--autofix` is provided, agent-lint attempts to automatically fix
violations for rules that have purely mechanical, unambiguous fixes. After
all possible fixes are applied, it runs a final validation pass and reports
any remaining issues with normal exit semantics (exit 1 if errors remain).

**Auto-fixable rules (12 of 225):**

| Rule | Code | Fix |
|------|------|-----|
| hook-not-executable | H005 | `chmod +x` on script |
| script-not-executable | G003 | `chmod +x` on script |
| frontmatter-name-mismatch | S006 | Set `name:` to match directory |
| frontmatter-field-empty | S007 | Remove empty optional field |
| name-has-xml | S013 | Strip XML tags from name |
| desc-has-xml | S018 | Strip XML tags from description |
| consecutive-bash | S021 | Merge adjacent bash blocks |
| backslash-path | S022 | Replace `\` with `/` in body |
| non-https-url | S031 | `http://` → `https://` |
| frontmatter-backslash | S043 | Replace `\` with `/` in frontmatter |
| tools-list-syntax | S045 | YAML list → comma-separated scalar |
| pwd-in-skill | G001 | `$PWD/` → `${CLAUDE_PLUGIN_ROOT}/` |

Each fix is logged to stderr.
