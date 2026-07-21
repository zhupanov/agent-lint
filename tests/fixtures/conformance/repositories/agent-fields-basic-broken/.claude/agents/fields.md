---
name: fields-broken
description: Exercises invalid agent frontmatter values for CLI conformance
model: sonet
permissionMode: yolo
memory: global
effort: turbo
isolation: container
background: yes
maxTurns: 0
tools: [Bash, Bsh]
disallowedTools: [Bash, Shh]
skills: [missing-skill, Bad_Skill]
colour: cyan
---
Return the result after checking the configured fields.
