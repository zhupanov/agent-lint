---
name: canonical
description: Use when exercising canonical frontmatter field-type parsing thoroughly
user-invocable: True  # YAML 1.2 casing plus a trailing comment stay valid
model: sonnet # fast default
"paths": ["src/**"]
allowed-tools: Bash(git add *) Bash(git commit *) Bash(git status *)
disallowed-tools: [WebFetch, WebSearch] # denied lookups stay recognized
---
Run the analysis and report the findings in a short structured summary.
