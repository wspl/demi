---
"@demicodes/utils": patch
"@demicodes/provider": patch
"@demicodes/provider-codex": patch
"@demicodes/provider-claude-code": patch
"@demicodes/provider-grok-build": patch
"@demicodes/provider-openai-api": patch
---

Merge helper duplicates into their owning packages: `errorCode` (errno-style
code guard) joins `@demicodes/utils`; `numberHeader`, `redactCredentialText`,
and `toolResultContentToText` join the `@demicodes/provider` kit and replace
the per-provider copies in codex, claude-code, grok-build, and openai-api.
The shared tool-result flattener renders `[<type>:<mediaType>]`, so the
openai-api provider now labels video blocks `[video:…]` instead of `[image:…]`.
