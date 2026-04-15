# GitHub Action

## Inputs

| Input | Description | Default |
|-------|-------------|---------|
| `version` | Version of agent-lint (e.g., `2.3.0`) | Latest release |
| `path` | Path to the repository to lint | `"."` |
| `github-token` | GitHub token for resolving latest version | `""` (see below) |
| `pedantic` | Enable pedantic mode (promote warnings to errors, except too-long) | `"false"` |
| `all` | Enable all mode (every rule fires as an error) | `"false"` |

> **Note:** Windows runners are not supported.

### About `github-token`

The `github-token` input is **optional** and has a narrow purpose: it is
used solely to call the GitHub API to resolve the latest release version
when no explicit `version` is provided. If you pin `version` (e.g.,
`version: "2.3.0"`), no API call is made and the token is never used.

**When omitted**, the action automatically falls back to the built-in
`github.token` that GitHub provides to every workflow run. You do not need
to configure or pass anything -- it just works.

**What the token can access**: the token is sent in a single read-only API
request to `api.github.com/repos/zhupanov/agent-lint/releases/latest` to
fetch the latest tag name. It is never passed to the `agent-lint` binary.
The linter itself only reads local files on disk -- it makes no network
requests and has no access to your repository's GitHub API.

**When you might set it explicitly**: if you use a fine-grained PAT or a
GitHub App token with restricted permissions, and the default
`github.token` cannot reach the public releases endpoint (uncommon).

```yaml
# Minimal -- token handled automatically:
- uses: zhupanov/agent-lint@v2
  with:
    version: "2.3.0"

# Explicit version -- no token needed at all:
- uses: zhupanov/agent-lint@v2
  with:
    version: "2.3.0"
```

## Add CI to Your Repo

Give this prompt to Claude running in your repository:

> **Add a GitHub Actions CI job called `agent-lint` that runs on pull requests
> to `main`. The job should use `ubuntu-latest`, have a 5-minute timeout,
> check out the repo with `actions/checkout@v4`, and then run
> `zhupanov/agent-lint@v2` with `path: "."` and `version: "2.3.0"`. Add it
> to the existing CI workflow if one exists, otherwise create
> `.github/workflows/ci.yaml` with `permissions: contents: read`.**
>
> **Pin to an exact version** (e.g., `version: "2.3.0"`) to protect your
> CI from breaking changes. agent-lint is under active development and
> minor/patch releases may change lint behavior.

The resulting job should look like:

```yaml
  agent-lint:
    runs-on: ubuntu-latest
    timeout-minutes: 5
    steps:
      - uses: actions/checkout@v4
      - uses: zhupanov/agent-lint@v2
        with:
          version: "2.3.0"
          path: "."
```
