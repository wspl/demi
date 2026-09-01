---
'@demicodes/agent': patch
---

`subagents.notifyParentOnIdle: false` now applies to the root level only. A subagent parent has no host-side message channel, so deeper levels always self-notify — a mid-tree parent that dispatched a background child is woken by that child's completion instead of closing with an unintegrated result.
