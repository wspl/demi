# @demicodes/provider-grok-build

A Demi provider backed by Grok Build's CLI chat proxy. Reuses the OAuth session
stored by the official Grok CLI in `~/.grok/auth.json` (subscription login).

```ts
import { createGrokBuildProvider } from '@demicodes/provider-grok-build'

const provider = createGrokBuildProvider()
// provider.auth, provider.quota, provider.credentials (multi-account pool by default)
```

## Auth and credentials

- Default material: `~/.grok/auth.json` (or `$GROK_HOME`); multi-entry map supported.
- Multi-credential pool under `$DEMI_HOME/credentials/grok-build/`; `importDefault`
  can snapshot each OIDC entry as its own pool credential.
- Lifecycle: `beginLogin` → `grok login` → `importDefault` → `setActive`.

Requires a prior login (or `beginLogin`) so vendor material exists before import.
See [docs/provider-global-credentials.md](../../docs/provider-global-credentials.md).

## Quota

- **probe** (cost: `free`): `/v1/billing` + `/v1/user?include=subscription` on
  cli-chat-proxy.
- **observe**: short-window ratelimit headers on chat responses (separate from monthly).

See [docs/provider-quota.md](../../docs/provider-quota.md).

Implements the [`@demicodes/provider`](../provider/README.md) contract. Part of
[Demi](../../README.md). Apache-2.0.
