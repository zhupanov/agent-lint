# YAML parser policy

Agent Lint parses YAML with [noyalib](https://github.com/sebastienrousseau/noyalib), a maintained, pure-Rust parser that supports Serde values, mappings, and parser locations on the project's Rust 1.85 minimum supported version. It was selected over `serde-saphyr` because Agent Lint consumes a YAML value DOM and mapping APIs at several validator boundaries; noyalib provides those with a smaller adapter. The crate is isolated in `src/yaml.rs`; validators use that adapter rather than calling the parser directly.

The adapter intentionally uses YAML 1.2 resolution and rejects duplicate mapping keys, preserving the safety behavior of the retired `serde_yaml` dependency. It accepts and resolves aliases and merge keys, and retains explicit tags in the value DOM; validators that require a scalar, mapping, or sequence reject tagged values that do not have that expected shape. YAML 1.1 forms are intentionally different: bare `yes`/`no`/`on`/`off` are strings, and bare `0644` is decimal while `0o644` is octal. Nulls, booleans, numeric forms, quoted scalars, mappings, and sequences are supported.

The parser applies its upstream resource limits for nesting, documents, mappings, sequences, aliases, and total input/value size. This keeps linted repository content bounded without adding parser policy to individual validators.
