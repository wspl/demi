---
'@demicodes/agent': minor
'@demicodes/coding-agent': minor
---

Subagent profiles: omitting `--profile` always selects the unnamed inherit profile (parent harness, model, Host, commands), which exists regardless of declared profiles and cannot be configured. `default` is now a reserved word rather than a profile name: a harness declaring a profile called `default` fails at assembly, `--profile default` is unknown, and persisted jobs store `profileName: null` for inherited children. `demi agent resume` resolves the profile before rewriting the archived job record, so a missing profile leaves the archive intact. `@demicodes/coding-agent` drops its declared `default` profile and ships only `explore`.
