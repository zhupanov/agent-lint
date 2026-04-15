# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| Latest  | Yes       |
| Older   | No        |

Only the latest released version receives security updates.

## Reporting a Vulnerability

If you discover a security vulnerability in agent-lint, please report it responsibly:

1. **Email**: Send details to <zhupanov@yahoo.com>
2. **Do not** open a public GitHub issue for security vulnerabilities
3. Include steps to reproduce the issue and any relevant context

You should receive an acknowledgment within 72 hours. We will work with you to understand the issue and coordinate a fix before any public disclosure.

## Trust Model

Agent-lint is a standalone Rust CLI tool that statically analyzes Claude Code configuration files (CLAUDE.md, settings, MCP configs, etc.) against a set of lint rules. It reads files from the filesystem but does not modify them or make network requests during linting. It invokes `git rev-parse --show-toplevel` as a read-only subprocess solely to locate the repository root.
