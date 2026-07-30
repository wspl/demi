---
"@demicodes/agent": patch
"@demicodes/provider": patch
"@demicodes/provider-anthropic-api": patch
"@demicodes/provider-claude-code": patch
"@demicodes/provider-codex": patch
"@demicodes/provider-google": patch
"@demicodes/provider-grok-build": patch
"@demicodes/provider-openai-api": patch
---

Add snapshot-copy session and provider cloning, and run compaction summaries
through the cloned session's normal context-cache-friendly conversation path.
