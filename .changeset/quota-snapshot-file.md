---
'@demicodes/provider': minor
'@demicodes/provider-codex': patch
'@demicodes/provider-grok-build': patch
'@demicodes/provider-claude-code': patch
---

`createProviderQuota` takes `snapshotFile`: the latest snapshot is kept there without the vendor's raw payload, a new quota object starts from it, and `clearLatest()` removes it. `quotaSnapshotFile(stateDir)` names the file, and the three subscription providers use it when given a `stateDir`, so usage survives the provider being rebuilt and the process restarting.
