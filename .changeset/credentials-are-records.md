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
other's tokens, and can be built for one named account (`credentialId`). A pool
that is not the machine user's never falls back to the vendor's own login.

The backend keeps accounts as encrypted control records with their own usage
snapshot, imports the pool directories of older versions on start, and can test
or refresh the usage of any account, not only the active one. Selecting another
account no longer clears usage. On a shared instance only the master sees
accounts, plan and usage.
