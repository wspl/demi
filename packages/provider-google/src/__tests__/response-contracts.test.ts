import { expect, test } from 'bun:test'
import { parseProviderData, ProviderDataError, readServerSentEvents, type ProviderEvent, type ServerSentEvent } from '@demicodes/provider'
import { googleResponseSchema } from '../response-schemas'
import { mapGoogleContentStream } from '../provider'

const completed = { candidates: [{ finishReason: 'STOP' }] }
const withPart = (part: unknown) => ({ candidates: [{ content: { parts: [part] }, finishReason: 'STOP' }] })

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

test('Google response schemas reject malformed consumed fields instead of empty values', () => {
  for (const value of [
    null, [], { candidates: null }, { candidates: {} }, { candidates: [3] },
    { candidates: [{ index: -1 }] }, { candidates: [{ index: '0' }] },
    { candidates: [{ content: [] }] }, { candidates: [{ content: { parts: {} } }] },
    { candidates: [{ content: { parts: [null] } }] },
    withPart({ text: 3 }), withPart({ text: null }), withPart({ thought: 'true' }),
    withPart({ thoughtSignature: {} }), withPart({ functionCall: [] }),
    withPart({ functionCall: { args: {} } }),
    withPart({ functionCall: { name: '' } }), withPart({ functionCall: { name: 'bad name' } }),
    withPart({ functionCall: { name: 'tool', id: 3 } }),
    withPart({ functionCall: { name: 'tool', id: '' } }),
    withPart({ functionCall: { name: 'tool', args: [] } }),
    withPart({ functionCall: { name: 'tool', args: null } }),
    withPart({ text: 'text', functionCall: { name: 'tool' } }),
    { usageMetadata: [] }, { usageMetadata: { promptTokenCount: '3' } },
    { usageMetadata: { thoughtsTokenCount: -1 } }, { usageMetadata: { candidatesTokenCount: 0.3 } },
    { usageMetadata: { promptTokenCount: 2, cachedContentTokenCount: 3 } },
    { error: { status: 3, message: 'error' } }, { error: { status: 'ERROR', message: {} } },
    { promptFeedback: { blockReason: [] } }, { candidates: [{ finishReason: 3 }] },
  ]) {
    expect(() => parseProviderData(googleResponseSchema, value, 'fixture')).toThrow(ProviderDataError)
  }
})

test('Google optional IDs and args are protocol defaults, but separate calls remain distinct', async () => {
  const events = await collect(mapGoogleContentStream(frames([
    { candidates: [{ content: { parts: [
      { functionCall: { name: 'tool' }, thoughtSignature: 'signed' },
      { functionCall: { name: 'tool' } },
      { functionCall: { name: 'tool', id: 'provider-id', args: { x: 1 } } },
    ] }, finishReason: 'STOP' }] },
  ])))
  const calls = events.filter((event) => event.type === 'tool_call_requested')
  expect(calls).toHaveLength(3)
  expect(new Set(calls.map((event) => event.toolUseId)).size).toBe(3)
  expect(calls[0]?.input).toEqual({})
  expect(calls[2]).toMatchObject({ toolUseId: 'provider-id', toolName: 'tool', input: { x: 1 } })
  expect(events[0]).toEqual({ type: 'thinking_start' })
  expect(events[1]).toEqual({ type: 'thinking_signature', signature: 'google:signed' })
})

test('Google validates present usage and counts cached input only once', async () => {
  const events = await collect(mapGoogleContentStream(frames([
    { extension: 'ignored', candidates: [{ content: { parts: [{ futurePart: 'ignored' }] } }] },
    completed,
    { usageMetadata: { promptTokenCount: 10, cachedContentTokenCount: 3, candidatesTokenCount: 2, thoughtsTokenCount: 4 } },
  ])))
  expect(events).toEqual([{
    type: 'response', usage: { inputTokens: 7, outputTokens: 6, cacheReadTokens: 3, cacheWriteTokens: 0 },
  }])
  const withoutUsage = await collect(mapGoogleContentStream(frames([completed])))
  expect(withoutUsage).toEqual([{
    type: 'response', usage: { inputTokens: 0, outputTokens: 0, cacheReadTokens: 0, cacheWriteTokens: 0 },
  }])
})

test('Google truncation and blocked or incomplete responses do not report success', async () => {
  for (const values of [[], [{}], [{ candidates: [{ content: { parts: [{ text: 'partial' }] } }] }]]) {
    await expect(collect(mapGoogleContentStream(frames(values)))).rejects.toBeInstanceOf(ProviderDataError)
  }
  for (const value of [
    { promptFeedback: { blockReason: 'SAFETY' } },
    { candidates: [{ finishReason: 'MAX_TOKENS' }] },
    { candidates: [{ finishReason: 'MALFORMED_FUNCTION_CALL', content: { parts: [{ functionCall: { name: 'tool' } }] } }] },
    { candidates: [{ finishReason: 'FUTURE_FINISH_REASON' }] },
  ]) {
    const events = await collect(mapGoogleContentStream(frames([value])))
    expect(events).toHaveLength(1)
    expect(events[0]?.type).toBe('error')
  }
})

test('Google validation failures release the SSE body', async () => {
  let cancelled = false
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(new TextEncoder().encode('data: {"candidates":42}\n\n'))
    },
    cancel() {
      cancelled = true
    },
  })
  await expect(collect(mapGoogleContentStream(readServerSentEvents(body)))).rejects.toBeInstanceOf(ProviderDataError)
  expect(cancelled).toBe(true)
  expect(body.locked).toBe(false)
})

test('Google preserves signatures on thought parts and rejects content after completion', async () => {
  const events = await collect(mapGoogleContentStream(frames([
    withPart({ thought: true, text: 'plan', thoughtSignature: 'signature' }),
  ])))
  expect(events.slice(0, 3)).toEqual([
    { type: 'thinking_start' },
    { type: 'thinking_delta', text: 'plan' },
    { type: 'thinking_signature', signature: 'google:signature' },
  ])
  await expect(collect(mapGoogleContentStream(frames([
    completed, withPart({ functionCall: { name: 'tool' } }),
  ])))).rejects.toBeInstanceOf(ProviderDataError)
})
