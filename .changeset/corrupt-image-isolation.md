---
"@demicodes/agent": patch
"@demicodes/utils": patch
---

Validate image integrity before attachment and inference, replacing corrupt image blocks with `Image data is corrupted.` while preserving neighboring content and existing transcript history. Apply the same guard to direct provider steering and session recovery.
