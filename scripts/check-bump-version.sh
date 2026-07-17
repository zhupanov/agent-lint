#!/usr/bin/env bash
# Check whether the current branch has a version-bump skill and verify that a
# subsequent bump added exactly one commit.  The script is deliberately
# standalone so it can run in a temporary repository during regression tests.
#
# --mode pre emits:
#   HAS_BUMP=true|false
#   COMMITS_BEFORE=<non-negative integer>
#   STATUS=ok|missing_main_ref|git_error
#
# --mode post --before-count <N> emits:
#   VERIFIED=true|false
#   COMMITS_AFTER=<non-negative integer>
#   EXPECTED=<N + 1>
#   STATUS=ok|missing_main_ref|git_error
#
# A non-ok status means the commit count is a safe fallback of zero, not a
# trustworthy count.  Post mode therefore fails closed: VERIFIED is true only
# when status is ok and the actual count matches the expected count.

set -euo pipefail

usage_error() {
  echo "ERROR: $*" >&2
  exit 1
}

MODE=""
BEFORE_COUNT=""
MODE_SET=false
BEFORE_COUNT_SET=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --mode)
      [[ $# -ge 2 && -n "${2:-}" ]] || usage_error "--mode requires pre or post"
      [[ "$MODE_SET" == false ]] || usage_error "--mode may be specified only once"
      MODE="$2"
      MODE_SET=true
      shift 2
      ;;
    --before-count)
      [[ $# -ge 2 && -n "${2:-}" ]] || usage_error "--before-count requires an integer"
      [[ "$BEFORE_COUNT_SET" == false ]] || usage_error "--before-count may be specified only once"
      BEFORE_COUNT="$2"
      BEFORE_COUNT_SET=true
      shift 2
      ;;
    *)
      usage_error "unknown argument: $1"
      ;;
  esac
done

case "$MODE" in
  pre)
    [[ -z "$BEFORE_COUNT" ]] || usage_error "--before-count is valid only with --mode post"
    ;;
  post)
    [[ "$BEFORE_COUNT" =~ ^[0-9]+$ ]] || usage_error "--mode post requires a non-negative --before-count"
    ;;
  *)
    usage_error "--mode must be pre or post"
    ;;
esac

count_commits() {
  local base count

  if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    COUNT=0
    STATUS="git_error"
    return
  fi

  if git rev-parse --verify main >/dev/null 2>&1; then
    base="main"
  elif git rev-parse --verify origin/main >/dev/null 2>&1; then
    base="origin/main"
  else
    COUNT=0
    STATUS="missing_main_ref"
    return
  fi

  if ! count=$(git rev-list --count "$base..HEAD" 2>/dev/null) || ! [[ "$count" =~ ^[0-9]+$ ]]; then
    COUNT=0
    STATUS="git_error"
    return
  fi

  COUNT="$count"
  STATUS="ok"
}

count_commits

if [[ "$MODE" == "pre" ]]; then
  if [[ -f ".claude/skills/bump-version/SKILL.md" ]]; then
    has_bump=true
  else
    has_bump=false
  fi

  echo "HAS_BUMP=$has_bump"
  echo "COMMITS_BEFORE=$COUNT"
  echo "STATUS=$STATUS"
  exit 0
fi

EXPECTED=$((BEFORE_COUNT + 1))
if [[ "$STATUS" == "ok" && "$COUNT" -eq "$EXPECTED" ]]; then
  verified=true
else
  verified=false
fi

echo "VERIFIED=$verified"
echo "COMMITS_AFTER=$COUNT"
echo "EXPECTED=$EXPECTED"
echo "STATUS=$STATUS"
