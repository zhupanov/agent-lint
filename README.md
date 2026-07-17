# Agent Lint

- A linter for Claude Code, Cursor, and Codex configuration.
- Validates `.claude/`, `.claude-plugin/`, `.cursor/`, `.cursorrules`,
  `.codex/`, `.codex-plugin/`, `AGENTS.md`, `.agents/skills/`, and MCP
  configuration.
- Implemented in Rust, and fully configurable.

## Features

- **286 lint rules** across 20 categories (Manifest, Hooks, Skills, Agents,
  Prompt Content, Claude Rules, Output Styles, Settings, Cursor Rules, Cursor
  Skills, Instruction Files, Hygiene, Email, User Config, MCP, Codex, Slack,
  Docs, Markdown Structure, Link/import integrity)
- **Two lint modes**:
  - **Basic mode** -- validates detected Claude, Cursor, Codex, and standalone
    MCP configuration
  - **Plugin mode** -- runs the full rule suite when `.claude-plugin/` is
    present
- **Configurable** -- suppress or downgrade rules via `agent-lint.toml`
- **GitHub Action** for CI integration
- **Cross-platform** binaries (Linux x86_64/aarch64, macOS aarch64)

## Quick Start

The recommended ways to use agent-lint are via
[CI integration](#github-action) and [pre-commit](#pre-commit).

### GitHub Action

```yaml
- uses: zhupanov/agent-lint@v2
  with:
    version: "2.6.0"
    path: "."
```

See [GitHub Action docs](docs/github-action.md) for all inputs and
token configuration.

### Pre-commit

Add to your `.pre-commit-config.yaml`:

```yaml
repos:
  - repo: https://github.com/zhupanov/agent-lint
    rev: v2.6.0  # pin to exact version
    hooks:
      - id: agent-lint
```

> **Pin to an exact version** (e.g., `rev: v2.6.0`) to protect your
> workflow from breaking changes. agent-lint is under active development
> and minor/patch releases may change lint behavior. Run
> `pre-commit autoupdate` when you are ready to upgrade.

The hook automatically downloads the pre-built binary for your platform
and caches it. Pass CLI flags via `args`:

```yaml
      - id: agent-lint
        args: [--pedantic]
```

### Install on macOS

```bash
curl -fsSL "$(curl -fsSL https://api.github.com/repos/zhupanov/agent-lint/releases/latest \
  | grep -o 'https://[^"]*aarch64-apple-darwin.tar.gz')" -o /tmp/agent-lint.tar.gz
tar -xzf /tmp/agent-lint.tar.gz -C /tmp
sudo mv /tmp/agent-lint /usr/local/bin/agent-lint
```

### CLI

```bash
agent-lint [OPTIONS] [PATH]
```

If `PATH` is omitted, the current directory is used. The tool detects the
repo root and selects Basic or Plugin mode automatically based on the
configuration it finds.

See [CLI Reference](docs/cli.md) for flags, exit codes, and `--autofix`.

## Lint Rules

Agent Lint ships **286 rules** organized into 19 categories:

| Category | Prefix | Rules | Description |
|----------|--------|-------|-------------|
| Manifest | M | 17 | `plugin.json` and `marketplace.json` validation, component path safety |
| Hooks | H | 25 | `hooks.json` / `settings.json` hook paths and hook object schema |
| Skills | S | 75 | Skill frontmatter, prompt contracts, execution fields, descriptions, shell fences, security |
| Agents | A | 28 | Agent frontmatter, field values, tool/evidence contracts, templates, description quality |
| Prompt Content | Q | 4 | Fence-aware quality checks shared by Claude instructions, skill bodies, and agent bodies |
| Claude Rules | R | 2 | `.claude/rules/` frontmatter `paths` globs and fields |
| Output Styles | O | 6 | `.claude/output-styles/` frontmatter, body, and naming |
| Settings | T | 2 | `.claude/settings.json` / `settings.local.json` field values |
| Hygiene | G | 11 | `$PWD` hygiene, script integrity, portability, GitHub payload safety, TODO detection |
| Email | E | 1 | Email format validation |
| User Config | U | 7 | `userConfig` structure, key format, and env var mapping |
| MCP | P | 13 | MCP server configuration, transport, security, and compatibility |
| Codex | CX | 60 | Codex configuration, instructions, plugins, and skills |
| Cursor Rules | CU | 19 | Cursor rules, hooks, subagents, and cloud environment configuration |
| Cursor Skills | CR-SK | 1 | Unsupported Cursor skill frontmatter fields |
| Slack | K | 1 | Slack fallback consistency |
| Docs | D | 5 | Docs pointers, CLAUDE.md import closure and size, TODO detection |
| Markdown Structure | X | 5 | Strict YAML frontmatter, unclosed fences, XML tag balance |
| Link/import integrity | L | 6 | `@import` graph (missing, circular, depth, duplicate) and markdown-link/script integrity for instruction files |

For the complete rule table with codes, names, defaults, and auto-fixable
rules, see **[docs/rules.md](docs/rules.md)**.

### Lint Modes

| Mode | Trigger | Scope |
|------|---------|-------|
| **Basic** | Claude, Cursor, Codex, or MCP configuration is present | Detected platform configuration plus always-mode Claude rules |
| **Plugin** | `.claude-plugin/` directory exists | All 286 rules including manifest, agents, hygiene, MCP, and plugin-only S- and L-rules |

If no supported agent or MCP configuration exists, the tool prints "Nothing to lint" and exits 0.

## Configuration

Agent Lint reads an optional **`agent-lint.toml`** from the repository root
to suppress, promote, or downgrade rules and exclude files from linting.

See [Configuration docs](docs/configuration.md) for the full reference.

## Documentation

| Document | Description |
|----------|-------------|
| [Rules Reference](docs/rules.md) | Complete rule table with codes, names, defaults, and auto-fixable rules |
| [CLI Reference](docs/cli.md) | Flags, exit codes, `--autofix`, `--list-scripts` |
| [GitHub Action](docs/github-action.md) | Action inputs, token configuration, adding CI to your repo |
| [Configuration](docs/configuration.md) | `agent-lint.toml` format, rule identifiers, file exclusion, strictness modes |
| [Development](docs/development.md) | Local setup, Makefile targets, project structure, CI/CD |
| [Architectural Guidelines](ARCHITECTURAL_GUIDELINES.md) | Preferred design, ownership, testing, and change practices |
| [Architectural Invariants](ARCHITECTURAL_INVARIANTS.md) | Absolute contracts for the lint pipeline and public behavior |
