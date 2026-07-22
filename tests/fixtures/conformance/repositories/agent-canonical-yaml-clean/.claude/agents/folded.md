---
"name": folded-reviewer
"description": >-
  Reviews folded YAML agent descriptions without line parser artifacts
tools: Bash(git *) Bash(npm install, npm test), Read Write
disallowedTools:
  - WebFetch # denied lookup stays recognized
  - Bash(rm *)
---
Return a concise review.
