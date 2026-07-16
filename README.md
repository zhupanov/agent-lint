# Agent Lint

- A linter for [Claude Code](https://docs.anthropic.com/en/docs/claude-code)
  configuration and plugins.
- Validates `.claude/`, `.claude-plugin/`, and MCP configuration.
- Implemented in Rust, and fully configurable.

## Features

- **217 lint rules** across 14 categories (Manifest, Hooks, Skills, Agents,
  Claude Rules, Output Styles, Settings, Hygiene, Email, User Config, MCP,
  Codex, Slack, Docs)
- **Two lint modes**:
  - **Basic mode** -- validates `.claude/` contents and standalone MCP
    configuration (settings, hooks, private skill frontmatter, private agents,
    script references, executability)
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
    version: "2.4.0"
    path: "."
```

See [GitHub Action docs](docs/github-action.md) for all inputs and
token configuration.

### Pre-commit

Add to your `.pre-commit-config.yaml`:

```yaml
repos:
  - repo: https://github.com/zhupanov/agent-lint
    rev: v2.4.0  # pin to exact version
    hooks:
      - id: agent-lint
```

> **Pin to an exact version** (e.g., `rev: v2.4.0`) to protect your
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

Agent Lint ships **217 rules** organized into 14 categories:

| Category | Prefix | Rules | Description |
|----------|--------|-------|-------------|
| Manifest | M | 11 | `plugin.json` and `marketplace.json` validation |
| Hooks | H | 25 | `hooks.json` / `settings.json` hook paths and hook object schema |
| Skills | S | 71 | Skill frontmatter, prompt contracts, execution fields, descriptions, shell fences, security |
| Agents | A | 27 | Agent frontmatter, field values, tool/evidence contracts, templates, description quality |
| Claude Rules | R | 2 | `.claude/rules/` frontmatter `paths` globs and fields |
| Output Styles | O | 6 | `.claude/output-styles/` frontmatter, body, and naming |
| Settings | T | 2 | `.claude/settings.json` / `settings.local.json` field values |
| Hygiene | G | 11 | `$PWD` hygiene, script integrity, portability, GitHub payload safety, TODO detection |
| Email | E | 1 | Email format validation |
| User Config | U | 6 | `userConfig` structure and env var mapping |
| MCP | P | 13 | MCP server configuration, transport, security, and compatibility |
| Codex | CX | 36 | Codex `config.toml` structure, profiles, providers, and security |
| Slack | K | 1 | Slack fallback consistency |
| Docs | D | 5 | Docs pointers, CLAUDE.md import closure and size, TODO detection |

For the complete rule table with codes, names, defaults, and auto-fixable
rules, see **[docs/rules.md](docs/rules.md)**.

### Lint Modes

| Mode | Trigger | Scope |
|------|---------|-------|
| **Basic** | `.claude/` directory or any `*.mcp.json` file exists | Settings hooks and hook schema, MCP configuration, private skill frontmatter, private agents (A002-A003, A008-A027), script refs, executability, always-mode S-rules |
| **Plugin** | `.claude-plugin/` directory exists | All 217 rules including manifest, agents, hygiene, MCP, and plugin-only S-rules |

If no Claude or MCP configuration exists, the tool prints "Nothing to lint" and exits 0.

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
