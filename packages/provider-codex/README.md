# @demicodes/provider-codex

A Demi provider backed by Codex (ChatGPT / Responses transport). Exposes
`createCodexProvider()`.

```ts
import { createCodexProvider } from '@demicodes/provider-codex'

const provider = createCodexProvider()
// provider.auth, provider.quota, provider.credentials (multi-account pool by default)
```

## Auth and credentials

- Default material: `~/.codex/auth.json` (or `$CODEX_HOME`).
- Multi-credential pool under `$DEMI_HOME/credentials/codex/` with global
  `provider.credentials.setActive(id)`.
- Lifecycle: `beginLogin` → vendor `codex login` → `importDefault` → `setActive`.

See [docs/provider-global-credentials.md](../../docs/provider-global-credentials.md).

## Quota

- **probe** (cost: `minimal_request`): tiny streamed Responses call; reads
  `x-codex-primary-*` / `x-codex-secondary-*` headers.
- **observe**: same headers on live inference responses (preferred steady-state).

See [docs/provider-quota.md](../../docs/provider-quota.md).

Implements the [`@demicodes/provider`](../provider/README.md) contract. Part of
[Demi](../../README.md). Apache-2.0.
