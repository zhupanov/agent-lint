#!/usr/bin/env bash
set -euo pipefail

# Shellcheck wrapper for claude-lint discovered scripts.
# Uses cargo run by default; override with CLAUDE_LINT_CMD for installed binary.
LINT_CMD="${CLAUDE_LINT_CMD:-cargo run --}"

scripts=$($LINT_CMD --list-scripts "${1:-.}")
if [ -z "$scripts" ]; then
  echo "No scripts found." >&2
  exit 0
fi
echo "$scripts" | xargs -r shellcheck
