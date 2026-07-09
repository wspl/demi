# Add a Provider

A provider adapts an inference backend (an API or a CLI) to Demi's contract in
`@demicodes/provider`. The runtime never imports your SDK directly — it only sees the
contract — so a provider is the single place that knows about one backend.

## The contract

```ts
import { defineProvider, type AgentProvider, type InferenceRequest, type ProviderRun } from '@demicodes/provider'
```

A `Provider` is a small descriptor plus a runtime factory:

```ts
interface Provider {
  id: string
  displayName: string
  auth?: ProviderAuth                         // optional: report authenticated/unauthenticated
  quota?: ProviderQuota                       // optional: subscription / rate-limit windows
  credentials?: ProviderCredentials           // optional: multi-cred pool + global setActive
  state?(): ProviderRuntimeState              // optional: report ready/unavailable
  listModels?(): Promise<ProviderModelList>   // optional: catalog for the model picker
}
// Subscription CLIs (codex / claude-code / grok-build): see docs/provider-global-credentials.md
// + createRuntime(selection) => AgentProvider   (provided via defineProvider)
```

The runtime is where a turn actually runs:

```ts
interface AgentProvider {
  run(request: InferenceRequest): ProviderRun  // an AsyncIterable<ProviderEvent>
  dispose?(): void | Promise<void>             // release long-lived resources (e.g. a CLI subprocess)
}
```

`run()` returns an async iterable of `ProviderEvent`s. Yield them as the backend
streams:

```ts
type ProviderEvent =
  | { type: 'thinking_start' }
  | { type: 'thinking_delta'; text: string }
  | { type: 'thinking_signature'; signature: string }
  | { type: 'redacted_thinking'; data: string }
  | { type: 'text_delta'; text: string }
  | { type: 'tool_call_requested'; toolUseId: string; toolName: string; input: unknown }
  | { type: 'response'; usage: TokenUsage }   // terminal: the turn completed
  | { type: 'error'; message: string; code: string | null }
  | { type: 'abort' }
```

End every successful turn with a single `response` event carrying token usage.

## A minimal provider

```ts
import { defineProvider, zeroUsage } from '@demicodes/provider'

export function createEchoProvider() {
  return defineProvider({
    id: 'echo',
    displayName: 'Echo',
    createRuntime() {
      return {
        run(request: InferenceRequest): ProviderRun {
          async function* events() {
            const lastUser = [...request.items].reverse().find((i) => i.type === 'user_message')
            const text = lastUser?.content.map((b) => (b.type === 'text' ? b.text : '')).join('') ?? ''
            yield { type: 'text_delta', text: `You said: ${text}` }
            yield { type: 'response', usage: zeroUsage() }
          }
          return events()
        },
      }
    },
  })
}
```

## Reuse the shared building blocks

Don't re-derive what `@demicodes/provider` already ships — the boundary test forbids
re-implementing several of these:

- `modelSelectionFromCatalog`, `withProviderId` — turn a `listModels()` catalog into
  selections / stamp the provider id.
- `redactSecretText`, `httpErrorCode`, `normalizeErrorCode`, `providerErrorFromUnknown`,
  `authStatusFromKey`, `httpRequestFailedEvent` — for HTTP backends.
- `zeroUsage` (from `@demicodes/core`) — a zeroed `TokenUsage`.
- `normalizeBaseUrl`, `parseJsonObject`, `numberOrZero` (from `@demicodes/utils`).

See `packages/provider-anthropic-api` (HTTP) and `packages/provider-codex` (CLI/OAuth)
for full references.

## Register it

The boundary contract requires concrete providers to depend only on `core`,
`provider`, and `utils`. Add your package to `docs/package-boundaries.md` and the
maps in `packages/core/src/__tests__/platform-entrypoints.test.ts`, then pass it to
the server:

```ts
const server = new AgentServer({ agent, providers: [createEchoProvider()] })
```
