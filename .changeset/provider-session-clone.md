---
'@demicodes/provider': major
'@demicodes/agent': major
'@demicodes/provider-anthropic-api': major
'@demicodes/provider-claude-code': major
'@demicodes/provider-codex': major
'@demicodes/provider-google': major
'@demicodes/provider-grok-build': major
'@demicodes/provider-openai-api': major
---

Require `AgentProvider.clone()` and add `AgentSession.clone()` for isolated session forks.

Every provider runtime must return an independently disposable clone with the same configuration but without shared live-process / continuation state. Sessions expose `.clone()` for point-in-time copies (optional provider/runtime/state/transcript overrides); parent persistence is never inherited. See `docs/provider-session-clone.md`.
