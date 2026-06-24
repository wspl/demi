# Provider Public API Plan

| | |
|---|---|
| Date | 2026-06-25 |
| Status | Implemented |
| Scope | Provider construction API, AgentServer composition, Web/REPL provider selection, OpenAI/Anthropic API providers |

## Goal

Demi's public provider API should match the way users think about agent assembly:

```ts
const agent = createCodingAgent({
  host,
  providers: [
    createClaudeCodeProvider({ claudePath: '/opt/homebrew/bin/claude' }),
    createCodexProvider({ codexHome: '~/.codex' }),
    createOpenAIApiProvider(),
    createAnthropicApiProvider(),
    createOpenAIApiProvider({
      id: 'openrouter',
      displayName: 'OpenRouter',
      baseUrl: 'https://openrouter.ai/api/v1',
      apiKey: () => process.env.OPENROUTER_API_KEY,
      models: [{ id: 'openai/gpt-5.1', displayName: 'GPT 5.1' }],
    }),
  ],
})
```

The user-facing concept is **Provider**. Users should not need to understand or instantiate
`ProviderDefinition`, `ProviderRegistry`, or serializable provider `config` objects. Those are
implementation details of transport and session management.

## Current Shape

The public assembly path is direct provider composition:

```ts
const providers = [
  createClaudeCodeProvider(),
  createCodexProvider(),
  createOpenAIApiProvider(),
  createAnthropicApiProvider(),
]

const server = new AgentServer({
  agent: harness,
  providers,
})
```

`AgentClient.open` and Web `prepareSession` carry only `ProviderSelection` (`providerId` +
model selection). Serializable provider config is no longer a browser-visible protocol concept.

## Final Public API

### Provider

`@demi/provider` should expose one public provider object type:

```ts
export interface Provider {
  id: string
  displayName: string
  state?(): Promise<ProviderRuntimeState> | ProviderRuntimeState
  listModels?(): Promise<ProviderModelList> | ProviderModelList
}
```

Concrete packages expose creation functions:

```ts
createClaudeCodeProvider(options?: ClaudeCodeProviderOptions): Provider
createCodexProvider(options?: CodexProviderOptions): Provider
createOpenAIApiProvider(options?: OpenAIApiProviderOptions): Provider
createAnthropicApiProvider(options?: AnthropicApiProviderOptions): Provider
```

The returned value is safe to pass around as an app-level provider. It is not the per-session live
runtime. For example, `createClaudeCodeProvider()` must not immediately start a Claude process.

### Internal Runtime

Each public `Provider` owns an internal runtime factory:

```ts
interface ProviderRuntimeFactory {
  createRuntime(selection: ProviderSelection): Promise<AgentProvider> | AgentProvider
}
```

This factory is not part of the user-facing API. It exists because live provider runtimes may hold
session-local state:

- Claude Code keeps a long-lived CLI process, pending tool state, sent user count, and model/thinking
  signature.
- Codex may choose an SSE/WebSocket transport and hold transport-level retry/timeout state.
- HTTP API providers may hold endpoint profile settings but should create stateless runtimes
  per session or provider switch.

Sharing one live `AgentProvider` instance across sessions is explicitly invalid.

### Agent Assembly

`AgentServer` should accept providers directly:

```ts
export interface AgentServerOptions {
  agent: AgentHarness<unknown>
  providers: Provider[]
  shell?: Omit<BashEnvironmentOptions, 'host' | 'commands'>
}
```

The server can build an internal provider map, but callers should not construct or mutate a
registry.

Product entry points should read like composition, not registration:

```ts
const providers = [
  createClaudeCodeProvider({ claudePath: options.claudePath }),
  createCodexProvider({ codexHome: options.codexHome }),
  createOpenAIApiProvider(),
  createAnthropicApiProvider(),
  ...configuredApiProviders(options),
]

startWebServer({ providers, cwd: options.cwd, ... })
```

### Provider Selection

The protocol between UI and server should carry only provider identity and model selection:

```ts
interface ProviderSelection {
  providerId: string
  modelId: string
  thinking: ThinkingConfig | null
  serviceTierId?: string | null
}
```

The UI should never receive raw provider options. In particular, API keys and secret-bearing config
must not round-trip through browser-visible frames.

The server resolves `providerId` against its in-memory provider map and asks that provider to create
a runtime for the selected model.

## Concrete Provider Options

### Claude Code

```ts
createClaudeCodeProvider({
  id?: string
  displayName?: string
  claudePath?: string
  models?: ModelPolicy
})
```

`models` is a catalog policy over the provider's native model catalog:

```ts
type ModelPolicy = {
  include?: string[]
  exclude?: string[]
  default?: string
}
```

The provider remains responsible for catalog capability metadata. The policy only filters and
chooses defaults.

### Codex

```ts
createCodexProvider({
  id?: string
  displayName?: string
  codexHome?: string
  baseUrl?: string
  transport?: 'auto' | 'sse' | 'websocket'
  models?: ModelPolicy
})
```

Codex keeps its existing auth reuse and Responses transport behavior. The creation function hides
the old config parser from normal users.

### OpenAI API

```ts
createOpenAIApiProvider({
  id?: string
  displayName?: string
  envPrefix?: string
  baseUrl?: string
  apiKey?: () => string | Promise<string> | null | undefined
  headers?: () => Record<string, string> | Promise<Record<string, string>>
  models?: ApiProviderModel[]
  defaultModelId?: string
  request?: OpenAIApiRequestOptions
})
```

With no options, this provider targets the official OpenAI API:

- `id: 'openai'`
- `displayName: 'OpenAI API'`
- `envPrefix: 'OPENAI'`
- `baseUrl: process.env.OPENAI_BASE_URL ?? 'https://api.openai.com/v1'`
- `apiKey: () => process.env.OPENAI_API_KEY`
- built-in official OpenAI model catalog metadata

Passing `baseUrl`, or setting the endpoint env var resolved by `envPrefix`, turns the same provider
into an OpenAI-compatible endpoint adapter, such as OpenRouter, LiteLLM, vLLM, or an internal
gateway. Non-official endpoints should pass explicit `models` metadata unless Demi ships a
first-class profile for that endpoint.

The wire contract is OpenAI Chat Completions:

- `POST {baseUrl}/chat/completions`
- streaming SSE chunks
- `tools: [{ type: 'function', function: ... }]`
- `choices[].delta.content` -> `text_delta`
- `choices[].delta.tool_calls[].function.arguments` accumulated into `tool_call_requested`
- optional `stream_options: { include_usage: true }`

It should not reuse the Codex Responses mapper. Codex and OpenAI API are different wire
contracts.

### Anthropic API

```ts
createAnthropicApiProvider({
  id?: string
  displayName?: string
  envPrefix?: string
  baseUrl?: string
  apiKey?: () => string | Promise<string> | null | undefined
  headers?: () => Record<string, string> | Promise<Record<string, string>>
  anthropicVersion?: string
  models?: ApiProviderModel[]
  defaultModelId?: string
  request?: AnthropicApiRequestOptions
})
```

With no options, this provider targets the official Anthropic API:

- `id: 'anthropic'`
- `displayName: 'Anthropic API'`
- `envPrefix: 'ANTHROPIC'`
- `baseUrl: process.env.ANTHROPIC_BASE_URL ?? 'https://api.anthropic.com/v1'`
- `apiKey: () => process.env.ANTHROPIC_API_KEY`
- a provider-owned default `anthropic-version`
- built-in official Anthropic model catalog metadata

Passing `baseUrl`, or setting the endpoint env var resolved by `envPrefix`, turns the same provider
into an Anthropic-compatible endpoint adapter. Non-official endpoints should pass explicit `models`
metadata unless Demi ships a first-class profile for that endpoint.

The wire contract is Anthropic Messages:

- `POST {baseUrl}/messages`
- event stream mapping for `message_start`, `content_block_start`, `content_block_delta`,
  `content_block_stop`, `message_delta`, and `message_stop`
- tool definitions in Anthropic `tools`
- tool results as user content blocks with `type: 'tool_result'`
- thinking only when the model profile explicitly supports it

It should not reuse the Claude Code JSONL/CLI mapper. Claude Code and Anthropic API are
different transport contracts.

### API Provider Model Metadata

Official OpenAI and Anthropic API providers can ship curated default model metadata. Compatible
endpoints often do not expose enough capability metadata, so Demi should require capability metadata
in config instead of guessing:

```ts
interface ApiProviderModel {
  id: string
  displayName?: string
  description?: string
  contextWindow?: number | null
  outputLimit?: number | null
  supportsTools?: boolean | null
  supportsAttachments?: boolean | null
  supportsReasoning?: boolean | null
  supportedThinkingEfforts?: string[] | null
  defaultThinkingEffort?: string | null
  canDisableThinking?: boolean | null
  serviceTiers?: ProviderServiceTier[] | null
  defaultServiceTierId?: string | null
}
```

If an API-compatible endpoint exposes `/models`, it may supplement ids and display names, but it
must not invent tool, attachment, context, or thinking capabilities.

### Endpoint Environment Variables

OpenAI API and Anthropic API providers should support endpoint configuration through env vars as a
first-class path. This keeps common setup simple and avoids forcing users to pass `baseUrl` in code.

Resolution order:

1. Explicit constructor option: `baseUrl`.
2. Environment variable: `${envPrefix}_BASE_URL`.
3. Official default endpoint.

API key resolution follows the same prefix:

1. Explicit constructor option: `apiKey`.
2. Environment variable: `${envPrefix}_API_KEY`.

`envPrefix` defaults to `OPENAI` for `createOpenAIApiProvider()` and `ANTHROPIC` for
`createAnthropicApiProvider()`.

Examples:

```ts
createOpenAIApiProvider()
// OPENAI_BASE_URL=https://api.openai.com/v1
// OPENAI_API_KEY=...

createAnthropicApiProvider()
// ANTHROPIC_BASE_URL=https://api.anthropic.com/v1
// ANTHROPIC_API_KEY=...

createOpenAIApiProvider({
  id: 'openrouter',
  displayName: 'OpenRouter',
  envPrefix: 'OPENROUTER',
  models: [...],
})
// OPENROUTER_BASE_URL=https://openrouter.ai/api/v1
// OPENROUTER_API_KEY=...

createAnthropicApiProvider({
  id: 'anthropic-gateway',
  displayName: 'Internal Anthropic Gateway',
  envPrefix: 'ANTHROPIC_GATEWAY',
  models: [...],
})
// ANTHROPIC_GATEWAY_BASE_URL=https://gateway.example.internal/anthropic/v1
// ANTHROPIC_GATEWAY_API_KEY=...
```

Explicit options win over env vars. Env vars win over official defaults. The provider should trim
trailing slashes before appending endpoint paths, but it should not silently rewrite arbitrary
middle path segments because gateways often mount providers under custom prefixes.

## Secret Boundary

API secrets and endpoint options must be resolved only in the provider creator closure:

```ts
createOpenAIApiProvider()
createAnthropicApiProvider()
createOpenAIApiProvider({
  id: 'openrouter',
  baseUrl: 'https://openrouter.ai/api/v1',
  apiKey: () => process.env.OPENROUTER_API_KEY,
  models: [...],
})
```

The default OpenAI and Anthropic providers resolve official API keys from `OPENAI_API_KEY` and
`ANTHROPIC_API_KEY` respectively, and endpoints from `OPENAI_BASE_URL` and `ANTHROPIC_BASE_URL`.
The browser-visible protocol may carry `providerId`, `modelId`, thinking, and service tier only. It
must not carry `apiKey`, `headers`, raw `baseUrl` values, `envPrefix`, or arbitrary serializable
provider config.

For Web, this means replacing `prepareSession -> ProviderConfig` with a server-side session
preparation step that returns a public `ProviderSelection` object and opens server sessions using
server-held providers.

## Migration

1. Introduce public `Provider` and `ProviderSelection` types in `@demi/provider`. Implemented.
2. Add `createClaudeCodeProvider` and `createCodexProvider` as the public creation functions. Implemented.
3. Change `AgentServerOptions` from `providerRegistry` to `providers`. Implemented.
4. Move provider id lookup into `AgentServer` as a private map. Implemented.
5. Change `AgentClient.open` / Web control flow to avoid browser-visible provider config. Implemented.
6. Update Web and REPL composition roots to pass `providers: [...]`. Implemented.
7. Add `@demi/provider-openai-api`. Implemented.
8. Add `@demi/provider-anthropic-api`. Implemented.

No compatibility shim should remain in the final state. `ProviderDefinition`, `ProviderRegistry`,
and `create*ProviderDefinition` are not public assembly concepts.

## Tests And Acceptance

### Provider Public API

Add or update tests under `packages/provider/src/__tests__/`:

- provider map rejects duplicate `id`
- provider list preserves display metadata
- selection resolves by `providerId`
- public API exports `Provider` / `ProviderSelection`, not a required `ProviderRegistry` creation path

### Agent Server

Update `packages/agent/src/__tests__/server.test.ts` and transport tests:

- `AgentServer` accepts `providers: [...]`
- opening a session creates a fresh live runtime from the selected provider
- two sessions using the same public provider do not share runtime state
- switching model within the same provider keeps runtime reuse semantics where the provider supports it
- switching provider disposes the previous runtime and creates the next one

### Web Secret Safety

Update `packages/web/src/server/__tests__/transport.e2e.test.ts`:

- `listProviders` exposes only provider ids, labels, and availability
- `listModels` exposes only model metadata
- `prepare/open` frames never include `apiKey`, custom secret headers, `baseUrl`, `envPrefix`, or raw
  provider options
- API provider secrets are read server-side via the provider closure
- API provider endpoints are read server-side from explicit options or env vars and do not leak to
  the browser-visible protocol

### OpenAI API Provider

Add `packages/provider-openai-api/src/__tests__/`:

- default provider uses `OPENAI_BASE_URL` when present and otherwise `https://api.openai.com/v1`
- default provider reads `OPENAI_API_KEY`
- explicit `baseUrl` and `apiKey` options take precedence over env vars
- custom `envPrefix` resolves `${envPrefix}_BASE_URL` and `${envPrefix}_API_KEY`
- request body maps text, image support gates, tool definitions, prior tool use/result, and service tier
- streaming parser maps text deltas, split tool call arguments, usage, provider errors, and abort
- malformed JSON tool arguments degrade predictably
- AgentSession tool roundtrip includes tool result in the next request
- provider-stream steer fallback materializes into the next request without native `steer()`

### Anthropic API Provider

Add `packages/provider-anthropic-api/src/__tests__/`:

- default provider uses `ANTHROPIC_BASE_URL` when present and otherwise `https://api.anthropic.com/v1`
- default provider reads `ANTHROPIC_API_KEY`
- explicit `baseUrl` and `apiKey` options take precedence over env vars
- custom `envPrefix` resolves `${envPrefix}_BASE_URL` and `${envPrefix}_API_KEY`
- request body groups user/tool_result and assistant/tool_use turns in Anthropic order
- streaming parser maps thinking, text, tool_use, message usage, provider errors, and abort
- thinking replay skips unsigned thinking unless the profile explicitly allows it
- AgentSession tool roundtrip preserves `tool_use_id` pairing
- provider-stream steer fallback materializes into the next request without native `steer()`

### Real Acceptance

Real-provider tests stay skipped by default and require explicit environment variables:

- `DEMI_OPENAI_API_E2E=1`
- `DEMI_ANTHROPIC_API_E2E=1`

Each real acceptance should cover:

- minimal text response
- tool roundtrip through the standard shell tools
- one steer during an active turn, accepted by AgentSession fallback and visible in the continuation

## Package Boundary Updates Required During Implementation

The implementation checkpoint must update `docs/package-boundaries.md` to reflect:

- `@demi/provider` owns public `Provider` / `ProviderSelection` contracts and internal provider
  lookup helpers, not a user-facing registry assembly API.
- `@demi/provider-openai-api` owns OpenAI Chat Completions API mapping, including official
  OpenAI defaults and configurable compatible endpoints.
- `@demi/provider-anthropic-api` owns Anthropic Messages API mapping, including official Anthropic
  defaults and configurable compatible endpoints.
- `@demi/web`, `@demi/repl`, and future product packages assemble providers by passing
  `providers: [...]` at creation time.

Until the migration is implemented, current package boundary text still describes the existing code.
