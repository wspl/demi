import { expect, test } from 'bun:test'
import { ProviderDataError, parseProviderData, providerErrorFromUnknown } from '@demicodes/provider'
import { codexResponseEventSchema, parseCodexWebSocketEvent, type CodexResponseStreamEvent } from '../response-schemas'
import { mapCodexResponseEvent } from '../responses'
import { parseSseChunk, parseSseResponseStream } from '../sse'

const valid = [
  { type: 'response.output_text.delta', delta: 'hello', future_field: 42 },
  { type: 'response.output_item.done', item: { type: 'message', content: [{ type: 'output_text', text: 'hello' }] } },
  { type: 'response.output_item.done', item: { type: 'function_call', id: 'item', call_id: 'call', name: 'tool', arguments: '{}' } },
  { type: 'response.completed', response: { usage: { input_tokens: 3, output_tokens: 1 } } },
  { type: 'response.created' },
] satisfies CodexResponseStreamEvent[]
const invalid = [
  null, [], {}, { type: 42 }, { type: 'response.unknown_terminal' },
  { type: 'response.output_text.delta', delta: {} },
  { type: 'response.output_item.done', item: { type: 'message', content: 42 } },
  { type: 'response.output_item.done', item: { type: 'message' } },
  { type: 'response.output_item.done', item: { type: 'reasoning', summary: [{ text: 42 }] } },
  { type: 'response.output_item.done', item: { type: 'function_call', name: 'tool', arguments: '{}' } },
  { type: 'response.output_item.done', item: { type: 'function_call', id: 'item', call_id: 'call', name: 'tool' } },
  { type: 'response.function_call_arguments.delta', item_id: 42, delta: '{}' },
  { type: 'response.completed' },
  { type: 'response.completed', response: { usage: { input_tokens: '3' } } },
  { type: 'response.completed', response: { usage: { input_tokens: -1 } } },
  { type: 'response.completed', response: { usage: { input_tokens: 1, input_tokens_details: { cached_tokens: 2 } } } },
  { type: 'response.failed', response: { error: { message: 42 } } },
  { type: 'error', error: { code: {} } },
]

test('SSE and WebSocket enforce one response contract', () => {
  for (const event of valid) {
    const json = JSON.stringify(event)
    expect(parseSseChunk(`data: ${json}`)).toEqual(event)
    expect(parseCodexWebSocketEvent(json)).toEqual(event)
    expect(parseCodexWebSocketEvent(JSON.stringify({ event }))).toEqual(event)
  }
  for (const event of invalid) {
    const json = JSON.stringify(event)
    expect(() => parseSseChunk(`data: ${json}`)).toThrow(ProviderDataError)
    expect(() => parseCodexWebSocketEvent(json)).toThrow(ProviderDataError)
  }
  expect(parseCodexWebSocketEvent('{"type":"response.done","response":{}}'))
    .toEqual({ type: 'response.completed', response: {} })
})

test('malformed JSON errors do not expose payload values or misclassify usage errors', () => {
  for (const parse of [parseCodexWebSocketEvent, (text: string) => parseSseChunk(`data: ${text}`)]) {
    try {
      parse('SYNTHETIC_SECRET_BAD_JSON')
      throw new Error('expected validation failure')
    } catch (error) {
      expect(error).toBeInstanceOf(ProviderDataError)
      expect(String(error)).not.toContain('SYNTHETIC_SECRET')
    }
  }
  const error = new ProviderDataError('usage response', 'invalid fields: usage')
  expect(providerErrorFromUnknown(error, null)).toMatchObject({ code: 'invalid_provider_response' })
})

test('tool calls use the completed item and preserve independent call identities', () => {
  for (const id of ['a', 'b']) {
    const parsed = parseProviderData(codexResponseEventSchema, {
      type: 'response.output_item.done',
      item: { type: 'function_call', id, call_id: `call-${id}`, name: 'tool', arguments: '{"x":1}' },
    }, 'fixture')
    expect([...mapCodexResponseEvent(parsed)]).toEqual([{
      type: 'tool_call_requested', toolUseId: `call-${id}|${id}`, toolName: 'tool', input: { x: 1 },
    }])
  }
})

test('SSE consumption cancels the body and releases its reader on errors and early return', async () => {
  for (const event of [valid[0], invalid[5]]) {
    let cancelled = false
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(new TextEncoder().encode(`data: ${JSON.stringify(event)}\n\n`))
      },
      cancel() {
        cancelled = true
      },
    })
    const iterator = parseSseResponseStream(body)[Symbol.asyncIterator]()
    if (event === valid[0]) {
      expect((await iterator.next()).done).toBe(false)
      await iterator.return?.()
    } else {
      await expect(iterator.next()).rejects.toBeInstanceOf(ProviderDataError)
    }
    expect(cancelled).toBe(true)
    expect(body.locked).toBe(false)
  }
})
