# Lint Rules Reference

Agent Lint ships 296 rules organized into 19 code-prefix categories. A category
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
| M009 | `marketplace-plugin-invalid` | `marketplace.json` plugin entry has invalid `name`/effective local `source`: missing/empty fields, duplicate names, unknown object source type, missing required per-type subfields, unsafe `metadata.pluginRoot`, or `git-subdir.path` traversal | Plugin | error |
| M010 | `marketplace-enriched-missing` | `marketplace.json` missing `owner.email` or plugin `category` | Plugin | warn |
| M011 | `plugin-enriched-missing` | `plugin.json` missing `description`, `author.email`, or `keywords` | Plugin | warn |
| M012 | `component-path-nested` | A component (`commands`/`agents`/`skills`/`hooks`/`output-styles`/`themes`/`monitors`) lives inside `.claude-plugin/`, or a plugin/marketplace component path points there | Plugin | error |
| M013 | `component-path-unsafe` | A plugin or marketplace component path (`commands`, `agents`, `skills`, `hooks`, `mcpServers`, `outputStyles`, `lspServers`, `experimental.themes`, or `experimental.monitors`) is absolute (`/…`, `C:\…`), uses `..` traversal, or does not start with exact `./` | Plugin | error |
| M014 | `author-name-missing` | `plugin.json` `author` object present but `author.name` is missing or not a non-empty string | Plugin | warn |
| M015 | `homepage-url-invalid` | `plugin.json` `homepage` is present but is not a valid http(s) URL | Plugin | warn |
| M016 | `lsp-server-invalid` | `plugin.json` `lspServers` entry missing `command` or `extensionToLanguage` | Plugin | error |
| M017 | `channel-server-missing` | `plugin.json` `channels` entry does not reference a `server` | Plugin | warn |
| M018 | `plugin-version-missing` | `plugin.json` omits optional `version`; Claude Code falls back to the Git commit SHA. | Plugin | warn |
| M019 | `marketplace-bare-path` | `marketplace.json` relative string `source` does not start with `./` while `metadata.pluginRoot` is absent | Plugin | warn |
| M020 | `author-type-invalid` | `plugin.json` `author` is present but not an object. Claude Code rejects non-object authors as manifest load errors. | Plugin | error |
| M021 | `marketplace-name-format` | Marketplace or plugin entry `name` is not kebab-case (`[a-z0-9]+(-[a-z0-9]+)*`); claude.ai marketplace sync rejects other forms | Plugin | warn |

M003, M004, and M018 follow the [Claude Code plugin reference](https://code.claude.com/docs/en/plugins-reference) and its [plugin manifest schema](https://www.schemastore.org/claude-code-plugin-manifest.json). M005, M008, M009, M019, and M021 follow the [Claude Code marketplace guide](https://code.claude.com/docs/en/plugin-marketplaces); M005 remains an agent-lint advisory for repositories that intend to publish a self-hosted marketplace.

M012/M013 apply the same lexical component-path contract to `plugin.json` and
to every marketplace plugin entry, including `commands.<name>.source`. Paths
must start with exact `./`; absolute paths and any POSIX or Windows `..`
segment take precedence over a missing-prefix report. This supersedes #278's
former bare-path concession: current Claude validation rejects bare component
paths as load errors. The extractor is shared with manifest-declared discovery,
which never probes an unsafe declaration.

## Hooks Rules (H)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| H001 | `hooks-json-missing` | A hook-config file declared by `plugin.json` cannot be found. Hook configuration is optional upstream; the conventional `hooks/hooks.json` is validated only when present. | Plugin | error |
| H002 | `hooks-json-invalid` | A discovered plugin hook-config file is not valid JSON | Plugin | error |
| H003 | `hooks-key-missing` | A file-backed hook config has no top-level `hooks` key | Plugin | error |
| H004 | `hook-command-missing` | Hook command script missing on disk | Always | error |
| H005 | `hook-not-executable` | Hook command script not executable (Unix only) | Always | error |
| H006 | `settings-json-invalid` | `.claude/settings.json` is not valid JSON | Always | error |
| H007 | `hooks-array-empty` | A syntactically valid plugin hook config has no handler entries | Plugin | error |
| H008 | `hook-event-invalid` | Hook event name is not a recognized Claude Code event | Always | error |
| H009 | `hook-matcher-invalid` | `matcher` is non-string or present on an event that takes no matcher | Always | error |
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
| H026 | `hook-config-malformed` | A `hooks` configuration value does not match the documented event → matcher-group → handlers shape | Always | error |

### Hook schema validation (H008--H026)

H008--H026 share one hook-object validation engine, applied to discovered
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
`hooks` array -> hook objects. H026 reports malformed nesting and continues to
inspect handler-looking flattened objects so handler-specific rules still apply.
A plugin hook configuration file whose `hooks` key is a flat array carries
no event context, so the schema engine skips it; only H001--H007 apply there.

The valid event list and handler-type table live in
`src/validators/hook_schema.rs` and track the
[Claude Code hooks reference](https://code.claude.com/docs/en/hooks.md);
expect them to change with Claude Code releases.

H009 requires a present matcher to be a string, then uses an explicit list of the events the hooks reference marks "no matcher
support": `UserPromptSubmit`, `PostToolBatch`, `Stop`, `TeammateIdle`,
`TaskCreated`, `TaskCompleted`, `CwdChanged`, `MessageDisplay`,
`WorktreeCreate`, and `WorktreeRemove`. Every other event filters on some
documented field -- not just the tool events, but also `SessionStart` (how the
session started), `SessionEnd` (exit reason), `PreCompact`/`PostCompact`
(`manual`/`auto`), `SubagentStop` (agent type), and `InstructionsLoaded` (load
reason) -- so a blanket "non-tool event" check would flag valid configs.

Hook `hooks:` keys in skill and agent frontmatter are validated by the same
engine once frontmatter parses as YAML (X001); schema findings still use
H008--H026 codes with a `… frontmatter` path label.

## Markdown Structure Rules (X)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| X001 | `frontmatter-yaml-invalid` | Skill/agent frontmatter does not parse as valid YAML | Always | error |
| X002 | `unclosed-code-fence` | Unclosed code fence in SKILL.md, agent `.md`, or CLAUDE.md | Always | error |
| X003 | `xml-tag-unclosed` | Unclosed XML tag in markdown body (fence/inline-code aware) | Always | warn |
| X004 | `xml-tag-mismatched` | Mismatched closing XML tag in markdown body | Always | warn |
| X005 | `xml-tag-orphan` | Closing XML tag with no matching opener | Always | warn |

X002 recognizes fence openers only when they have at most three leading
spaces; tab-indented and four-space-indented fence-lookalikes are treated as
prose. Consequently, a real fence nested in a list item at four or more spaces
is not tracked: balanced nested fences are treated as prose and unclosed ones
may be missed. This deliberate warning-grade false-negative trade-off avoids
error-grade false positives on indented code blocks; full list-context tracking
would require a block parser.

## Skills Rules (S)

### Structure and Frontmatter (S001--S008)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| S001 | `skills-dir-missing` | `skills/` directory is missing (deprecated — no longer fires; config alias retained) | Plugin | error |
| S002 | `skill-md-missing` | A conventional or manifest-declared skill directory entry is missing `SKILL.md` | Plugin | error |
| S003 | `no-exported-skills` | A present conventional `skills/` tree exports no skills through `skills/`, manifest-declared skill roots, commands, or the root fallback | Plugin | error |
| S004 | `frontmatter-malformed` | `SKILL.md` has malformed frontmatter (must start/end with `---`) | Always | error |
| S005 | `frontmatter-field-missing` | `SKILL.md` required `name` or `description` is missing or not a non-empty string | Always | error |
| S006 | `frontmatter-name-mismatch` | Frontmatter `name` does not match directory name | Always | error |
| S007 | `frontmatter-field-empty` | Optional frontmatter field present but empty | Always | error |
| S008 | `shared-md-missing` | Shared markdown reference missing on disk from an active exported plugin `SKILL.md` | Plugin | error |

Plugin skill discovery includes immediate `skills/*/SKILL.md` entries (except
`skills/shared/`), safe manifest-declared skill directories, and the root
`SKILL.md` fallback when neither a `skills/` directory nor a `skills` manifest
field is present. `commands/*.md` and manifest-selected command files count as
exports for S003 but are otherwise command-style files; they do not receive the
SKILL.md content-rule suite. Private `.claude/skills/shared/SKILL.md` is an
ordinary private skill.

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
| S014 | `desc-too-long` | Skill description exceeds 1024 characters | All skill surfaces | error |
| S015 | `desc-truncated` | Combined canonical `description` and `when_to_use` exceed the configurable per-entry listing cap (1536 by default); Claude Code can also truncate below this cap when its separate global listing budget overflows, which S015 does not model | Always | warn |
| S016 | `desc-uses-person` | Skill description uses first/second person | Plugin | warn |
| S017 | `desc-no-trigger` | Skill description lacks trigger context (e.g., "Use when...") | Plugin | warn |
| S018 | `desc-has-xml` | Skill description contains XML/HTML tags | Always | error |
| S034 | `desc-too-short` | Skill description under 20 characters | All skill surfaces | warn |
| S050 | `desc-vague-content` | Skill description content is too vague/generic | Plugin | warn |
| S074 | `skill-desc-overlap` | Two active Claude skill or command routing descriptions in the same simultaneously available namespace are exact duplicates or conservatively high Jaccard overlap (≥ 0.85 after normalization) | Always | warn |

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
| S058 | `skill-invoke-missing` | An active SKILL.md `allowed-tools` includes `Skill` without a clear Skill tool invocation step, or uses ambiguous `Invoke /name` prose | Always | error |
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
| S031 | `non-https-url` | Non-HTTPS URL (`http://`) found in skill content. XML-namespace/DOCTYPE/`schemaLocation`/`targetNamespace` identifiers and reserved-name hosts (`www.w3.org`, RFC 2606/6761 `*.test`/`*.example`/`*.invalid`/`*.localhost`, `example.com`/`.org`/`.net`, loopback) are opaque identifiers, not fetchable links, and are exempt | All skill surfaces | error |
| S032 | `hardcoded-secret` | Potential hardcoded secret/API key detected; scans the full `SKILL.md` source and reports only safe key/category evidence | All skill surfaces | error |
| S036 | `ref-no-toc` | Referenced `.md` file exceeds 100 lines with no headings (levels 1–6, outside fences) | Plugin | warn |
| S048 | `ref-name-generic` | Non-descriptive reference file name in skill directory | Always | warn |
| S054 | `desc-body-misalign` | Skill description keywords not reflected in body | Plugin | warn |
| S068 | `injection-overflow` | More than 3 dynamic context injections (`!`…``) in skill body | Always | warn |
| S069 | `hint-no-args` | `argument-hint` set but body never references `$ARGUMENTS` | Always | warn |

## Agent Rules (A)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| A001 | `agents-dir-missing` | A manifest-declared plugin agent path (a `plugin.json` `agents` string or array entry) does not exist. The implicit default `agents/` directory is optional, so its absence is never reported | Plugin | error |
| A002 | `agent-frontmatter-malformed` | Agent `.md` has malformed frontmatter | Always | error |
| A003 | `agent-field-missing` | Agent `.md` missing required field (`name` or `description`) | Always | error |
| A004 | `no-agent-files` | A present plugin agent root (default `agents/` or a manifest-declared path) has no agent `.md` files after recursive discovery. An all-excluded root stays silent; an absent root reports nothing (A001 owns declared absence) | Plugin | error |
| A005 | `template-file-missing` | An opted-in larch agent derives from a missing or unreadable `skills/shared/reviewer-templates.md` | Plugin | warn |
| A006 | `template-marker-missing` | An opted-in top-level agent lacks the larch derivation marker | Plugin | warn |
| A007 | `template-count-mismatch` | Opted-in larch agent count differs from semantic reviewer-section count | Plugin | warn |
| A008 | `agent-desc-long` | Agent description exceeds 1024 characters | Always | error |
| A009 | `agent-desc-short` | Agent description under 20 characters | Always | error |
| A010 | `agent-name-invalid` | Agent name contains characters outside `[a-z0-9-]` | Always | error |
| A011 | `agent-desc-redundant` | Agent description too similar to agent name | Always | error |
| A012 | `agent-read-mismatch` | Explicit agent tools omit `Read` while live prose explicitly requires the `Read` tool | Always | error |
| A013 | `agent-output-unsafe` | Machine-only evidence output lacks both an unreadable-evidence outcome and never-invent language | Always | error |
| A014 | `agent-model-invalid` | Agent `model` must be a recognized alias (`sonnet`/`opus`/`haiku`/`fable`/`inherit`/…) or `claude-…` ID (same vocabulary as S063) | Always | error |
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

> **Agent field input (A012–A029).** These rules consume the single strict,
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
> references is not mechanically unambiguous. A012/A013 additionally consume
> only source-aware live prose: frontmatter, code, quotations, comments, and
> example scopes are inert. A012 requires an explicit operative `Read`-tool
> mandate; A013 requires an operative file-evidence read mandate plus an
> exclusive JSON/JSONL output mandate, and then requires both an unreadable
> evidence outcome and a direct never-invent/fabricate/guess prohibition.
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
> load together form one runtime-union namespace (`agents/` ∪ manifest-declared
> agent roots ∪ `.claude/agents/`, `skills/` ∪ `.claude/skills/` in Plugin mode);
> agent roots are scanned recursively, so a nested agent joins the pool and
> carries its subdirectory path. When Cursor is active, its
> runtime skill namespace is `**/.cursor/skills/**/SKILL.md` ∪
> `**/.agents/skills/**/SKILL.md`; it includes nested project locations and
> compares cross-tree pairs exactly once. When Cursor is inactive, shared
> `.agents/skills/` stays separate. The Cursor target also has a separate
> `.cursor/agents/**/*.md` namespace for A030; omitted Cursor `description`
> fields are legal and stay out of that overlap pool. Agents are never compared
> with skills. Findings are pathless
> multi-source diagnostics that name both repository-relative paths in
> `related_subjects` and the score in the message; global `suppress` works, but per-file overrides
> cannot match them.
> **Agent discovery (A002-A004, A008-A030).** Every agent rule except the larch
> template convention discovers agent files recursively, matching Claude Code:
> `.claude/agents/`, the plugin `agents/` default, and every repository-safe
> `plugin.json` `agents` root (string or array) are scanned into their
> subdirectories, because an agent's identity comes from its `name` field, not
> its path. Nested files carry their full repository-relative path, and
> exclusions and per-file overrides match that full path. **Agent field-value
> rules (A014-A027).** These spec-grounded checks run on
> agent frontmatter in both `agents/` (Plugin mode) and `.claude/agents/`
> (Basic mode). They catch typos and invalid enum values (e.g. `model: sonet`,
> `permissionMode: yolo`, `tools: [Bsh]`, dangling `skills:` references) with
> near-zero false-positive risk. **Larch template convention (A005-A007).**
> These are self-activating, Plugin-only rules for public top-level
> `agents/*.md`; they are not a Claude Code requirement and do not apply to
> nested, private, custom, or manifest-declared agent roots. They activate only
> when `skills/shared/reviewer-templates.md` exists or an included agent has a
> live derivation marker. A marker is a complete non-quoted, non-fenced,
> non-inline-code prose line (optionally prefixed by a Markdown list marker)
> that begins case-insensitively with `Derived from` and contains the exact
> `skills/shared/reviewer-templates.md` token; the same normalized marker is
> accepted in one complete standalone HTML comment, including a multiline
> comment. Frontmatter, block quotes, examples, filename substrings, and
> `${CLAUDE_PLUGIN_ROOT}` variants do not activate it. With a readable template,
> A006 reports each included agent without that marker. A007 counts only live
> level-2 ATX or setext headings whose canonical text is `Reviewer` or begins
> `Reviewer` followed by whitespace, `:`, or `-`; quoted, fenced, commented,
> other-level, and `Reviewership` headings do not count. Excluding the template
> disables all three rules; excluding any top-level agent still allows A006 for
> included agents but suppresses A007 because the participant set is incomplete.
> All three default to warnings (and follow normal pedantic/all/config policy),
> have no autofix, and A007 identifies participating agents as related subjects.
> Model aliases/`claude-…` IDs share one vocabulary with S063 (`skill model`);
> the known-tool list is shared with S040
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
(`**/.cursor/rules/**/*.mdc`, `.cursorrules`, and the active Cursor runtime
skill inventory: `**/.cursor/skills/**/SKILL.md` plus
`**/.agents/skills/**/SKILL.md`) in both Basic and Plugin modes. They skip
frontmatter where the surface defines it, fenced and inline code, and
identifiable quoted examples. Missing frontmatter, and malformed frontmatter
with a closing delimiter, do not exempt the remaining live prose. An exact
opening frontmatter delimiter without a closing delimiter has no deterministic
body boundary, so Q rules deliberately skip that file. Q004 applies only when
both root
`CLAUDE.md` and `README.md` exist.

Q001-Q003 share one operativity contract: within its sentence a directive
phrase counts only when, after list markers and an optional `always`/`please`,
it opens the instruction, a current-agent subject (`you`, `the agent`, `this
agent`, `agent`, `agents`, `assistant`, `model`) uses `must`/`shall`/`should`/
`will`/`need to` before it, or it follows an `if`/`when`/`before`/`after`/
`unless`/`while` setup clause closed by a comma. Descriptive, historical, and
interrogative prose and every example scope are inert. Each of Q001-Q003
reports every violating source line (not only the first), sorted by line, with
the matched source range, bounded masked evidence, and a concrete suggestion.

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| Q001 | `prompt-generic-filler` | Generic instruction that adds no actionable guidance, matched only inside an operative directive | Always | warn |
| Q002 | `prompt-negative-only` | Operative style/behavior negative without an operative `instead`/`rather`/`prefer` alternative in the same Markdown instruction scope (same paragraph or list item, no heading/example/fence/blank boundary, within three source lines); precise safety and integrity prohibitions are exempt | Always | error |
| Q003 | `prompt-weak-critical` | Operative `should`/`try to`/`consider`/`maybe` inside a live critical or important Markdown section | Always | error |
| Q004 | `claude-readme-duplicate` | More than 40% of eligible `CLAUDE.md` live-prose lines are duplicated in `README.md`, counted as a multiset (at least three matched lines) | Always | warn |
| Q005 | `prompt-unbounded-retry` | Operative unbounded retry or continuation instruction without an applicable bound or concrete failure outcome | Always | error |
| Q006 | `prompt-output-conflict` | Two mechanically incompatible operative output instructions (exclusive formats, or contradictory size/shape bounds) in one response scope | Always | warn |

Q001 recognizes: `be helpful`, `be accurate`, `be concise`, `follow
instructions`, `do your best`, `be professional`, `use best judgment`, and
`provide high-quality`, at word boundaries within an operative directive.
Prefer a concrete project-specific requirement over these phrases.

Q002 exempts precise prohibitions against secret/private-data disclosure,
authorization bypass, destructive or irreversible actions, fabricated evidence,
and explicit legal or security policy violations. Safety-adjacent words
elsewhere in a sentence do not exempt an unrelated style negative. A conjoined
negative after a safety-exempt one (`Never expose credentials, and never
apologize.`) is still evaluated. A positive alternative repairs a negative only
within the same instruction scope, never across a heading, example, fence, or
blank boundary.

Q003 activates only under a live `critical` or `important` heading whose example
scope is false; a heading naming an example (`# Important examples`) is an
example boundary. A sentence-leading `Should` is treated as a conditional
inversion (`Should any test fail, stop.`) and is not weak language, while a
mid-sentence agent `should` (`You should verify.`) still reports.

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
evaluated as joined sentences while retaining source locations. A control is
associated with each retry independently: it applies in the same sentence or
in an immediately adjacent sentence of the same paragraph or list item. An
adjacent control does not apply when text after its recognized phrase contains
`while`, `when`, `during`, or `before`, or a `for (the) <noun>` adjunct for a
report, summary, output, artifact, file, dependency, scan, inspection, or
indexing. The existing concrete fallback form `before reporting the failure`
remains part of the same control.

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
examples stay clean. Ordinary hard-wrapped prose is evaluated as joined sentences
(same contiguous-scope model as Q005) while retaining source locations, so a
leading conditional guard still applies when the directive continues on the next
line. The diagnostic exposes both conflicting constraints — each with its line
and column — as structured evidence and suggests clarification without choosing
between them; it has no autofix. Typed frontmatter output-contract conflicts are
intentionally out of scope for the first version.

## Claude Configuration Rules (R/O/T)

These optional rules scan `.claude/rules/`, `.claude/output-styles/`, and
`.claude/settings.json` / `.claude/settings.local.json` in both Basic and
Plugin modes. They are silent when the corresponding directories or files do
not exist.

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| R001 | `rules-glob-invalid` | A `.claude/rules/` frontmatter `paths` glob is invalid | Always | error |
| R002 | `rules-field-unknown` | `.claude/rules/` frontmatter contains an unknown field | Always | warn |
| O001 | `style-description-missing` | Output-style `description` is missing, non-string, or blank | Always | warn |
| O002 | `style-instructions-invalid` | Output-style `keep-coding-instructions` is not `true`, `false`, `"true"`, or `"false"` | Always | error |
| O003 | `style-field-unsupported` | Output-style frontmatter contains an unsupported field or private-only placement | Always | warn |
| O004 | `style-body-empty` | Output style has no non-whitespace effective body | Always | warn |
| O006 | `style-frontmatter-invalid` | Output-style attempted frontmatter is malformed, invalid YAML, or not a mapping | Always | error |

Output styles are discovered recursively below `.claude/output-styles/` using
the shared exclusion, pruned-directory, and no-symlink traversal policy. A
body-only file is valid: its whole content is the effective body. O005
(`style-name-long`) is retired but remains an inert compatibility selector;
Claude Code has no output-style name-length limit, so it has no active rule
row or finding.
| T001 | `pr-template-invalid` | `prUrlTemplate` must be a trimmed non-empty string, use a documented placeholder only, and render to an absolute HTTP(S) URL with a host | Always | warn |
| T002 | `channels-enabled-unsupported` | Repository `channelsEnabled` is ignored; configure this managed-policy-only field through organization policy instead | Always | warn |

T001 accepts the documented `{host}`, `{owner}`, `{repo}`, `{number}`, and
`{url}` placeholders (including repetitions). It first checks type,
blank/surrounding whitespace, placeholder presence, unknown placeholders, then
the rendered HTTP(S) URL in that order. T002 applies to either fixed settings
path even when `[lint].exclude` names it; global and per-file rule policy still
applies. Neither rule has an autofix because removing configuration or choosing
a replacement URL/policy is not an unambiguous edit.

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

S032 uses that same source-positioned scanner for every `SKILL.md` surface,
including frontmatter, fences, and inline code, and emits the earliest match
once per file with line metadata. It has the same sensitive-key vocabulary and
signature families as I002, but intentionally accepts additional safe skill
documentation forms: `$(...)`, backtick command substitution, and angle
placeholders with hyphens or spaces (such as `<your-api-key>`). A signature
whose entire post-prefix payload is one repeated character is also clean for
S032 (for example `sk-xxxxxxxxxxxxxxxxxxxxxxxx`), while I002 deliberately
continues to report it. This is an intentional S032-vs-I002 divergence: I002
is a stricter instruction-file policy, whereas S032 must not reject examples
of safe credential indirection. Literal remainders and non-empty defaults
remain findings. S032 evidence is only the assignment key token or a fixed
signature category; it never includes a candidate value or its source line.

I003 scans inline-code spans from the shared Comrak Markdown adapter, including
arbitrary backtick delimiter lengths (for example ``docs/guide.md``). Fence
delimiters and fence interiors are ignored. It treats explicit relative paths
(for example `docs/guide.md`, `missing.md`, `Node.js`, `api.example.com`, or
`./script`) as filesystem references after normalizing `\` to `/`. A slash-free
dotted token is a path only when its final component starts with a lowercase
ASCII letter and is one to twelve lowercase ASCII letters or digits. This
excludes version literals such as `3.12`, `1.2.3`, and `v20.11.1`. Bare
extension and glob notation is prose, not a path: a bare extension is one
leading dot followed by one to twelve lowercase ASCII letters or digits, so
markers such as `.ts`, `.java`, `.properties`, and `.tsx` do not depend on a
fixed extension allowlist. Recognizable dotfile and dot-directory entries take
precedence and remain existence-sensitive: `.env`, `.gitignore`, `.claude`,
`.claude-plugin`, `.github`, `.vscode`, `.codex`, `.cursor`, `.venv`, `.husky`,
`.idea`, and `.devcontainer` are reported when missing. Unlisted short
lowercase dot tokens remain extension notation; uppercase,
punctuation-bearing, and over-twelve-character dot-prefixed tokens are treated
as dotfiles. URLs, variables, placeholders, tokens containing whitespace, and
non-path words are excluded.

Before probing a path, I003 and D005 both remove one `#fragment` and one
`::symbol` suffix while retaining the original token as diagnostic evidence.
Both resolve through one repository-safe path probe that rejects absolute
paths, repository-escaping `..`, a symlink in any existing path component, and
canonical escape. I003 additionally treats any authored `..` segment as unsafe
(the #241 parent-traversing policy). I003 resolves relative to the owning
`AGENTS.md`; D005 resolves from the repository root because its
`instruction-files` and `inline-path-prefixes` are repository-relative
contracts, and additionally requires a configured `inline-path-prefixes`
match. Each rule emits once per distinct normalized target per source file,
ordered by source byte position, using the first spelling and location.
Findings carry a structured span, bounded original-token evidence, and either
`correct the path or create the referenced repository file` or
`replace it with a non-symlinked repository-relative path`. The D005-only
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
| CX016 | `codex-model-type` | `model` is not a string | Always | error |
| CX017 | `codex-provider-type` | `model_provider` is not a string | Always | error |
| CX018 | `codex-reasoning-summary` | Invalid `model_reasoning_summary` | Always | error |
| CX019 | `codex-history-type` | `history` is not a TOML table | Always | error |
| CX020 | `codex-tui-type` | `tui` is not a TOML table | Always | error |
| CX021 | `codex-opener-type` | `file_opener` is not a string | Always | error |
| CX022 | `codex-mcp-credentials` | Invalid MCP OAuth credential-store mode | Always | error |
| CX023 | `codex-context-window` | `model_context_window` is not positive | Always | warn |
| CX024 | `codex-compact-limit` | `model_auto_compact_token_limit` is not positive | Always | warn |
| CX025 | `codex-approval-field` | Unknown granular approval field | Always | warn |
| CX026 | `codex-approval-reviewer` | Invalid approvals reviewer | Always | error |
| CX027 | `codex-service-tier-type` | `service_tier` is not a string | Always | error |
| CX028 | `codex-bearer-token` | Inline MCP bearer token is forbidden | Always | error |
| CX029 | `codex-agent-threads` | `agents.max_threads` is not an integer greater than zero | Always | error |
| CX030 | `codex-app-approval` | Invalid app default-tools approval mode | Always | error |
| CX031 | `codex-skills-type` | `skills` is not a TOML table | Always | error |
| CX032 | `codex-profile-type` | `profile` is not a string | Always | error |
| CX033 | `codex-top-key` | Unknown top-level Codex key | Always | warn |
| CX034 | `codex-feature-key` | Unknown Codex feature flag | Always | warn |
| CX035 | `codex-network-field` | Unknown `permissions.network` field | Always | warn |
| CX036 | `codex-windows-sandbox` | Invalid Windows sandbox mode | Always | error |
| CX061 | `codex-approval-shape` | Granular approval policy has an invalid shape or field type | Always | error |
| CX062 | `codex-config-container-type` | A structured Codex configuration value is not a table | Always | error |

CX023 and CX024 are advisory positivity checks; they are not claims about a
Codex parser rejection.

### Codex Instruction, Plugin, and Skill Rules (CX)

These optional rules run in Basic and Plugin modes whenever the corresponding
Codex surface exists. Codex-specific `AGENTS.md` policy runs only when Codex is
active; the shared instruction rules above run independently.

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| CX040 | `codex-project-doc-budget` | Active Codex project-document chain exceeds its cumulative byte budget | Always | warn |
| CX045 | `codex-project-doc-conflict` | Live Codex project-document assertion conflicts with `.codex/config.toml` | Always | warn |
| CX046 | `codex-plugin-path` | Deprecated — no longer emitted; any recognized manifest directory establishes a valid plugin root | Always | error |
| CX047 | `codex-plugin-invalid` | Codex plugin manifest is unreadable, invalid JSON, a non-object root, or has an invalid field type | Always | error |
| CX048--CX049 | — | Codex plugin name is missing/blank or not kebab-case | Always | error |
| CX050--CX052 | — | Component path lacks `./`, escapes the plugin root, or is bare `./` | Always | error |
| CX053--CX056 | — | Too many/empty/over-long `interface.defaultPrompt` entries or an unusable interface URL | Always | warn |
| CX057 | `codex-plugin-asset` | Interface asset path (`composerIcon`, `logo`, `logoDark`, `screenshots[]`) is bare `./`, missing `./`, or escapes the plugin root | Always | error |
| CX058 | `codex-plugin-hooks` | Deprecated — no longer emitted; Codex loads plugin-bundled hooks, and hook path strings participate in CX050–CX052 | Always | warn |
| CX059 | `codex-plugin-description` | Codex plugin manifest `description` is missing, blank, or not a string (agent-lint install-surface recommendation) | Always | warn |
| CX060 | `codex-skill-frontmatter` | Codex skill uses ignored behavior frontmatter (`allowed-tools`, `when_to_use`, `argument-hint`, `arguments`, `disable-model-invocation`, `user-invocable`, `model`, `effort`, `context`, `agent`, `hooks`, `paths`, `shell`). Nested/block-scalar/quoted-portable fields stay clean; surfaces include nested `.agents/skills` and selected plugin skill roots | Always | warn |
| CX063 | `codex-prompt-field` | `interface.default_prompt` / `interface.default_prompts` are ignored by Codex; rename to `interface.defaultPrompt` | Always | warn |

CX040 uses Codex's default 32,768-byte cumulative project-document budget unless
`.codex/config.toml` sets `project_doc_max_bytes`; its compatibility lookup alias is
`codex-agents-limit`. CX045's compatibility lookup alias is `codex-agents-conflict`.
Codex plugin discovery
recognizes `.codex-plugin/`, `.claude-plugin/`, and `.cursor-plugin/`
`plugin.json` beneath every plugin root (Codex precedence order); a manifest
directory is matched by exact parent-directory component, never by path suffix.
CX047–CX055 are runtime-compatibility checks, CX048/CX049 are the public
authoring name policy, and CX056/CX057/CX059/CX063 are publishing/install
quality. The three-prompt and 128-Unicode-scalar limits, the recognized manifest
paths, and the canonical `interface.defaultPrompt` field come from
[`openai/codex` commit `7442f5f`](https://github.com/openai/codex/blob/7442f5f9323d116755dfe630e22c931a8aeaa5c7/codex-rs/core-plugins/src/manifest.rs)
and the public [authoring documentation](https://developers.openai.com/codex/plugins/build#plugin-structure),
checked on 2026-07-21. `interface.default_prompt` and
`interface.default_prompts` are read by no Codex runtime; each triggers CX063.
CX060 is a compatibility advisory (default warning, non-autofixable): Codex
loads the skill but ignores the listed behavior fields. It parses frontmatter
through the shared strict YAML parser and inspects only top-level mapping keys
(so nested mappings, block-scalar body text, and comments never count). Hard
negatives include nested/block-scalar false positives, quoted portable fields,
and `SKILL.md` nested below a skill directory. Discovery covers every
repository `.agents/skills/<skill>/SKILL.md` and selected plugin skill roots
from `platforms::codex_plugin_manifests` (declared `skills` when non-empty,
otherwise default `skills/`). Sources: Codex skills authoring guide, Agent
Skills frontmatter spec, Claude Code frontmatter reference, and openai/codex
`core-skills` loader at `7442f5f` (retrieved 2026-07-21).

## Cursor Configuration Rules (CU / CR)

Cursor rules run when a Cursor surface is present and are otherwise inert.
They run in both Basic and Plugin modes.

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| CU001 | `cursor-rule-empty` | A `**/.cursor/rules/**/*.mdc` file or `.cursorrules` has no instructions | Always | error |
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
| CU014 | `cursor-agent-invalid` | Cursor subagent frontmatter is invalid (present fields and filename-derived identifiers) | Always | error |
| CU015 | `cursor-body-empty` | Cursor subagent body is empty | Always | warn |
| CU016 | `cursor-environment-invalid` | `.cursor/environment.json` schema is invalid | Always | error |
| CU017 | `cursor-hook-invalid` | Cursor hook timeout, loop limit, fail-closed, or matcher type is invalid | Always | warn |
| CU018 | `cursor-prompt-missing` | Prompt hook lacks a non-empty string `prompt` | Always | error |
| CU019 | `cursor-model-invalid` | Present prompt hook `model` is not a non-empty string | Always | error |
| CU020 | `cursor-rule-extension` | A `.md` file below a repository-wide `.cursor/rules/` directory is not a live Cursor rule; rename it to the same basename with `.mdc` | Always | warn |
| CR-SK-001 | `cursor-skill-unsupported` | Cursor skill uses a frontmatter key other than `name`, `description`, `paths`, `disable-model-invocation`, or `metadata`; it checks the active recursive Cursor runtime inventory, including shared `.agents/skills` locations | Always | warn |

## Hygiene / Scripts Rules (G)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| G001 | `pwd-in-skill` | Existing bundled plugin asset uses `$PWD/` or `${PWD}/` instead of `${CLAUDE_PLUGIN_ROOT}/` | Plugin | error |
| G002 | `script-ref-missing` | Script reference missing on disk | Always | error |
| G003 | `script-not-executable` | Directly executed script file is not executable (Unix only) | Always | error |
| G004 | `dead-script` | Script has no executable invocation reference | Plugin | warn |
| G005 | `security-policy-missing` | No repository-local `SECURITY.md` in a GitHub-supported location (root, `.github/`, or `docs/`) | Plugin | warn |
| G006 | `todo-in-skill` | Syntactic unfinished-work marker (`TODO:` / `FIXME(owner):` / comment or unchecked-task form) in published skill body | Plugin | warn |
| G007 | `todo-in-agent` | Syntactic unfinished-work marker in agent `.md` body | Plugin | warn |
| G008 | `gh-inline-body` | Shipped script passes a GitHub body or release notes inline instead of using a file-backed option | Always | warn |
| G009 | `bash-replacement-unsafe` | Bash pattern substitution uses a replacement that can reinterpret `&` | Always | error |
| G010 | `bash32-incompatible` | Shipped shell uses syntax unavailable in macOS Bash 3.2 | Always | warn |
| G011 | `awk-regex-nonascii` | Awk regex operand contains non-ASCII text with locale-dependent behavior | Always | warn |
| G012 | `hardcoded-machine-path` | `SKILL.md` uses a machine-specific or ambiguous runtime path | Plugin | warn |

G002 resolves `${CLAUDE_PLUGIN_ROOT}`, `${CLAUDE_PROJECT_DIR}`, `$CLAUDE_PLUGIN_ROOT`, `$CLAUDE_PROJECT_DIR`, and `$PWD` forms lexically within the repository. Escaping `..` paths are unresolvable; existing directories are valid references, while `*` globs expand safely within the repository and `?`/`[` patterns are ignored. Workflow YAML scans `run` values and block-scalar continuation lines, not descriptive keyed values. G003 is Unix-only and applies only when a regular file is invoked directly; interpreter-launched and sourced files do not require an execute bit. G004 treats supported command surfaces and allowed Claude permission rules as reachability, but not comments, prose, self-references, directories, or denied permissions. G004 is a warning because static reachability is incomplete; use the existing reason-bearing per-file suppression for intentional inventory entries.

G005 accepts an exact-case, regular, non-symlink `SECURITY.md` in the
repository root, `.github/`, or `docs/`, following GitHub's supported
community-health locations and documented `.github` → root → `docs`
precedence ([supported file types](https://docs.github.com/en/communities/setting-up-your-project-for-healthy-contributions/creating-a-default-community-health-file#supported-file-types),
retrieved 2026-07-21). A directory, a wrong-case name, or a symlink does not
satisfy the rule. An organization default served from a public `.github`
repository cannot be observed locally, so G005 stays a warning and normal
suppression is the escape hatch for that inherited policy.

G009-G011 share one shell/awk lexical layer (`validators/shell.rs`) rather than
matching regexes against raw lines: a scanner masks comments, single-quoted
text, ANSI-C `$'...'`, and double-quoted literals while keeping executable code
and live expansions, so an inert construct in a comment or string is never
mistaken for live code. G009 flags a pattern substitution `${v/pat/repl}` or
`${v//pat/repl}` only when the replacement still carries a live expansion (a
bare `$rep`, `${rep}`, `$(cmd)`, positional, or arithmetic form) that can inject
an unquoted `&`; quoted, ANSI-C, escaped, and literal replacements stay clean.
G010 flags a sourced, probe-verified matrix of Bash-4+ syntax, builtins, and
options (`declare -A`/`-g`/`-n`, `typeset -A`, `local -n`, `mapfile`/`readarray`,
case conversion, negative subscripts, stepped brace expansion, `coproc`, `&>>`,
`;&`, `;;&`, `|&`, `shopt -s globstar`, `wait -n`). Two additional G010 hazards
are gated on the option that makes them fatal: an `if`/`elif` `command <cmd>`
condition fires only when the file lexically enables `set -e` (or is a sourced
`.inc.bash` library), and an unguarded empty-array `"${arr[@]}"` fires only
under `set -u` (or `.inc.bash`), analyzed with conservative, function-scoped
control flow that stays silent on any ambiguous branch. G011 analyzes the actual
awk regex operand — a `/.../` or string literal used in `~`/`!~`, `match`,
`sub`/`gsub`/`gensub`, `split`/`patsplit`, an `FS`/`-F` value, or a `-v`
variable traced to a regex use — so display-only text and ASCII regexes stay
clean.

G008-G011 use conventional script discovery unless `[lint].script-inventory`
is configured. Script discovery and the inventory use one matrix: `.sh`,
`.bash`, `.inc.bash`, `.awk`, `.py`, `.js`, `.mjs`, and extensionless files.
An explicit inventory remains authoritative when global exclusions match an
entry and is scanned in deterministic order on every run. G009 and G010 analyze
shell files (`.sh`, `.bash`, `.inc.bash`); G011 analyzes awk commands inside
shell files plus standalone `.awk` files; `.py`, `.js`, `.mjs`, and
extensionless files receive only G008. G009 stays a hard error for a definite
renderer hazard; G010 and G011 are default warnings, and a repository targeting
Bash 3.2 or ASCII-only portable awk promotes them explicitly with
`error = ["G010", "G011"]`. `script-inventory` selects files only and never
implies severity.

G001 applies only when a `$PWD/` or `${PWD}/` reference resolves to an existing
bundled plugin component (`scripts`, `skills`, `agents`, `commands`, `hooks`,
`output-styles`, `themes`, `monitors`, or `.claude-plugin`), and its conservative
autofix replaces only that prefix. G012 reports every other `$PWD` reference and
machine-specific POSIX or Windows path without autofixing it; select
`${CLAUDE_PLUGIN_ROOT}`, `${CLAUDE_PROJECT_DIR}`, or `${CLAUDE_PLUGIN_DATA}`
according to whether the referenced path is bundled, project-local, or persistent.

G006, G007, and D003 share one unfinished-work marker classifier. A marker word
(`TODO` / `FIXME` / `HACK` / `XXX`, case-insensitive) warns only when it is
syntactically recognizable as unfinished work: at line start after optional
whitespace and an optional Markdown heading, list, or unchecked-task prefix, or
immediately after a source-comment introducer (`#`, `//`, `/*`, `*`, or `<!--`),
and followed by `:`, an owner parenthesis, or end-of-line. Frontmatter, fenced
or indented code, inline code, block quotes, Markdown links/images, and balanced
quoted prose are ignored; qualifying HTML comments, headings, and unchecked-task
labels remain visible. Each rule reports at most once per file — the first
qualifying marker in document order (top-to-bottom, leftmost on the line) — with
a structured source span, marker-only evidence, and a fixed removal suggestion.
None of these rules is autofixable. Prose that discusses, prohibits, quotes, or
teaches about marker words stays clean unless it also contains a syntactic debt
marker.

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

U009 is an agent-lint security convention stricter than the manifest schema,
which structurally permits a `default` alongside `sensitive`. A `default` in the
world-readable manifest is injected into every `${user_config.KEY}` consumer at
runtime, so a `default` on a `sensitive: true` option, or a secret-shaped
string/string-array `default` on any option, is treated as a committed
credential. No output channel (message, evidence, or suggestion) ever echoes the
default value.

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| U001 | `userconfig-not-object` | Present `userConfig` container in `.claude-plugin/plugin.json` (top-level or channel) is not an object | Plugin | error |
| U002 | `userconfig-desc-missing` | `userConfig` entry missing, non-string, empty, or whitespace-only description | Plugin | error |
| U004 | `userconfig-sensitive-type` | Present `userConfig` `sensitive` value is not a boolean | Plugin | error |
| U005 | `userconfig-title-missing` | `userConfig` entry missing, non-string, empty, or whitespace-only title | Plugin | error |
| U006 | `userconfig-type-missing` | `userConfig` entry missing or invalid `type` (must be `string`, `number`, `boolean`, `directory`, or `file`) | Plugin | error |
| U007 | `userconfig-key-invalid` | `userConfig` key is not a valid identifier (`^[A-Za-z_][A-Za-z0-9_]*$`) | Plugin | error |
| U008 | `userconfig-option-invalid` | `userConfig` option entry is not an object, has an unknown field, or has an invalid optional/semantic field shape | Plugin | error |
| U009 | `userconfig-default-secret` | `userConfig` option `default` is declared for a `sensitive: true` option, or is a secret-shaped string/string-array literal (shared possible-secret heuristic); the default value is never echoed | Plugin | warn |

## MCP Configuration Rules (P)

MCP input is adapted by platform before P rules run. Claude standalone inputs
are repository `.mcp.json` files (at the project or plugin root); Claude plugin
manifests may provide `.claude-plugin/plugin.json#mcpServers` as an inline
server-map object, a string path to an MCP config file, or an array of string
paths and inline server-map objects; and Cursor project input is
`.cursor/mcp.json`. Path-referenced plugin MCP configs (after substituting a
leading `${CLAUDE_PLUGIN_ROOT}/` with the plugin root) receive the same Claude
standalone P-rule walk when the file exists; missing paths stay M-owned.
Invalid JSON in an inline plugin manifest remains M002-owned. Claude settings
files and Codex TOML are not MCP P-rule inputs: settings retain their
Claude-validator diagnostics, and `.codex/config.toml` remains CX-owned. These
rules run in both Basic and Plugin modes. The Claude transport matrix follows
Claude Code's [remote HTTP](https://code.claude.com/docs/en/mcp#option-1-add-a-remote-http-server),
[WebSocket](https://code.claude.com/docs/en/mcp#option-4-add-a-remote-websocket-server),
and [legacy SSE](https://code.claude.com/docs/en/mcp#option-2-add-a-remote-sse-server)
documentation: `streamable-http` is the HTTP alias, `ws` uses WebSocket URLs,
and legacy `sse` remains supported but deprecated. A Claude entry that has a
`url` member but no `type` is a documented configuration error owned by P027
(Claude Code skips that server); P009 remains for url-less stdio entries.

P027 owns MCP document and entry shape failures. Standalone Claude and Cursor
MCP documents require a top-level object-valued `mcpServers`. An inline plugin
manifest may omit `mcpServers`; a present value must be an object, a string
config path, or an array whose elements are string paths or inline server-map
objects (other element types are P027). Cursor selector presence still requires
exactly one of `command` or `url`, and the selected value must be a non-empty,
non-blank string. P024 is reserved for an entry that is exactly `{}`; scalar,
null, and array server entries are P027. Duplicate top-level `mcpServers` keys
are P027, while P023 remains limited to duplicate names in a valid server map.
P027 is diagnostic-only and has no autofix.

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

P026 reserved names follow Claude Code's documented built-in server list
([MCP docs](https://code.claude.com/docs/en/mcp), retrieved 2026-07-21): `workspace`,
`claude-in-chrome`, `computer-use`, `Claude Preview`, and `Claude Browser`
(exact, case-sensitive). Cursor MCP is unaffected.

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| P001 | `mcp-json-invalid` | MCP configuration is not valid JSON | Always | error |
| P009 | `mcp-stdio-command` | `stdio` server (including omitted type) has no non-empty `command` | Always | error |
| P010 | `mcp-http-url` | Remote server has no syntactically valid URL for its selected transport (`http`/`streamable-http`/`sse`: `http(s)`; `ws`: `ws(s)`) | Always | error |
| P011 | `mcp-type-invalid` | Server `type` is not `stdio`, `http`, `streamable-http`, `sse`, or `ws` | Always | error |
| P012 | `mcp-sse-deprecated` | `sse` transport is deprecated; use Streamable HTTP | Always | warn |
| P017 | `mcp-insecure-url` | Non-local `http://` or `ws://` server URL is insecure (use `https://` or `wss://`). `localhost` and `*.localhost` (RFC 6761) are local | Always | error |
| P018 | `mcp-env-secret` | Secret-like environment variable contains a literal plaintext value | Always | warn |
| P019 | `mcp-command-dangerous` | Server command contains a dangerous shell pattern | Always | warn |
| P022 | `mcp-args-invalid` | `args` is not an array of strings | Always | error |
| P023 | `mcp-duplicate-server` | `mcpServers` contains a duplicate server name | Always | error |
| P024 | `mcp-server-empty` | Server configuration is an empty object | Always | error |
| P025 | `mcp-alwaysload-invalid` | `alwaysLoad` is not a boolean | Always | warn |
| P026 | `mcp-server-reserved` | Server name is one of Claude Code's reserved built-ins (`workspace`, `claude-in-chrome`, `computer-use`, `Claude Preview`, `Claude Browser`) | Always | error |
| P027 | `mcp-structure-invalid` | Required standalone server map is missing or invalid; an inline map, path/array form, server entry, duplicate top-level map key, url-without-type Claude entry, or adapter selector has an invalid shape | Always | error |

## Docs Rules (D)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| D001 | `docs-ref-missing` | Docs reference in `CLAUDE.md` Canonical sources not found on disk | Always | error |
| D002 | `claudemd-too-large` | `CLAUDE.md` exceeds 500 lines (advisory) | Always | warn |
| D003 | `todo-in-docs` | Syntactic unfinished-work marker in root `CLAUDE.md` | Always | warn |
| D004 | `claude-import-large` | Repository-local `CLAUDE.md` `@`-import closure exceeds a global, path-specific, or total line budget | Always | warn |
| D005 | `inline-path-missing` | Path-shaped inline-code pointer in a configured instruction file is dead or escapes the repository | Always | warn |

D001 runs in every Basic or Plugin mode when an included root `CLAUDE.md` is
present. It owns only the level-2 heading whose trimmed case-insensitive text is
exactly `canonical sources` (similarly prefixed headings are not matches),
continues through nested level-3+ subsections, and ends at the next level-1/2
heading or EOF. Within that section it recognizes repository-root `docs/...md`
references from local Markdown link destinations, inline-code nodes, and plain
prose/list tokens whose whole token begins with `docs/` (so
`website/docs/intro.md` and `mydocs/foo.md` are out of scope). Fenced/indented
code, images, block quotes, and identifiable example scopes are skipped.
Link destinations are percent-decoded once; all candidates strip one `#fragment`
before the shared repository-root safe probe. One error is emitted per distinct
normalized target at the first path-token span, with bounded evidence and
suggestion `create the canonical document or correct this reference`. Missing,
non-regular, escaping, and symlink-component targets all fail; the resolver never
follows or discloses an outside path. Not autofixable.

D002 is a project-maintainability advisory (not a Claude platform hard limit). It
warns when the root `CLAUDE.md` has more than 500 Unicode text lines (`str::lines`
semantics: a final terminator adds no phantom line), emits one file-level finding
with evidence `{N} lines` and suggestion
`split detailed guidance into referenced documents`, and fabricates no point span.
It shares D001's Always dispatch, exclusion/suppression policy, and non-autofixable
status. Unreadable/non-UTF-8 root files keep both rules silent; a missing optional
root stays clean.

D005 scans inline-code pointers from the shared Markdown adapter in the
configured `instruction-files` (default `AGENTS.md`, `SECURITY.md`, and
`CLAUDE.md`). It uses the shared I003 lexical classifier after normalizing `\`
to `/`, then the repository-root-relative safe probe after applying its
`inline-path-prefixes` scope. Its documented
`<!-- lint-doc-pointer-paths: ok reason -->` marker is intentionally D005-only.

D003 uses the same unfinished-work classifier and Markdown context policy as
G006/G007 (see Hygiene / Scripts Rules above). It runs in every Basic or Plugin
mode when an included root `CLAUDE.md` is present, reports the first qualifying
marker only, and is not autofixable.

## Link/import integrity Rules (L)

These rules validate the repository-local `@import` graph and relative
markdown-link integrity of each configured instruction file (see
`instruction-files` in [configuration](configuration.md); default
`AGENTS.md`, `SECURITY.md`, `CLAUDE.md`). Import tokens may name any file
extension (or none), are source-relative after lexical normalization, and are
read only when they remain regular UTF-8 files inside the repository. The
extractor ignores frontmatter, code, links, quotes, blockquotes, and examples.
Absolute and `~/` imports are supported by Claude Code but intentionally sit
outside repository integrity and D004 budget scope. Excluded imported sources
are opaque: they are neither parsed nor measured, while a reference to one is
accepted for L001. Cycles and depth use the complete shared graph, so a graph
can report both independently.

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| L001 | `import-path-missing` | Repository-relative `@import` target is missing or unreadable | Always | error |
| L002 | `circular-import` | Canonical reachable `@import` cycle detected once per configured root | Always | error |
| L003 | `import-depth-exceeded` | Longest simple repository-local `@import` path exceeds 5 hops | Always | error |
| L004 | `duplicate-import` | Duplicate normalized direct `@import` edge within one instruction file | Always | warn |
| L005 | `broken-markdown-link` | Broken relative `[text](path.md)` link target in a configured instruction file; external URLs, anchors, image nodes, and non-`.md` destinations are skipped | Always | warn |
| L006 | `npm-script-missing` | Actionable `npm run` / `npm run-script` commands in configured instruction files whose script is missing from the root `package.json` `scripts` object. Scans inline code, shell fences (`bash`/`sh`/`shell`/`zsh`/`console`, with one leading `$` or `>` console prompt plus its trailing space stripped), and live prose `npm` tokens after line start/whitespace/opening punctuation; skips non-shell fences, quotes, example scopes, same-clause prose negation, package-qualified flags (`--workspace`/`-w`/`--workspaces`/`--prefix`/`--global`/`-g`), substitutions/heredocs/malformed shell, and value-taking non-qualifier flags. Silent when root `package.json` is absent, unreadable, invalid, or has no object-valued `scripts`. One diagnostic per missing script per file at the first command span, with script-name evidence and a correction suggestion. Not autofixable. | Always | warn |

L005 uses the shared Comrak Markdown adapter so image nodes are never treated
as links. Destinations may use CommonMark angle brackets, titles, escaped
parentheses, fragments, and percent-encoded bytes; percent-decoding happens
exactly once before the source-relative safe probe. There is no repository-root
fallback, so a same-named root file cannot shadow a missing nested link. One
finding is emitted per distinct normalized `.md` target per source at the
destination span, with bounded evidence and the same create-or-replace
suggestions as I003/D005.

## Auto-Fixable Rules

When `--autofix` is provided, agent-lint attempts to automatically fix
violations for rules that have purely mechanical, unambiguous fixes. After
all possible fixes are applied, it runs a final validation pass and reports
any remaining issues with normal exit semantics (exit 1 if errors remain).

**Auto-fixable rules (11 of 298):**

| Rule | Code | Fix |
|------|------|-----|
| hook-not-executable | H005 | `chmod +x` on script |
| script-not-executable | G003 | `chmod +x` on script |
| frontmatter-name-mismatch | S006 | Set a single-line canonical `name:` scalar to match the directory, only on surfaces selected for the run |
| frontmatter-field-empty | S007 | Remove a bare empty optional field only when it has no YAML continuation or child lines |
| desc-has-xml | S018 | Strip XML tags from description |
| consecutive-bash | S021 | Merge adjacent bash blocks |
| backslash-path | S022 | Replace every separator in each detected body path run with `/` |
| non-https-url | S031 | `http://` → `https://` (Claude surfaces only: `skills/` and `.claude/skills/`; `.agents/skills/` and `.cursor/skills/` report diagnostics without rewriting). Exempt identifier and reserved-name matches (shared with the checker) are left byte-identical, so an XML namespace such as `xmlns="http://www.w3.org/2000/svg"` is never rewritten |
| frontmatter-backslash | S043 | Replace `\` with `/` in frontmatter |
| tools-list-syntax | S045 | YAML list → comma-separated scalar |
| pwd-in-skill | G001 | Existing bundled asset `$PWD/` or `${PWD}/` → `${CLAUDE_PLUGIN_ROOT}/` |

Each fix is logged to stderr. H005 enforcement and its `chmod +x` autofix are
Unix-only because executable-bit permissions are not available on every
platform.
