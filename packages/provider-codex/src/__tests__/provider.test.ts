import { expect, test } from 'bun:test'
import type { ModelSelection } from '@demi/core'
import { AgentSession, createStandardAgentTools, type AgentHarnessRuntime } from '@demi/agent'
import { providerRuntime, type InferenceRequest, type ProviderSelection } from '@demi/provider'
import { BashEnvironment } from '@demi/shell'
import { LocalHost } from '@demi/host-local'
import { StaticCodexAuthStore, type CodexResolvedAuth } from '../auth'
import { CodexProvider, buildCodexHeaders, createCodexProvider, parseCodexProviderConfig, responsesUrlForAuth } from '../provider'
import type { CodexResponseStreamEvent } from '../responses'
import {
  AutoCodexResponsesTransport,
  CodexHttpError,
  WebSocketCodexResponsesTransport,
  type CodexResponsesTransport,
  type CodexTransportRequest,
} from '../transport'

const chatgptAuth: CodexResolvedAuth = {
  kind: 'chatgpt',
  mode: 'chatgpt',
  accessToken: 'access-token',
  refreshToken: 'refresh-token',
  accountId: 'account-1',
  email: 'dev@example.com',
  isFedrampAccount: false,
  expiresAt: null,
  authFile: '/tmp/auth.json',
}

const model: ModelSelection = {
  providerId: 'codex',
  model: {
    id: 'gpt-5.4',
    name: 'GPT-5.4',
    contextWindow: 100_000,
    inputLimit: null,
    thinking: [],
    acceptedExtensions: [],
  },
  thinking: { type: 'effort', effort: 'medium', summary: null },
}

function providerSelection(): ProviderSelection {
  return { providerId: 'codex', model }
}

test('Codex public provider only accepts serializable config fields', async () => {
  const injectedTransport = new FakeCodexTransport([])
  expect(
    parseCodexProviderConfig({
      codexHome: '/tmp/codex-home',
      baseUrl: 'https://example.test/backend-api',
      transport: 'sse',
      headers: { 'x-test': 'ok' },
      authStore: new StaticCodexAuthStore(chatgptAuth),
      transportImpl: injectedTransport,
    }),
  ).toEqual({
    codexHome: '/tmp/codex-home',
    baseUrl: 'https://example.test/backend-api',
    transport: 'sse',
    headers: { 'x-test': 'ok' },
  })
  expect(() => parseCodexProviderConfig(1)).toThrow('must be an object')
  expect(() => parseCodexProviderConfig({ transport: 'stdio' })).toThrow('transport')
  expect(() => parseCodexProviderConfig({ headers: { ok: 1 } })).toThrow('headers.ok')

  const provider = await providerRuntime(createCodexProvider({
    authStore: new StaticCodexAuthStore(chatgptAuth),
    transportImpl: injectedTransport,
  } as never), providerSelection())
  expect((provider as unknown as { transport?: unknown }).transport).not.toBe(injectedTransport)
})

test('buildCodexHeaders and responsesUrlForAuth follow Codex auth routing', () => {
  const headers = buildCodexHeaders(chatgptAuth, makeRequest(), { userAgent: 'demi-test/1.0' })
  expect(headers.get('Authorization')).toBe('Bearer access-token')
  expect(headers.get('ChatGPT-Account-ID')).toBe('account-1')
  expect(headers.get('OpenAI-Beta')).toBe('responses=experimental')
  expect(headers.get('session-id')).toBe('session-1')
  expect(headers.get('thread-id')).toBe('session-1')
  expect(headers.get('x-client-request-id')).toBe('request-1')
  expect(headers.get('User-Agent')).toBe('demi-test/1.0')

  expect(responsesUrlForAuth(chatgptAuth)).toBe('https://chatgpt.com/backend-api/codex/responses')
  expect(
    responsesUrlForAuth({
      kind: 'apiKey',
      mode: 'apiKey',
      apiKey: 'sk-test',
      authFile: null,
    }),
  ).toBe('https://api.openai.com/v1/responses')
})

test('CodexProvider streams text, thinking, tool calls, and usage through transport events', async () => {
  const transport = new FakeCodexTransport([
    [
      { type: 'response.output_item.added', item: { type: 'reasoning', id: 'rs_1' } },
      { type: 'response.reasoning_text.delta', delta: 'think' },
      { type: 'response.output_item.done', item: { type: 'reasoning', id: 'rs_1', encrypted_content: 'enc' } },
      { type: 'response.output_text.delta', delta: 'hello' },
      { type: 'response.output_item.done', item: { type: 'function_call', id: 'fc_1', call_id: 'call_1', name: 'shell_exec', arguments: '{"script":"pwd"}' } },
      { type: 'response.completed', response: { usage: { input_tokens: 10, output_tokens: 3 } } },
    ],
  ])
  const provider = new CodexProvider({
    authStore: new StaticCodexAuthStore(chatgptAuth),
    transportImpl: transport,
    transport: 'sse',
  })
  const events = []
  for await (const event of provider.run(makeRequest([{ type: 'user_message', content: [{ type: 'text', text: 'hi' }] }]))) {
    events.push(event)
  }

  expect(events).toEqual([
    { type: 'thinking_start' },
    { type: 'thinking_delta', text: 'think' },
    { type: 'thinking_signature', signature: JSON.stringify({ type: 'reasoning', id: 'rs_1', encrypted_content: 'enc' }) },
    { type: 'text_delta', text: 'hello' },
    { type: 'tool_call_requested', toolUseId: 'call_1|fc_1', toolName: 'shell_exec', input: { script: 'pwd' } },
    { type: 'response', usage: { inputTokens: 10, outputTokens: 3, cacheReadTokens: 0, cacheWriteTokens: 0 } },
  ])
  expect(transport.requests[0]?.body).toMatchObject({ model: 'gpt-5.4', prompt_cache_key: 'session-1' })
})

test('CodexProvider refreshes auth once after a 401 response and retries the request', async () => {
  const refreshedAuth: CodexResolvedAuth = { ...chatgptAuth, accessToken: 'new-access-token' }
  const authStore = new RecordingAuthStore([chatgptAuth, refreshedAuth])
  const transport = new FakeCodexTransport([
    new CodexHttpError(401, 'Unauthorized', '{"error":{"message":"expired"}}'),
    [{ type: 'response.output_text.delta', delta: 'ok' }, { type: 'response.completed', response: { usage: { input_tokens: 1 } } }],
  ])
  const provider = new CodexProvider({
    authStore,
    transportImpl: transport,
    maxRetries: 0,
  })
  const events = []
  for await (const event of provider.run(makeRequest())) events.push(event)

  expect(authStore.forceRefreshes).toEqual([false, true])
  expect(transport.requests).toHaveLength(2)
  expect(transport.requests[0]?.headers.get('Authorization')).toBe('Bearer access-token')
  expect(transport.requests[1]?.headers.get('Authorization')).toBe('Bearer new-access-token')
  expect(events).toEqual([
    { type: 'text_delta', text: 'ok' },
    { type: 'response', usage: { inputTokens: 1, outputTokens: 0, cacheReadTokens: 0, cacheWriteTokens: 0 } },
  ])
})

test('AutoCodexResponsesTransport falls back to SSE only before WebSocket has emitted events', async () => {
  const beforeStart = new AutoCodexResponsesTransport(
    new FakeCodexTransport([new Error('connect failed')]),
    new FakeCodexTransport([[{ type: 'response.output_text.delta', delta: 'sse' }]]),
  )
  const beforeEvents: CodexResponseStreamEvent[] = []
  for await (const event of beforeStart.stream(makeTransportRequest())) beforeEvents.push(event)
  expect(beforeEvents).toEqual([{ type: 'response.output_text.delta', delta: 'sse' }])

  const afterStart = new AutoCodexResponsesTransport(
    new YieldThenThrowTransport([{ type: 'response.output_text.delta', delta: 'ws' }], new Error('after start')),
    new FakeCodexTransport([[{ type: 'response.output_text.delta', delta: 'sse' }]]),
  )
  const afterEvents: CodexResponseStreamEvent[] = []
  await expect((async () => {
    for await (const event of afterStart.stream(makeTransportRequest())) afterEvents.push(event)
  })()).rejects.toThrow('after start')
  expect(afterEvents).toEqual([{ type: 'response.output_text.delta', delta: 'ws' }])
})

test('WebSocketCodexResponsesTransport uses the Responses WebSocket beta header', async () => {
  CapturingWebSocket.instances = []
  const headers = new Headers({
    accept: 'text/event-stream',
    'content-type': 'application/json',
    'OpenAI-Beta': 'responses=experimental',
    Authorization: 'Bearer token',
  })
  const transport = new WebSocketCodexResponsesTransport({
    WebSocket: CapturingWebSocket,
  })
  const events: CodexResponseStreamEvent[] = []

  for await (const event of transport.stream({
    ...makeTransportRequest(),
    headers,
    body: { model: 'gpt-5.4' },
  })) {
    events.push(event)
  }

  expect(events).toEqual([{ type: 'response.output_text.delta', delta: 'ws' }])
  expect(CapturingWebSocket.instances).toHaveLength(1)
  expect(CapturingWebSocket.instances[0]?.headers).toMatchObject({
    authorization: 'Bearer token',
    'openai-beta': 'responses_websockets=2026-02-06',
  })
  expect(CapturingWebSocket.instances[0]?.headers.accept).toBeUndefined()
  expect(CapturingWebSocket.instances[0]?.headers['content-type']).toBeUndefined()
})

test('WebSocketCodexResponsesTransport finishes when a response.completed event arrives without a close frame', async () => {
  NonClosingCompletedWebSocket.instances = []
  const transport = new WebSocketCodexResponsesTransport({
    WebSocket: NonClosingCompletedWebSocket,
  })
  const events: CodexResponseStreamEvent[] = []

  for await (const event of transport.stream({
    ...makeTransportRequest(),
    body: { model: 'gpt-5.4' },
  })) {
    events.push(event)
  }

  expect(events).toEqual([
    { type: 'response.output_text.delta', delta: 'ws' },
    { type: 'response.completed', response: { usage: { input_tokens: 1, output_tokens: 1 } } },
  ])
  expect(NonClosingCompletedWebSocket.instances[0]?.closeReasons).toContain('response_done')
})

test('CodexProvider integrates with AgentSession and shell tools for function calls', async () => {
  const transport = new FakeCodexTransport([
    [
      { type: 'response.output_item.done', item: { type: 'function_call', id: 'fc_1', call_id: 'call_1', name: 'shell_exec', arguments: '{"script":"printf demi-codex","yieldAfterMs":1000}' } },
      { type: 'response.completed', response: { usage: { input_tokens: 4, output_tokens: 2 } } },
    ],
    [
      { type: 'response.output_text.delta', delta: 'done' },
      { type: 'response.completed', response: { usage: { input_tokens: 8, output_tokens: 2 } } },
    ],
  ])
  const provider = new CodexProvider({
    authStore: new StaticCodexAuthStore(chatgptAuth),
    transportImpl: transport,
    transport: 'sse',
  })
  const environment = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'codex-shell-session',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })
  const runtime: AgentHarnessRuntime<Record<string, never>> = {
    harnessName: 'codex-shell-test',
    initialState: () => ({}),
    systemPrompt: () => 'system',
    tools: () =>
      createStandardAgentTools({
        environment,
        scheduleYield: (_ctx, durationMs) => ({
          output: [{ type: 'text', text: `yield scheduled\nwakeupId: test\ndurationMs: ${durationMs}` }],
          stopAfterToolResult: true,
        }),
      }),
  }
  const session = new AgentSession({ provider, model, cwd: process.cwd(), runtime }, { agentSessionId: 'agent-session-1' })

  await session.send([{ type: 'text', text: 'run shell' }])

  expect(session.transcript().blocks.map((block) => block.type)).toEqual(['user', 'tool_call', 'response', 'text', 'response'])
  const toolBlock = session.transcript().blocks.find((block) => block.type === 'tool_call')
  expect(toolBlock).toMatchObject({ type: 'tool_call', toolName: 'shell_exec', status: 'completed' })
  expect(toolBlock?.type === 'tool_call' ? toolBlock.output[0]?.type === 'text' && toolBlock.output[0].text : '').toContain('demi-codex')
  expect(transport.requests).toHaveLength(2)
  expect(transport.requests[0]?.body).toMatchObject({ prompt_cache_key: 'agent-session-1' })
  expect(JSON.stringify(transport.requests[1]?.body)).toContain('"function_call_output"')
  expect(JSON.stringify(transport.requests[1]?.body)).toContain('demi-codex')
})

test('CodexProvider replays provider-stream steers in a same-turn follow-up before queued sends drain', async () => {
  const transport = new GateCodexTransport([
    [
      { type: 'response.output_text.delta', delta: 'first' },
      { type: 'response.completed', response: { usage: { input_tokens: 4, output_tokens: 2 } } },
    ],
    [
      { type: 'response.output_text.delta', delta: 'continued' },
      { type: 'response.completed', response: { usage: { input_tokens: 8, output_tokens: 2 } } },
    ],
    [
      { type: 'response.output_text.delta', delta: 'queued' },
      { type: 'response.completed', response: { usage: { input_tokens: 12, output_tokens: 2 } } },
    ],
  ])
  const provider = new CodexProvider({
    authStore: new StaticCodexAuthStore(chatgptAuth),
    transportImpl: transport,
    transport: 'sse',
  })
  const runtime: AgentHarnessRuntime<Record<string, never>> = {
    harnessName: 'codex-steer-test',
    initialState: () => ({}),
    systemPrompt: () => 'system',
    tools: () => [],
  }
  const session = new AgentSession({ provider, model, cwd: process.cwd(), runtime }, { agentSessionId: 'agent-session-1' })

  const activeTurn = session.send([{ type: 'text', text: 'start' }])
  await transport.waitForRequest(0)
  const queuedTurn = session.send([{ type: 'text', text: 'queued next' }])
  await session.steer([{ type: 'text', text: 'steer current' }])

  transport.release(0)
  await transport.waitForRequest(1)

  expect(session.queuedMessages()).toMatchObject([{ text: 'queued next' }])
  const secondBody = JSON.stringify(transport.requests[1]?.body)
  expect(secondBody).toContain('first')
  expect(secondBody).toContain('steer current')
  expect(secondBody.indexOf('first')).toBeLessThan(secondBody.indexOf('steer current'))
  expect(secondBody).not.toContain('queued next')

  transport.release(1)
  await activeTurn
  await transport.waitForRequest(2)
  expect(JSON.stringify(transport.requests[2]?.body)).toContain('queued next')

  transport.release(2)
  await queuedTurn
  expect(session.transcript().blocks.map((block) => block.type)).toEqual([
    'user',
    'text',
    'response',
    'steer',
    'text',
    'response',
    'user',
    'text',
    'response',
  ])
})

function makeRequest(items: InferenceRequest['items'] = []): InferenceRequest {
  return {
    sessionId: 'session-1',
    turnId: 'turn-1',
    requestId: 'request-1',
    modelId: 'gpt-5.4',
    systemPrompt: 'system',
    cwd: '/tmp',
    items,
    tools: [{ name: 'shell_exec', description: 'Execute shell', inputSchema: { type: 'object' } }],
    thinking: { type: 'effort', effort: 'medium', summary: null },
    cancel: new AbortController().signal,
  }
}

function makeTransportRequest(): CodexTransportRequest {
  return {
    url: 'https://example.test/codex/responses',
    websocketUrl: 'wss://example.test/codex/responses',
    headers: new Headers(),
    body: {},
    signal: new AbortController().signal,
  }
}

class FakeCodexTransport implements CodexResponsesTransport {
  readonly requests: CodexTransportRequest[] = []
  private index = 0

  constructor(private readonly scripts: Array<CodexResponseStreamEvent[] | Error>) {}

  async *stream(request: CodexTransportRequest): AsyncIterable<CodexResponseStreamEvent> {
    this.requests.push(request)
    while (this.index < this.scripts.length) {
      const script = this.scripts[this.index]
      this.index += 1
      if (script instanceof Error) throw script
      for (const event of script) yield event
      if (script.length > 0) return
    }
  }
}

class GateCodexTransport implements CodexResponsesTransport {
  readonly requests: CodexTransportRequest[] = []
  private readonly gates: Array<Deferred<void>>
  private readonly started = new Map<number, Deferred<void>>()

  constructor(private readonly scripts: CodexResponseStreamEvent[][]) {
    this.gates = scripts.map(() => deferred<void>())
  }

  async *stream(request: CodexTransportRequest): AsyncIterable<CodexResponseStreamEvent> {
    const index = this.requests.length
    this.requests.push(request)
    this.started.get(index)?.resolve(undefined)
    await this.gates[index]?.promise
    for (const event of this.scripts[index] ?? []) yield event
  }

  waitForRequest(index: number): Promise<void> {
    if (this.requests.length > index) return Promise.resolve()
    const existing = this.started.get(index)
    if (existing) return existing.promise
    const next = deferred<void>()
    this.started.set(index, next)
    return next.promise
  }

  release(index: number): void {
    this.gates[index]?.resolve(undefined)
  }
}

class YieldThenThrowTransport implements CodexResponsesTransport {
  constructor(
    private readonly events: CodexResponseStreamEvent[],
    private readonly error: Error,
  ) {}

  async *stream(): AsyncIterable<CodexResponseStreamEvent> {
    for (const event of this.events) yield event
    throw this.error
  }
}

class RecordingAuthStore {
  readonly forceRefreshes: boolean[] = []
  private index = 0

  constructor(private readonly auths: CodexResolvedAuth[]) {}

  async status() {
    return { status: 'authenticated' as const }
  }

  async resolveAuth(options: { forceRefresh?: boolean } = {}): Promise<CodexResolvedAuth> {
    this.forceRefreshes.push(options.forceRefresh === true)
    const auth = this.auths[this.index] ?? this.auths[this.auths.length - 1]
    this.index += 1
    if (!auth) throw new Error('No fake auth left')
    return auth
  }
}

class CapturingWebSocket {
  static instances: CapturingWebSocket[] = []

  readonly headers: Record<string, string>
  private readonly listeners: Record<string, Array<(event: unknown) => void>> = {}

  constructor(
    readonly url: string,
    options: { headers?: Record<string, string> } = {},
  ) {
    this.headers = options.headers ?? {}
    CapturingWebSocket.instances.push(this)
    queueMicrotask(() => this.emit('open', {}))
  }

  send(): void {
    queueMicrotask(() => {
      this.emit('message', { data: JSON.stringify({ type: 'response.output_text.delta', delta: 'ws' }) })
      this.emit('close', {})
    })
  }

  close(): void {
    // Test double; closing is idempotent.
  }

  addEventListener(type: 'open' | 'message' | 'error' | 'close', listener: (event: never) => void): void {
    this.listeners[type] ??= []
    this.listeners[type].push(listener as (event: unknown) => void)
  }

  removeEventListener(type: 'open' | 'message' | 'error' | 'close', listener: (event: never) => void): void {
    this.listeners[type] = (this.listeners[type] ?? []).filter((candidate) => candidate !== (listener as (event: unknown) => void))
  }

  private emit(type: string, event: unknown): void {
    for (const listener of this.listeners[type] ?? []) listener(event)
  }
}

class NonClosingCompletedWebSocket {
  static instances: NonClosingCompletedWebSocket[] = []

  readonly closeReasons: string[] = []
  private readonly listeners: Record<string, Array<(event: unknown) => void>> = {}

  constructor() {
    NonClosingCompletedWebSocket.instances.push(this)
    queueMicrotask(() => this.emit('open', {}))
  }

  send(): void {
    queueMicrotask(() => {
      this.emit('message', { data: JSON.stringify({ type: 'response.output_text.delta', delta: 'ws' }) })
      this.emit('message', {
        data: JSON.stringify({
          type: 'response.completed',
          response: { usage: { input_tokens: 1, output_tokens: 1 } },
        }),
      })
    })
  }

  close(_code?: number, reason?: string): void {
    if (reason) this.closeReasons.push(reason)
  }

  addEventListener(type: 'open' | 'message' | 'error' | 'close', listener: (event: never) => void): void {
    this.listeners[type] ??= []
    this.listeners[type].push(listener as (event: unknown) => void)
  }

  removeEventListener(type: 'open' | 'message' | 'error' | 'close', listener: (event: never) => void): void {
    this.listeners[type] = (this.listeners[type] ?? []).filter((candidate) => candidate !== (listener as (event: unknown) => void))
  }

  private emit(type: string, event: unknown): void {
    for (const listener of this.listeners[type] ?? []) listener(event)
  }
}

interface Deferred<T> {
  promise: Promise<T>
  resolve: (value: T) => void
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((innerResolve) => {
    resolve = innerResolve
  })
  return { promise, resolve }
}
