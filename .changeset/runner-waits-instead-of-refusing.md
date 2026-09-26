---
"@demicodes/web": patch
---

The runner no longer refuses work because other work is in flight:
filesystem, working-tree and sync requests wait for a slot, so the page no
longer meets the working-tree error code `busy` or the `changes_busy` answer.
