import { expect, test } from 'bun:test'
import { zeroUsage } from '@demicodes/core'
import { providerRuntime, type InferenceRequest, type ProviderEvent, type ProviderSelection } from '@demicodes/provider'
import { StaticGrokAuthStore, type GrokResolvedAuth } from '../auth'
import { buildGrokChatCompletionsBody, mapGrokChatCompletionStream, type ServerSentEvent } from '../chat'
import { modelListFromGrokModelsPayload } from '../models'
import { createGrokBuildProvider } from '../provider'

const staticAuth: GrokResolvedAuth = {
  accessToken: 'session-token',
  refreshToken: 'refresh',
  expiresAt: new Date('2030-01-01T00:00:00.000Z'),
  email: 'user@example.com',
  issuer: 'https://auth.x.ai',
  clientId: 'client-1',
  entryKey: 'https://auth.x.ai::client-1',
  authFile: '/tmp/auth.json',
}

test('Grok Build provider posts chat completions with CLI session headers', async () => {
  const requests: CapturedRequest[] = []
  const provider = createGrokBuildProvider({
    authStore: new StaticGrokAuthStore(staticAuth),
    baseUrl: 'https://cli-chat-proxy.example/v1',
    fetch: captureFetch(requests, sseResponse(['data: {"choices":[{"delta":{"content":"hi"}}]}\n\n', 'data: [DONE]\n\n'])),
  })
  const runtime = await providerRuntime(provider, selection('grok-build', 'grok-4.5'))
  const events = await collect(runtime.run(request({ modelId: 'grok-4.5' })))

  expect(events).toEqual([
    { type: 'text_delta', text: 'hi' },
    { type: 'response', usage: zeroUsage() },
  ])
  expect(requests[0]?.url).toBe('https://cli-chat-proxy.example/v1/chat/completions')
  expect(requests[0]?.headers.get('authorization')).toBe('Bearer session-token')
  expect(requests[0]?.headers.get('X-XAI-Token-Auth')).toBe('xai-grok-cli')
  expect(requests[0]?.headers.get('x-grok-client-surface')).toBe('grok-build')
  expect(requests[0]?.headers.get('x-grok-model-override')).toBe('grok-4.5')
  expect(requests[0]?.headers.get('x-grok-session-id')).toBe('session-1')
})

test('Grok Build provider refreshes once on HTTP 401 then retries', async () => {
  const requests: CapturedRequest[] = []
  let resolveCalls = 0
  const authStore = {
    async status() {
      return { status: 'authenticated' as const, accountLabel: 'user@example.com' }
    },
    async resolveAuth(options?: { forceRefresh?: boolean }) {
      resolveCalls += 1
      if (options?.forceRefresh) {
        return { ...staticAuth, accessToken: 'refreshed-token' }
      }
      return staticAuth
    },
  }
  const provider = createGrokBuildProvider({
    authStore,
    fetch: async (input, init) => {
      const url = String(input)
      const headers = new Headers(init?.headers)
      requests.push({ url, headers, body: typeof init?.body === 'string' ? init.body : null })
      if (headers.get('authorization') === 'Bearer session-token') {
        return new Response('unauthorized', { status: 401 })
      }
      return sseResponse(['data: {"choices":[{"delta":{"content":"ok"}}]}\n\n', 'data: [DONE]\n\n'])()
    },
  })
  const runtime = await providerRuntime(provider, selection('grok-build', 'grok-4.5'))
  const events = await collect(runtime.run(request({ modelId: 'grok-4.5' })))

  expect(resolveCalls).toBe(2)
  expect(requests).toHaveLength(2)
  expect(requests[1]?.headers.get('authorization')).toBe('Bearer refreshed-token')
  expect(events).toEqual([
    { type: 'text_delta', text: 'ok' },
    { type: 'response', usage: zeroUsage() },
  ])
})

test('chat body maps tools, tool replay, and reasoning effort', () => {
  const body = buildGrokChatCompletionsBody(
    request({
      systemPrompt: 'system',
      thinking: { type: 'effort', effort: 'high', summary: null },
      items: [
        { type: 'user_message', content: [{ type: 'text', text: 'hello' }] },
        { type: 'assistant_text', modelId: 'grok-4.5', text: 'Use tool' },
        { type: 'tool_use', modelId: 'grok-4.5', toolUseId: 'call-1', toolName: 'read_file', input: { path: 'a.ts' } },
        { type: 'tool_result', toolUseId: 'call-1', output: [{ type: 'text', text: 'contents' }], isError: false },
      ],
      tools: [
        {
          name: 'read_file',
          description: 'Read a file',
          inputSchema: { type: 'object', properties: { path: { type: 'string' } } },
        },
      ],
    }),
  )

  expect(body).toMatchObject({
    model: 'gpt-test',
    stream: true,
    tool_choice: 'auto',
    reasoning_effort: 'high',
  })
  expect(body.messages[0]).toEqual({ role: 'system', content: 'system' })
  expect(body.messages[1]).toEqual({ role: 'user', content: 'hello' })
  expect(body.messages[2]).toMatchObject({
    role: 'assistant',
    content: 'Use tool',
    tool_calls: [{ id: 'call-1', type: 'function', function: { name: 'read_file', arguments: '{"path":"a.ts"}' } }],
  })
  expect(body.messages[3]).toEqual({ role: 'tool', tool_call_id: 'call-1', content: 'contents' })
})

test('chat stream maps reasoning_content and tool calls', async () => {
  const events = await collect(
    mapGrokChatCompletionStream(
      (async function* (): AsyncIterable<ServerSentEvent> {
        yield {
          event: null,
          data: [
            JSON.stringify({
              choices: [{ delta: { reasoning_content: 'think', role: 'assistant' } }],
            }),
          ],
        }
        yield {
          event: null,
          data: [
            JSON.stringify({
              choices: [
                {
                  delta: {
                    tool_calls: [{ index: 0, id: 'c1', function: { name: 'shell_exec', arguments: '{"cmd"' } }],
                  },
                },
              ],
            }),
          ],
        }
        yield {
          event: null,
          data: [
            JSON.stringify({
              choices: [
                {
                  delta: { tool_calls: [{ index: 0, function: { arguments: ':"ls"}' } }] },
                  finish_reason: 'tool_calls',
                },
              ],
            }),
          ],
        }
        yield { event: null, data: ['[DONE]'] }
      })(),
    ),
  )

  expect(events).toEqual([
    { type: 'thinking_start' },
    { type: 'thinking_delta', text: 'think' },
    { type: 'tool_call_requested', toolUseId: 'c1', toolName: 'shell_exec', input: { cmd: 'ls' } },
    { type: 'response', usage: zeroUsage() },
  ])
})

test('model catalog maps Grok /v1/models payload', () => {
  const catalog = modelListFromGrokModelsPayload(
    {
      object: 'list',
      data: [
        {
          id: 'grok-4.5',
          name: 'Grok 4.5',
          description: 'frontier',
          context_window: 500000,
          supports_reasoning_effort: true,
          reasoning_effort: 'high',
          reasoning_efforts: [
            { id: 'high', value: 'high', default: true },
            { id: 'medium', value: 'medium', default: false },
            { id: 'low', value: 'low', default: false },
          ],
        },
        {
          id: 'grok-composer-2.5-fast',
          name: 'Composer 2.5',
          context_window: 200000,
        },
      ],
    },
    'grok-build',
  )

  expect(catalog.defaultModelId).toBe('grok-4.5')
  expect(catalog.models.map((model) => model.id)).toEqual(['grok-4.5', 'grok-composer-2.5-fast'])
  expect(catalog.models[0]).toMatchObject({
    displayName: 'Grok 4.5',
    contextWindow: 500_000,
    supportedThinkingEfforts: ['high', 'medium', 'low'],
    defaultThinkingEffort: 'high',
  })
})

interface CapturedRequest {
  url: string
  headers: Headers
  body: string | null
}

function captureFetch(
  requests: CapturedRequest[],
  responseFactory: () => Response,
): (input: string | URL | Request, init?: RequestInit) => Promise<Response> {
  return async (input, init) => {
    requests.push({
      url: String(input),
      headers: new Headers(init?.headers),
      body: typeof init?.body === 'string' ? init.body : null,
    })
    return responseFactory()
  }
}

function sseResponse(chunks: string[]): () => Response {
  return () =>
    new Response(
      new ReadableStream({
        start(controller) {
          const encoder = new TextEncoder()
          for (const chunk of chunks) controller.enqueue(encoder.encode(chunk))
          controller.close()
        },
      }),
      { status: 200, headers: { 'content-type': 'text/event-stream' } },
    )
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

function request(overrides: Partial<InferenceRequest> = {}): InferenceRequest {
  return {
    sessionId: 'session-1',
    turnId: 'turn-1',
    requestId: 'req-1',
    modelId: 'gpt-test',
    systemPrompt: '',
    cwd: '/workspace',
    items: [{ type: 'user_message', content: [{ type: 'text', text: 'hi' }] }],
    tools: [],
    thinking: null,
    serviceTierId: null,
    cancel: new AbortController().signal,
    ...overrides,
  }
}

async function collect(events: AsyncIterable<ProviderEvent>): Promise<ProviderEvent[]> {
  const out: ProviderEvent[] = []
  for await (const event of events) out.push(event)
  return out
}
