#!/usr/bin/env bash
# classify-bump.sh — Deterministic semver classifier for /bump-version skill.
#
# Scope: inspects agent-lint's public rule registry, CLI flags, and validator
# modules, plus the existing skills/** and agents/** plugin surface.
#
# Rules (highest severity wins):
#   MAJOR — removed public rule ID or CLI flag; deleted/renamed SKILL.md or
#           agents/*.md; changed `name:` frontmatter; removed `--flag` in
#           argument-hint
#   MINOR — added public rule ID, validator module, or CLI flag; new SKILL.md or
#           agents/*.md; new `--flag` in argument-hint
#   PATCH — default (every PR bumps at least PATCH)
#
# Idempotent no-op: if HEAD is a commit matching
# `^Bump version to X\.Y\.Z$`, emits BUMP_TYPE=NONE and exits 0.
#
# Output (stdout, KEY=VALUE):
#   CURRENT_VERSION=<x.y.z>
#   NEW_VERSION=<x.y.z>                (same as current if BUMP_TYPE=NONE)
#   BUMP_TYPE=MAJOR|MINOR|PATCH|NONE
#   REASONING_FILE=<path>
#
# Reasoning log: ${IMPLEMENT_TMPDIR:-$(mktemp -d)}/bump-version-reasoning.md
#
# Exit codes: 0 success, 1 validation failure

set -euo pipefail

VERSION_FILE="$PWD/package.json"

err() {
  echo "ERROR: $*" >&2
  exit 1
}

# Validate package.json exists and parses.
[[ -f "$VERSION_FILE" ]] || err "$VERSION_FILE not found"
jq empty "$VERSION_FILE" 2>/dev/null || err "$VERSION_FILE is not valid JSON"

# Read current version.
CURRENT_VERSION=$(jq -r '.version // empty' "$VERSION_FILE")
[[ -n "$CURRENT_VERSION" ]] || err "$VERSION_FILE missing .version field"
[[ "$CURRENT_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || err "version '$CURRENT_VERSION' is not semver (expected X.Y.Z)"

# Best-effort fetch so origin/main is fresh. Non-fatal.
git fetch origin main --quiet 2>/dev/null || true

# Resolve BASE: prefer local main, fall back to origin/main.
BASE=""
if git rev-parse --verify main >/dev/null 2>&1; then
  BASE=$(git merge-base main HEAD 2>/dev/null || true)
fi
if [[ -z "$BASE" ]] && git rev-parse --verify origin/main >/dev/null 2>&1; then
  BASE=$(git merge-base origin/main HEAD 2>/dev/null || true)
fi
[[ -n "$BASE" ]] || err "could not resolve merge-base against main or origin/main"

# Reasoning log path.
# Prefer IMPLEMENT_TMPDIR (set by /implement workflow), fall back to a fresh
# temp directory. Avoids writing into .git/ which triggers permission prompts.
if [[ -n "${IMPLEMENT_TMPDIR:-}" ]]; then
  REASONING_DIR="$IMPLEMENT_TMPDIR"
  [[ -d "$REASONING_DIR" ]] || err "IMPLEMENT_TMPDIR is set but does not exist: $REASONING_DIR"
else
  REASONING_DIR="$(mktemp -d)"
  # Clean up on failure; caller owns cleanup on success (needs to read REASONING_FILE).
  trap 'rm -rf "$REASONING_DIR"' ERR
fi
REASONING_FILE="$REASONING_DIR/bump-version-reasoning.md"

# Helper: append to reasoning log.
log() {
  printf '%s\n' "$*" >> "$REASONING_FILE"
}

# Initialize log.
{
  echo "# Version Bump Reasoning"
  echo ""
  echo "- **Base commit**: \`$(git rev-parse --short "$BASE")\` ($(git log -1 --format=%s "$BASE" 2>/dev/null || echo '?'))"
  echo "- **Current version**: \`$CURRENT_VERSION\`"
  echo "- **Classification scope**: lint rule IDs (\`src/rules.rs\`), CLI flags (\`src/main.rs\`), validator modules (\`src/validators/**\`), \`skills/**\`, and \`agents/**\`."
  echo ""
} > "$REASONING_FILE"

# Idempotency check: is HEAD itself a version-bump commit?
HEAD_SUBJECT=$(git log -1 --format=%s HEAD 2>/dev/null || true)
if [[ "$HEAD_SUBJECT" =~ ^Bump\ version\ to\ [0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  log "## Result: NONE (already bumped)"
  log ""
  log "HEAD is a version bump commit: \`$(git rev-parse --short HEAD)\` — \"$HEAD_SUBJECT\""
  log ""
  log "No additional bump will be applied."

  echo "CURRENT_VERSION=$CURRENT_VERSION"
  echo "NEW_VERSION=$CURRENT_VERSION"
  echo "BUMP_TYPE=NONE"
  echo "REASONING_FILE=$REASONING_FILE"
  exit 0
fi

# Collect file-level changes in the legacy plugin surface.
# Use -M for rename detection.
NAME_STATUS=$(git diff -M --name-status "$BASE" HEAD -- skills agents 2>/dev/null || true)

# Track evidence.
MAJOR_REASONS=()
MINOR_REASONS=()

# Process file-level changes.
while IFS=$'\t' read -r status old new_or_blank; do
  [[ -z "${status:-}" ]] && continue

  case "$status" in
    D)
      # Deleted file in public surface.
      if [[ "$old" == skills/*/SKILL.md || "$old" == agents/*.md ]]; then
        MAJOR_REASONS+=("Deleted \`$old\`")
      fi
      ;;
    A)
      # Added file in public surface.
      if [[ "$old" == skills/*/SKILL.md || "$old" == agents/*.md ]]; then
        MINOR_REASONS+=("Added \`$old\`")
      fi
      ;;
    R*)
      # Renamed file: $old is source, $new_or_blank is destination.
      if [[ "$old" == skills/*/SKILL.md ]]; then
        MAJOR_REASONS+=("Renamed skill \`$old\` → \`$new_or_blank\`")
      elif [[ "$old" == agents/*.md ]]; then
        MAJOR_REASONS+=("Renamed agent \`$old\` → \`$new_or_blank\`")
      fi
      ;;
    M)
      # Modified file — inspect full file content for flag/name changes.
      if [[ "$old" == skills/*/SKILL.md || "$old" == agents/*.md ]]; then
        OLD_FILE=$(git show "$BASE:$old" 2>/dev/null || true)
        NEW_FILE=$(git show "HEAD:$old" 2>/dev/null || true)

        extract_frontmatter() {
          awk '
            BEGIN { state=0; n=0 }
            state==0 && /^---$/ { state=1; next }
            state==1 && /^---$/ {
              for (i=1; i<=n; i++) print buf[i]
              exit
            }
            state==1 { buf[++n]=$0 }
          '
        }

        OLD_FRONTMATTER=$(printf '%s\n' "$OLD_FILE" | extract_frontmatter)
        NEW_FRONTMATTER=$(printf '%s\n' "$NEW_FILE" | extract_frontmatter)

        # name: frontmatter field.
        OLD_NAME=$(printf '%s\n' "$OLD_FRONTMATTER" | awk '/^name: / { sub(/^name: */, ""); print; exit }')
        NEW_NAME=$(printf '%s\n' "$NEW_FRONTMATTER" | awk '/^name: / { sub(/^name: */, ""); print; exit }')
        if [[ -n "$OLD_NAME" && -z "$NEW_NAME" ]]; then
          MAJOR_REASONS+=("Removed \`name:\` frontmatter from \`$old\`")
        elif [[ -n "$OLD_NAME" && -n "$NEW_NAME" && "$OLD_NAME" != "$NEW_NAME" ]]; then
          MAJOR_REASONS+=("Renamed \`name:\` frontmatter in \`$old\` ($OLD_NAME → $NEW_NAME)")
        fi

        # argument-hint: frontmatter field — compare flag token SETS.
        OLD_ARG_HINT=$(printf '%s\n' "$OLD_FRONTMATTER" | awk '/^argument-hint: / { sub(/^argument-hint: */, ""); print; exit }')
        NEW_ARG_HINT=$(printf '%s\n' "$NEW_FRONTMATTER" | awk '/^argument-hint: / { sub(/^argument-hint: */, ""); print; exit }')
        if [[ -n "$OLD_ARG_HINT" || -n "$NEW_ARG_HINT" ]]; then
          OLD_AH_TOKENS=$(printf '%s\n' "$OLD_ARG_HINT" | grep -oE '\-\-[a-zA-Z0-9_-]+' | sort -u || true)
          NEW_AH_TOKENS=$(printf '%s\n' "$NEW_ARG_HINT" | grep -oE '\-\-[a-zA-Z0-9_-]+' | sort -u || true)
          _emit_tokens() {
            if [[ -n "$1" ]]; then printf '%s\n' "$1"; fi
          }
          REMOVED_TOKENS=$(comm -23 <(_emit_tokens "$OLD_AH_TOKENS") <(_emit_tokens "$NEW_AH_TOKENS") 2>/dev/null || true)
          ADDED_TOKENS=$(comm -13 <(_emit_tokens "$OLD_AH_TOKENS") <(_emit_tokens "$NEW_AH_TOKENS") 2>/dev/null || true)
          if [[ -n "$REMOVED_TOKENS" ]]; then
            while IFS= read -r tok; do
              [[ -n "$tok" ]] && MAJOR_REASONS+=("Removed \`$tok\` from argument-hint in \`$old\`")
            done <<< "$REMOVED_TOKENS"
          fi
          if [[ -n "$ADDED_TOKENS" ]]; then
            while IFS= read -r tok; do
              [[ -n "$tok" ]] && MINOR_REASONS+=("Added \`$tok\` to argument-hint in \`$old\`")
            done <<< "$ADDED_TOKENS"
          fi
        fi
      fi
      ;;
  esac
done <<< "$NAME_STATUS"

# Compare the stable rule registry. Rule IDs are user-facing because users can
# configure severities and exclusions by code, so removing one is breaking and
# adding one expands the lint product's behavior.
emit_lines() {
  if [[ -n "$1" ]]; then
    printf '%s\n' "$1"
  fi
}

extract_lint_rules() {
  awk '
    /^[[:space:]]*pub enum LintRule[[:space:]]*\{/ { in_enum=1; next }
    in_enum && /^[[:space:]]*\}/ { exit }
    in_enum && /^[[:space:]]*[A-Za-z_][A-Za-z0-9_]*,[[:space:]]*$/ {
      rule=$0
      sub(/^[[:space:]]*/, "", rule)
      sub(/,[[:space:]]*$/, "", rule)
      print rule
    }
  '
}

OLD_RULES=$(git show "$BASE:src/rules.rs" 2>/dev/null | extract_lint_rules | sort -u || true)
NEW_RULES=$(git show "HEAD:src/rules.rs" 2>/dev/null | extract_lint_rules | sort -u || true)
REMOVED_RULES=$(comm -23 <(emit_lines "$OLD_RULES") <(emit_lines "$NEW_RULES") 2>/dev/null || true)
ADDED_RULES=$(comm -13 <(emit_lines "$OLD_RULES") <(emit_lines "$NEW_RULES") 2>/dev/null || true)

if [[ -n "$REMOVED_RULES" ]]; then
  while IFS= read -r rule; do
    [[ -n "$rule" ]] && MAJOR_REASONS+=("Removed lint rule \`$rule\`")
  done <<< "$REMOVED_RULES"
fi
if [[ -n "$ADDED_RULES" ]]; then
  while IFS= read -r rule; do
    [[ -n "$rule" ]] && MINOR_REASONS+=("Added lint rule \`$rule\`")
  done <<< "$ADDED_RULES"
fi

# CLI flags are part of the executable's public contract. Extract long flags
# from the argument parser and help text so additions and removals are compared
# as sets rather than as incidental source edits.
extract_cli_flags() {
  grep -oE '"--[A-Za-z0-9][A-Za-z0-9-]*"' | tr -d '"' | sort -u || true
}

OLD_FLAGS=$(git show "$BASE:src/main.rs" 2>/dev/null | extract_cli_flags || true)
NEW_FLAGS=$(git show "HEAD:src/main.rs" 2>/dev/null | extract_cli_flags || true)
REMOVED_FLAGS=$(comm -23 <(emit_lines "$OLD_FLAGS") <(emit_lines "$NEW_FLAGS") 2>/dev/null || true)
ADDED_FLAGS=$(comm -13 <(emit_lines "$OLD_FLAGS") <(emit_lines "$NEW_FLAGS") 2>/dev/null || true)

if [[ -n "$REMOVED_FLAGS" ]]; then
  while IFS= read -r flag; do
    [[ -n "$flag" ]] && MAJOR_REASONS+=("Removed CLI flag \`$flag\`")
  done <<< "$REMOVED_FLAGS"
fi
if [[ -n "$ADDED_FLAGS" ]]; then
  while IFS= read -r flag; do
    [[ -n "$flag" ]] && MINOR_REASONS+=("Added CLI flag \`$flag\`")
  done <<< "$ADDED_FLAGS"
fi

# A new validator module is a user-visible capability even before its rule ID
# is wired into the registry. Existing module paths are implementation details.
VALIDATOR_STATUS=$(git diff -M --name-status "$BASE" HEAD -- src/validators 2>/dev/null || true)
while IFS=$'\t' read -r status old new_or_blank; do
  [[ -z "${status:-}" ]] && continue
  case "$status" in
    A)
      [[ "$old" == src/validators/*.rs ]] && MINOR_REASONS+=("Added validator module \`$old\`")
      ;;
  esac
done <<< "$VALIDATOR_STATUS"

# Determine bump type.
if [[ ${#MAJOR_REASONS[@]} -gt 0 ]]; then
  BUMP_TYPE="MAJOR"
elif [[ ${#MINOR_REASONS[@]} -gt 0 ]]; then
  BUMP_TYPE="MINOR"
else
  BUMP_TYPE="PATCH"
fi

# Compute new version.
IFS='.' read -r MAJ MIN PAT <<< "$CURRENT_VERSION"
case "$BUMP_TYPE" in
  MAJOR) NEW_VERSION="$((MAJ + 1)).0.0" ;;
  MINOR) NEW_VERSION="${MAJ}.$((MIN + 1)).0" ;;
  PATCH) NEW_VERSION="${MAJ}.${MIN}.$((PAT + 1))" ;;
esac

# Log reasoning.
log "## Result: $BUMP_TYPE"
log ""
log "- **New version**: \`$NEW_VERSION\`"
log ""

if [[ ${#MAJOR_REASONS[@]} -gt 0 ]]; then
  log "### MAJOR evidence"
  for r in "${MAJOR_REASONS[@]}"; do log "- $r"; done
  log ""
fi

if [[ ${#MINOR_REASONS[@]} -gt 0 ]]; then
  log "### MINOR evidence"
  for r in "${MINOR_REASONS[@]}"; do log "- $r"; done
  log ""
fi

if [[ "$BUMP_TYPE" == "PATCH" ]]; then
  log "### PATCH rationale"
  log ""
  log "No MAJOR or MINOR evidence found in the public surface. Defaulting to PATCH per policy (\"every PR must bump at least PATCH\")."
  log ""
fi

# Emit machine-parseable output.
echo "CURRENT_VERSION=$CURRENT_VERSION"
echo "NEW_VERSION=$NEW_VERSION"
echo "BUMP_TYPE=$BUMP_TYPE"
echo "REASONING_FILE=$REASONING_FILE"
