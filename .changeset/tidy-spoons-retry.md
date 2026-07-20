---
'@demicodes/agent': minor
'@demicodes/core': minor
'@demicodes/provider': minor
'@demicodes/provider-codex': minor
---

Move transient inference retry into the agent runtime with four safe attempts and capped jitter backoff. Classify server, HTTP 5xx, timeout, network, and socket failures as overloaded; carry bounded request diagnostics through retry and terminal error frames; preserve completed tool progress on resume; and keep empty reasoning lifecycle events from suppressing retry.
