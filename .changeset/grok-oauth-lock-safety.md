---
"@demicodes/provider-grok-build": patch
---

Keep live OAuth refresh locks regardless of age and verify lock-file identity
before deleting it, preventing concurrent refreshes from removing each other's
locks.
