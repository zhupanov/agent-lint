# Lint Rules Reference

Agent Lint ships 104 rules across 9 categories. Every rule has a unique
code (e.g., `M001`) and a human-readable name (e.g., `plugin-json-missing`).
Either form can be used in `agent-lint.toml` to configure rule severity.

**Default column key:**

- **error** -- rule fires as an error by default
- **warn** -- rule fires as a warning by default (non-blocking)
- **off** -- rule is silently skipped by default (enable via `[lint] error`)

**Strictness modes** (`--pedantic` / `--all`) override these defaults.
`--pedantic` promotes warnings (both `warn`-listed and default-warning
rules) to errors (except `name-too-long`, `desc-too-long`,
`body-too-long`, `compat-too-long`). `--all` forces every rule to error
regardless of config. See README for details.

**Mode column key:**

- **Plugin** -- runs only when `.claude-plugin/` is present
- **Both** -- runs in both Basic (`.claude/` only) and Plugin modes

## Manifest Rules (M)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| M001 | `plugin-json-missing` | `.claude-plugin/plugin.json` is missing | Plugin | error |
| M002 | `plugin-json-invalid` | `plugin.json` is not valid JSON | Plugin | error |
| M003 | `plugin-field-missing` | `plugin.json` missing required field (`name` or `version`) | Plugin | error |
| M004 | `plugin-version-format` | `plugin.json` version is not strict `MAJOR.MINOR.PATCH` semver | Plugin | error |
| M005 | `marketplace-json-missing` | `marketplace.json` is missing | Plugin | error |
| M006 | `marketplace-json-invalid` | `marketplace.json` is not valid JSON | Plugin | error |
| M007 | `marketplace-field-missing` | `marketplace.json` missing required field (`name` or `owner.name`) | Plugin | error |
| M008 | `marketplace-plugins-empty` | `marketplace.json` plugins array is empty | Plugin | error |
| M009 | `marketplace-plugin-invalid` | `marketplace.json` plugin entry has invalid `name` or `source` | Plugin | error |
| M010 | `marketplace-enriched-missing` | `marketplace.json` missing `owner.email` or plugin `category` | Plugin | warn |
| M011 | `plugin-enriched-missing` | `plugin.json` missing `description`, `author.email`, or `keywords` | Plugin | warn |

## Hooks Rules (H)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| H001 | `hooks-json-missing` | `hooks/hooks.json` is missing | Plugin | error |
| H002 | `hooks-json-invalid` | `hooks/hooks.json` is not valid JSON | Plugin | error |
| H003 | `hooks-key-missing` | `hooks.json` missing top-level `hooks` key | Plugin | error |
| H004 | `hook-command-missing` | Hook command script missing on disk | Both | error |
| H005 | `hook-not-executable` | Hook command script not executable | Both | error |
| H006 | `settings-json-invalid` | `.claude/settings.json` is not valid JSON | Both | error |
| H007 | `hooks-array-empty` | `hooks.json` has empty `hooks` array | Plugin | error |

## Skills Rules (S)

### Structure and Frontmatter (S001--S008)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| S001 | `skills-dir-missing` | `skills/` directory is missing (deprecated — no longer fires) | Plugin | error |
| S002 | `skill-md-missing` | `skills/{name}/` missing `SKILL.md` | Plugin | error |
| S003 | `no-exported-skills` | No plugin-exported skills found under `skills/` | Plugin | error |
| S004 | `frontmatter-malformed` | `SKILL.md` has malformed frontmatter (must start/end with `---`) | Both | error |
| S005 | `frontmatter-field-missing` | `SKILL.md` missing required field (`name` or `description`) | Both | error |
| S006 | `frontmatter-name-mismatch` | Frontmatter `name` does not match directory name | Plugin | error |
| S007 | `frontmatter-field-empty` | Optional frontmatter field present but empty | Both | error |
| S008 | `shared-md-missing` | Shared markdown reference missing on disk | Plugin | error |

### Name Validation (S009--S013, S033, S049)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| S009 | `name-too-long` | Skill name exceeds 64 characters | Both | error |
| S010 | `name-invalid-chars` | Skill name contains characters outside `[a-z0-9-]` | Both | error |
| S011 | `name-bad-hyphens` | Skill name starts/ends with hyphen or has consecutive hyphens | Both | error |
| S012 | `name-reserved-word` | Skill name contains reserved word (`anthropic` or `claude`) | Both | error |
| S013 | `name-has-xml` | Skill name contains XML/HTML tags | Both | error |
| S033 | `name-vague` | Skill name is too vague/generic (`helper`, `utils`, `tools`, etc.) | Plugin | warn |
| S049 | `name-not-gerund` | Skill name not in gerund (verb+ing) form | Plugin | off |

### Description Validation (S014--S018, S034, S050)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| S014 | `desc-too-long` | Skill description exceeds 1024 characters | Both | error |
| S015 | `desc-truncated` | Skill description exceeds 250 characters (truncated in listings) | Plugin | warn |
| S016 | `desc-uses-person` | Skill description uses first/second person | Plugin | error |
| S017 | `desc-no-trigger` | Skill description lacks trigger context (e.g., "Use when...") | Plugin | error |
| S018 | `desc-has-xml` | Skill description contains XML/HTML tags | Both | error |
| S034 | `desc-too-short` | Skill description under 20 characters | Both | warn |
| S050 | `desc-vague-content` | Skill description content is too vague/generic | Plugin | warn |

### Body Content (S019--S022, S037--S038, S041, S046--S047, S051--S053, S055--S057)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| S019 | `body-too-long` | `SKILL.md` body exceeds 500 lines | Both | warn |
| S020 | `body-empty` | `SKILL.md` has no content after frontmatter | Both | error |
| S021 | `consecutive-bash` | Consecutive bash code blocks that could be combined | Both | warn |
| S022 | `backslash-path` | Windows-style backslash paths in skill content | Both | error |
| S037 | `body-no-refs` | Body exceeds 300 lines with no file references | Plugin | warn |
| S038 | `time-sensitive` | Body contains time-sensitive date/year patterns | Plugin | warn |
| S041 | `fork-no-task` | `context: fork` set but body lacks task instructions | Both | error |
| S046 | `body-no-workflow` | Body exceeds 300 lines with no workflow structure | Plugin | warn |
| S047 | `body-no-examples` | Body exceeds 200 lines with no examples or templates | Plugin | warn |
| S051 | `script-deps-missing` | Script-backed skill lacks dependency/package documentation | Plugin | warn |
| S052 | `script-verify-missing` | Script-backed skill lacks verification/validation steps | Plugin | warn |
| S053 | `terminology-inconsistent` | Uses 3+ variants from the same synonym group | Plugin | warn |
| S055 | `script-errhand-missing` | Script file lacks error handling patterns (`set -e`/`trap` for shell, `try`/`except` for Python) | Plugin | warn |
| S056 | `body-no-default` | Body lists alternatives without stating a default recommendation | Plugin | warn |
| S057 | `magic-number-undoc` | Undocumented magic number in code block (no justification comment) | Plugin | warn |

### Frontmatter Field Types (S023--S027)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| S023 | `bool-field-invalid` | Boolean fields (`user-invocable`, `disable-model-invocation`) must be `true`/`false` | Both | error |
| S024 | `context-field-invalid` | `context` field must be `fork` (if present) | Both | error |
| S025 | `effort-field-invalid` | `effort` field must be `low`/`medium`/`high`/`max` (if present) | Both | error |
| S026 | `shell-field-invalid` | `shell` field must be `bash`/`powershell` (if present) | Both | error |
| S027 | `skill-unreachable` | Skill unreachable: `disable-model-invocation: true` AND `user-invocable: false` | Both | error |

### Extended Frontmatter (S035, S039--S040, S042--S045)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| S035 | `compat-too-long` | `compatibility` field exceeds 500 characters | Both | warn |
| S039 | `metadata-not-string` | Metadata map values must be strings | Both | error |
| S040 | `tools-unknown` | `allowed-tools` lists unrecognized tool name | Both | warn |
| S042 | `dmi-empty-desc` | `disable-model-invocation: true` with empty/missing description | Both | error |
| S043 | `frontmatter-backslash` | Windows-style backslash paths in frontmatter fields | Both | error |
| S044 | `mcp-tool-unqualified` | MCP tool reference without server prefix | Both | warn |
| S045 | `tools-list-syntax` | `allowed-tools` uses YAML list syntax instead of comma-separated scalar | Both | warn |

### Cross-Field and Structural (S028--S032, S036, S048, S054)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| S028 | `args-no-hint` | Body uses `$ARGUMENTS` but frontmatter has no `argument-hint` field | Both | error |
| S029 | `nested-ref-deep` | Referenced shared `.md` itself references other shared `.md` files | Plugin | warn |
| S030 | `orphaned-skill-files` | Files in skill `scripts/` not referenced from `SKILL.md` | Both | error |
| S031 | `non-https-url` | Non-HTTPS URL (`http://`) found in skill content | Both | error |
| S032 | `hardcoded-secret` | Potential hardcoded secret/API key detected | Both | error |
| S036 | `ref-no-toc` | Referenced `.md` file exceeds 100 lines with no `##` headings | Plugin | warn |
| S048 | `ref-name-generic` | Non-descriptive reference file name in skill directory | Both | warn |
| S054 | `desc-body-misalign` | Skill description keywords not reflected in body | Plugin | warn |

## Agent Rules (A)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| A001 | `agents-dir-missing` | `agents/` directory is missing | Plugin | error |
| A002 | `agent-frontmatter-malformed` | Agent `.md` has malformed frontmatter | Plugin | error |
| A003 | `agent-field-missing` | Agent `.md` missing required field (`name` or `description`) | Plugin | error |
| A004 | `no-agent-files` | `agents/` has no `.md` files | Plugin | error |
| A005 | `template-file-missing` | `skills/shared/reviewer-templates.md` is missing | Plugin | warn |
| A006 | `template-marker-missing` | Agent `.md` missing "Derived from" marker | Plugin | warn |
| A007 | `template-count-mismatch` | Agent-template count mismatch | Plugin | warn |
| A008 | `agent-desc-long` | Agent description exceeds 1024 characters | Plugin | error |
| A009 | `agent-desc-short` | Agent description under 20 characters | Plugin | error |
| A010 | `agent-name-invalid` | Agent name contains characters outside `[a-z0-9-]` | Plugin | error |
| A011 | `agent-desc-redundant` | Agent description too similar to agent name | Plugin | error |

## Hygiene / Scripts Rules (G)

| Code | Name | Description | Mode | Default |
|------|------|-------------|------|---------|
| G001 | `pwd-in-skill` | `SKILL.md` uses `$PWD/` or hardcoded path instead of `${CLAUDE_PLUGIN_ROOT}/` | Plugin | error |
| G002 | `script-ref-missing` | Script reference missing on disk | Both | error |
| G003 | `script-not-executable` | Script file not executable | Both | error |
| G004 | `dead-script` | Dead script with no structured invocation reference | Plugin | error |
| G005 | `security-md-missing` | `SECURITY.md` is missing from repo root | Plugin | warn |
| G006 | `todo-in-skill` | `TODO`/`FIXME`/`HACK`/`XXX` marker in published skill body | Plugin | warn |
| G007 | `todo-in-agent` | `TODO`/`FIXME`/`HACK`/`XXX` marker in agent `.md` body | Plugin | warn |

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
