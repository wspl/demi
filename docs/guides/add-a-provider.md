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
// + createRuntime(selection) => AgentProvider   (provided via defineProvider)
//
// Subscription CLIs (codex / claude-code / grok-build):
//   - quota:       docs/provider-quota.md
//   - credentials: docs/provider-global-credentials.md
```

The runtime is where a turn actually runs:

```ts
interface AgentProvider {
  run(request: InferenceRequest): ProviderRun  // an AsyncIterable<ProviderEvent>
  clone(): AgentProvider                       // independent runtime, same configuration
  dispose?(): void | Promise<void>             // release long-lived resources (e.g. a CLI subprocess)
}
```

`clone()` must return a runtime that can be disposed independently. Auth stores and
quota observers may be shared; subprocesses, sockets, and continuation state must not.

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
import { zeroUsage } from '@demicodes/core'
import { defineProvider, type AgentProvider, type InferenceRequest, type ProviderRun } from '@demicodes/provider'

export function createEchoProvider() {
  return defineProvider({
    id: 'echo',
    displayName: 'Echo',
    createRuntime: createEchoRuntime,
  })
}

function createEchoRuntime(): AgentProvider {
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
    clone: () => createEchoRuntime(),
  }
}
```

## Reuse the shared building blocks

Don't re-derive what `@demicodes/provider` already ships — the boundary test forbids
re-implementing several of these:

- `modelSelectionFromCatalog`, `withProviderId` — turn a `listModels()` catalog into
  selections / stamp the provider id.
- `redactSecretText`, `httpErrorCode`, `normalizeErrorCode`, `providerErrorFromUnknown`,
  `authStatusFromKey`, `httpRequestFailedEvent` — for HTTP backends.
- `createProviderQuota`, `ensureQuota`, percent/severity helpers — subscription rate-limit
  surface (`docs/provider-quota.md`).
- `zeroUsage` (from `@demicodes/core`) — a zeroed `TokenUsage`.
- `parseProviderData`, `parseProviderJson`, `ProviderDataError` — validate a concrete
  provider's schema and classify failures without including payload values.
- `normalizeBaseUrl` (from `@demicodes/utils`). JSON decoding helpers do not validate
  wire data; numeric fallback helpers do not replace a usage schema.

## Validate incoming data

Define the fields the mapper consumes, derive its types, and validate at transport
ingress. Do not cast decoded JSON to an event type. Extra fields may be permitted
without accepting malformed known fields. Specify exactly which event tags are
ignored; unsupported tags must not silently swallow terminal or tool-call events.

This runnable example illustrates a provider-owned wire protocol:

```ts
import { z } from 'zod'
import { zeroUsage } from '@demicodes/core'
import { parseProviderJson, type ProviderEvent } from '@demicodes/provider'

const eventSchema = z.discriminatedUnion('type', [
  z.looseObject({ type: z.literal('text'), text: z.string() }),
  z.looseObject({ type: z.literal('complete') }),
  z.looseObject({ type: z.literal('progress') }),
])

function mapEvent(event: z.infer<typeof eventSchema>): ProviderEvent | null {
  switch (event.type) {
    case 'text':
      return { type: 'text_delta', text: event.text }
    case 'complete':
      return { type: 'response', usage: zeroUsage() }
    case 'progress':
      return null
  }
}

export function receiveEvent(json: string): ProviderEvent | null {
  return mapEvent(parseProviderJson(eventSchema, json, 'Example stream'))
}

receiveEvent('{"type":"text","text":"hello","extra":1}') // Text delta.
receiveEvent('{"type":"progress"}') // Explicitly ignored.
// Both throw ProviderDataError with field paths, excluding payload values:
// receiveEvent('{"type":"text","text":{}}')
// receiveEvent('{"type":"unknown_terminal"}')
```

For the complete transport/schema/mapper path, see Codex's `sse.ts`,
`response-schemas.ts` (WebSocket envelope), `responses.ts`, and
`response-contracts.test.ts` under `packages/provider-codex/src`, together with
its shared wire contract in `packages/provider/src/responses-wire.ts`. Follow [Data Contracts](../data-contracts.md) for
missing/null policies, persistent data, and tests with synthetic inputs.

## Optional: quota

For subscription or rate-limited backends, attach `quota` on the public provider shell:

```ts
import { createProviderQuota, defineProvider } from '@demicodes/provider'

const quota = createProviderQuota({
  providerId: 'acme',
  canProbe: true,
  canObserve: true,
  probeCost: 'free',
  probe: async ({ signal }) => {
    // fetch vendor usage API → { plan, accountLabel, windows }
    return { windows: [], plan: null, accountLabel: null }
  },
  observe: ({ headers }) => {
    // map response headers → windows, or return null
    return null
  },
})

return defineProvider({
  id: 'acme',
  displayName: 'Acme',
  quota,
  // Runtime must implement AgentProvider.clone() (independent live process / continuation state).
  createRuntime: () => new AcmeProvider({ quota }),
})
```

Products read `provider.quota?.latest()` or `ensureQuota(provider.quota)`. Agent frames
never carry secrets or raw vendor billing payloads.

Full design: [docs/provider-quota.md](../provider-quota.md).

## Optional: multi-credential (global active)

Subscription CLIs often have one vendor login slot. Demi still supports multiple stored
credentials under `$DEMI_HOME/credentials/<providerId>/` with a **process-global**
active pointer:

```ts
await provider.credentials?.beginLogin?.()   // invoke vendor CLI login (no id)
const entry = await provider.credentials?.importDefault?.() // snapshot → id
await provider.credentials?.setActive(entry!.id)
```

Do **not** mint a new `Provider` id per account. Switching is `credentials.setActive`,
not multi-instance providers. After switch, call `quota.clearLatest()` (kits already do
this when wired together).

Full design: [docs/provider-global-credentials.md](../provider-global-credentials.md).

## Register it

The boundary contract requires concrete providers to depend only on `core`,
`provider`, and `utils`. Add your package to `docs/package-boundaries.md` and the
maps in `packages/core/src/__tests__/platform-entrypoints.test.ts`, then pass it to
the server:

```ts
const server = new AgentServer({ agent, providers: [createEchoProvider()] })
```
