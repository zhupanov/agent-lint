---
name: release-agent-lint
description: Create and publish an agent-lint release through a version pull request and a manually dispatched release workflow.
argument-hint: "[--dry-run] [--bump major|minor|patch]"
allowed-tools: Bash(git:*), Bash(gh:*), Bash(mktemp:*), Bash(rm:*), Bash(date:*), Bash(sleep:*), Bash(head:*), Bash(.claude/skills/bump-version/scripts/classify-bump.sh:*), Bash(.claude/skills/bump-version/scripts/apply-bump.sh:*), Read
disable-model-invocation: true
---

# Release agent-lint

Run this operator-only skill from the repository root to publish one
agent-lint release. A merge to `main` requires an explicit release trigger;
prefer keeping publication under the release workflow's explicit control. This
skill creates a version PR, waits for it to pass CI, merges it, and explicitly
dispatches the release workflow.

## Flags

| Flag | Purpose |
| --- | --- |
| `--dry-run` | Inspect the proposed version and release window without creating a branch, PR, release, or tag. |
| `--bump major\|minor\|patch` | Override the bump classifier. Use only to escalate a release after reviewing its changes. |

Parse `$ARGUMENTS` before running any command. Reject unknown flags and a
missing or invalid value for `--bump`. All flags default to false or unset.

## 1. Guard and synchronize

Before making any change, require that the current branch is `main` and the
working tree is clean. Refuse to continue if local `main` has unpublished
commits or diverges from `origin/main`; rather than rebasing it automatically,
preserve its branch state. Fetch
and fast-forward only when local `main` is behind.

```bash
git status --short
git branch --show-current
git fetch origin main --quiet
git merge --ff-only origin/main
```

Find the latest version tag and list the commits that will be released. Stop
when no commits have landed since that tag, unless the operator explicitly
confirms an empty release window.

```bash
BASELINE_TAG=$(git describe --tags --match 'v[0-9]*' --abbrev=0 origin/main)
git log --oneline "${BASELINE_TAG}..origin/main"
```

For `--dry-run`, run the classifier in a clean temporary worktree or report
its expected default (PATCH) and stop before creating a branch or changing
any remote state.

## 2. Create the version PR

Create `release/v<new-version>` from the synchronized `main`. Run the existing
version classifier, read its reasoning file, and apply the highest justified
bump. `--bump` may only escalate the classifier result; otherwise, preserve it.

```bash
CLASSIFIER_OUTPUT=$("$PWD/.claude/skills/bump-version/scripts/classify-bump.sh")
while IFS= read -r line; do
  case "$line" in
    NEW_VERSION=*) NEW_VERSION=${line#NEW_VERSION=} ;;
    REASONING_FILE=*) REASONING_FILE=${line#REASONING_FILE=} ;;
  esac
done <<< "$CLASSIFIER_OUTPUT"
git switch -c "release/v${NEW_VERSION}" main
"$PWD/.claude/skills/bump-version/scripts/apply-bump.sh" --new-version "$NEW_VERSION"
git push --set-upstream origin "release/v${NEW_VERSION}"
RELEASE_BODY=$(mktemp)
{
  printf '# Release v%s\n\n' "$NEW_VERSION"
  printf '## Version bump rationale\n\n'
  while IFS= read -r line; do printf '%s\n' "$line"; done < "$REASONING_FILE"
  printf '\n## Changes since %s\n\n' "$BASELINE_TAG"
  git log --format='- %s (%h)' "${BASELINE_TAG}..main"
} > "$RELEASE_BODY"
gh pr create --title "Release v${NEW_VERSION}" --body-file "$RELEASE_BODY"
gh pr checks --watch --interval 30
rm -f "$RELEASE_BODY"
```

Create a pull request titled `Release v<new-version>`. Write its body to a
temporary file, including the classifier reasoning and the commits since the
baseline tag, then pass that file with `gh pr create --body-file` rather than
passing release notes inline.

## 3. Verify and merge

Wait for every required PR check. Refresh no more frequently than every
30 seconds. If any check fails, stop and fix the version PR before continuing.

After checks are green, merge the release PR using the repository's normal
merge policy, then confirm its commit is on `origin/main`.

```bash
gh pr merge --merge --delete-branch
git fetch origin main --quiet
MERGE_COMMIT=$(gh pr view --json mergeCommit --jq '.mergeCommit.oid')
git merge-base --is-ancestor "$MERGE_COMMIT" origin/main
```

## 4. Publish and clean up

Explicitly dispatch the release workflow from `main`; it creates the immutable
version tag, GitHub Release, and artifacts. Identify the dispatched run for
the merged `origin/main` commit, then wait at a 30-second refresh interval and
stop if it fails.

```bash
DISPATCHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
gh workflow run release.yml --ref main
RUN_ID=""
for ((attempt = 1; attempt <= 10; attempt++)); do
  RUN_ID=$(gh run list --workflow release.yml --branch main --event workflow_dispatch \
    --limit 20 --json databaseId,headSha,createdAt \
    --jq ".[] | select(.headSha == \"${MERGE_COMMIT}\" and .createdAt >= \"${DISPATCHED_AT}\") | .databaseId" \
    | head -n 1)
  test -n "$RUN_ID" && break
  if [ "$attempt" -lt 10 ]; then sleep 30; fi
done
test -n "$RUN_ID" || { echo "Release workflow run was not found after five minutes." >&2; exit 1; }
gh run watch "$RUN_ID" --exit-status --interval 30
```

Only after that workflow succeeds, update the floating major tag locally using
the operator's already-authenticated `origin` remote. This avoids relying on
the default Actions token, which GitHub blocks from moving a ref across a
workflow-file change. The immutable version tag and release remain created by
the workflow; `v2` is a final local promotion step. Fetch the version tag,
force-update the annotated major tag, push it, and verify the remote peeled
commit equals the version tag's commit before declaring success.

```bash
MAJOR=${NEW_VERSION%%.*}
git fetch origin "refs/tags/v${NEW_VERSION}:refs/tags/v${NEW_VERSION}" --force
VERSION_OID=$(git rev-parse "v${NEW_VERSION}^{commit}")
git tag -fa "v${MAJOR}" "v${NEW_VERSION}" -m "Update v${MAJOR} to v${NEW_VERSION}"
git push origin "v${MAJOR}" --force
git fetch origin "refs/tags/v${MAJOR}:refs/tags/v${MAJOR}" --force
REMOTE_MAJOR_OID=$(git rev-parse "v${MAJOR}^{commit}")
test "$REMOTE_MAJOR_OID" = "$VERSION_OID"
```

When the workflow succeeds, return to `main`, fast-forward from `origin/main`,
and delete the local release branch. Report the released version, PR URL, and
workflow URL to the operator.

```bash
git switch main
git pull --ff-only origin main
git branch -d "release/v${NEW_VERSION}"
```

## Failed release recovery

If the release workflow fails, stop publication and inspect the failed job log
before retrying. Treat a failed GitHub API request (including HTTP 503) as
unknown state, then verify the release and tag explicitly before proceeding.

Check the release state explicitly: the version tag, GitHub Release, uploaded
artifacts, and floating major tag must all agree. A failure after GitHub Release
creation can leave a partial release: for example, the version tag and release
may exist while the floating `v2` tag remains on the previous release. Repair
that state with the local major-tag promotion step above; keep `v2` promotion
outside the Actions workflow.

Fix the workflow in a separate PR, wait for its checks, merge it according to
repository policy, and only then explicitly re-dispatch the release workflow
from `main`. Verify the successful recovery run and every expected release
output before reporting completion. Reuse the existing release version when
recovering a partial release.
