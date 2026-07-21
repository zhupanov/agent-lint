---
"name": plugin-clean
'description': >-
  Validates canonical YAML spellings in a plugin agent without findings.
model: >-
  sonnet
tools: [Bash, Read]
disallowedTools: [Write]
memory: project
effort: high
isolation: remote
color: cyan
background: false
maxTurns: 2
---
Return the result after checking the configured fields.
