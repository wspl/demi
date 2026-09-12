import { expect, test } from 'bun:test'
import { ProviderDataError, readServerSentEvents, type ServerSentEvent } from '../index'

async function collect(body: ReadableStream<Uint8Array> | null, signal?: AbortSignal): Promise<ServerSentEvent[]> {
  const events: ServerSentEvent[] = []
  for await (const event of readServerSentEvents(body, signal)) {
    events.push(event)
  }
  return events
}

function chunked(bytes: Uint8Array, split: number): ReadableStream<Uint8Array> {
  return new ReadableStream({
    start(controller) {
      controller.enqueue(bytes.slice(0, split))
      controller.enqueue(bytes.slice(split))
      controller.close()
    },
  })
}

test('SSE framing preserves payloads across every byte split and line ending', async () => {
  for (const newline of ['\n', '\r', '\r\n']) {
    const text = ['\uFEFF: comment', 'event: ignored', '', 'event: named ',
      'data:  中文', 'data:\tvalue', 'id: irrelevant', '', 'data', '',
      'data: incomplete'].join(newline)
    const bytes = new TextEncoder().encode(text)
    for (let split = 0; split <= bytes.length; split++) {
      const body = chunked(bytes, split)
      expect(await collect(body)).toEqual([
        { event: 'named ', data: ' 中文\n\tvalue' },
        { event: null, data: '' },
      ])
      expect(body.locked).toBe(false)
    }
  }
})

test('SSE ignores incomplete frames and resets an event-only frame', async () => {
  for (const text of ['data: unfinished', 'data: unfinished\n', ': comment\n\n']) {
    expect(await collect(new Response(text).body)).toEqual([])
  }
  expect(await collect(new Response('event: ignored\n\ndata: value\n\n').body))
    .toEqual([{ event: null, data: 'value' }])
})

test('SSE rejects missing bodies and malformed UTF-8 without exposing data', async () => {
  await expect(collect(null)).rejects.toBeInstanceOf(ProviderDataError)
  for (const bytes of [new Uint8Array([0xff]), new Uint8Array([0xe4, 0xb8])]) {
    const body = chunked(bytes, 1)
    await expect(collect(body)).rejects.toThrow('invalid UTF-8')
    expect(body.locked).toBe(false)
  }
})

test('SSE cancellation wakes a pending read and releases the abort listener', async () => {
  let cancellations = 0
  let listeners = 0
  const controller = new AbortController()
  const originalAdd = controller.signal.addEventListener.bind(controller.signal)
  const originalRemove = controller.signal.removeEventListener.bind(controller.signal)
  controller.signal.addEventListener = (...args) => {
    listeners++
    originalAdd(...args)
  }
  controller.signal.removeEventListener = (...args) => {
    listeners--
    originalRemove(...args)
  }
  const body = new ReadableStream<Uint8Array>({
    cancel() {
      cancellations++
    },
  })
  const pending = collect(body, controller.signal)
  controller.abort()
  await expect(pending).rejects.toMatchObject({ name: 'AbortError' })
  expect(cancellations).toBe(1)
  expect(listeners).toBe(0)
  expect(body.locked).toBe(false)
})

test('SSE early return and failure cancel once and unlock the body', async () => {
  let cancellations = 0
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(new TextEncoder().encode('data: first\n\ndata: second\n\n'))
    },
    cancel() {
      cancellations++
    },
  })
  for await (const event of readServerSentEvents(body)) {
    expect(event.data).toBe('first')
    break
  }
  expect(cancellations).toBe(1)
  expect(body.locked).toBe(false)
  const failed = new ReadableStream<Uint8Array>({
    start(controller) {
      controller.error(new Error('synthetic transport failure'))
    },
  })
  await expect(collect(failed)).rejects.toThrow('synthetic transport failure')
  expect(failed.locked).toBe(false)
})
