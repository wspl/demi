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
import { defineProvider, zeroUsage, type AgentProvider, type InferenceRequest, type ProviderRun } from '@demicodes/provider'

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

- `modelSelectionFromCatalog`, `withCatalogProviderId`, `withModelProviderId` — turn a
  `listModels()` catalog into selections / stamp the provider id onto a catalog or one
  model.
- `modelLimitsSchema` — the `contextWindow` / `outputLimit` constraint every catalog
  entry must satisfy.
- `thinkingToReasoningEffort` — the request-body field
  the OpenAI-compatible formats spell the same way.
- `redactSecretText`, `httpErrorCode`, `normalizeErrorCode`, `providerErrorFromUnknown`,
  `authStatusFromKey`, `httpRequestFailedEvent` — for HTTP backends.
- `createProviderQuota`, `ensureQuota`, percent/severity helpers — subscription rate-limit
  surface (`docs/provider-quota.md`).
- `zeroUsage` (from `@demicodes/core`) — a zeroed `TokenUsage`.
- `normalizeBaseUrl`, `parseJsonObject`, `numberOrZero`, `decodeJwtPayload`, `stringifyToolInput` (from
  `@demicodes/utils`; `decodeJwtPayload` returns the claims as `unknown` and verifies
  nothing — validate what you read from it).

See `packages/provider-anthropic-api` / `packages/provider-google` (HTTP) and
`packages/provider-codex` (CLI/OAuth) for full references.

## Read the vendor's stream

Everything the vendor sends — SSE frames, CLI stdout, OAuth responses, cached
credential files — is untrusted input. Decode it with a schema at the point of entry
and derive your types from that schema. `JSON.parse(text) as VendorEvent` is not a
check: the first malformed field then surfaces as a `TypeError` deep in a mapper.

Do not copy another provider's decoder. `@demicodes/provider` owns these:

- `readServerSentEvents(body, signal)` / `ServerSentEvent` — SSE framing. One frame's
  `data:` fields arrive joined with `\n`, so a JSON event is one `data` string. It is
  the same reader for every provider; there is no vendor variation to fork for.
- `responses.ts` — the OpenAI Responses API: `decodeResponsesEvent`, the event, item
  and usage schemas, and `tokenUsageFromResponsesUsage`.
- `chat-completions.ts` — the OpenAI Chat Completions stream:
  `decodeChatCompletionChunk`, the chunk/delta/tool-call schemas, and
  `tokenUsageFromChatCompletionsUsage`. Every OpenAI-compatible gateway uses these.
- `mapResponsesStream(frames, vendorLabel, signal)` / `mapResponsesEvents` and
  `mapChatCompletionsStream(frames, vendorLabel, signal)` — the whole stream, decoded
  and mapped to `ProviderEvent`s. A Responses or Chat Completions provider writes only
  its request body: the mapper is shared, and the vendor's name reaches the user
  through `vendorLabel` in the error text.
- `taggedUnion(branches)` — for a vendor whose events carry their own `type` tag
  (Anthropic, Claude Code, xAI). Register a schema per tag you map.
- For a login flow: `decodeJsonResponse(response, schema, onMalformed)` decodes one
  OAuth response and reports a body that does not match through your own error class,
  and `oauthSecondsSchema` / `pollIntervalSecondsSchema` / `lifetimeSecondsSchema` read
  the RFC 8628 durations a server states as either a number or a digit string.
  `writeJsonFileAtomic` (from `@demicodes/provider/credentials-pool`) writes the
  vendor `auth.json` a refresh produces.

`taggedUnion` fixes the failure policy so every provider treats the vendor the same
way: **an unregistered `type` decodes to `null` and is ignored** (a vendor shipping a
new event must not break a live stream), while **a registered `type` with a malformed
payload throws**, with the offending field in the message. Describe only the fields
you consume, and use `z.looseObject` so the vendor's other fields survive a decode —
a reasoning item is re-sent to the vendor whole.

```ts
import { taggedUnion, readServerSentEvents } from '@demicodes/provider'
import { z } from 'zod'

const acmeEvent = taggedUnion({
  message_delta: z.looseObject({ type: z.literal('message_delta'), text: z.string() }),
  message_stop: z.looseObject({ type: z.literal('message_stop') }),
})

for await (const frame of readServerSentEvents(response.body, request.cancel)) {
  const event = acmeEvent.parse(JSON.parse(frame.data))
  if (!event)
    continue                                  // an event type this adapter does not map
  if (event.type === 'message_delta')
    yield { type: 'text_delta', text: event.text }
}
```

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
`provider`, `utils`, and `zod` — declare `zod` in your `package.json`, since the
decoders above are your own schemas. Add your package to
`docs/package-boundaries.md` and the maps in
`packages/core/src/__tests__/platform-entrypoints.test.ts`, then pass it to the
server:

```ts
const server = new AgentServer({ agent, providers: [createEchoProvider()] })
```
