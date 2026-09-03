---
'@demicodes/agent': patch
---

`demi agent resume` now resolves the archived child's profile before rewriting its job record. A profile that no longer exists fails the resume and leaves the archive intact instead of turning it into an orphaned live record that neither `list` nor `resume` can reach.
