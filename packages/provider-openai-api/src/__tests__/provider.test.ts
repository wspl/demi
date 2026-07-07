import { expect, test } from 'bun:test'
import { providerRuntime, type InferenceRequest, type ProviderEvent, type ProviderSelection } from '@demicodes/provider'
import {
  buildOpenAIChatCompletionsBody,
  buildOpenAIResponsesBody,
  createOpenAIApiProvider,
  mapOpenAIChatCompletionStream,
  mapOpenAIResponseStream,
  type ServerSentEvent,
} from '../provider'

test('OpenAI API provider resolves Responses endpoint and API key from env vars', async () => {
  await withEnv(
    {
      OPENAI_BASE_URL: 'https://openai-gateway.example/v1/',
      OPENAI_API_KEY: 'env-openai-key',
    },
    async () => {
      const requests: CapturedRequest[] = []
      const provider = createOpenAIApiProvider({ fetch: captureFetch(requests) })
      const runtime = await providerRuntime(provider, selection('openai', 'gpt-test'))

      const events = await collect(runtime.run(request({ modelId: 'gpt-test' })))

      expect(events).toEqual([{ type: 'response', usage: zeroUsage() }])
      expect(requests[0]?.url).toBe('https://openai-gateway.example/v1/responses')
      expect(requests[0]?.headers.get('authorization')).toBe('Bearer env-openai-key')
    },
  )
})

test('OpenAI API provider can use Chat Completions wire API with explicit endpoint options', async () => {
  await withEnv(
    {
      ROUTER_BASE_URL: 'https://env-router.example/api/v1',
      ROUTER_API_KEY: 'env-key',
    },
    async () => {
      const requests: CapturedRequest[] = []
      const provider = createOpenAIApiProvider({
        id: 'router',
        wireApi: 'chat-completions',
        envPrefix: 'ROUTER',
        baseUrl: 'https://explicit-router.example/openai/v1',
        apiKey: () => 'explicit-key',
        headers: () => ({ 'x-router': '1' }),
        fetch: captureFetch(requests),
      })
      const runtime = await providerRuntime(provider, selection('router', 'router-model'))

      await collect(runtime.run(request({ modelId: 'router-model' })))

      expect(requests[0]?.url).toBe('https://explicit-router.example/openai/v1/chat/completions')
      expect(requests[0]?.headers.get('authorization')).toBe('Bearer explicit-key')
      expect(requests[0]?.headers.get('x-router')).toBe('1')
    },
  )
})

test('OpenAI API model catalog mirrors Codex-visible defaults and explicit models replace it', async () => {
  const defaults = await createOpenAIApiProvider().listModels?.()

  expect(defaults?.defaultModelId).toBe('gpt-5.5')
  expect(defaults?.models.map((model) => model.id)).toEqual([
    'gpt-5.5',
    'gpt-5.4',
    'gpt-5.4-mini',
    'gpt-5.3-codex-spark',
  ])
  expect(defaults?.models[0]).toMatchObject({
    displayName: 'GPT-5.5',
    contextWindow: 272_000,
    supportedThinkingEfforts: ['low', 'medium', 'high', 'xhigh'],
    serviceTiers: [{ id: 'priority', label: 'Fast', description: '1.5x speed, increased usage' }],
  })

  const custom = await createOpenAIApiProvider({
    models: [
      {
        id: 'deepseek-v4-pro',
        displayName: 'DeepSeek V4 Pro',
        contextWindow: 1_000_000,
        supportsReasoning: true,
        supportedThinkingEfforts: ['low', 'medium', 'high', 'xhigh', 'max'],
        defaultThinkingEffort: 'medium',
        canDisableThinking: false,
      },
    ],
    defaultModelId: 'deepseek-v4-pro',
  }).listModels?.()

  expect(custom?.defaultModelId).toBe('deepseek-v4-pro')
  expect(custom?.models.map((model) => model.id)).toEqual(['deepseek-v4-pro'])
  expect(custom?.models[0]).toMatchObject({
    displayName: 'DeepSeek V4 Pro',
    contextWindow: 1_000_000,
    supportsReasoning: true,
    supportedThinkingEfforts: ['low', 'medium', 'high', 'xhigh', 'max'],
    defaultThinkingEffort: 'medium',
    canDisableThinking: false,
  })
})

test('OpenAI Responses request body maps text, tools, tool replay, service tier, and reasoning', () => {
  const body = buildOpenAIResponsesBody(
    request({
      systemPrompt: 'system instructions',
      serviceTierId: 'priority',
      thinking: { type: 'effort', effort: 'high', summary: null },
      items: [
        { type: 'user_message', content: [{ type: 'text', text: 'hello' }] },
        { type: 'assistant_text', modelId: 'gpt-test', text: 'Use tool' },
        { type: 'tool_use', modelId: 'gpt-test', toolUseId: 'call-1|fc-1', toolName: 'read_file', input: { path: 'a.ts' } },
        { type: 'tool_result', toolUseId: 'call-1|fc-1', output: [{ type: 'text', text: 'contents' }], isError: false },
      ],
      tools: [
        {
          name: 'read_file',
          description: 'Read a file',
          inputSchema: { type: 'object', properties: { path: { type: 'string' } } },
        },
      ],
    }),
    undefined,
  )

  expect(body).toMatchObject({
    model: 'gpt-test',
    instructions: 'system instructions',
    stream: true,
    store: false,
    tool_choice: 'auto',
    parallel_tool_calls: true,
    service_tier: 'priority',
    reasoning: { effort: 'high', summary: 'auto' },
    include: ['reasoning.encrypted_content'],
    prompt_cache_key: 'session-1',
  })
  expect(body.input[0]).toEqual({ role: 'user', content: [{ type: 'input_text', text: 'hello' }] })
  // 回放的 assistant 消息用最小 item 形状——不带 id/status，严格校验的网关会拒绝未知参数
  expect(body.input[1]).toEqual({
    type: 'message',
    role: 'assistant',
    content: [{ type: 'output_text', text: 'Use tool', annotations: [] }],
  })
  expect(body.input[2]).toEqual({
    type: 'function_call',
    id: 'fc-1',
    call_id: 'call-1',
    name: 'read_file',
    arguments: '{"path":"a.ts"}',
  })
  expect(body.input[3]).toEqual({ type: 'function_call_output', call_id: 'call-1', output: 'contents' })
  expect(body.tools).toEqual([
    {
      type: 'function',
      name: 'read_file',
      description: 'Read a file',
      parameters: { type: 'object', properties: { path: { type: 'string' } } },
    },
  ])
  expect(body.stream_options).toBeUndefined()
})

test('OpenAI Responses stream maps thinking, split text, tool call arguments, and usage', async () => {
  const reasoning = { type: 'reasoning' as const, id: 'rs-1', encrypted_content: 'enc' }
  const events = await collect(mapOpenAIResponseStream(eventsFromData([
    { type: 'response.output_item.added', item: { type: 'reasoning', id: 'rs-1' } },
    { type: 'response.reasoning_text.delta', delta: 'think' },
    { type: 'response.output_item.done', item: reasoning },
    { type: 'response.output_text.delta', delta: 'hi ' },
    { type: 'response.output_text.delta', delta: 'there' },
    { type: 'response.output_item.added', item: { type: 'function_call', id: 'fc-1', call_id: 'call-1', name: 'read_file', arguments: '' } },
    { type: 'response.function_call_arguments.delta', item_id: 'fc-1', delta: '{"path"' },
    { type: 'response.function_call_arguments.done', item_id: 'fc-1', arguments: '{"path":"a.ts"}' },
    { type: 'response.output_item.done', item: { type: 'function_call', id: 'fc-1', call_id: 'call-1', name: 'read_file' } },
    {
      type: 'response.completed',
      response: {
        usage: {
          input_tokens: 12,
          output_tokens: 5,
          input_tokens_details: { cached_tokens: 2 },
        },
      },
    },
  ])))

  expect(events).toEqual([
    { type: 'thinking_start' },
    { type: 'thinking_delta', text: 'think' },
    { type: 'thinking_signature', signature: JSON.stringify(reasoning) },
    { type: 'text_delta', text: 'hi ' },
    { type: 'text_delta', text: 'there' },
    { type: 'tool_call_requested', toolUseId: 'call-1|fc-1', toolName: 'read_file', input: { path: 'a.ts' } },
    { type: 'response', usage: { inputTokens: 10, outputTokens: 5, cacheReadTokens: 2, cacheWriteTokens: 0 } },
  ])
})

test('OpenAI Chat Completions request body maps text, tools, tool replay, service tier, and reasoning effort', () => {
  const body = buildOpenAIChatCompletionsBody(
    request({
      systemPrompt: 'system instructions',
      serviceTierId: 'priority',
      thinking: { type: 'effort', effort: 'high', summary: null },
      items: [
        { type: 'user_message', content: [{ type: 'text', text: 'hello' }] },
        { type: 'assistant_text', modelId: 'gpt-test', text: 'Use tool' },
        { type: 'tool_use', modelId: 'gpt-test', toolUseId: 'call-1', toolName: 'read_file', input: { path: 'a.ts' } },
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
    undefined,
  )

  expect(body).toMatchObject({
    model: 'gpt-test',
    stream: true,
    tool_choice: 'auto',
    service_tier: 'priority',
    reasoning_effort: 'high',
  })
  expect(body.messages).toEqual([
    { role: 'system', content: 'system instructions' },
    { role: 'user', content: 'hello' },
    {
      role: 'assistant',
      content: 'Use tool',
      tool_calls: [
        {
          id: 'call-1',
          type: 'function',
          function: { name: 'read_file', arguments: '{"path":"a.ts"}' },
        },
      ],
    },
    { role: 'tool', tool_call_id: 'call-1', content: 'contents' },
  ])
  expect(body.tools).toEqual([
    {
      type: 'function',
      function: {
        name: 'read_file',
        description: 'Read a file',
        parameters: { type: 'object', properties: { path: { type: 'string' } } },
      },
    },
  ])
  expect(body.stream_options).toEqual({ include_usage: true })
})

test('OpenAI Chat Completions stream maps split text, tool call arguments, and usage', async () => {
  const events = await collect(mapOpenAIChatCompletionStream(eventsFromData([
    {
      choices: [
        {
          delta: {
            content: 'hi ',
            tool_calls: [
              { index: 0, id: 'call-1', function: { name: 'read_file', arguments: '{"path"' } },
            ],
          },
        },
      ],
    },
    { choices: [{ delta: { content: 'there', tool_calls: [{ index: 0, function: { arguments: ':"a.ts"}' } }] } }] },
    { choices: [{ delta: {}, finish_reason: 'tool_calls' }] },
    {
      choices: [],
      usage: {
        prompt_tokens: 12,
        completion_tokens: 5,
        prompt_tokens_details: { cached_tokens: 2 },
      },
    },
    '[DONE]',
  ])))

  expect(events).toEqual([
    { type: 'text_delta', text: 'hi ' },
    { type: 'text_delta', text: 'there' },
    { type: 'tool_call_requested', toolUseId: 'call-1', toolName: 'read_file', input: { path: 'a.ts' } },
    { type: 'response', usage: { inputTokens: 10, outputTokens: 5, cacheReadTokens: 2, cacheWriteTokens: 0 } },
  ])
})

test('OpenAI Chat Completions stream maps compatible reasoning content', async () => {
  const events = await collect(mapOpenAIChatCompletionStream(eventsFromData([
    { choices: [{ delta: { role: 'assistant', content: null, reasoning_content: '' } }] },
    { choices: [{ delta: { content: null, reasoning_content: 'think ' } }] },
    { choices: [{ delta: { content: null, reasoning_content: 'more' } }] },
    { choices: [{ delta: { content: 'answer' } }] },
    '[DONE]',
  ])))

  expect(events).toEqual([
    { type: 'thinking_start' },
    { type: 'thinking_delta', text: 'think ' },
    { type: 'thinking_delta', text: 'more' },
    { type: 'text_delta', text: 'answer' },
    { type: 'response', usage: zeroUsage() },
  ])
})

test('OpenAI Chat Completions stream preserves malformed tool arguments as a string', async () => {
  const events = await collect(mapOpenAIChatCompletionStream(eventsFromData([
    { choices: [{ delta: { tool_calls: [{ index: 0, id: 'call-1', function: { name: 'bad', arguments: '{' } }] } }] },
    { choices: [{ delta: {}, finish_reason: 'tool_calls' }] },
    '[DONE]',
  ])))

  expect(events[0]).toEqual({ type: 'tool_call_requested', toolUseId: 'call-1', toolName: 'bad', input: '{' })
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
    return new Response('data: [DONE]\n\n', { status: 200 })
  }
}

function request(overrides: Partial<InferenceRequest> = {}): InferenceRequest {
  const controller = new AbortController()
  return {
    sessionId: 'session-1',
    turnId: 'turn-1',
    requestId: 'request-1',
    modelId: 'gpt-test',
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

async function* eventsFromData(values: Array<Record<string, unknown> | string>): AsyncIterable<ServerSentEvent> {
  for (const value of values) {
    yield { event: null, data: [typeof value === 'string' ? value : JSON.stringify(value)] }
  }
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
