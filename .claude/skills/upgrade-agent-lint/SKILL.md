---
name: upgrade-agent-lint
description: Download, verify, and install the latest published agent-lint binary on the local machine.
allowed-tools: Bash(.claude/skills/upgrade-agent-lint/scripts/upgrade-agent-lint.sh:*)
---

# Upgrade agent-lint

Use this repository-private skill to replace the local `agent-lint` executable
with the latest stable GitHub Release asset.

## Run the upgrade

From the repository root, run:

```bash
"$PWD/.claude/skills/upgrade-agent-lint/scripts/upgrade-agent-lint.sh"
```

The helper:

1. Detects the supported operating-system and CPU target.
2. Resolves the latest published release from `zhupanov/agent-lint`.
3. Downloads the matching archive and release checksum manifest.
4. Verifies the archive checksum before extracting it.
5. Installs the executable to `/usr/local/bin/agent-lint`.
6. Runs the installed executable and verifies its reported version.

Set `AGENT_LINT_INSTALL_DIR` to an existing absolute directory to override the
default installation directory. The helper uses `sudo install` only when the
chosen directory is not writable.

## Result

Success output includes `AGENT_LINT_VERSION`, `AGENT_LINT_BINARY`, and
`AGENT_LINT_UPGRADED=true`. Report the installed version and path. On failure,
report the failed stage and leave any existing installation in place.
