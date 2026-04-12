# Claude Lint

Claude Lint is a configuration linter for Claude Code.

## Usage

Add to your GitHub Actions workflow:

```yaml
- uses: zhupanov/claude-lint@v0
  with:
    args: "."
```

### Inputs

| Input | Description | Default |
|-------|-------------|---------|
| `version` | Version of claude-lint (e.g., `0.2.0`) | Latest release |
| `args` | Arguments to pass to claude-lint | `""` |
| `github-token` | GitHub token for API requests | `${{ github.token }}` |

> **Note:** Windows runners are not supported.

## Installation

## Prerequisites

## Features

## Skills

## Review Agents

## Linting

## Environment Variables

## Detailed Documentation
