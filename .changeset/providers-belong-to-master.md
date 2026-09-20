---
'@demicodes/backend': minor
'@demicodes/web': patch
---

Every provider entry belongs to a user. A shared instance's providers are the
master's: only the master configures them and every user infers with them.
Ownerless entries of an older shared instance move to the master on start, and
the instance no longer refuses to start under the other mode.
