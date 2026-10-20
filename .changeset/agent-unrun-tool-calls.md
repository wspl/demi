---
"@demicodes/agent": patch
---

Drop tool calls a failed provider stream requested before deciding whether to retry: they never ran, so the attempt can be retried silently, and later requests never replay a call without a result.
