---
"@demicodes/provider": patch
"@demicodes/provider-codex": patch
"@demicodes/provider-claude-code": patch
"@demicodes/provider-grok-build": patch
"@demicodes/host-local": patch
---

Consolidate the credential pool: the three byte-identical per-provider
`credentials-pool.ts` copies merge into one implementation behind the
node-only `@demicodes/provider/credentials-pool` subpath (the provider main
entry stays platform-neutral), which also becomes the canonical home of
`resolveDemiHome` (host-local re-exports it). Pool mutations are now
serialized by a create-exclusive lock with unique temp names, closing the
torn-write race between concurrent imports, and grok's `importDefault`
activates the vendor-preferred entry deterministically by identity key
instead of label/detail guessing.
