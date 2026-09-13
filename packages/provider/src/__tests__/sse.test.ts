import { describe, expect, it } from 'bun:test'
import { encodeUtf8 } from '@demicodes/utils'
import { readServerSentEvents, type ServerSentEvent } from '../index'

/** A body that delivers `chunks` in order, exactly as split. */
function bodyOf(chunks: readonly string[]): ReadableStream<Uint8Array> {
  return new ReadableStream<Uint8Array>({
    start(controller) {
      for (const chunk of chunks) controller.enqueue(encodeUtf8(chunk))
      controller.close()
    },
  })
}

async function collect(
  chunks: readonly string[],
  signal?: AbortSignal
): Promise<ServerSentEvent[]> {
  const events: ServerSentEvent[] = []
  for await (const event of readServerSentEvents(bodyOf(chunks), signal))
    events.push(event)
  return events
}

describe('readServerSentEvents', () => {
  it('frames CRLF streams and keeps the event name', async () => {
    expect(await collect(['event: delta\r\ndata: {"a":1}\r\n\r\n'])).toEqual([
      { event: 'delta', data: '{"a":1}' },
    ])
  })

  it('joins multi-line data fields with a newline, per the spec', async () => {
    expect(await collect(['data: line one\ndata: line two\n\n'])).toEqual([
      { event: null, data: 'line one\nline two' },
    ])
  })

  it('passes the [DONE] sentinel through as a payload', async () => {
    expect(await collect(['data: {"a":1}\n\n', 'data: [DONE]\n\n'])).toEqual([
      { event: null, data: '{"a":1}' },
      { event: null, data: '[DONE]' },
    ])
  })

  it('yields a final frame that has no terminating blank line', async () => {
    expect(await collect(['data: {"a":1}'])).toEqual([
      { event: null, data: '{"a":1}' },
    ])
  })

  it('reassembles frames split across chunks, including multi-byte text', async () => {
    const bytes = encodeUtf8('data: héllo\n\n')
    const head = bytes.slice(0, 8)
    const tail = bytes.slice(8)
    const events: ServerSentEvent[] = []
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(head)
        controller.enqueue(tail)
        controller.close()
      },
    })
    for await (const event of readServerSentEvents(body)) events.push(event)
    expect(events).toEqual([{ event: null, data: 'héllo' }])
  })

  it('drops a frame that carried no data field', async () => {
    expect(await collect([': keep-alive\n\nevent: ping\n\ndata: x\n\n']))
      .toEqual([{ event: null, data: 'x' }])
  })

  it('stops at an aborted signal without yielding the partial frame', async () => {
    const controller = new AbortController()
    controller.abort()
    expect(await collect(['data: {"a":1}\n\n'], controller.signal)).toEqual([])
  })

  it('yields nothing for a body-less response', async () => {
    const events: ServerSentEvent[] = []
    for await (const event of readServerSentEvents(null)) events.push(event)
    expect(events).toEqual([])
  })
})
