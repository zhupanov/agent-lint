#!/usr/bin/env bash
# Black-box regression harness for the changed-path Rust Clippy driver.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DRIVER="$REPO_ROOT/scripts/rust-check.sh"
TEST_ROOT="$(mktemp -d)"
trap 'rm -rf "$TEST_ROOT"' EXIT

fail() {
    printf 'FAIL: %s\n' "$*" >&2
    exit 1
}

assert_eq() {
    local actual="$1"
    local expected="$2"

    [[ "$actual" == "$expected" ]] || fail "expected '$expected', got '$actual'"
}

assert_contains() {
    local output="$1"
    local expected="$2"

    [[ "$output" == *"$expected"* ]] || fail "expected '$expected' in output: $output"
}

assert_not_contains() {
    local output="$1"
    local unexpected="$2"

    [[ "$output" != *"$unexpected"* ]] || fail "did not expect '$unexpected' in output: $output"
}

line_count() {
    awk 'NF { count += 1 } END { print count + 0 }'
}

write_metadata_fixture() {
    local fixture="$1"
    local metadata_root

    metadata_root="$(cd "$fixture" && pwd -P)"

    jq -n --arg root "$metadata_root" '
        {
          workspace_root: $root,
          workspace_members: ["root-app", "util"],
          packages: [
            {
              id: "root-app",
              name: "root-app",
              manifest_path: ($root + "/Cargo.toml"),
              targets: [
                { name: "root-lib", kind: ["lib"], src_path: ($root + "/src/lib.rs") },
                { name: "root-app", kind: ["bin"], src_path: ($root + "/src/main.rs") },
                { name: "api", kind: ["test"], src_path: ($root + "/tests/api.rs") },
                { name: "demo", kind: ["example"], src_path: ($root + "/examples/demo.rs") },
                { name: "perf", kind: ["bench"], src_path: ($root + "/benches/perf.rs") },
                { name: "admin", kind: ["bin"], src_path: ($root + "/tools/admin.rs") }
              ]
            },
            {
              id: "util",
              name: "util",
              manifest_path: ($root + "/crates/util/Cargo.toml"),
              targets: [
                { name: "util", kind: ["lib"], src_path: ($root + "/crates/util/src/lib.rs") }
              ]
            }
          ]
        }
    ' > "$fixture/metadata.json"
}

write_fake_cargo() {
    local fixture="$1"

    mkdir -p "$fixture/fake-bin"
    cat > "$fixture/fake-bin/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

record() {
    local command_name="$1"
    shift

    {
        printf '%s' "$command_name"
        for argument in "$@"; do
            printf ' <%s>' "$argument"
        done
        printf '\n'
    } >> "$FAKE_CARGO_LOG"
}

case "${1:-}" in
    metadata)
        shift
        record metadata "$@"
        command cat "$FAKE_METADATA"
        ;;
    clippy)
        shift
        {
            printf 'clippy CARGO_INCREMENTAL=%s CARGO_PROFILE_DEV_DEBUG=%s CARGO_PROFILE_TEST_DEBUG=%s' \
                "${CARGO_INCREMENTAL:-}" \
                "${CARGO_PROFILE_DEV_DEBUG:-}" \
                "${CARGO_PROFILE_TEST_DEBUG:-}"
            for argument in "$@"; do
                printf ' <%s>' "$argument"
            done
            printf '\n'
        } >> "$FAKE_CARGO_LOG"
        ;;
    *)
        printf 'unexpected cargo command: %s\n' "${1:-}" >&2
        exit 1
        ;;
esac
EOF
    chmod +x "$fixture/fake-bin/cargo"
}

create_fixture() {
    local name="$1"
    local fixture="$TEST_ROOT/$name"

    mkdir -p "$fixture/src" "$fixture/tests" "$fixture/examples" "$fixture/benches" \
        "$fixture/tools" "$fixture/.cargo" "$fixture/crates/util/src"
    printf '%s\n' '[package]' 'name = "root-app"' 'version = "0.1.0"' > "$fixture/Cargo.toml"
    printf '%s\n' '[package]' 'name = "util"' 'version = "0.1.0"' > "$fixture/crates/util/Cargo.toml"
    printf '%s\n' 'pub fn value() -> u8 { 1 }' > "$fixture/src/lib.rs"
    printf '%s\n' 'fn main() {}' > "$fixture/src/main.rs"
    printf '%s\n' '#[test]' 'fn works() {}' > "$fixture/tests/api.rs"
    printf '%s\n' 'fn main() {}' > "$fixture/examples/demo.rs"
    printf '%s\n' 'fn main() {}' > "$fixture/benches/perf.rs"
    printf '%s\n' 'fn main() {}' > "$fixture/tools/admin.rs"
    printf '%s\n' 'pub fn value() -> u8 { 2 }' > "$fixture/crates/util/src/lib.rs"
    printf '%s\n' '# generated lock fixture' > "$fixture/Cargo.lock"
    printf '%s\n' '[build]' 'incremental = false' > "$fixture/.cargo/config.toml"

    git init --quiet "$fixture"
    git -C "$fixture" config user.name "Rust check test"
    git -C "$fixture" config user.email "rust-check@example.com"
    git -C "$fixture" add .
    git -C "$fixture" commit --quiet -m "Initial fixture"
    git -C "$fixture" branch -M main
    git -C "$fixture" checkout --quiet -b feature

    write_metadata_fixture "$fixture"
    write_fake_cargo "$fixture"
    printf '%s\n' "$fixture"
}

run_driver() {
    local fixture="$1"
    local status=0
    shift

    : > "$fixture/cargo.log"
    (
        cd "$fixture"
        PATH="$fixture/fake-bin:$PATH" \
            FAKE_CARGO_LOG="$fixture/cargo.log" \
            FAKE_METADATA="$fixture/metadata.json" \
            bash "$DRIVER" "$@"
    ) > "$fixture/stdout" 2> "$fixture/stderr" || status=$?

    if [[ "$status" -ne 0 && "${RUST_CHECK_TEST_DEBUG:-}" == 1 ]]; then
        printf 'rust-check failed for fixture %s:\n' "$fixture" >&2
        sed -n '1,240p' "$fixture/stderr" >&2
    fi

    return "$status"
}

clippy_lines() {
    grep '^clippy ' "$1/cargo.log" || true
}

metadata_lines() {
    grep '^metadata' "$1/cargo.log" || true
}

assert_safe_clippy_invocation() {
    local output="$1"

    assert_contains "$output" 'CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0'
    assert_contains "$output" '<--locked>'
    assert_contains "$output" '<--offline>'
    assert_contains "$output" '<--> <-D> <warnings>'
    assert_not_contains "$output" '<--all-targets>'
    assert_not_contains "$output" '<--all-features>'
    assert_not_contains "$output" '<--features>'
    assert_not_contains "$output" '<--no-default-features>'
    assert_not_contains "$output" '<--release>'
}

production_fixture="$(create_fixture production)"
run_driver "$production_fixture" src/lib.rs
production_clippy="$(clippy_lines "$production_fixture")"
assert_eq "$(printf '%s\n' "$production_clippy" | line_count)" 1
assert_safe_clippy_invocation "$production_clippy"
assert_contains "$production_clippy" '<--package> <root-app>'
assert_not_contains "$production_clippy" '<--bin>'
assert_contains "$(metadata_lines "$production_fixture")" '<--format-version> <1> <--no-deps> <--offline> <--locked>'

target_fixture="$(create_fixture target-selection)"
run_driver "$target_fixture" examples/demo.rs src/main.rs tests/api.rs tools/admin.rs benches/perf.rs tests/api.rs
target_clippy="$(clippy_lines "$target_fixture")"
assert_eq "$(printf '%s\n' "$target_clippy" | line_count)" 1
assert_safe_clippy_invocation "$target_clippy"
assert_contains "$target_clippy" '<--package> <root-app>'
assert_contains "$target_clippy" '<--bench> <perf>'
assert_contains "$target_clippy" '<--bin> <admin>'
assert_contains "$target_clippy" '<--bin> <root-app>'
assert_contains "$target_clippy" '<--example> <demo>'
assert_contains "$target_clippy" '<--test> <api>'

coalesced_fixture="$(create_fixture coalesced-packages)"
run_driver "$coalesced_fixture" crates/util/src/lib.rs src/lib.rs src/lib.rs
coalesced_clippy="$(clippy_lines "$coalesced_fixture")"
assert_eq "$(printf '%s\n' "$coalesced_clippy" | line_count)" 2
assert_safe_clippy_invocation "$coalesced_clippy"
assert_contains "$coalesced_clippy" '<--package> <root-app>'
assert_contains "$coalesced_clippy" '<--package> <util>'
coalesced_first="$(printf '%s\n' "$coalesced_clippy" | sed -n '1p')"
coalesced_second="$(printf '%s\n' "$coalesced_clippy" | sed -n '2p')"
assert_contains "$coalesced_first" '<--package> <root-app>'
assert_contains "$coalesced_second" '<--package> <util>'

control_fixture="$(create_fixture cargo-controls)"
run_driver "$control_fixture" Cargo.toml Cargo.lock rust-toolchain.toml .cargo/config.toml
control_clippy="$(clippy_lines "$control_fixture")"
assert_eq "$(printf '%s\n' "$control_clippy" | line_count)" 1
assert_safe_clippy_invocation "$control_clippy"
assert_contains "$control_clippy" '<--workspace>'
assert_not_contains "$control_clippy" '<--package>'

unmappable_fixture="$(create_fixture unmappable)"
set +e
run_driver "$unmappable_fixture" scratch/unmapped.rs
unmappable_status=$?
set -e
assert_eq "$unmappable_status" 1
assert_contains "$(< "$unmappable_fixture/stderr")" "cannot safely map Rust path 'scratch/unmapped.rs'"
assert_eq "$(clippy_lines "$unmappable_fixture" | line_count)" 0

discovery_fixture="$(create_fixture discovery)"
printf '%s\n' '// committed branch change' >> "$discovery_fixture/src/lib.rs"
git -C "$discovery_fixture" add src/lib.rs
git -C "$discovery_fixture" commit --quiet -m "Change library"
printf '%s\n' '// staged change' >> "$discovery_fixture/tests/api.rs"
git -C "$discovery_fixture" add tests/api.rs
printf '%s\n' '// unstaged change' >> "$discovery_fixture/examples/demo.rs"
mkdir -p "$discovery_fixture/benches/perf"
printf '%s\n' 'fn helper() {}' > "$discovery_fixture/benches/perf/helper.rs"
run_driver "$discovery_fixture"
discovery_clippy="$(clippy_lines "$discovery_fixture")"
assert_eq "$(printf '%s\n' "$discovery_clippy" | line_count)" 2
assert_safe_clippy_invocation "$discovery_clippy"
assert_contains "$discovery_clippy" '<--package> <root-app>'
assert_contains "$discovery_clippy" '<--bench> <perf>'
assert_contains "$discovery_clippy" '<--example> <demo>'
assert_contains "$discovery_clippy" '<--test> <api>'

origin_fallback_fixture="$(create_fixture origin-fallback)"
git -C "$origin_fallback_fixture" remote add origin "$origin_fallback_fixture"
git -C "$origin_fallback_fixture" fetch --quiet origin main:refs/remotes/origin/main
git -C "$origin_fallback_fixture" branch -D main >/dev/null
printf '%s\n' '// origin fallback change' >> "$origin_fallback_fixture/src/lib.rs"
run_driver "$origin_fallback_fixture"
origin_fallback_clippy="$(clippy_lines "$origin_fallback_fixture")"
assert_eq "$(printf '%s\n' "$origin_fallback_clippy" | line_count)" 1
assert_safe_clippy_invocation "$origin_fallback_clippy"
assert_contains "$origin_fallback_clippy" '<--package> <root-app>'

pre_commit_config="$(sed -n '1,240p' "$REPO_ROOT/.pre-commit-config.yaml")"
assert_contains "$pre_commit_config" 'entry: cargo fmt -- --check'
assert_contains "$pre_commit_config" 'entry: bash scripts/rust-check.sh'
assert_contains "$pre_commit_config" 'pass_filenames: true'
assert_contains "$(sed -n '1,180p' "$REPO_ROOT/Makefile")" 'rust-check:'
assert_contains "$(sed -n '1,180p' "$REPO_ROOT/.claude/skills/relevant-checks/scripts/run-checks.sh")" 'make rust-check'

for prohibited in 'cargo build' 'cargo check' 'cargo run' 'cargo test' 'cargo llvm-cov' '--all-targets' '--all-features' '--release'; do
    if grep -F -- "$prohibited" \
        "$REPO_ROOT/.pre-commit-config.yaml" \
        "$REPO_ROOT/Makefile" \
        "$REPO_ROOT/scripts/rust-check.sh" \
        "$REPO_ROOT/.claude/skills/relevant-checks/scripts/run-checks.sh" >/dev/null; then
        fail "default local route still contains prohibited command or option: $prohibited"
    fi
done

echo "Rust changed-path check tests passed."
