import { expect, test } from 'bun:test'
import { ProviderDataError, readServerSentEvents, type ProviderEvent, type ServerSentEvent } from '@demicodes/provider'
import { mapAnthropicMessageStream } from '../provider'
import { parseAnthropicEvent } from '../response-schemas'

const stop = { type: 'message_stop' }
const startTool = {
  type: 'content_block_start', index: 2,
  content_block: { type: 'tool_use', id: 'tool-2', name: 'read_file', input: {} },
}

function parse(value: unknown, event: string | null = null) {
  return parseAnthropicEvent({ event, data: JSON.stringify(value) })
}

async function* frames(values: unknown[]): AsyncIterable<ServerSentEvent> {
  for (const value of values) {
    yield { event: null, data: JSON.stringify(value) }
  }
}

async function collect(events: AsyncIterable<ProviderEvent>): Promise<ProviderEvent[]> {
  const result: ProviderEvent[] = []
  for await (const event of events) {
    result.push(event)
  }
  return result
}

test('Anthropic consumed fields reject missing, null, wrong and partial shapes', () => {
  for (const value of [
    null, [], {}, { type: 3 }, { type: '' },
    { type: 'message_start' }, { type: 'message_start', message: [] },
    { type: 'message_start', message: { usage: null } },
    { type: 'message_delta', usage: { output_tokens: '3' } },
    { type: 'message_delta', usage: { input_tokens: -1 } },
    { type: 'message_delta', usage: { cache_read_input_tokens: 1.2 } },
    { ...startTool, index: '2' }, { ...startTool, index: -1 },
    { ...startTool, index: 0.1 }, { ...startTool, index: null },
    { ...startTool, content_block: [] },
    { ...startTool, content_block: { type: 'tool_use', name: 'tool', input: {} } },
    { ...startTool, content_block: { type: 'tool_use', id: 'id', name: '', input: {} } },
    { ...startTool, content_block: { type: 'tool_use', id: 'id', name: 'tool' } },
    { ...startTool, content_block: { type: 'tool_use', id: 'id', name: 'tool', input: [] } },
    { ...startTool, content_block: { type: 'text', text: {} } },
    { ...startTool, content_block: { type: 'thinking', thinking: 3 } },
    { type: 'content_block_delta', index: 0, delta: { type: 'text_delta' } },
    { type: 'content_block_delta', index: 0, delta: { type: 'input_json_delta', partial_json: {} } },
    { type: 'content_block_stop' },
    { type: 'error' }, { type: 'error', error: { type: 'overloaded_error', message: 3 } },
  ]) {
    expect(() => parse(value)).toThrow(ProviderDataError)
  }
  expect(() => parse(stop, 'message_delta')).toThrow(ProviderDataError)
  expect(() => parseAnthropicEvent({ event: null, data: 'SYNTHETIC_SECRET' }))
    .toThrow('Anthropic SSE event: invalid JSON')
})

test('Anthropic extension variants are ignored without masking malformed known variants', async () => {
  expect(parse({ type: 'new_event', extra: ['anything'] })).toEqual({ type: 'ignored' })
  const events = await collect(mapAnthropicMessageStream(frames([
    { type: 'new_event' },
    { ...startTool, content_block: { type: 'server_tool_use', input: 'opaque' } },
    { type: 'content_block_delta', index: 2, delta: { type: 'future_delta', raw: 5 } },
    { type: 'content_block_stop', index: 2 },
    { type: 'ping', future_field: {} },
    stop,
  ])))
  expect(events).toEqual([{
    type: 'response',
    usage: { inputTokens: 0, outputTokens: 0, cacheReadTokens: 0, cacheWriteTokens: 0 },
  }])
})

test('Anthropic cumulative usage preserves absent counters and accepts explicit zero', async () => {
  const events = await collect(mapAnthropicMessageStream(frames([
    { type: 'message_start', message: { usage: {
      input_tokens: 8, output_tokens: 4, cache_read_input_tokens: 2, cache_creation_input_tokens: 3,
    } } },
    { type: 'message_delta', usage: { output_tokens: 0, cache_read_input_tokens: 0 } },
    stop,
  ])))
  expect(events).toEqual([{
    type: 'response',
    usage: { inputTokens: 8, outputTokens: 0, cacheReadTokens: 0, cacheWriteTokens: 3 },
  }])
})

test('Anthropic tool fragments execute only at their block stop with supplied identity', async () => {
  const events = await collect(mapAnthropicMessageStream(frames([
    startTool,
    { type: 'content_block_delta', index: 2, delta: { type: 'input_json_delta', partial_json: '{"path":' } },
    { type: 'content_block_delta', index: 2, delta: { type: 'input_json_delta', partial_json: '"a.ts"}' } },
    { type: 'content_block_stop', index: 2 },
    stop,
  ])))
  expect(events[0]).toEqual({
    type: 'tool_call_requested', toolUseId: 'tool-2', toolName: 'read_file', input: { path: 'a.ts' },
  })
  expect(events.filter((event) => event.type === 'tool_call_requested')).toHaveLength(1)
})

test('Anthropic invalid block sequences and truncated streams do not report success', async () => {
  const delta = { type: 'content_block_delta', index: 2, delta: { type: 'input_json_delta', partial_json: '{}' } }
  for (const values of [
    [], [startTool], [startTool, stop], [startTool, startTool, stop],
    [delta, stop], [{ type: 'content_block_stop', index: 2 }, stop],
    [startTool, { ...delta, delta: { type: 'text_delta', text: 'wrong block' } }, stop],
  ]) {
    const emitted: ProviderEvent[] = []
    try {
      for await (const event of mapAnthropicMessageStream(frames(values))) {
        emitted.push(event)
      }
      throw new Error('expected failure')
    } catch (error) {
      expect(error).toBeInstanceOf(ProviderDataError)
    }
    expect(emitted.some((event) => event.type === 'response')).toBe(false)
    expect(emitted.some((event) => event.type === 'tool_call_requested')).toBe(false)
  }
})

test('Anthropic schema errors cancel the underlying SSE body', async () => {
  let cancelled = false
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(new TextEncoder().encode('data: {"type":"content_block_start","index":"bad"}\n\n'))
    },
    cancel() {
      cancelled = true
    },
  })
  await expect(collect(mapAnthropicMessageStream(readServerSentEvents(body))))
    .rejects.toBeInstanceOf(ProviderDataError)
  expect(cancelled).toBe(true)
  expect(body.locked).toBe(false)
})
