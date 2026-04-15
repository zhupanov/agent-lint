# Agent Lint

- A linter for [Claude Code](https://docs.anthropic.com/en/docs/claude-code)
  configuration and plugins.
- Validates `.claude/` and `.claude-plugin/`.
- Implemented in Rust, and fully configurable.

## Features

- **104 lint rules** across 9 categories (Manifest, Hooks, Skills, Agents,
  Hygiene, Email, User Config, Slack, Docs)
- **Two lint modes**:
  - **Basic mode** -- validates `.claude/` contents (settings, hooks, private
    skill frontmatter, script references, executability)
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
    version: "2.3.0"
    path: "."
```

See [GitHub Action docs](docs/github-action.md) for all inputs and
token configuration.

### Pre-commit

Add to your `.pre-commit-config.yaml`:

```yaml
repos:
  - repo: https://github.com/zhupanov/agent-lint
    rev: v2.3.0  # pin to exact version
    hooks:
      - id: agent-lint
```

> **Pin to an exact version** (e.g., `rev: v2.3.0`) to protect your
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
presence of `.claude-plugin/`.

See [CLI Reference](docs/cli.md) for flags, exit codes, and `--autofix`.

## Lint Rules

Agent Lint ships **104 rules** organized into 9 categories:

| Category | Prefix | Rules | Description |
|----------|--------|-------|-------------|
| Manifest | M | 11 | `plugin.json` and `marketplace.json` validation |
| Hooks | H | 7 | `hooks.json` and `settings.json` hook paths |
| Skills | S | 57 | Skill frontmatter, naming, descriptions, body content, security |
| Agents | A | 11 | Agent frontmatter, templates, description quality, name format |
| Hygiene | G | 7 | `$PWD` hygiene, script integrity, executability, dead scripts, TODO detection |
| Email | E | 1 | Email format validation |
| User Config | U | 6 | `userConfig` structure and env var mapping |
| Slack | K | 1 | Slack fallback consistency |
| Docs | D | 3 | Docs file references, CLAUDE.md size, TODO detection |

For the complete rule table with codes, names, defaults, and auto-fixable
rules, see **[docs/rules.md](docs/rules.md)**.

### Lint Modes

| Mode | Trigger | Scope |
|------|---------|-------|
| **Basic** | `.claude/` directory exists | Settings hooks, private skill frontmatter, script refs, executability, always-mode S-rules |
| **Plugin** | `.claude-plugin/` directory exists | All 104 rules including manifest, agents, hygiene, and plugin-only S-rules |

If neither directory exists, the tool prints "Nothing to lint" and exits 0.

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
