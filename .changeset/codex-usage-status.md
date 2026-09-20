---
'@demicodes/provider-codex': minor
---

Codex usage is probed for free from the account's usage status (`GET …/wham/usage`, the request the open-source Codex CLI makes) instead of a minimal inference request, so it answers while the limit is reached, and it carries the plan. `probeCost` is `free`, `probeModelId` is removed, `mapCodexUsageStatus` is exported, and windows are named by their length (`5-hour`, `Weekly`) from either source.
