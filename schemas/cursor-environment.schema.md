# Cursor environment schema provenance

`cursor-environment.schema.json` is an unmodified, repository-tracked snapshot
of Cursor's canonical Cloud Environment schema.

- Upstream URL: <https://www.cursor.com/schemas/environment.schema.json>
- Retrieved: 2026-07-21
- SHA-256: `62b13994164f4186198b1f002ff957605df37ba5eee803e6afe69c981af001d6`

Agent Lint always compiles the checked-in schema, so lint runs remain offline
and deterministic. To intentionally refresh the snapshot, run the following
from the repository root, review the diff, then update this retrieval date:

```sh
curl --fail --silent --show-error --location \
  https://www.cursor.com/schemas/environment.schema.json \
  --output schemas/cursor-environment.schema.json
```

The focused `cursor_environment_schema_is_checked_in_and_compiles` test ensures
the artifact is valid JSON Schema and that its provenance remains recorded.
