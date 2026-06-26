# @demi/provider

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

```ts
import { defineProvider, zeroUsage } from '@demi/provider'
```

See [Add a Provider](../../docs/guides/add-a-provider.md). Part of
[Demi](../../README.md). Apache-2.0.
