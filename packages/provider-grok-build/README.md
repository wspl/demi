# @demicodes/provider-grok-build

A Demi provider backed by Grok Build's CLI chat proxy. Reuses the OAuth session
stored by the official Grok CLI in `~/.grok/auth.json` (subscription login).

```ts
import { createGrokBuildProvider } from '@demicodes/provider-grok-build'
```

Requires a prior `grok login` (or equivalent) so `~/.grok/auth.json` contains an
OIDC session. Implements the [`@demicodes/provider`](../provider/README.md)
contract. Part of [Demi](../../README.md). Apache-2.0.
