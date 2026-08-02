#!/usr/bin/env bash
# Run the smallest safe default-feature Clippy target set for changed Rust paths.
#
# With explicit arguments, every path must be repository-relative. Without
# arguments, the script deterministically collects branch, staged, unstaged,
# and untracked changes without fetching or otherwise accessing the network.

set -euo pipefail

error() {
    printf 'error: rust-check: %s\n' "$*" >&2
    exit 1
}

repo_root="$(git rev-parse --show-toplevel 2>/dev/null)" || error "run inside a Git repository"
cd "$repo_root" || error "cannot enter repository root '$repo_root'"

command -v cargo >/dev/null 2>&1 || error "cargo is required"
command -v jq >/dev/null 2>&1 || error "jq is required to read cargo metadata"

changed_paths=()

append_nul_paths() {
    local changed_path

    while IFS= read -r -d '' changed_path; do
        changed_paths+=("$changed_path")
    done
}

resolve_branch_base() {
    local candidate_ref
    local resolved_base

    for candidate_ref in main origin/main; do
        if git rev-parse --verify --quiet "$candidate_ref" >/dev/null 2>&1; then
            if resolved_base="$(git merge-base "$candidate_ref" HEAD 2>/dev/null)"; then
                printf '%s\n' "$resolved_base"
                return 0
            fi
        fi
    done

    return 1
}

if [[ $# -eq 0 ]]; then
    branch_base="$(resolve_branch_base)" || error "cannot resolve a merge-base against main or origin/main; pass changed paths explicitly"
    append_nul_paths < <(git diff --name-only -z "$branch_base...HEAD")
    append_nul_paths < <(git diff --cached --name-only -z)
    append_nul_paths < <(git diff --name-only -z)
    append_nul_paths < <(git ls-files --others --exclude-standard -z)
else
    changed_paths=("$@")
fi

normalise_path() {
    local input_path="$1"
    local normalised_path="$input_path"

    while [[ "$normalised_path" == ./* ]]; do
        normalised_path="${normalised_path#./}"
    done

    case "$normalised_path" in
        ''|.|..|/*|../*|*/../*|*/..|*/./*|*/.|*//*)
            error "changed path must be a normalised repository-relative path: '$input_path'"
            ;;
    esac

    case "$normalised_path" in
        *$'\n'*|*$'\r'*|*$'\t'*)
            error "changed path contains an unsupported control character: '$input_path'"
            ;;
    esac

    printf '%s\n' "$normalised_path"
}

is_cargo_control_path() {
    case "$1" in
        Cargo.toml|*/Cargo.toml|Cargo.lock|*/Cargo.lock|rust-toolchain|rust-toolchain.toml|*/rust-toolchain|*/rust-toolchain.toml|.cargo/config|.cargo/config.toml|*/.cargo/config|*/.cargo/config.toml)
            return 0
            ;;
        *)
            return 1
            ;;
    esac
}

normalised_paths=()
has_rust_input=false
for changed_path in "${changed_paths[@]}"; do
    normalised_path="$(normalise_path "$changed_path")"
    normalised_paths+=("$normalised_path")

    if [[ "$normalised_path" == *.rs ]] || is_cargo_control_path "$normalised_path"; then
        has_rust_input=true
    fi
done

if [[ "$has_rust_input" != true ]]; then
    printf 'rust-check: no changed Rust or Cargo inputs\n'
    exit 0
fi

metadata_file="$(mktemp "${TMPDIR:-/tmp}/agent-lint-rust-check.XXXXXX")" || error "cannot create temporary metadata file"
metadata_rows="${metadata_file}.rows"
trap 'rm -f "$metadata_file" "$metadata_rows"' EXIT

if ! cargo metadata --format-version 1 --no-deps --offline --locked > "$metadata_file"; then
    error "cargo metadata failed; cannot determine a safe Clippy target set"
fi

metadata_root="$(jq -r '.workspace_root // empty' "$metadata_file")" || error "cannot read the Cargo workspace root"
[[ -n "$metadata_root" && "$metadata_root" != null ]] || error "cargo metadata did not expose a workspace root"

# Use NUL-delimited fields so whitespace in a repository path cannot alter the
# package or target mapping. Cargo target names themselves are valid CLI values.
if ! jq -j --arg root "$metadata_root" '
    def repository_relative:
        . as $path
        | if $path == $root then "."
          elif ($path | startswith($root + "/")) then $path[($root | length) + 1:]
          else error("cargo metadata path is outside the repository: \($path)")
          end;
    . as $metadata
    | $metadata.workspace_members as $workspace_members
    | $metadata.packages[]
    | select(.id as $package_id | $workspace_members | index($package_id))
    | . as $package
    | (.manifest_path | repository_relative) as $manifest_path
    | ($manifest_path | split("/") | .[0:-1] | join("/") | if . == "" then "." else . end) as $package_root
    | .targets[]
    | [
        $package.name,
        $package_root,
        .name,
        (.kind | join(",")),
        (.src_path | repository_relative)
      ]
    | .[] + "\u0000"
' "$metadata_file" > "$metadata_rows"; then
    error "cannot read Cargo target metadata"
fi

package_names=()
package_roots=()
target_names=()
target_kinds=()
target_sources=()

while IFS= read -r -d '' package_name \
    && IFS= read -r -d '' package_root \
    && IFS= read -r -d '' target_name \
    && IFS= read -r -d '' target_kind \
    && IFS= read -r -d '' target_source; do
    package_names+=("$package_name")
    package_roots+=("$package_root")
    target_names+=("$target_name")
    target_kinds+=("$target_kind")
    target_sources+=("$target_source")
done < "$metadata_rows"

if [[ ${#target_names[@]} -eq 0 ]]; then
    error "cargo metadata did not expose any workspace targets"
fi

target_path_matches() {
    local changed_path="$1"
    local target_source="$2"
    local target_directory
    local target_file
    local target_stem

    [[ "$changed_path" == "$target_source" ]] && return 0
    [[ "$target_source" == *.rs ]] || return 1

    if [[ "$target_source" == */* ]]; then
        target_directory="${target_source%/*}"
    else
        target_directory="."
    fi
    target_file="${target_source##*/}"
    target_stem="${target_file%.rs}"

    if [[ "$target_directory" == . ]]; then
        [[ "$changed_path" == "$target_stem/"* ]]
    else
        [[ "$changed_path" == "$target_directory/$target_stem/"* ]]
    fi
}

default_packages=()
target_selections=()

add_default_package() {
    default_packages+=("$1")
}

add_target_selection() {
    target_selections+=("$1|$2|$3")
}

map_rust_path() {
    local changed_path="$1"
    local index
    local matching_index=""
    local matching_count=0
    local target_kind
    local target_directory
    local source_root
    local relative_source_path
    local candidate_package=""

    for ((index = 0; index < ${#target_sources[@]}; index++)); do
        if target_path_matches "$changed_path" "${target_sources[$index]}"; then
            matching_index="$index"
            matching_count=$((matching_count + 1))
        fi
    done

    if [[ "$matching_count" -gt 1 ]]; then
        error "cannot safely map Rust path '$changed_path': it matches multiple Cargo targets"
    fi

    if [[ "$matching_count" -eq 1 ]]; then
        target_kind="${target_kinds[$matching_index]}"
        case "$target_kind" in
            bin|test|example|bench)
                add_target_selection "${package_names[$matching_index]}" "$target_kind" "${target_names[$matching_index]}"
                return 0
                ;;
            lib|proc-macro|custom-build)
                add_default_package "${package_names[$matching_index]}"
                return 0
                ;;
            *)
                error "cannot safely map Rust path '$changed_path': unsupported Cargo target kind '$target_kind'"
                ;;
        esac
    fi

    # A module below a nonstandard library root belongs to that package's
    # default production targets. Do not guess about src/bin/**: Cargo metadata
    # must identify an explicit binary target for those paths.
    for ((index = 0; index < ${#target_sources[@]}; index++)); do
        target_kind="${target_kinds[$index]}"
        case "$target_kind" in
            lib|proc-macro)
                if [[ "${target_sources[$index]}" == */* ]]; then
                    target_directory="${target_sources[$index]%/*}"
                else
                    target_directory="."
                fi

                if [[ "$target_directory" == . ]]; then
                    relative_source_path="$changed_path"
                elif [[ "$changed_path" == "$target_directory/"* ]]; then
                    relative_source_path="${changed_path#"$target_directory/"}"
                else
                    continue
                fi

                [[ "$relative_source_path" == bin/* ]] && continue
                if [[ -n "$candidate_package" && "$candidate_package" != "${package_names[$index]}" ]]; then
                    error "cannot safely map Rust path '$changed_path': it belongs to multiple package source roots"
                fi
                candidate_package="${package_names[$index]}"
                ;;
        esac
    done

    # Conventional source modules below <package>/src use Cargo's default
    # production target selection. This includes a binary-only package whose
    # source modules are shared by its default executable target.
    for ((index = 0; index < ${#package_roots[@]}; index++)); do
        if [[ "${package_roots[$index]}" == . ]]; then
            source_root="src/"
        else
            source_root="${package_roots[$index]}/src/"
        fi

        if [[ "$changed_path" != "$source_root"* ]]; then
            continue
        fi

        relative_source_path="${changed_path#"$source_root"}"
        [[ "$relative_source_path" == bin/* ]] && continue
        if [[ -n "$candidate_package" && "$candidate_package" != "${package_names[$index]}" ]]; then
            error "cannot safely map Rust path '$changed_path': it belongs to multiple package source roots"
        fi
        candidate_package="${package_names[$index]}"
    done

    if [[ -z "$candidate_package" ]]; then
        error "cannot safely map Rust path '$changed_path'; add it to a Cargo target or pass a Cargo control file"
    fi

    add_default_package "$candidate_package"
}

run_clippy() {
    CARGO_INCREMENTAL=0 \
        CARGO_PROFILE_DEV_DEBUG=0 \
        CARGO_PROFILE_TEST_DEBUG=0 \
        cargo clippy --locked --offline "$@" -- -D warnings
}

workspace_scope=false
for changed_path in "${normalised_paths[@]}"; do
    if is_cargo_control_path "$changed_path"; then
        workspace_scope=true
    elif [[ "$changed_path" == *.rs ]]; then
        map_rust_path "$changed_path"
    fi
done

if [[ "$workspace_scope" == true ]]; then
    run_clippy --workspace
    exit 0
fi

if [[ ${#default_packages[@]} -gt 0 ]]; then
    while IFS= read -r package_name; do
        [[ -n "$package_name" ]] || continue
        run_clippy --package "$package_name"
    done < <(printf '%s\n' "${default_packages[@]}" | LC_ALL=C sort -u)
fi

if [[ ${#target_selections[@]} -gt 0 ]]; then
    current_package=""
    target_arguments=()

    while IFS='|' read -r selection_package selection_kind selection_name; do
        [[ -n "$selection_package" ]] || continue

        if [[ -n "$current_package" && "$current_package" != "$selection_package" ]]; then
            run_clippy --package "$current_package" "${target_arguments[@]}"
            target_arguments=()
        fi

        current_package="$selection_package"
        target_arguments+=("--$selection_kind" "$selection_name")
    done < <(printf '%s\n' "${target_selections[@]}" | LC_ALL=C sort -u)

    if [[ -n "$current_package" ]]; then
        run_clippy --package "$current_package" "${target_arguments[@]}"
    fi
fi
