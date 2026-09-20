---
'@demicodes/provider': minor
'@demicodes/provider-codex': minor
'@demicodes/provider-grok-build': minor
'@demicodes/provider-claude-code': minor
'@demicodes/backend': minor
'@demicodes/web-ui': minor
'@demicodes/web': minor
---

Subscription accounts are kept by whoever owns the provider, not assumed to be
files. A kit reads and writes accounts through an injected `CredentialPool`,
refreshes tokens with a versioned write so concurrent refreshers keep each
other's tokens, and can be built for one named account (`credentialId`).

Every keeper implements that one contract: a product's own storage, Demi's
files (`FileCredentialPool`), and the vendor CLI's login on the machine
(`codexVendorPool`, `grokVendorPool`, `claudeCodeVendorPool`). A kit given no
pool composes the last two with `fallbackCredentialPool`; a kit given a pool
never reads the machine's vendor login.

Breaking for library callers: each kit's auth store takes a
`CredentialDocument` (`CodexDocumentAuthStore`, `GrokDocumentAuthStore`,
`ClaudeCodeDocumentAuthStore`); `FileCodexAuthStore` / `FileGrokAuthStore` are
the vendor file's store; Claude Code's `oauthFile` and `accessToken` store
options are gone (use a pool document, or `StaticClaudeCodeAuthStore`);
`create*Credentials` takes `importFrom` instead of a vendor home.

The backend keeps accounts as encrypted control records with their own usage
snapshot, imports the pool directories of older versions on start, and can test
or refresh the usage of any account, not only the active one. Selecting another
account no longer clears usage. On a shared instance only the master sees
accounts, plan and usage.
