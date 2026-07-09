# @demicodes/provider

The provider contract for Demi: one inference interface the runtime talks to, so
backends (APIs or CLIs) stay decoupled. Also ships the shared building blocks for
writing providers.

- **Contract** — `Provider`, `AgentProvider` (`run()` → `ProviderRun` of
  `ProviderEvent`s), `ProviderSelection`, `ProviderModelList`; `defineProvider`,
  `providerRuntime`, `applyModelPolicy`.
- **Catalog** — `modelSelectionFromCatalog`, `thinkingCapabilitiesFromProviderModel`,
  `withProviderId`.
- **HTTP helpers** — `redactSecretText`, `httpErrorCode`, `normalizeErrorCode`,
  `providerErrorFromUnknown`, `authStatusFromKey`, `httpRequestFailedEvent`.
- **Quota** — optional `Provider.quota` (`ProviderQuota` / `ProviderQuotaSnapshot`),
  `createProviderQuota`, `ensureQuota`. See
  [docs/provider-quota.md](../../docs/provider-quota.md).
- **Credentials** — optional `Provider.credentials` for multi-account pool + global
  `setActive` (subscription CLIs). See
  [docs/provider-global-credentials.md](../../docs/provider-global-credentials.md).

```ts
import {
  defineProvider,
  createProviderQuota,
  ensureQuota,
  type ProviderCredentials,
  type ProviderQuota,
} from '@demicodes/provider'
```

See [Add a Provider](../../docs/guides/add-a-provider.md). Part of
[Demi](../../README.md). Apache-2.0.
