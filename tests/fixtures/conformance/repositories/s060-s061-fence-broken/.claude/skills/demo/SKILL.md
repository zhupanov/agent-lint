---
name: demo
description: Use when exercising S060 and S061 fence shell-command precision failures
---

```bash
first=$(awk '{print $1}' data.txt)
rg pattern > out.txt
command grep needle ../shared/config
```
