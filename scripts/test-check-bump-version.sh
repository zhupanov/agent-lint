#!/usr/bin/env bash
# Black-box regression harness for check-bump-version.sh.

set -euo pipefail

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
CHECKER="$REPO_ROOT/scripts/check-bump-version.sh"
TEST_ROOT=$(mktemp -d)
trap 'rm -rf "$TEST_ROOT"' EXIT

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

assert_contains() {
  local output="$1"
  local expected="$2"
  [[ "$output" == *"$expected"* ]] || fail "expected '$expected' in output: $output"
}

assert_exit() {
  local actual="$1"
  local expected="$2"
  [[ "$actual" -eq "$expected" ]] || fail "expected exit $expected, got $actual"
}

create_fixture() {
  local name="$1"
  local feature_commits="$2"
  local with_skill="$3"
  local fixture="$TEST_ROOT/$name"
  local index

  git init --quiet "$fixture"
  git -C "$fixture" config user.name "Test User"
  git -C "$fixture" config user.email "test@example.com"
  git -C "$fixture" commit --allow-empty --quiet -m "Base fixture"
  git -C "$fixture" branch -M main
  git -C "$fixture" switch --quiet -c feature

  if [[ "$with_skill" == true ]]; then
    mkdir -p "$fixture/.claude/skills/bump-version"
    printf '%s\n' '---' 'name: bump-version' '---' > "$fixture/.claude/skills/bump-version/SKILL.md"
  fi

  for ((index = 1; index <= feature_commits; index++)); do
    git -C "$fixture" commit --allow-empty --quiet -m "Feature commit $index"
  done

  printf '%s\n' "$fixture"
}

run_checker() {
  local fixture="$1"
  shift
  (
    cd "$fixture"
    bash "$CHECKER" "$@"
  )
}

skill_fixture=$(create_fixture "has-bump-skill" 2 true)
pre_output=$(run_checker "$skill_fixture" --mode pre)
assert_contains "$pre_output" "HAS_BUMP=true"
assert_contains "$pre_output" "COMMITS_BEFORE=2"
assert_contains "$pre_output" "STATUS=ok"

no_skill_fixture=$(create_fixture "no-bump-skill" 0 false)
pre_output=$(run_checker "$no_skill_fixture" --mode pre)
assert_contains "$pre_output" "HAS_BUMP=false"
assert_contains "$pre_output" "COMMITS_BEFORE=0"
assert_contains "$pre_output" "STATUS=ok"

# The checker follows the same local-main then origin/main resolution order as
# the version classifier, so validate the remote-only fallback explicitly.
origin_only_fixture=$(create_fixture "origin-main-only" 1 true)
git -C "$origin_only_fixture" remote add origin "$origin_only_fixture"
git -C "$origin_only_fixture" fetch --quiet origin main:refs/remotes/origin/main
git -C "$origin_only_fixture" branch -D main >/dev/null
pre_output=$(run_checker "$origin_only_fixture" --mode pre)
assert_contains "$pre_output" "COMMITS_BEFORE=1"
assert_contains "$pre_output" "STATUS=ok"

git -C "$skill_fixture" commit --allow-empty --quiet -m "Bump version to 2.8.0"
post_output=$(run_checker "$skill_fixture" --mode post --before-count 2)
assert_contains "$post_output" "VERIFIED=true"
assert_contains "$post_output" "COMMITS_AFTER=3"
assert_contains "$post_output" "EXPECTED=3"
assert_contains "$post_output" "STATUS=ok"

off_by_one_fixture=$(create_fixture "off-by-one" 2 true)
post_output=$(run_checker "$off_by_one_fixture" --mode post --before-count 2)
assert_contains "$post_output" "VERIFIED=false"
assert_contains "$post_output" "COMMITS_AFTER=2"
assert_contains "$post_output" "EXPECTED=3"
assert_contains "$post_output" "STATUS=ok"

missing_main_fixture="$TEST_ROOT/missing-main"
git init --quiet "$missing_main_fixture"
git -C "$missing_main_fixture" config user.name "Test User"
git -C "$missing_main_fixture" config user.email "test@example.com"
git -C "$missing_main_fixture" commit --allow-empty --quiet -m "Only feature branch"
git -C "$missing_main_fixture" branch -M feature
pre_output=$(run_checker "$missing_main_fixture" --mode pre)
assert_contains "$pre_output" "COMMITS_BEFORE=0"
assert_contains "$pre_output" "STATUS=missing_main_ref"
post_output=$(run_checker "$missing_main_fixture" --mode post --before-count 0)
assert_contains "$post_output" "VERIFIED=false"
assert_contains "$post_output" "STATUS=missing_main_ref"

shim_dir="$TEST_ROOT/git-shim"
mkdir -p "$shim_dir"
real_git=$(command -v git)
cat > "$shim_dir/git" <<EOF
#!/usr/bin/env bash
if [[ "\${1:-}" == "rev-list" ]]; then
  exit 1
fi
exec "$real_git" "\$@"
EOF
chmod +x "$shim_dir/git"
git_error_fixture=$(create_fixture "git-error" 1 true)
git_error_output=$(cd "$git_error_fixture" && PATH="$shim_dir:$PATH" bash "$CHECKER" --mode post --before-count 0)
assert_contains "$git_error_output" "VERIFIED=false"
assert_contains "$git_error_output" "COMMITS_AFTER=0"
assert_contains "$git_error_output" "STATUS=git_error"

set +e
bash "$CHECKER" --mode post >/dev/null 2>&1
exit_code=$?
set -e
assert_exit "$exit_code" 1

set +e
bash "$CHECKER" --mode invalid >/dev/null 2>&1
exit_code=$?
set -e
assert_exit "$exit_code" 1

set +e
bash "$CHECKER" --mode pre --mode post >/dev/null 2>&1
exit_code=$?
set -e
assert_exit "$exit_code" 1

echo "Version bump check tests passed."
