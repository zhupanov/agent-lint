---
name: demo
description: Use when exercising clean S060 and S061 fence shell-command forms
---

```bash
first=$(awk 'BEGIN { print v }' data.txt)
git ls-files | xargs grep -l pattern
command grep -e '../escape' log.txt
command grep '../escape' log.txt
command grep pat < input.txt
```
