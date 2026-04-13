# Lint Rules Reference

Claude Lint ships 88 rules across 9 categories. Every rule has a unique
code (e.g., `M001`) and a human-readable name (e.g., `plugin-json-missing`).
Either form can be used in `claude-lint.toml` to ignore or downgrade a rule.

**Mode column key:**

- **Plugin** -- runs only when `.claude-plugin/` is present
- **Both** -- runs in both Basic (`.claude/` only) and Plugin modes

## Manifest Rules (M)

| Code | Name | Description | Mode |
|------|------|-------------|------|
| M001 | `plugin-json-missing` | `.claude-plugin/plugin.json` is missing | Plugin |
| M002 | `plugin-json-invalid` | `plugin.json` is not valid JSON | Plugin |
| M003 | `plugin-field-missing` | `plugin.json` missing required field (`name` or `version`) | Plugin |
| M004 | `plugin-version-format` | `plugin.json` version is not strict `MAJOR.MINOR.PATCH` semver | Plugin |
| M005 | `marketplace-json-missing` | `marketplace.json` is missing | Plugin |
| M006 | `marketplace-json-invalid` | `marketplace.json` is not valid JSON | Plugin |
| M007 | `marketplace-field-missing` | `marketplace.json` missing required field (`name` or `owner.name`) | Plugin |
| M008 | `marketplace-plugins-empty` | `marketplace.json` plugins array is empty | Plugin |
| M009 | `marketplace-plugin-invalid` | `marketplace.json` plugin entry has invalid `name` or `source` | Plugin |
| M010 | `marketplace-enriched-missing` | `marketplace.json` missing `owner.email` or plugin `category` | Plugin |
| M011 | `plugin-enriched-missing` | `plugin.json` missing `description`, `author.email`, or `keywords` | Plugin |

## Hooks Rules (H)

| Code | Name | Description | Mode |
|------|------|-------------|------|
| H001 | `hooks-json-missing` | `hooks/hooks.json` is missing | Plugin |
| H002 | `hooks-json-invalid` | `hooks/hooks.json` is not valid JSON | Plugin |
| H003 | `hooks-key-missing` | `hooks.json` missing top-level `hooks` key | Plugin |
| H004 | `hook-command-missing` | Hook command script missing on disk | Both |
| H005 | `hook-not-executable` | Hook command script not executable | Both |
| H006 | `settings-json-invalid` | `.claude/settings.json` is not valid JSON | Both |

## Skills Rules (S)

### Structure and Frontmatter (S001--S008)

| Code | Name | Description | Mode |
|------|------|-------------|------|
| S001 | `skills-dir-missing` | `skills/` directory is missing | Plugin |
| S002 | `skill-md-missing` | `skills/{name}/` missing `SKILL.md` | Plugin |
| S003 | `no-exported-skills` | No plugin-exported skills found under `skills/` | Plugin |
| S004 | `frontmatter-malformed` | `SKILL.md` has malformed frontmatter (must start/end with `---`) | Both |
| S005 | `frontmatter-field-missing` | `SKILL.md` missing required field (`name` or `description`) | Both |
| S006 | `frontmatter-name-mismatch` | Frontmatter `name` does not match directory name | Plugin |
| S007 | `frontmatter-field-empty` | Optional frontmatter field present but empty | Both |
| S008 | `shared-md-missing` | Shared markdown reference missing on disk | Plugin |

### Name Validation (S009--S013, S033)

| Code | Name | Description | Mode |
|------|------|-------------|------|
| S009 | `name-too-long` | Skill name exceeds 64 characters | Both |
| S010 | `name-invalid-chars` | Skill name contains characters outside `[a-z0-9-]` | Both |
| S011 | `name-bad-hyphens` | Skill name starts/ends with hyphen or has consecutive hyphens | Both |
| S012 | `name-reserved-word` | Skill name contains reserved word (`anthropic` or `claude`) | Both |
| S013 | `name-has-xml` | Skill name contains XML/HTML tags | Both |
| S033 | `name-vague` | Skill name is too vague/generic (`helper`, `utils`, `tools`, etc.) | Plugin |

### Description Validation (S014--S018, S034)

| Code | Name | Description | Mode |
|------|------|-------------|------|
| S014 | `desc-too-long` | Skill description exceeds 1024 characters | Both |
| S015 | `desc-truncated` | Skill description exceeds 250 characters (truncated in listings) | Plugin |
| S016 | `desc-uses-person` | Skill description uses first/second person | Plugin |
| S017 | `desc-no-trigger` | Skill description lacks trigger context (e.g., "Use when...") | Plugin |
| S018 | `desc-has-xml` | Skill description contains XML/HTML tags | Both |
| S034 | `desc-too-short` | Skill description under 20 characters | Both |

### Body Content (S019--S022, S037--S038, S041)

| Code | Name | Description | Mode |
|------|------|-------------|------|
| S019 | `body-too-long` | `SKILL.md` body exceeds 500 lines | Both |
| S020 | `body-empty` | `SKILL.md` has no content after frontmatter | Both |
| S021 | `consecutive-bash` | Consecutive bash code blocks that could be combined | Both |
| S022 | `backslash-path` | Windows-style backslash paths in skill content | Both |
| S037 | `body-no-refs` | Body exceeds 300 lines with no file references | Plugin |
| S038 | `time-sensitive` | Body contains time-sensitive date/year patterns | Plugin |
| S041 | `fork-no-task` | `context: fork` set but body lacks task instructions | Both |

### Frontmatter Field Types (S023--S027)

| Code | Name | Description | Mode |
|------|------|-------------|------|
| S023 | `bool-field-invalid` | Boolean fields (`user-invocable`, `disable-model-invocation`) must be `true`/`false` | Both |
| S024 | `context-field-invalid` | `context` field must be `fork` (if present) | Both |
| S025 | `effort-field-invalid` | `effort` field must be `low`/`medium`/`high`/`max` (if present) | Both |
| S026 | `shell-field-invalid` | `shell` field must be `bash`/`powershell` (if present) | Both |
| S027 | `skill-unreachable` | Skill unreachable: `disable-model-invocation: true` AND `user-invocable: false` | Both |

### Extended Frontmatter (S035, S039--S040, S042--S043)

| Code | Name | Description | Mode |
|------|------|-------------|------|
| S035 | `compat-too-long` | `compatibility` field exceeds 500 characters | Both |
| S039 | `metadata-not-string` | Metadata map values must be strings | Both |
| S040 | `tools-unknown` | `allowed-tools` lists unrecognized tool name | Both |
| S042 | `dmi-empty-desc` | `disable-model-invocation: true` with empty/missing description | Both |
| S043 | `frontmatter-backslash` | Windows-style backslash paths in frontmatter fields | Both |

### Cross-Field and Structural (S028--S032, S036)

| Code | Name | Description | Mode |
|------|------|-------------|------|
| S028 | `args-no-hint` | Body uses `$ARGUMENTS` but frontmatter has no `argument-hint` field | Both |
| S029 | `nested-ref-deep` | Referenced shared `.md` itself references other shared `.md` files | Plugin |
| S030 | `orphaned-skill-files` | Files in skill `scripts/` not referenced from `SKILL.md` | Both |
| S031 | `non-https-url` | Non-HTTPS URL (`http://`) found in skill content | Both |
| S032 | `hardcoded-secret` | Potential hardcoded secret/API key detected | Both |
| S036 | `ref-no-toc` | Referenced `.md` file exceeds 100 lines with no `##` headings | Plugin |

## Agent Rules (A)

| Code | Name | Description | Mode |
|------|------|-------------|------|
| A001 | `agents-dir-missing` | `agents/` directory is missing | Plugin |
| A002 | `agent-frontmatter-malformed` | Agent `.md` has malformed frontmatter | Plugin |
| A003 | `agent-field-missing` | Agent `.md` missing required field (`name` or `description`) | Plugin |
| A004 | `no-agent-files` | `agents/` has no `.md` files | Plugin |
| A005 | `template-file-missing` | `skills/shared/reviewer-templates.md` is missing | Plugin |
| A006 | `template-marker-missing` | Agent `.md` missing "Derived from" marker | Plugin |
| A007 | `template-count-mismatch` | Agent-template count mismatch | Plugin |
| A008 | `agent-desc-long` | Agent description exceeds 1024 characters | Plugin |
| A009 | `agent-desc-short` | Agent description under 20 characters | Plugin |
| A010 | `agent-name-invalid` | Agent name contains characters outside `[a-z0-9-]` | Plugin |

## Hygiene / Scripts Rules (G)

| Code | Name | Description | Mode |
|------|------|-------------|------|
| G001 | `pwd-in-skill` | `SKILL.md` uses `$PWD/` or hardcoded path instead of `${CLAUDE_PLUGIN_ROOT}/` | Plugin |
| G002 | `script-ref-missing` | Script reference missing on disk | Both |
| G003 | `script-not-executable` | Script file not executable | Both |
| G004 | `dead-script` | Dead script with no structured invocation reference | Plugin |
| G005 | `security-md-missing` | `SECURITY.md` is missing from repo root | Plugin |
| G006 | `todo-in-skill` | `TODO`/`FIXME`/`HACK`/`XXX` marker in published skill body | Plugin |
| G007 | `todo-in-agent` | `TODO`/`FIXME`/`HACK`/`XXX` marker in agent `.md` body | Plugin |

## Email Rules (E)

| Code | Name | Description | Mode |
|------|------|-------------|------|
| E001 | `invalid-email-format` | Email address is not a valid format | Plugin |

## User Config Rules (U)

| Code | Name | Description | Mode |
|------|------|-------------|------|
| U001 | `userconfig-not-object` | `userConfig` in `.claude/settings.json` must be an object | Plugin |
| U002 | `userconfig-desc-missing` | `userConfig` entry missing or invalid description | Plugin |
| U003 | `userconfig-env-missing` | `userConfig` key has no corresponding env var reference in `scripts/` | Plugin |
| U004 | `userconfig-sensitive-type` | `userConfig` `sensitive` field must be a boolean | Plugin |
| U005 | `userconfig-title-missing` | `userConfig` entry missing or invalid title | Plugin |
| U006 | `userconfig-type-missing` | `userConfig` entry missing or invalid type | Plugin |

## Slack Rules (K)

| Code | Name | Description | Mode |
|------|------|-------------|------|
| K001 | `slack-fallback-mismatch` | Slack fallback variable without corresponding `CLAUDE_PLUGIN_OPTION_` reference | Plugin |

## Docs Rules (D)

| Code | Name | Description | Mode |
|------|------|-------------|------|
| D001 | `docs-ref-missing` | Docs reference in `CLAUDE.md` not found on disk | Plugin |
| D002 | `claudemd-too-large` | `CLAUDE.md` exceeds 500 lines | Plugin |
| D003 | `todo-in-docs` | `TODO`/`FIXME`/`HACK`/`XXX` marker in `CLAUDE.md` (outside code fences) | Plugin |
