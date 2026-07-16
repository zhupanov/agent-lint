#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
CLASSIFIER_SOURCE="$REPO_ROOT/.claude/skills/bump-version/scripts/classify-bump.sh"
TEST_ROOT=$(mktemp -d)
trap 'rm -rf "$TEST_ROOT"' EXIT

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

assert_contains() {
  local output="$1"
  local expected="$2"
  [[ "$output" == *"$expected"* ]] || fail "expected '$expected' in classifier output: $output"
}

create_fixture() {
  local name="$1"
  local fixture="$TEST_ROOT/$name"

  git init --quiet "$fixture"
  git -C "$fixture" config user.name "Test User"
  git -C "$fixture" config user.email "test@example.com"
  mkdir -p "$fixture/.claude/skills/bump-version/scripts" "$fixture/src/validators"
  cp "$CLASSIFIER_SOURCE" "$fixture/.claude/skills/bump-version/scripts/classify-bump.sh"
  chmod +x "$fixture/.claude/skills/bump-version/scripts/classify-bump.sh"

  printf '%s\n' '{"version":"2.4.0"}' > "$fixture/package.json"
  cat > "$fixture/src/rules.rs" <<'EOF'
pub enum LintRule {
    ExistingRule,
}
EOF
  cat > "$fixture/src/main.rs" <<'EOF'
fn main() {
    let existing = "--existing";
    println!("{existing}");
}
EOF
  cat > "$fixture/src/validators/mod.rs" <<'EOF'
mod existing;
EOF
  printf '%s\n' 'pub fn validate() {}' > "$fixture/src/validators/existing.rs"

  git -C "$fixture" add .
  git -C "$fixture" commit --quiet -m "Base fixture"
  git -C "$fixture" branch -M main
  git -C "$fixture" switch --quiet -c feature
  printf '%s\n' "$fixture"
}

run_classifier() {
  local fixture="$1"
  (
    cd "$fixture"
    .claude/skills/bump-version/scripts/classify-bump.sh
  )
}

rule_fixture=$(create_fixture "added-rule")
cat > "$rule_fixture/src/rules.rs" <<'EOF'
pub enum LintRule {
    ExistingRule,
    AddedRule,
}
EOF
git -C "$rule_fixture" add src/rules.rs
git -C "$rule_fixture" commit --quiet -m "Add lint rule"
rule_output=$(run_classifier "$rule_fixture")
assert_contains "$rule_output" "BUMP_TYPE=MINOR"
assert_contains "$rule_output" "NEW_VERSION=2.5.0"

validator_fixture=$(create_fixture "added-validator")
printf '%s\n' 'pub fn validate() {}' > "$validator_fixture/src/validators/new_surface.rs"
git -C "$validator_fixture" add src/validators/new_surface.rs
git -C "$validator_fixture" commit --quiet -m "Add validator module"
validator_output=$(run_classifier "$validator_fixture")
assert_contains "$validator_output" "BUMP_TYPE=MINOR"

flag_fixture=$(create_fixture "removed-flag")
cat > "$flag_fixture/src/main.rs" <<'EOF'
fn main() {
    println!("no flags");
}
EOF
git -C "$flag_fixture" add src/main.rs
git -C "$flag_fixture" commit --quiet -m "Remove CLI flag"
flag_output=$(run_classifier "$flag_fixture")
assert_contains "$flag_output" "BUMP_TYPE=MAJOR"
assert_contains "$flag_output" "NEW_VERSION=3.0.0"

echo "Version bump classifier tests passed."
