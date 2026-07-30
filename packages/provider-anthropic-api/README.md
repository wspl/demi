# @demicodes/provider-anthropic-api

A Demi provider for the Anthropic Messages API. Exposes
`createAnthropicApiProvider()`.

```ts
import { createAnthropicApiProvider } from '@demicodes/provider-anthropic-api'
```

## Base URL

`baseUrl` (or `${envPrefix}_BASE_URL`, default `ANTHROPIC_BASE_URL`) must already
include the version prefix, usually `/v1`. The provider only appends
`/messages`, and leaves the value alone when it already ends with `/messages`.

Default: `https://api.anthropic.com/v1` → `…/v1/messages`.

This is not the same as Claude Code / Kimi Coding, where `ANTHROPIC_BASE_URL` is
a root such as `https://api.kimi.com/coding/` and the client appends
`/v1/messages`. Those roots are not drop-in values here — pass
`https://api.kimi.com/coding/v1` (or the full `…/v1/messages` URL).

Implements the [`@demicodes/provider`](../provider/README.md) contract. Part of
[Demi](../../README.md). Apache-2.0.
