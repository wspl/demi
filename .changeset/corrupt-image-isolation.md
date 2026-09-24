---
"@demicodes/agent": patch
"@demicodes/utils": patch
---

Validate new user and tool images before storage or delivery, replacing corrupt image blocks with `Image data is corrupted.` while preserving neighboring content. Existing history is not scanned or rewritten during inference or recovery.
