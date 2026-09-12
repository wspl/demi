import { expect, test } from 'bun:test'
import { ProviderDataError, readServerSentEvents, type ProviderEvent, type ServerSentEvent, type InferenceRequest } from '@demicodes/provider'
import { buildOpenAIResponsesBody, mapOpenAIResponseStream } from '../provider'

const completed = { type: 'response.completed', response: {} }
const call = { type: 'function_call', id: 'item', call_id: 'call', name: 'tool', arguments: '{"value":1}' }

async function* frames(values: unknown[]): AsyncIterable<ServerSentEvent> {
  for (const value of values)
    yield { event: null, data: typeof value === 'string' ? value : JSON.stringify(value) }
}

async function collect(events: AsyncIterable<ProviderEvent>): Promise<ProviderEvent[]> {
  const result: ProviderEvent[] = []
  for await (const event of events) {
    result.push(event)
  }
  return result
}

test('OpenAI Responses rejects malformed consumed payloads and unrecognized events', async () => {
  for (const value of [
    null, [], {}, { type: 42 }, { type: 'response.future_terminal' },
    { type: 'response.output_text.delta', delta: {} },
    { type: 'response.output_item.done', item: { type: 'message', content: 3 } },
    { type: 'response.output_item.done', item: { type: 'reasoning', summary: [{}] } },
    { type: 'response.output_item.done', item: { ...call, id: undefined } },
    { type: 'response.output_item.done', item: { ...call, arguments: undefined } },
    { type: 'response.function_call_arguments.delta', item_id: 3, delta: '{}' },
    { type: 'response.completed' },
    { type: 'response.completed', response: { usage: { input_tokens: '1' } } },
    { type: 'response.completed', response: { usage: { output_tokens: -1 } } },
    { type: 'response.completed', response: { usage: { input_tokens: 1, input_tokens_details: { cached_tokens: 2 } } } },
    { type: 'response.failed', response: { error: { message: [] } } },
  ]) {
    await expect(collect(mapOpenAIResponseStream(frames([value, completed]))))
      .rejects.toBeInstanceOf(ProviderDataError)
  }
})

test('OpenAI Responses uses complete tool arguments, tolerates extensions and stops at terminal events', async () => {
  const events = await collect(mapOpenAIResponseStream(frames([
    { type: 'response.created', unrelated: true },
    { type: 'response.output_item.added', item: { ...call, arguments: '' } },
    { type: 'response.function_call_arguments.delta', item_id: 'item', delta: '{"wrong":' },
    { type: 'response.output_item.done', item: { ...call, extension: [] } },
    completed,
    { type: 'invalid_after_terminal' },
  ])))
  expect(events).toEqual([
    { type: 'tool_call_requested', toolUseId: 'call|item', toolName: 'tool', input: { value: 1 } },
    { type: 'response', usage: { inputTokens: 0, outputTokens: 0, cacheReadTokens: 0, cacheWriteTokens: 0 } },
  ])
})

test('OpenAI Responses errors and truncation cannot produce a fabricated success', async () => {
  for (const values of [[], ['[DONE]'], [{ type: 'response.output_text.delta', delta: 'partial' }]]) {
    await expect(collect(mapOpenAIResponseStream(frames(values)))).rejects.toBeInstanceOf(ProviderDataError)
  }
  for (const value of [
    { type: 'response.failed', response: {} },
    { type: 'response.incomplete', response: { incomplete_details: { reason: 'max_output_tokens' } } },
    { type: 'error', error: { code: 'rate_limit_exceeded', message: 'try later' } },
  ]) {
    const events = await collect(mapOpenAIResponseStream(frames([value, completed])))
    expect(events).toHaveLength(1)
    expect(events[0]?.type).toBe('error')
  }
})

test('OpenAI reasoning replay validates its format and skips foreign opaque signatures', () => {
  const request: InferenceRequest = {
    sessionId: 'session', turnId: 'turn', requestId: 'request', modelId: 'test', outputLimit: null,
    systemPrompt: '', cwd: '/workspace', items: [], tools: [], thinking: null, serviceTierId: null,
    cancel: new AbortController().signal,
  }
  const body = (signature: string) => buildOpenAIResponsesBody({
    ...request,
    items: [{ type: 'assistant_thinking', modelId: 'test', text: 'thought', signature }],
  }, undefined)
  expect(body('foreign:opaque').input).toEqual([])
  expect(body('{"type":"other","value":3}').input).toEqual([])
  expect(() => body('{"type":"reasoning","summary":42}')).toThrow(ProviderDataError)
  expect(body('{"type":"reasoning","id":"id","encrypted_content":"enc","status":"done"}').input)
    .toEqual([{ type: 'reasoning', id: 'id', encrypted_content: 'enc' }])
})

test('OpenAI Responses releases the transport on malformed input', async () => {
  let cancelled = false
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(new TextEncoder().encode('data: {"type":"response.output_text.delta","delta":42}\n\n'))
    },
    cancel() {
      cancelled = true
    },
  })
  await expect(collect(mapOpenAIResponseStream(readServerSentEvents(body)))).rejects.toBeInstanceOf(ProviderDataError)
  expect(cancelled).toBe(true)
  expect(body.locked).toBe(false)
})
