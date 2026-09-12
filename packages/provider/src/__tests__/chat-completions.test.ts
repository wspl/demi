import { expect, test } from 'bun:test'
import { mapChatCompletionStream, chatCompletionChunkSchema, parseProviderData, ProviderDataError, readServerSentEvents, type ProviderEvent, type ServerSentEvent } from '../index'

const withTool = (tool: unknown) => ({ choices: [{ delta: { tool_calls: [tool] } }] })
const tool = { index: 0, id: 'call', function: { name: 'read_file', arguments: '{}' } }

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

test('Chat wire rejects malformed containers, indexes, delta fields and usage', () => {
  for (const value of [
    null, [], {}, { choices: null }, { choices: {} }, { choices: [null] },
    { choices: [{}] }, { choices: [{ delta: [] }] }, { choices: [{ delta: {}, index: '0' }] },
    { choices: [{ delta: {}, index: -1 }] }, { choices: [{ delta: {}, index: 1.5 }] },
    { choices: [{ delta: { content: [] } }] }, { choices: [{ delta: { reasoning_content: {} } }] },
    { choices: [{ delta: { tool_calls: {} } }] },
    withTool({ function: {} }), withTool({ ...tool, index: '0' }),
    withTool({ ...tool, index: -1 }), withTool({ ...tool, id: 42 }), withTool({ ...tool, id: '' }),
    withTool({ ...tool, type: 'custom' }), withTool({ ...tool, function: [] }),
    withTool({ ...tool, function: { name: 3 } }), withTool({ ...tool, function: { arguments: {} } }),
    { choices: [], usage: [] }, { choices: [], usage: { prompt_tokens: '3' } },
    { choices: [], usage: { completion_tokens: -1 } },
    { choices: [], usage: { prompt_tokens: 2, prompt_tokens_details: { cached_tokens: 3 } } },
    { choices: [{ delta: {}, finish_reason: 5 }] }, { error: [] }, { error: { message: 3 } },
    { choices: [{ index: 0, delta: {} }, { index: 0, delta: {} }] },
  ]) {
    expect(() => parseProviderData(chatCompletionChunkSchema, value, 'fixture')).toThrow(ProviderDataError)
  }
})

test('Chat partial arguments and nullable text share one contract across endpoint labels', async () => {
  for (const source of ['OpenAI Chat Completions', 'Grok Build']) {
    const events = await collect(mapChatCompletionStream(frames([
      { choices: [{ delta: { content: null, reasoning_content: 'plan', future_field: 3 } }], usage: null },
      withTool({ index: 0, id: 'call', function: { name: 'read_file', arguments: '{"x":' } }),
      withTool({ index: 0, function: { arguments: '1}' } }),
      { choices: [{ delta: null, finish_reason: 'tool_calls' }] },
      { choices: [], usage: { prompt_tokens: 5, completion_tokens: 2, prompt_tokens_details: { cached_tokens: 3 } } },
      '[DONE]',
    ]), { source }))
    expect(events).toEqual([
      { type: 'thinking_start' }, { type: 'thinking_delta', text: 'plan' },
      { type: 'tool_call_requested', toolUseId: 'call', toolName: 'read_file', input: { x: 1 } },
      { type: 'response', usage: { inputTokens: 2, outputTokens: 2, cacheReadTokens: 3, cacheWriteTokens: 0 } },
    ])
  }
})

test('Chat choice-local tool indexes do not merge independent calls', async () => {
  const events = await collect(mapChatCompletionStream(frames([
    { choices: [
      { index: 1, delta: { tool_calls: [{ ...tool, id: 'one', function: { name: 'one', arguments: '{"a":' } }] } },
      { index: 2, delta: { tool_calls: [{ ...tool, id: 'two', function: { name: 'two', arguments: '{"b":' } }] } },
    ] },
    { choices: [
      { index: 2, delta: { tool_calls: [{ index: 0, function: { arguments: '2}' } }] }, finish_reason: 'tool_calls' },
      { index: 1, delta: { tool_calls: [{ index: 0, function: { arguments: '1}' } }] }, finish_reason: 'tool_calls' },
    ] },
    '[DONE]',
  ])))
  expect(events.slice(0, 2)).toEqual([
    { type: 'tool_call_requested', toolUseId: 'two', toolName: 'two', input: { b: 2 } },
    { type: 'tool_call_requested', toolUseId: 'one', toolName: 'one', input: { a: 1 } },
  ])
})

test('Chat missing identity or arguments, changed identities and duplicate calls never execute', async () => {
  for (const values of [
    [withTool({ index: 0, function: tool.function }), '[DONE]'],
    [withTool({ index: 0, id: 'id', function: { arguments: '{}' } }), '[DONE]'],
    [withTool({ index: 0, id: 'id', function: { name: 'tool' } }), '[DONE]'],
    [withTool(tool), withTool({ ...tool, id: 'changed' }), '[DONE]'],
    [withTool(tool), withTool({ ...tool, function: { name: 'changed' } }), '[DONE]'],
    [{ choices: [{ delta: { tool_calls: [tool, { ...tool, index: 1 }] } }] }, '[DONE]'],
    [withTool(tool)],
  ]) {
    const emitted: ProviderEvent[] = []
    try {
      for await (const event of mapChatCompletionStream(frames(values)))
        emitted.push(event)
      throw new Error('expected invalid stream')
    } catch (error) {
      expect(error).toBeInstanceOf(ProviderDataError)
    }
    expect(emitted.some((event) => event.type === 'tool_call_requested' || event.type === 'response')).toBe(false)
  }
})

test('Chat does not execute a tool when its choice was truncated or filtered', async () => {
  for (const finish_reason of ['length', 'content_filter', 'future_reason']) {
    const events = await collect(mapChatCompletionStream(frames([
      { choices: [{ delta: { tool_calls: [tool] }, finish_reason }] }, '[DONE]',
    ])))
    expect(events).toHaveLength(1)
    expect(events[0]?.type).toBe('error')
  }
  await expect(collect(mapChatCompletionStream(frames([
    { choices: [{ delta: { content: 'partial' } }] },
  ])))).rejects.toBeInstanceOf(ProviderDataError)
})

test('Chat malformed wire cancels its body and omits raw payloads from errors', async () => {
  for (const data of ['SYNTHETIC_SECRET', '{"choices":[{"delta":{"tool_calls":"SYNTHETIC_SECRET"}}]}']) {
    let cancelled = false
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(new TextEncoder().encode(`data: ${data}\n\n`))
      },
      cancel() {
        cancelled = true
      },
    })
    try {
      await collect(mapChatCompletionStream(readServerSentEvents(body)))
      throw new Error('expected invalid stream')
    } catch (error) {
      expect(error).toBeInstanceOf(ProviderDataError)
      expect(String(error)).not.toContain('SYNTHETIC_SECRET')
    }
    expect(cancelled).toBe(true)
    expect(body.locked).toBe(false)
  }
})
