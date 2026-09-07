# Codex Provider

The Codex provider owns the client version used to request the backend model
catalog. Its default is Codex CLI 0.153.4. Applications use this default without
installing or spawning the Codex CLI; `clientVersion` remains an explicit override
for integrations that need a different catalog version. Backend model visibility
also depends on the authenticated account.

## Tests

`packages/provider-codex/src/__tests__/models.test.ts` uses injected fetch and auth
implementations to cover the default catalog request version, explicit version
overrides, authentication headers and refresh, model capability mapping, hidden
models, and stale-cache fallback. It does not call a real model.
