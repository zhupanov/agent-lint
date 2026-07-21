# Lint Rules Reference

Agent Lint ships 295 rules organized into 19 code-prefix categories. A category
is one rule-code prefix in the registry. Every rule has a unique code (e.g.,
`M001`) and a human-readable name (e.g., `plugin-json-missing`). Either form can
be used in `agent-lint.toml` to configure rule severity.

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
Use repeatable, comma-delimited `--only` values to run a focused set by code or
canonical name; selected rules retain the active strictness and suppression
policy.

**Mode column key:**

- **Plugin** -- runs only when `.claude-plugin/` is present
- **Always** -- runs in both Basic (any detected supported configuration) and Plugin modes
- **All skill surfaces** -- runs for `skills/`, `.claude/skills/`, `.agents/skills/`, and `.cursor/skills/` when each surface is active

## Platform Rule Namespaces

`CU` is reserved for Cursor rules and `CX` is reserved for Codex rules.
Platform-specific rules are activated independently of Basic/Plugin mode: they
run only when their platform is auto-detected or force-enabled in
`[platforms]`. See [configuration](configuration.md#platform-activation).

## Manifest Rules (M)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| M001 | `plugin-json-missing` | `.claude-plugin/plugin.json` is missing when no marketplace manifest file is present. Claude Code permits an omitted plugin manifest, so requiring at least one manifest file is an agent-lint convention. | Plugin | error |
| M002 | `plugin-json-invalid` | `plugin.json` is not valid JSON | Plugin | error |
| M003 | `plugin-field-missing` | `plugin.json` missing required `name`. A `name` that is absent, empty, or whitespace-only counts as missing; this matches the Claude Code plugin manifest contract. | Plugin | error |
| M004 | `plugin-version-format` | Present `plugin.json` version is not valid Semantic Versioning 2.0.0 (pre-release and build metadata are accepted). This follows the Claude Code manifest contract. | Plugin | error |
| M005 | `marketplace-json-missing` | `.claude-plugin/marketplace.json` is missing. Plugin-only repositories are valid in Claude Code, so this is advisory. | Plugin | warn |
| M006 | `marketplace-json-invalid` | `marketplace.json` is not valid JSON | Plugin | error |
| M007 | `marketplace-field-missing` | `marketplace.json` missing required field (`name`, `owner.name`, or `plugins`) | Plugin | error |
| M008 | `marketplace-plugins-empty` | `marketplace.json` `plugins` is empty or has the wrong type. Claude Code treats an empty array as a non-blocking warning. | Plugin | warn |
| M009 | `marketplace-plugin-invalid` | `marketplace.json` plugin entry has invalid `name`/`source`: missing/empty fields, duplicate names, unknown object source type, missing required per-type subfields, `..` traversal, or absolute paths | Plugin | error |
| M010 | `marketplace-enriched-missing` | `marketplace.json` missing `owner.email` or plugin `category` | Plugin | warn |
| M011 | `plugin-enriched-missing` | `plugin.json` missing `description`, `author.email`, or `keywords` | Plugin | warn |
| M012 | `component-path-nested` | A component (`commands`/`agents`/`skills`/`hooks`/`output-styles`/`themes`/`monitors`) lives inside `.claude-plugin/`, or a manifest path (`commands`, `agents`, `skills`, `hooks`, `mcpServers`, `outputStyles`, `lspServers`, `experimental.themes`, or `experimental.monitors`) points there | Plugin | error |
| M013 | `component-path-unsafe` | A manifest component path (`commands`, `agents`, `skills`, `hooks`, `mcpServers`, `outputStyles`, `lspServers`, `experimental.themes`, or `experimental.monitors`) is absolute (`/…`, `C:\…`) or uses `..` traversal | Plugin | error |
| M014 | `author-name-missing` | `plugin.json` `author` object present but `author.name` is missing or not a non-empty string | Plugin | warn |
| M015 | `homepage-url-invalid` | `plugin.json` `homepage` is present but is not a valid http(s) URL | Plugin | warn |
| M016 | `lsp-server-invalid` | `plugin.json` `lspServers` entry missing `command` or `extensionToLanguage` | Plugin | error |
| M017 | `channel-server-missing` | `plugin.json` `channels` entry does not reference a `server` | Plugin | warn |
| M018 | `plugin-version-missing` | `plugin.json` omits optional `version`; Claude Code falls back to the Git commit SHA. | Plugin | warn |
| M019 | `marketplace-bare-path` | `marketplace.json` relative string `source` does not start with `./` while `metadata.pluginRoot` is absent | Plugin | warn |
| M020 | `author-type-invalid` | `plugin.json` `author` is present but not an object. Claude Code rejects non-object authors as manifest load errors. | Plugin | error |
| M021 | `marketplace-name-format` | Marketplace or plugin entry `name` is not kebab-case (`[a-z0-9]+(-[a-z0-9]+)*`); claude.ai marketplace sync rejects other forms | Plugin | warn |

M003, M004, and M018 follow the [Claude Code plugin reference](https://code.claude.com/docs/en/plugins-reference) and its [plugin manifest schema](https://www.schemastore.org/claude-code-plugin-manifest.json). M005, M008, M009, M019, and M021 follow the [Claude Code marketplace guide](https://code.claude.com/docs/en/plugin-marketplaces); M005 remains an agent-lint advisory for repositories that intend to publish a self-hosted marketplace.

## Hooks Rules (H)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| H001 | `hooks-json-missing` | A hook-config file declared by `plugin.json` cannot be found. Hook configuration is optional upstream; the conventional `hooks/hooks.json` is validated only when present. | Plugin | error |
| H002 | `hooks-json-invalid` | A discovered plugin hook-config file is not valid JSON | Plugin | error |
| H003 | `hooks-key-missing` | A file-backed hook config has no top-level `hooks` key, or that value is not an object or array | Plugin | error |
| H004 | `hook-command-missing` | Hook command script missing on disk | Always | error |
| H005 | `hook-not-executable` | Hook command script not executable (Unix only) | Always | error |
| H006 | `settings-json-invalid` | `.claude/settings.json` is not valid JSON | Always | error |
| H007 | `hooks-array-empty` | A plugin hook config has an empty `hooks` collection | Plugin | error |
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
| H023 | `hook-command-dangerous` | Dangerous command pattern in hook command (`rm -rf` / split or long-form recursive+force, `git reset --hard`, `curl \| sh`, ...) | Always | warn |
| H024 | `hook-headers-interpolated` | HTTP hook headers interpolate `$VAR` without `allowedEnvVars` | Always | warn |
| H025 | `settings-local-invalid` | `.claude/settings.local.json` is not valid JSON | Always | error |

### Hook schema validation (H008--H024)

H008--H024 share one hook-object validation engine, applied to discovered
plugin hook config files (including `hooks/hooks.json` and paths declared by
`plugin.json`), inline `plugin.json` hooks, `.claude/settings.json`, and
`.claude/settings.local.json`. The [Claude Code plugin reference](https://code.claude.com/docs/en/plugins-reference)
defines these optional plugin hook locations and inline configuration; H001's
declared-path requirement is the agent-lint convention that a manifest path
must resolve.
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

Hook `hooks:` keys in skill and agent frontmatter are validated by the same
engine once frontmatter parses as YAML (X001); schema findings still use
H008--H024 codes with a `… frontmatter` path label.

## Markdown Structure Rules (X)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| X001 | `frontmatter-yaml-invalid` | Skill/agent frontmatter does not parse as valid YAML | Always | error |
| X002 | `unclosed-code-fence` | Unclosed code fence in SKILL.md, agent `.md`, or CLAUDE.md | Always | error |
| X003 | `xml-tag-unclosed` | Unclosed XML tag in markdown body (fence/inline-code aware) | Always | warn |
| X004 | `xml-tag-mismatched` | Mismatched closing XML tag in markdown body | Always | warn |
| X005 | `xml-tag-orphan` | Closing XML tag with no matching opener | Always | warn |

## Skills Rules (S)

### Structure and Frontmatter (S001--S008)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| S001 | `skills-dir-missing` | `skills/` directory is missing (deprecated — no longer fires) | Plugin | error |
| S002 | `skill-md-missing` | `skills/{name}/` missing `SKILL.md` | Plugin | error |
| S003 | `no-exported-skills` | No plugin-exported skills found under `skills/` | Plugin | error |
| S004 | `frontmatter-malformed` | `SKILL.md` has malformed frontmatter (must start/end with `---`) | Always | error |
| S005 | `frontmatter-field-missing` | `SKILL.md` required `name` or `description` is missing or not a non-empty string | Always | error |
| S006 | `frontmatter-name-mismatch` | Frontmatter `name` does not match directory name | Always | error |
| S007 | `frontmatter-field-empty` | Optional frontmatter field present but empty | Always | error |
| S008 | `shared-md-missing` | Shared markdown reference missing on disk | Plugin | error |

### Name Validation (S009--S011, S033, S049)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| S009 | `name-too-long` | Skill name exceeds 64 characters | All skill surfaces | error |
| S010 | `name-invalid-chars` | Skill name contains characters outside `[a-z0-9-]` | All skill surfaces | error |
| S011 | `name-bad-hyphens` | Skill name starts/ends with hyphen or has consecutive hyphens | All skill surfaces | error |
| S033 | `name-vague` | Exact published skill name is a domainless implementation label (`helper`, `helpers`, `utils`, `utility`, `tools`); add a domain or task. Broad subject nouns such as `data`/`files`/`documents` and compounds are allowed | Plugin | warn |
| S049 | `name-not-gerund` | Skill name not in gerund (verb+ing) form (deprecated — no longer fires; config alias retained) | Plugin | suppressed |

### Description Validation (S014--S018, S034, S050, S074)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| S014 | `desc-too-long` | Skill description exceeds 1024 characters | Always | error |
| S015 | `desc-truncated` | Combined canonical `description` and `when_to_use` exceed the configurable per-entry listing cap (1536 by default); Claude Code can also truncate below this cap when its separate global listing budget overflows, which S015 does not model | Always | warn |
| S016 | `desc-uses-person` | Skill description uses first/second person | Plugin | warn |
| S017 | `desc-no-trigger` | Skill description lacks trigger context (e.g., "Use when...") | Plugin | warn |
| S018 | `desc-has-xml` | Skill description contains XML/HTML tags | Always | error |
| S034 | `desc-too-short` | Skill description under 20 characters | Always | warn |
| S050 | `desc-vague-content` | Skill description content is too vague/generic | Plugin | warn |
| S074 | `skill-desc-overlap` | Two skill routing descriptions in the same simultaneously available namespace are exact duplicates or conservatively high Jaccard overlap (≥ 0.85 after normalization) | Always | warn |

Description-content rules (S014--S018, S034, S050, and S054) use the canonical parsed YAML string scalar. Invalid or non-mapping YAML frontmatter and missing, empty, or non-string descriptions are skipped by these rules; X001 and the required-frontmatter rules retain ownership of those conditions.

With a spec-valid description of 1,024 characters or fewer and no
`when_to_use`, S015 cannot fire at its default cap: S014 owns the description
spec limit as an error, while S015 owns the larger listing cap as a warning.

### Body Content (S019--S022, S037--S038, S041, S046--S047, S051--S053, S055--S057)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| S019 | `body-too-long` | `SKILL.md` body exceeds 500 lines | Always | suppressed |
| S020 | `body-empty` | `SKILL.md` has no content after frontmatter | Always | error |
| S021 | `consecutive-bash` | Consecutive bash code blocks, including reference-file blocks separated by short breadcrumbs/comments, that could be combined | Always | warn |
| S022 | `backslash-path` | Windows-style backslash paths in skill content (single-letter first path segments and adjacent named TeX escapes such as `\alpha\beta` are accepted false negatives to avoid rewriting escapes) | Always | error |
| S037 | `body-no-refs` | Body exceeds 300 lines with no file references | Plugin | warn |
| S038 | `time-sensitive` | Body contains time-sensitive date/year patterns | Plugin | warn |
| S041 | `fork-no-task` | `context: fork` set but body lacks task instructions | Always | warn |
| S046 | `body-no-workflow` | Body exceeds 300 lines with no workflow structure | Plugin | warn |
| S047 | `body-no-examples` | Body exceeds 200 lines with no examples or templates | Plugin | suppressed |
| S051 | `script-deps-missing` | Script-backed skill lacks dependency/package documentation | Plugin | warn |
| S052 | `script-verify-missing` | Script-backed skill lacks verification/validation steps | Plugin | warn |
| S053 | `terminology-inconsistent` | Uses 3+ variants from the same synonym group | Plugin | warn |
| S055 | `script-errhand-missing` | Script under `scripts/` (recursive; `.sh`/`.bash`/`.py` or shebang) lacks error handling (`set -e`/`trap` for shell, `try`/`except` for Python); subject is the script path | Plugin | warn |
| S056 | `body-no-default` | Body lists alternatives without stating a default recommendation | Plugin | warn |
| S057 | `magic-number-undoc` | Undocumented magic number in code block (no justification comment) | Plugin | warn |

### Prompt and Invocation Contracts (S058--S062)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| S058 | `skill-invoke-missing` | `allowed-tools` includes `Skill` without a clear Skill tool invocation step, or uses ambiguous `Invoke /name` prose | Always | error |
| S059 | `skill-flag-mismatch` | A flag in a fenced shipped-script invocation is not accepted by that script; forwarding scripts are skipped. Use `lint-skill-md-flag-signature: ok <reason>` on the logical command line only for reviewed exceptions. Recognized invocation roots are `${CLAUDE_PLUGIN_ROOT}`, `${CLAUDE_PROJECT_DIR}`, `$CLAUDE_PLUGIN_ROOT`, `$CLAUDE_PROJECT_DIR`, and `$PWD`; skill-local `scripts/...` paths take precedence over repository-root paths. | Always | error |
| S060 | `awk-field-ref` | Awk positional fields such as `$0` or `$1` appear inside a `SKILL.md` shell fence | Always | error |
| S061 | `unsafe-grep-probe` | A shell fence contains unbounded grep-family input, bare top-level `grep`, or a parent-directory ascent | Always | error |
| S062 | `skill-closure-large` | A compatible skill closure or configured named prompt-source metric exceeds its cap | Always | warn |

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
| S072 | `skill-dir-oversized` | Skill directory exceeds 8MB platform upload limit (counts build/dependency trees; skips `.git`; does not follow directory symlinks) | Always | warn |
| S073 | `skill-ref-nested` | Skill-relative `.md` link nested deeper than one directory level (`..` counts; URI schemes and non-`.md` targets are skipped) | Always | error |

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
| S030 | `orphaned-skill-files` | Files in skill `scripts/` not referenced from any skill-local `.md` (with name-boundary matching) | Always | error |
| S031 | `non-https-url` | Non-HTTPS URL (`http://`) found in skill content | All skill surfaces | error |
| S032 | `hardcoded-secret` | Potential hardcoded secret/API key detected | All skill surfaces | error |
| S036 | `ref-no-toc` | Referenced `.md` file exceeds 100 lines with no headings (levels 1–6, outside fences) | Plugin | warn |
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
| A024 | `agent-isolation-invalid` | Agent `isolation` is not `worktree` or `remote` | Always | error |
| A025 | `agent-background-invalid` | Agent `background` is not a boolean | Always | warn |
| A026 | `agent-maxturns-invalid` | Agent `maxTurns` is not a positive integer | Always | error |
| A027 | `agent-field-unknown` | Unrecognized agent frontmatter field (possible typo) | Always | warn |
| A028 | `agent-field-unsupported` | Agent frontmatter uses `hooks`, `mcpServers`, or `permissionMode`, which are unsupported for plugin agents | Plugin | warn |
| A029 | `agent-stop-missing` | Tool-using agent has no explicit stop control or failure outcome | Always | warn |
| A030 | `agent-desc-overlap` | Two agent routing descriptions in the same simultaneously available namespace are exact duplicates or conservatively high Jaccard overlap (≥ 0.85 after normalization) | Always | warn |

> **Agent field input (A014–A028).** These rules consume the single strict,
> canonical YAML mapping for the agent frontmatter; comments, quoted keys, flow
> syntax, and folded scalars therefore have their YAML meaning. `model`,
> `permissionMode`, `memory`, `effort`, and `isolation` require strings;
> `background` requires a YAML boolean; and `maxTurns` requires a positive YAML
> integer. `tools`, `disallowedTools`, and `skills` accept either a string or a
> sequence of strings. Malformed list shapes produce only the owning rule and
> do not cascade into per-entry diagnostics.
>
> Skill references are resolved from discovered runtime candidates, never by
> treating the reference as a path. Basic/private agent validation uses only
> `.claude/skills`; Plugin validation uses that namespace plus plugin-exported
> skills. For plugin-shipped agents, `hooks`, `mcpServers`, and
> `permissionMode` are owned solely by A028; private agents may use them. None
> of A014–A028 has an autofix because selecting replacement values or runtime
> references is not mechanically unambiguous.
> **Routing-description overlap (A030 / S074).** These warnings compare
> frontmatter `description` values with a deterministic shared helper: Unicode
> lowercase tokenization, punctuation and stopword removal, stripping of
> routing boilerplate at token boundaries (`use when` / `use this` / `use for`
> / `trigger when` / `do not trigger`), and Jaccard similarity with a checked-in 0.85 threshold.
> Exact normalized duplicates with at least one meaningful token report even
> below the four-token floor; the floor applies only to non-exact comparison.
> Missing, empty, non-string, or under-20-character descriptions, and
> descriptions in invalid or non-mapping YAML frontmatter, stay owned by
> existing structural/short/missing rules and are skipped. Claude private and plugin trees that can
> load together form one runtime-union namespace (`agents/` ∪ `.claude/agents/`,
> `skills/` ∪ `.claude/skills/` in Plugin mode). Cross-client `.agents/skills/`
> stays separate. Agents are never compared with skills. Findings are pathless
> multi-source diagnostics that name both repository-relative paths in
> `related_subjects` and the score in the message; global `suppress` works, but per-file overrides
> cannot match them.
> **Agent field-value rules (A014-A027).** These spec-grounded checks run on
> agent frontmatter in both `agents/` (Plugin mode) and `.claude/agents/`
> (Basic mode). They catch typos and invalid enum values (e.g. `model: sonet`,
> `permissionMode: yolo`, `tools: [Bsh]`, dangling `skills:` references) with
> near-zero false-positive risk. The larch-specific template rules A005-A007
> remain Plugin-only. The known-tool list is shared with S040
> (`skill allowed-tools`); `mcp__<server>__<tool>` names are accepted.
> **Stop controls (A029).** A029 applies only to valid Claude/plugin agent
> frontmatter that explicitly declares an execution-capable tool: `Agent`,
> `Bash`, `Edit`, `NotebookEdit`, `Task`, `WebFetch`, `WebSearch`, `Write`, or
> a qualified `mcp__<server>__<tool>`. Read-only discovery and task-status
> tools do not activate the rule. A positive `maxTurns`, an explicit numeric
> attempt/tool-call/step bound, an explicit time/token/cost budget, or a
> stop/report/escalation fallback after failure or no progress satisfies it.
> Attempt counts accept digits and small word numbers, including `limit of 5
> attempts`, `retry at most 3 times`, and `stop after three attempts`.
> Failure fallbacks include `on`/`upon failure` and `cannot make progress`.
> A body control must be an operative instruction for the current agent;
> example scopes and historical or descriptive mentions do not satisfy the
> rule. Frontmatter other than `maxTurns`, code, and quoted examples are
> ignored.

## Prompt Content Rules (Q)

These shared, source-aware checks run on root and nested `CLAUDE.md`, Claude
and shared-agent skill bodies (`.claude/skills`, `skills`, and
`.agents/skills`), Claude, Cursor, and plugin agent bodies, every included
`AGENTS.md`, `AGENTS.override.md`, and active Cursor rule and skill bodies
(`.cursor/rules/**/*.{md,mdc}`, `.cursorrules`, and
`.cursor/skills/*/SKILL.md`) in both Basic and Plugin modes. They skip
frontmatter where the surface defines it, fenced and inline code, and
identifiable quoted examples. A missing or malformed frontmatter block does
not exempt the remaining live prose. Q004 applies only when both root
`CLAUDE.md` and `README.md` exist.

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| Q001 | `prompt-generic-filler` | Generic instruction that adds no actionable guidance | Always | warn |
| Q002 | `prompt-negative-only` | Operative style/behavior negative without `instead`/`rather`/`prefer` within three prose lines; precise safety and integrity prohibitions are exempt | Always | error |
| Q003 | `prompt-weak-critical` | `should`/`try to`/`consider`/`maybe` inside a critical or important Markdown section | Always | error |
| Q004 | `claude-readme-duplicate` | More than 40% of eligible `CLAUDE.md` live-prose lines are duplicated in `README.md`, counted as a multiset (at least three matched lines) | Always | warn |
| Q005 | `prompt-unbounded-retry` | Operative unbounded retry or continuation instruction without an applicable bound or concrete failure outcome | Always | error |
| Q006 | `prompt-output-conflict` | Two mechanically incompatible operative output instructions (exclusive formats, or contradictory size/shape bounds) in one response scope | Always | warn |

Q001 recognizes: `be helpful`, `be accurate`, `be concise`, `follow
instructions`, `do your best`, `be professional`, `use best judgment`, and
`provide high-quality`. Prefer a concrete project-specific requirement over
these phrases.

Q002 evaluates explicit sentence-scoped patterns. It exempts precise
prohibitions against secret/private-data disclosure, authorization bypass,
destructive or irreversible actions, fabricated evidence, and explicit legal
or security policy violations. Safety-adjacent words elsewhere in a sentence
do not exempt an unrelated style negative.

Q005 recognizes narrow, operative forms such as `continue indefinitely`, `loop
forever`, `retry indefinitely`, `keep trying until it succeeds`, `try again
until it works`, `retry until success`, and `do not stop until it succeeds`.
It ignores examples,
quotations, code, frontmatter, explicit prohibitions of unbounded retry, finite
non-retry workflows, and instructions with an applicable attempt, tool-call,
step, timeout, token/cost budget, deadline, concrete failure outcome, or a
validated agent `maxTurns` bound. Its diagnostic reports bounded matched
evidence and asks for a bound or failure outcome; it has no autofix. Its bound
and fallback vocabulary is shared with A029, and ordinary hard-wrapped prose is
evaluated as joined sentences while retaining source locations.

Q006 models each operative output directive as a typed constraint (an exclusive
format requirement, or a size/shape bound) and reports only the pairs that
cannot both hold within one response scope. Response scopes follow the heading
tree: a requirement inherits every operative constraint from its ancestor
sections, and ordinary organizational headings do not begin a new scope. A
heading delineates a distinct child scope only when its words carry a fixed
boundary cue — a recognized output format; one of the fixed cue words `mode`,
`format`, `response`, `reply`, `request`, or `output` (the last four also match
their plural forms); leading `if`, `when`, `unless`, or `otherwise` wording; or
a recognized non-response artifact (commit message, PR/issue description,
changelog, documentation, file/path, or log). Sibling and nested boundary scopes
do not inherit each other's incompatible constraints, so explicitly delineated
response modes and artifacts stay clean while an ordinary subsection cannot hide
a same-response conflict. It never counts raw format keywords: mere multi-format
mention, conditional routing, either/or alternatives, input-format mentions, and
examples stay clean. The diagnostic exposes both conflicting constraints — each
with its line and column — as structured evidence and suggests clarification
without choosing between them; it has no autofix. Typed frontmatter
output-contract conflicts are intentionally out of scope for the first version.

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

## Shared Instruction File Rules (I)

These rules validate root and nested `AGENTS.md` files independently of any
active platform. Discovery respects configured exclusions and skips repository
metadata, dependencies, and conventional build output.

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| I001 | `instruction-file-empty` | `AGENTS.md` is empty or whitespace-only | Always | error |
| I002 | `instruction-file-secret` | `AGENTS.md` contains a potential hardcoded credential | Always | error |
| I003 | `instruction-file-path` | Backtick-quoted path in `AGENTS.md` is missing | Always | warn |
| I004 | `instruction-file-generic` | `AGENTS.md` prose is only exact generic guidance phrases | Always | warn |

I004 parses live Markdown prose with the shared document model. Headings are
ignored as organization, and frontmatter, fenced or indented code, inline code,
links, quotes, and identifiable examples are excluded. Remaining non-empty
prose clauses must each be exactly one of `be helpful`, `be accurate`,
`write good code`, or `follow best practices`, or a conjunction composed only
of those complete phrases, after case, ASCII punctuation, and whitespace
normalization. Substring matches are not enough. Empty or whitespace-only
files remain exclusively I001. I004 emits once per file with a span on the
first qualifying clause, bounded evidence, and suggestion
`add concrete project commands, paths, or constraints`. It is not auto-fixable.

I002 scans the entire Markdown source (including frontmatter, fences, and
inline code) and emits at most one error per `AGENTS.md` for the earliest
match in byte order. It does not autofix. Sensitive assignment keys are matched
case-insensitively by the shared segmented vocabulary `SECRET`, `TOKEN`,
`PASSWORD`, `PASSWD`, `PRIVATE_KEY`, `ACCESS_KEY`, `API_KEY`, and
`CLIENT_SECRET` (underscore/hyphen-joined multiword forms included;
substrings such as `TOKENIZER_MODEL` are not). Assignments accept `:` or `=`
with optional whitespace and quoted or unquoted values; every non-empty
literal value is a finding regardless of length. Exact placeholders
`$NAME`, `${NAME}`, `{{NAME}}`, and `<NAME>` (with
`NAME = [A-Za-z_][A-Za-z0-9_]*`) are clean, as are empty values and empty
`${NAME:-}` defaults. Leading/trailing text and `${NAME:-nonempty-default}`
are findings. Independently, these literal signatures are findings anywhere in
the document: `sk-` plus at least 20 ASCII alphanumerics; `ghp_` plus exactly
36 ASCII alphanumerics; `xoxb-`/`xoxp-` plus the Slack body grammar; `AKIA`/
`ASIA` plus 16 uppercase ASCII alphanumerics; `glpat-` plus at least 20 ASCII
alphanumerics/hyphens/underscores; `github_pat_` plus at least 20 ASCII
alphanumerics/underscores; and a PEM opener matching
`-----BEGIN (RSA |EC |OPENSSH )?PRIVATE KEY-----`. Assignment findings expose
only the key token as evidence; signature findings expose only a fixed category
label (`openai-api-key-signature`, `github-token-signature`,
`github-fine-grained-token-signature`, `slack-token-signature`,
`aws-access-key-signature`, `gitlab-token-signature`, or `private-key-block`).
Message, evidence, suggestion, and rendered output never include the credential
value or surrounding source line. Suggestion:
`replace the literal with an environment-variable or secret-store reference`.
Read failures stay outside I002; I001 remains the exclusive empty-file rule.

I003 scans paired backticks on individual prose lines; fence delimiters and
fence interiors are ignored. It treats explicit relative paths (for example
`docs/guide.md`, `missing.md`, `Node.js`, `api.example.com`, or `./script`) as
filesystem references. A slash-free dotted token is a path only when its final
component starts with a lowercase ASCII letter and is one to twelve lowercase
ASCII letters or digits. This excludes version literals such as `3.12`,
`1.2.3`, and `v20.11.1`. Bare extension and glob notation is prose, not a
path: a bare extension is one leading dot followed by one to twelve lowercase
ASCII letters or digits, so markers such as `.ts`, `.java`, `.properties`,
and `.tsx` do not depend on a fixed extension allowlist. Recognizable dotfile
and dot-directory entries take precedence and remain existence-sensitive:
`.env`, `.gitignore`, `.claude`, `.claude-plugin`, `.github`, `.vscode`,
`.codex`, `.cursor`, `.venv`, `.husky`, `.idea`, and `.devcontainer` are
reported when missing. Unlisted short lowercase dot tokens remain extension
notation; uppercase, punctuation-bearing, and over-twelve-character
dot-prefixed tokens are treated as dotfiles. URLs, variables, placeholders,
tokens containing whitespace, and non-path words are excluded.

Before probing a path, I003 and D005 both remove one `#fragment` and one
`::symbol` suffix while retaining the original token as diagnostic evidence.
Both report absolute, parent-traversing, and symlink probes. I003 resolves a
reference relative to the owning `AGENTS.md`; D005 additionally requires a
configured `inline-path-prefixes` match. The D005-only
`<!-- lint-doc-pointer-paths: ok reason -->` marker suppresses its source line
when it includes a non-empty reason; it does not suppress I003.

The former CX037, CX038, CX041, and CX043 identifiers and names remain
accepted as configuration aliases for these shared rules. The retired I005 /
`instruction-file-structure` rule and its CX044 / `codex-agents-structure`
aliases are no longer recognized.

## Codex Configuration Rules (CX)

These optional rules validate a project-local `.codex/config.toml` in Basic
and Plugin modes. The allowlists were rechecked with official `codex-cli 0.144.6`
on 2026-07-21. Codex's legacy `approvalMode` and `fullAutoErrorMode` keys are
not registered as standalone rules because they are absent from the current
schema and are covered by the unknown-key rules.

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| CX001 | `codex-toml-invalid` | `.codex/config.toml` cannot be read as UTF-8 or parsed as TOML | Always | error |
| CX002 | `codex-doc-bytes` | `project_doc_max_bytes` is not a nonnegative integer | Always | error |
| CX003 | `codex-doc-names` | project documentation fallback names are not an array of strings | Always | error |
| CX004 | `codex-key-unknown` | Unknown supported nested configuration key | Always | warn |
| CX005 | `codex-approval-policy` | Invalid scalar approval policy | Always | error |
| CX006 | `codex-sandbox-mode` | Invalid sandbox mode | Always | error |
| CX007 | `codex-reasoning-effort` | Model reasoning effort is not a non-empty string | Always | error |
| CX008 | `codex-model-verbosity` | Invalid model verbosity | Always | error |
| CX009 | `codex-personality` | Invalid personality | Always | error |
| CX011 | `codex-shell-inherit` | Invalid shell environment inheritance mode | Always | error |
| CX012 | `codex-mcp-transport` | MCP server has an invalid transport shape or field | Always | error |
| CX013 | `codex-secret-literal` | Codex MCP `env` or `http_headers` contains a literal credential, or another literal MCP value has an explicit token signature | Always | error |
| CX014 | `codex-cli-credentials` | Invalid credential-store mode | Always | error |
| CX015 | `codex-workspace-write` | Invalid workspace-write field type | Always | error |
| CX016–CX024 | — | Invalid Codex scalar/table type or model token limit | Always | warn/error |
| CX025 | `codex-approval-field` | Unknown granular approval field | Always | warn |
| CX026 | `codex-approval-reviewer` | Invalid approvals reviewer | Always | warn |
| CX027 | `codex-service-tier-type` | `service_tier` is not a string | Always | error |
| CX028 | `codex-bearer-token` | Inline MCP bearer token is forbidden | Always | error |
| CX029 | `codex-agent-threads` | `agents.max_threads` is not an integer greater than zero | Always | error |
| CX030–CX032 | — | Invalid app approval mode, skills table, or profile type | Always | error/warn |
| CX033 | `codex-top-key` | Unknown top-level Codex key | Always | warn |
| CX034 | `codex-feature-key` | Unknown Codex feature flag | Always | warn |
| CX035 | `codex-network-field` | Unknown `permissions.network` field | Always | warn |
| CX036 | `codex-windows-sandbox` | Invalid Windows sandbox mode | Always | error |
| CX061 | `codex-approval-shape` | Granular approval policy has an invalid shape or field type | Always | error |
| CX062 | `codex-config-container-type` | A structured Codex configuration value is not a table | Always | error |

### Codex Instruction, Plugin, and Skill Rules (CX)

These optional rules run in Basic and Plugin modes whenever the corresponding
Codex surface exists. Codex-specific `AGENTS.md` policy runs only when Codex is
active; the shared instruction rules above run independently.

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| CX039 | `codex-agents-large` | `AGENTS.md` exceeds 100,000 bytes | Always | warn |
| CX040 | `codex-agents-limit` | `AGENTS.md` exceeds the effective Codex document limit | Always | warn |
| CX042 | `codex-agents-override` | Root `AGENTS.override.md` is tracked by Git | Always | warn |
| CX045 | `codex-agents-conflict` | Explicit `AGENTS.md` setting conflicts with `.codex/config.toml` | Always | error |
| CX046 | `codex-plugin-path` | Codex plugin manifest is not at `.codex-plugin/plugin.json` | Always | error |
| CX047 | `codex-plugin-invalid` | `.codex-plugin/plugin.json` is invalid JSON | Always | error |
| CX048--CX049 | — | Missing/invalid Codex plugin name | Always | error |
| CX050--CX052 | — | Component path lacks `./`, traverses, or is bare `./` | Always | error |
| CX053--CX056 | — | Invalid default prompts or interface URL | Always | warn |
| CX057 | `codex-plugin-asset` | Interface asset path lacks `./` or traverses | Always | error |
| CX058--CX059 | — | Unsupported `hooks` field or missing description | Always | warn |
| CX060 | `codex-skill-frontmatter` | Codex skill uses Claude-only frontmatter (`context`, `agent`, or `hooks`) | Always | warn |

CX040 uses Codex's default 32,768-byte project-document budget unless
`.codex/config.toml` sets `project_doc_max_bytes`. CX053 and CX054 use the
three-prompt and 128-character limits from
[`openai/codex` commit `18110b8`](https://github.com/openai/codex/blob/18110b810f0a328147f6cd85e6f1ab6414927366/codex-rs/core-plugins/src/manifest.rs),
checked on 2026-07-16. The canonical manifest field is
`interface.defaultPrompt`; `default_prompts` is accepted by the linter only to
make migrations diagnosable.

## Cursor Configuration Rules (CU / CR)

Cursor rules run when a Cursor surface is present and are otherwise inert.
They run in both Basic and Plugin modes.

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| CU001 | `cursor-rule-empty` | `.cursor/rules/*.mdc` or `.cursorrules` has no instructions | Always | error |
| CU002 | `cursor-frontmatter-missing` | `.mdc` rule lacks YAML frontmatter | Always | warn |
| CU003 | `cursor-frontmatter-invalid` | `.mdc` frontmatter is invalid YAML | Always | error |
| CU004 | `cursor-glob-invalid` | `.mdc` `globs` has an invalid pattern | Always | error |
| CU005 | `cursor-field-unknown` | `.mdc` frontmatter uses an unknown field | Always | warn |
| CU006 | `cursor-legacy-rules` | Legacy `.cursorrules` file is present | Always | warn |
| CU007 | `cursor-always-globs` | `alwaysApply: true` has redundant `globs` | Always | warn |
| CU008 | `cursor-always-invalid` | `alwaysApply` is not a boolean | Always | error |
| CU009 | `cursor-description-missing` | Agent-requested `.mdc` rule lacks a description | Always | warn |
| CU010 | `cursor-hooks-invalid` | `.cursor/hooks.json` top-level or entry schema is invalid | Always | error |
| CU011 | `cursor-event-unknown` | Cursor hook event is not recognized | Always | warn |
| CU012 | `cursor-command-missing` | Cursor hook entry lacks a non-empty `command` | Always | error |
| CU013 | `cursor-type-invalid` | Cursor hook `type` is not `command` or `prompt` | Always | error |
| CU014 | `cursor-agent-invalid` | Cursor subagent frontmatter is invalid | Always | error |
| CU015 | `cursor-body-empty` | Cursor subagent body is empty | Always | warn |
| CU016 | `cursor-environment-invalid` | `.cursor/environment.json` schema is invalid | Always | error |
| CU017 | `cursor-hook-invalid` | Cursor hook timeout, loop limit, or fail-closed type is invalid | Always | warn |
| CU018 | `cursor-prompt-missing` | Prompt hook lacks `prompt` | Always | warn |
| CU019 | `cursor-model-invalid` | Prompt hook `model` is not a string | Always | error |
| CR-SK-001 | `cursor-skill-unsupported` | Cursor skill uses frontmatter unsupported by Cursor | Always | warn |

## Hygiene / Scripts Rules (G)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| G001 | `pwd-in-skill` | Existing bundled plugin asset uses `$PWD/` or `${PWD}/` instead of `${CLAUDE_PLUGIN_ROOT}/` | Plugin | error |
| G002 | `script-ref-missing` | Script reference missing on disk | Always | error |
| G003 | `script-not-executable` | Directly executed script file is not executable (Unix only) | Always | error |
| G004 | `dead-script` | Script has no executable invocation reference | Plugin | warn |
| G005 | `security-policy-missing` | No repository-local `SECURITY.md` in a GitHub-supported location (root, `.github/`, or `docs/`) | Plugin | warn |
| G006 | `todo-in-skill` | `TODO`/`FIXME`/`HACK`/`XXX` marker in published skill body | Plugin | warn |
| G007 | `todo-in-agent` | `TODO`/`FIXME`/`HACK`/`XXX` marker in agent `.md` body | Plugin | warn |
| G008 | `gh-inline-body` | Shipped script passes a GitHub body or release notes inline instead of using a file-backed option | Always | warn |
| G009 | `bash-replacement-unsafe` | Bash global substitution uses a variable replacement that can reinterpret `&` | Always | error |
| G010 | `bash32-incompatible` | Shipped shell uses syntax unavailable in macOS Bash 3.2 | Always | error |
| G011 | `awk-regex-nonascii` | Dynamic awk regex contains non-ASCII text with implementation-dependent behavior | Always | error |
| G012 | `hardcoded-machine-path` | `SKILL.md` uses a machine-specific or ambiguous runtime path | Plugin | warn |

G002 resolves `${CLAUDE_PLUGIN_ROOT}`, `${CLAUDE_PROJECT_DIR}`, `$CLAUDE_PLUGIN_ROOT`, `$CLAUDE_PROJECT_DIR`, and `$PWD` forms lexically within the repository. Escaping `..` paths are unresolvable; symlink targets are intentionally not audited. G003 is Unix-only and applies only when a regular file is invoked directly; interpreter-launched and sourced files do not require an execute bit. G004 is a warning because static reachability is incomplete; use the existing reason-bearing per-file suppression for intentional inventory entries.

G005 accepts an exact-case, regular, non-symlink `SECURITY.md` in the
repository root, `.github/`, or `docs/`, following GitHub's supported
community-health locations and documented `.github` → root → `docs`
precedence ([supported file types](https://docs.github.com/en/communities/setting-up-your-project-for-healthy-contributions/creating-a-default-community-health-file#supported-file-types),
retrieved 2026-07-21). A directory, a wrong-case name, or a symlink does not
satisfy the rule. An organization default served from a public `.github`
repository cannot be observed locally, so G005 stays a warning and normal
suppression is the escape hatch for that inherited policy.

G009-G011 use conventional script discovery unless `[lint].script-inventory`
is configured. An explicit inventory supports `.sh`, `.inc.bash`, and `.awk`
files, remains authoritative when global exclusions match an entry, and is
scanned in deterministic order on every run. G010 and G011 are hard errors by
default; listing `error = ["G010", "G011"]` explicitly is also supported when a
repository wants its portability policy visible in configuration.

G001 applies only when a `$PWD/` or `${PWD}/` reference resolves to an existing
bundled plugin component (`scripts`, `skills`, `agents`, `commands`, `hooks`,
`output-styles`, `themes`, `monitors`, or `.claude-plugin`), and its conservative
autofix replaces only that prefix. G012 reports every other `$PWD` reference and
machine-specific POSIX or Windows path without autofixing it; select
`${CLAUDE_PLUGIN_ROOT}`, `${CLAUDE_PROJECT_DIR}`, or `${CLAUDE_PLUGIN_DATA}`
according to whether the referenced path is bundled, project-local, or persistent.

## Email Rules (E)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| E001 | `invalid-email-format` | Present `author.email` or `owner.email` string does not meet the conservative ASCII contact-metadata convention | Plugin | warn |
| E002 | `email-type-invalid` | Present `author.email` or `owner.email` value is not a string | Plugin | error |

E001 is an agent-lint quality convention, not a Claude Code load requirement,
deliverability check, or full RFC email implementation. It accepts 3–254-byte
ASCII addresses with one `@`, a 1–64-byte dot-atom local part, and a dotted
hostname domain whose final label is at least two ASCII letters or a valid
`xn--` punycode label. Quoted local parts, domain literals, Unicode, controls,
whitespace, repeated dots, and out-of-range labels are rejected. Missing-email
policy remains owned by M010/M011. E001 and E002 report only the field name and
redacted evidence; neither exposes contact values.

## User Config Rules (U)

Top-level `.claude-plugin/plugin.json#userConfig` and every
`channels[].userConfig` / `channels.<name>.userConfig` share the same schema.
Title and description require a non-empty string after Unicode trimming; that
usability check is intentionally stricter than the upstream JSON schema.
U003 (`userconfig-env-missing`) was removed: agent-lint does not infer option
use from repository text.

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| U001 | `userconfig-not-object` | Present `userConfig` container in `.claude-plugin/plugin.json` (top-level or channel) is not an object | Plugin | error |
| U002 | `userconfig-desc-missing` | `userConfig` entry missing, non-string, empty, or whitespace-only description | Plugin | error |
| U004 | `userconfig-sensitive-type` | Present `userConfig` `sensitive` value is not a boolean | Plugin | error |
| U005 | `userconfig-title-missing` | `userConfig` entry missing, non-string, empty, or whitespace-only title | Plugin | error |
| U006 | `userconfig-type-missing` | `userConfig` entry missing or invalid `type` (must be `string`, `number`, `boolean`, `directory`, or `file`) | Plugin | error |
| U007 | `userconfig-key-invalid` | `userConfig` key is not a valid identifier (`^[A-Za-z_][A-Za-z0-9_]*$`) | Plugin | error |
| U008 | `userconfig-option-invalid` | `userConfig` option entry is not an object, has an unknown field, or has an invalid optional/semantic field shape | Plugin | error |

## MCP Configuration Rules (P)

MCP input is adapted by platform before P rules run. Claude standalone inputs
are repository `.mcp.json` files (at the project or plugin root); Claude plugin
manifests may provide an inline `.claude-plugin/plugin.json#mcpServers` object;
and Cursor project input is `.cursor/mcp.json`. Invalid JSON in an inline
plugin manifest remains M002-owned. Claude settings files and Codex TOML are
not MCP P-rule inputs: settings retain their Claude-validator diagnostics, and
`.codex/config.toml` remains CX-owned. These rules run in both Basic and Plugin
modes. The Claude transport matrix follows Claude Code's [remote HTTP](https://code.claude.com/docs/en/mcp#option-1-add-a-remote-http-server),
[WebSocket](https://code.claude.com/docs/en/mcp#option-4-add-a-remote-websocket-server),
and [legacy SSE](https://code.claude.com/docs/en/mcp#option-2-add-a-remote-sse-server)
documentation: `streamable-http` is the HTTP alias, `ws` uses WebSocket URLs,
and legacy `sse` remains supported but deprecated.

P027 owns MCP document and entry shape failures. Standalone Claude and Cursor
MCP documents require a top-level object-valued `mcpServers`; an inline plugin
manifest may omit it, but a present value must be an object. P024 is reserved
for an entry that is exactly `{}`; scalar, null, and array entries are P027.
Duplicate top-level `mcpServers` keys are P027, while P023 remains limited to
duplicate names in a valid server map. P027 is diagnostic-only and has no
autofix.

P018 treats only exact Claude expansion forms `${NAME}` and `${NAME:-DEFAULT}`
(with `NAME` matching `[A-Za-z_][A-Za-z0-9_]*`) as references on Claude MCP
surfaces. Cursor MCP has no documented expansion grammar here, so sensitive
values there are treated as literals. Unsupported `$...` / `{{...}}` strings and
non-empty defaults on sensitive keys are literals. Sensitive keys are matched by
ASCII identifier segments (`SECRET`, `TOKEN`, `PASSWORD`, `PASSWD`, plus
`PRIVATE_KEY` / `ACCESS_KEY` / `API_KEY` / `CLIENT_SECRET`), so names like
`TOKENIZER_MODEL` stay clean. Diagnostics name the env key and never echo the
value.

P019 preserves command/argv boundaries. Shell and Windows-interpreter payloads
are inspected only when the selected executable is a known shell/`cmd`/
PowerShell/`pwsh` and a `-c` / `/c`/`/k` / `-Command` (or documented abbreviation)
payload is present. Direct `rm`/`sudo rm` recursive+force against `/`, and
Windows `rd`/`rmdir /s /q` against a drive root, are detected from argv.
Inert argument text (for example `echo` receiving `curl ... | sh`) does not warn.

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| P001 | `mcp-json-invalid` | MCP configuration is not valid JSON | Always | error |
| P009 | `mcp-stdio-command` | `stdio` server (including omitted type) has no non-empty `command` | Always | error |
| P010 | `mcp-http-url` | Remote server has no syntactically valid URL for its selected transport (`http`/`streamable-http`/`sse`: `http(s)`; `ws`: `ws(s)`) | Always | error |
| P011 | `mcp-type-invalid` | Server `type` is not `stdio`, `http`, `streamable-http`, `sse`, or `ws` | Always | error |
| P012 | `mcp-sse-deprecated` | `sse` transport is deprecated; use Streamable HTTP | Always | warn |
| P017 | `mcp-insecure-url` | Non-local `http://` or `ws://` server URL is insecure (use `https://` or `wss://`) | Always | error |
| P018 | `mcp-env-secret` | Secret-like environment variable contains a literal plaintext value | Always | warn |
| P019 | `mcp-command-dangerous` | Server command contains a dangerous shell pattern | Always | warn |
| P022 | `mcp-args-invalid` | `args` is not an array of strings | Always | error |
| P023 | `mcp-duplicate-server` | `mcpServers` contains a duplicate server name | Always | error |
| P024 | `mcp-server-empty` | Server configuration is an empty object | Always | error |
| P025 | `mcp-alwaysload-invalid` | `alwaysLoad` is not a boolean | Always | warn |
| P026 | `mcp-server-reserved` | Server name is reserved by Claude Code | Always | error |
| P027 | `mcp-structure-invalid` | Required standalone server map is missing or invalid; an inline map, server entry, duplicate top-level map key, or adapter selector has an invalid shape | Always | error |

## Docs Rules (D)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| D001 | `docs-ref-missing` | Docs reference in `CLAUDE.md` not found on disk | Plugin | error |
| D002 | `claudemd-too-large` | `CLAUDE.md` exceeds 500 lines | Plugin | warn |
| D003 | `todo-in-docs` | `TODO`/`FIXME`/`HACK`/`XXX` marker in `CLAUDE.md` (outside code fences) | Plugin | warn |
| D004 | `claude-import-large` | Recursive `CLAUDE.md` `@`-import closure exceeds a global, path-specific, or total line budget | Always | warn |
| D005 | `inline-path-missing` | Path-shaped inline-code pointer in a configured instruction file is dead or escapes the repository | Always | warn |

D005 scans inline-code pointers outside fenced code blocks in the configured
`instruction-files` (default `AGENTS.md`, `SECURITY.md`, and `CLAUDE.md`). It
uses the shared I003 lexical and probe policy above after applying its
`inline-path-prefixes` scope. Its documented
`<!-- lint-doc-pointer-paths: ok reason -->` marker is intentionally D005-only.

## Link/import integrity Rules (L)

These rules validate the `@import` graph and relative markdown-link
integrity of each configured instruction file (see `instruction-files` in
[configuration](configuration.md); default `AGENTS.md`, `SECURITY.md`,
`CLAUDE.md`). `@import` traversal is fence-aware (imports inside code
fences are ignored), bounded to one visit per file per root, and reports
the offending chain for cycles and depth violations. Non-markdown
imports are legitimate in Claude Code and are not flagged.

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| L001 | `import-path-missing` | `@import` target markdown file does not exist on disk | Plugin | error |
| L002 | `circular-import` | Circular `@import` chain detected (the offending chain is reported) | Plugin | error |
| L003 | `import-depth-exceeded` | `@import` chain depth exceeds 5 hops (Claude Code's documented limit) | Plugin | error |
| L004 | `duplicate-import` | Duplicate `@import` of the same file within one instruction file (`./` prefixes normalized) | Plugin | warn |
| L005 | `broken-markdown-link` | Broken relative `[text](path.md)` link target in a configured instruction file; external URLs, anchors, and links inside code fences are skipped | Plugin | warn |
| L006 | `npm-script-missing` | Actionable `npm run` / `npm run-script` commands in configured instruction files whose script is missing from the root `package.json` `scripts` object. Scans inline code, shell fences (`bash`/`sh`/`shell`/`zsh`/`console`, with one leading `$` or `>` console prompt plus its trailing space stripped), and live prose `npm` tokens after line start/whitespace/opening punctuation; skips non-shell fences, quotes, example scopes, same-clause prose negation, package-qualified flags (`--workspace`/`-w`/`--workspaces`/`--prefix`/`--global`/`-g`), substitutions/heredocs/malformed shell, and value-taking non-qualifier flags. Silent when root `package.json` is absent, unreadable, invalid, or has no object-valued `scripts`. One diagnostic per missing script per file at the first command span, with script-name evidence and a correction suggestion. Not autofixable. | Always | warn |

## Auto-Fixable Rules

When `--autofix` is provided, agent-lint attempts to automatically fix
violations for rules that have purely mechanical, unambiguous fixes. After
all possible fixes are applied, it runs a final validation pass and reports
any remaining issues with normal exit semantics (exit 1 if errors remain).

**Auto-fixable rules (11 of 295):**

| Rule | Code | Fix |
|------|------|-----|
| hook-not-executable | H005 | `chmod +x` on script |
| script-not-executable | G003 | `chmod +x` on script |
| frontmatter-name-mismatch | S006 | Set a single-line canonical `name:` scalar to match the directory, only on surfaces selected for the run |
| frontmatter-field-empty | S007 | Remove a bare empty optional field only when it has no YAML continuation or child lines |
| desc-has-xml | S018 | Strip XML tags from description |
| consecutive-bash | S021 | Merge adjacent bash blocks |
| backslash-path | S022 | Replace every separator in each detected body path run with `/` |
| non-https-url | S031 | `http://` → `https://` (Claude surfaces only: `skills/` and `.claude/skills/`; `.agents/skills/` and `.cursor/skills/` report diagnostics without rewriting) |
| frontmatter-backslash | S043 | Replace `\` with `/` in frontmatter |
| tools-list-syntax | S045 | YAML list → comma-separated scalar |
| pwd-in-skill | G001 | Existing bundled asset `$PWD/` or `${PWD}/` → `${CLAUDE_PLUGIN_ROOT}/` |

Each fix is logged to stderr. H005 enforcement and its `chmod +x` autofix are
Unix-only because executable-bit permissions are not available on every
platform.
