import { expect, test } from 'bun:test'
import { providerRuntime, type InferenceRequest, type ProviderEvent, type ProviderSelection } from '@demi/provider'
import {
  buildAnthropicMessagesBody,
  createAnthropicApiProvider,
  mapAnthropicMessageStream,
  type ServerSentEvent,
} from '../provider'

test('Anthropic API provider resolves endpoint and API key from env vars', async () => {
  await withEnv(
    {
      ANTHROPIC_BASE_URL: 'https://anthropic-gateway.example/v1/',
      ANTHROPIC_API_KEY: 'env-anthropic-key',
    },
    async () => {
      const requests: CapturedRequest[] = []
      const provider = createAnthropicApiProvider({ fetch: captureFetch(requests) })
      const runtime = await providerRuntime(provider, selection('anthropic', 'claude-test'))

      const events = await collect(runtime.run(request({ modelId: 'claude-test' })))

      expect(events).toEqual([{ type: 'response', usage: zeroUsage() }])
      expect(requests[0]?.url).toBe('https://anthropic-gateway.example/v1/messages')
      expect(requests[0]?.headers.get('x-api-key')).toBe('env-anthropic-key')
      expect(requests[0]?.headers.get('anthropic-version')).toBe('2023-06-01')
    },
  )
})

test('Anthropic API provider explicit baseUrl and apiKey take precedence over env vars', async () => {
  await withEnv(
    {
      ANTHROPIC_GATEWAY_BASE_URL: 'https://env-gateway.example/anthropic/v1',
      ANTHROPIC_GATEWAY_API_KEY: 'env-key',
    },
    async () => {
      const requests: CapturedRequest[] = []
      const provider = createAnthropicApiProvider({
        id: 'anthropic-gateway',
        envPrefix: 'ANTHROPIC_GATEWAY',
        baseUrl: 'https://explicit-gateway.example/anthropic/v1',
        apiKey: () => 'explicit-key',
        headers: () => ({ 'x-extra': '1' }),
        anthropicVersion: '2026-01-01',
        fetch: captureFetch(requests),
      })
      const runtime = await providerRuntime(provider, selection('anthropic-gateway', 'claude-gateway'))

      await collect(runtime.run(request({ modelId: 'claude-gateway' })))

      expect(requests[0]?.url).toBe('https://explicit-gateway.example/anthropic/v1/messages')
      expect(requests[0]?.headers.get('x-api-key')).toBe('explicit-key')
      expect(requests[0]?.headers.get('x-extra')).toBe('1')
      expect(requests[0]?.headers.get('anthropic-version')).toBe('2026-01-01')
    },
  )
})

test('Anthropic API request body groups user/tool_result and assistant/tool_use turns', () => {
  const body = buildAnthropicMessagesBody(
    request({
      systemPrompt: 'system instructions',
      serviceTierId: 'standard_only',
      thinking: { type: 'budget', budgetTokens: 1024 },
      items: [
        { type: 'user_message', content: [{ type: 'text', text: 'hello' }] },
        { type: 'assistant_text', modelId: 'claude-test', text: 'Use tool' },
        { type: 'tool_use', modelId: 'claude-test', toolUseId: 'toolu-1', toolName: 'read_file', input: { path: 'a.ts' } },
        { type: 'tool_result', toolUseId: 'toolu-1', output: [{ type: 'text', text: 'contents' }], isError: false },
      ],
      tools: [
        {
          name: 'read_file',
          description: 'Read a file',
          inputSchema: { type: 'object', properties: { path: { type: 'string' } } },
        },
      ],
    }),
    { maxTokens: 8192 },
  )

  expect(body).toMatchObject({
    model: 'claude-test',
    max_tokens: 8192,
    stream: true,
    system: 'system instructions',
    service_tier: 'standard_only',
    thinking: { type: 'enabled', budget_tokens: 1024 },
  })
  expect(body.messages).toEqual([
    { role: 'user', content: [{ type: 'text', text: 'hello' }] },
    {
      role: 'assistant',
      content: [
        { type: 'text', text: 'Use tool' },
        { type: 'tool_use', id: 'toolu-1', name: 'read_file', input: { path: 'a.ts' } },
      ],
    },
    {
      role: 'user',
      content: [{ type: 'tool_result', tool_use_id: 'toolu-1', content: [{ type: 'text', text: 'contents' }] }],
    },
  ])
  expect(body.tools).toEqual([
    {
      name: 'read_file',
      description: 'Read a file',
      input_schema: { type: 'object', properties: { path: { type: 'string' } } },
    },
  ])
})

test('Anthropic API stream maps thinking, text, tool use, and usage', async () => {
  const events = await collect(mapAnthropicMessageStream(eventsFromData([
    {
      event: 'message_start',
      data: {
        type: 'message_start',
        message: { usage: { input_tokens: 12, cache_read_input_tokens: 2, cache_creation_input_tokens: 1 } },
      },
    },
    { event: 'content_block_start', data: { type: 'content_block_start', index: 0, content_block: { type: 'thinking' } } },
    { event: 'content_block_delta', data: { type: 'content_block_delta', index: 0, delta: { type: 'thinking_delta', thinking: 'plan' } } },
    { event: 'content_block_delta', data: { type: 'content_block_delta', index: 0, delta: { type: 'signature_delta', signature: 'sig' } } },
    { event: 'content_block_start', data: { type: 'content_block_start', index: 1, content_block: { type: 'text', text: '' } } },
    { event: 'content_block_delta', data: { type: 'content_block_delta', index: 1, delta: { type: 'text_delta', text: 'hello' } } },
    {
      event: 'content_block_start',
      data: { type: 'content_block_start', index: 2, content_block: { type: 'tool_use', id: 'toolu-1', name: 'read_file', input: {} } },
    },
    {
      event: 'content_block_delta',
      data: { type: 'content_block_delta', index: 2, delta: { type: 'input_json_delta', partial_json: '{"path":"a.ts"}' } },
    },
    { event: 'content_block_stop', data: { type: 'content_block_stop', index: 2 } },
    { event: 'message_delta', data: { type: 'message_delta', usage: { output_tokens: 5 } } },
    { event: 'message_stop', data: { type: 'message_stop' } },
  ])))

  expect(events).toEqual([
    { type: 'thinking_start' },
    { type: 'thinking_delta', text: 'plan' },
    { type: 'thinking_signature', signature: 'sig' },
    { type: 'text_delta', text: 'hello' },
    { type: 'tool_call_requested', toolUseId: 'toolu-1', toolName: 'read_file', input: { path: 'a.ts' } },
    { type: 'response', usage: { inputTokens: 12, outputTokens: 5, cacheReadTokens: 2, cacheWriteTokens: 1 } },
  ])
})

interface CapturedRequest {
  url: string
  headers: Headers
  body: unknown
}

function captureFetch(requests: CapturedRequest[]) {
  return async (input: string | URL | Request, init?: RequestInit): Promise<Response> => {
    requests.push({
      url: String(input),
      headers: new Headers(init?.headers),
      body: init?.body ? JSON.parse(String(init.body)) : null,
    })
    return new Response('event: message_stop\ndata: {"type":"message_stop"}\n\n', { status: 200 })
  }
}

function request(overrides: Partial<InferenceRequest> = {}): InferenceRequest {
  const controller = new AbortController()
  return {
    sessionId: 'session-1',
    turnId: 'turn-1',
    requestId: 'request-1',
    modelId: 'claude-test',
    systemPrompt: '',
    cwd: '/workspace',
    items: [{ type: 'user_message', content: [{ type: 'text', text: 'hello' }] }],
    tools: [],
    thinking: null,
    serviceTierId: null,
    cancel: controller.signal,
    ...overrides,
  }
}

function selection(providerId: string, modelId: string): ProviderSelection {
  return {
    providerId,
    model: {
      providerId,
      model: { id: modelId, name: modelId, contextWindow: 0, inputLimit: null, thinking: [], acceptedExtensions: [] },
      thinking: null,
      serviceTierId: null,
    },
  }
}

async function collect(iterable: AsyncIterable<ProviderEvent>): Promise<ProviderEvent[]> {
  const events: ProviderEvent[] = []
  for await (const event of iterable) events.push(event)
  return events
}

async function* eventsFromData(values: Array<{ event: string; data: Record<string, unknown> }>): AsyncIterable<ServerSentEvent> {
  for (const value of values) yield { event: value.event, data: [JSON.stringify(value.data)] }
}

async function withEnv(env: Record<string, string>, run: () => Promise<void>): Promise<void> {
  const previous = new Map<string, string | undefined>()
  for (const [key, value] of Object.entries(env)) {
    previous.set(key, process.env[key])
    process.env[key] = value
  }
  try {
    await run()
  } finally {
    for (const [key, value] of previous) {
      if (value === undefined) delete process.env[key]
      else process.env[key] = value
    }
  }
}

function zeroUsage() {
  return { inputTokens: 0, outputTokens: 0, cacheReadTokens: 0, cacheWriteTokens: 0 }
}
