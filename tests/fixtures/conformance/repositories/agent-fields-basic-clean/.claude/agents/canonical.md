---
"name": basic-clean
'description': >-
  Validates canonical YAML spellings without agent field findings.
model: >-
  sonnet
permissionMode: acceptEdits
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
