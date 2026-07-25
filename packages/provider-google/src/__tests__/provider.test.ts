import { Buffer } from 'node:buffer'
import { expect, test } from 'bun:test'
import { providerRuntime, type InferenceRequest, type ProviderEvent, type ProviderSelection } from '@demicodes/provider'
import {
  buildGoogleGenerateContentBody,
  createGoogleProvider,
  inferenceItemsToGoogleContents,
  mapGoogleContentStream,
  type ServerSentEvent,
} from '../provider'

test('Google provider resolves endpoint and API key from env vars', async () => {
  await withEnv(
    { GOOGLE_BASE_URL: 'https://gemini-gateway.example/v1beta/', GOOGLE_API_KEY: 'env-google-key' },
    async () => {
      const requests: CapturedRequest[] = []
      const provider = createGoogleProvider({ fetch: captureFetch(requests) })
      const runtime = await providerRuntime(provider, selection('google', 'gemini-3.6-flash'))

      const events = await collect(runtime.run(request({ modelId: 'gemini-3.6-flash' })))

      expect(events).toEqual([{ type: 'response', usage: zeroUsage() }])
      expect(requests[0]?.url).toBe(
        'https://gemini-gateway.example/v1beta/models/gemini-3.6-flash:streamGenerateContent?alt=sse',
      )
      expect(requests[0]?.headers.get('x-goog-api-key')).toBe('env-google-key')
    },
  )
})

test('system prompt, tools and thinking budget land in the generateContent body', () => {
  const base = request({
    systemPrompt: 'you are a shell',
    items: [{ type: 'user_message', content: [{ type: 'text', text: 'hi' }] }],
    tools: [{ name: 'shell_exec', description: 'run', inputSchema: { type: 'object' } }],
  })

  const body = buildGoogleGenerateContentBody({ ...base, thinking: { type: 'effort', effort: 'high', summary: null } }, undefined)
  expect(body.systemInstruction).toEqual({ parts: [{ text: 'you are a shell' }] })
  expect(body.tools).toEqual([
    { functionDeclarations: [{ name: 'shell_exec', description: 'run', parameters: { type: 'object' } }] },
  ])
  expect(body.generationConfig?.thinkingConfig).toEqual({ includeThoughts: true, thinkingBudget: 32_768 })

  const custom = buildGoogleGenerateContentBody(
    { ...base, thinking: { type: 'effort', effort: 'high', summary: null } },
    { effortBudgetTokens: { high: 2_048 } },
  )
  expect(custom.generationConfig?.thinkingConfig?.thinkingBudget).toBe(2_048)

  const disabled = buildGoogleGenerateContentBody({ ...base, thinking: { type: 'disabled' } }, undefined)
  expect(disabled.generationConfig?.thinkingConfig).toEqual({ includeThoughts: false, thinkingBudget: 0 })
})

test('a tool call replays with the thought signature parked on the thinking item in front of it', () => {
  // Gemini rejects the whole request when a replayed functionCall has no
  // thoughtSignature, so this pairing is what keeps a multi-turn agent alive.
  const contents = inferenceItemsToGoogleContents([
    { type: 'user_message', content: [{ type: 'text', text: 'list files' }] },
    { type: 'assistant_thinking', modelId: 'gemini', text: '', signature: 'sig-abc' },
    { type: 'tool_use', modelId: 'gemini', toolUseId: 'call-1', toolName: 'shell_exec', input: { command: 'ls' } },
    { type: 'tool_result', toolUseId: 'call-1', output: [{ type: 'text', text: 'a.md' }], isError: false },
  ])

  expect(contents).toEqual([
    { role: 'user', parts: [{ text: 'list files' }] },
    {
      role: 'model',
      parts: [
        {
          functionCall: { name: 'shell_exec', args: { command: 'ls' }, id: 'call-1' },
          thoughtSignature: 'sig-abc',
        },
      ],
    },
    {
      role: 'user',
      parts: [{ functionResponse: { name: 'shell_exec', id: 'call-1', response: { output: 'a.md' } } }],
    },
  ])
})

test('video rides as a native inline part and tool-returned media follows the function response', () => {
  const contents = inferenceItemsToGoogleContents([
    {
      type: 'user_message',
      content: [
        { type: 'text', text: 'watch this' },
        { type: 'video', source: { type: 'binary', mediaType: 'video/mp4', data: new TextEncoder().encode('VID') } },
      ],
    },
    { type: 'tool_use', modelId: 'gemini', toolUseId: 'call-2', toolName: 'look', input: {} },
    {
      type: 'tool_result',
      toolUseId: 'call-2',
      output: [
        { type: 'text', text: 'rendered' },
        { type: 'image', source: { mediaType: 'image/png', data: 'UE5H' } },
      ],
      isError: false,
    },
  ])

  expect(contents[0]).toEqual({
    role: 'user',
    parts: [{ text: 'watch this' }, { inlineData: { mimeType: 'video/mp4', data: Buffer.from('VID').toString('base64') } }],
  })
  // functionResponse carries JSON only, so the picture has to travel beside it.
  expect(contents[2]).toEqual({
    role: 'user',
    parts: [
      { functionResponse: { name: 'look', id: 'call-2', response: { output: 'rendered' } } },
      { inlineData: { mimeType: 'image/png', data: 'UE5H' } },
    ],
  })
})

test('thought parts stream as thinking and a function call emits its signature first', async () => {
  const events = await collect(
    mapGoogleContentStream(
      streamOf([
        { candidates: [{ content: { parts: [{ text: 'weighing options', thought: true }] } }] },
        {
          candidates: [
            { content: { parts: [{ functionCall: { name: 'shell_exec', args: { command: 'ls' }, id: 'c1' }, thoughtSignature: 'sig-1' }] } },
          ],
        },
        { candidates: [{ content: { parts: [{ text: 'done' }] } }] },
        {
          usageMetadata: {
            promptTokenCount: 10,
            candidatesTokenCount: 4,
            thoughtsTokenCount: 20,
            cachedContentTokenCount: 3,
          },
        },
      ]),
    ),
  )

  expect(events).toEqual([
    { type: 'thinking_start' },
    { type: 'thinking_delta', text: 'weighing options' },
    { type: 'thinking_signature', signature: 'sig-1' },
    { type: 'tool_call_requested', toolUseId: 'c1', toolName: 'shell_exec', input: { command: 'ls' } },
    { type: 'text_delta', text: 'done' },
    // Thinking is billed apart from the answer; both are output tokens.
    { type: 'response', usage: { inputTokens: 10, outputTokens: 24, cacheReadTokens: 3, cacheWriteTokens: 0 } },
  ])
})

test('a stream error surfaces as an error event instead of an empty response', async () => {
  const events = await collect(
    mapGoogleContentStream(streamOf([{ error: { status: 'RESOURCE_EXHAUSTED', message: 'quota exceeded' } }])),
  )
  expect(events[0]).toMatchObject({ type: 'error', message: 'quota exceeded' })
})

// ── helpers ─────────────────────────────────────────────────────────

interface CapturedRequest {
  url: string
  headers: Headers
  body: string
}

function captureFetch(requests: CapturedRequest[]) {
  return async (input: string | URL | Request, init?: RequestInit): Promise<Response> => {
    requests.push({
      url: String(input),
      headers: new Headers(init?.headers),
      body: typeof init?.body === 'string' ? init.body : '',
    })
    return new Response('', { status: 200, headers: { 'content-type': 'text/event-stream' } })
  }
}

function request(overrides: Partial<InferenceRequest> = {}): InferenceRequest {
  return {
    sessionId: 'session',
    turnId: 'turn',
    requestId: 'request',
    modelId: 'gemini-3.6-flash',
    systemPrompt: '',
    cwd: '/tmp',
    items: [],
    tools: [],
    thinking: null,
    cancel: new AbortController().signal,
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

async function* streamOf(values: Array<Record<string, unknown>>): AsyncIterable<ServerSentEvent> {
  for (const value of values) yield { event: null, data: [JSON.stringify(value)] }
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
