/**
 * Server-sent event framing, shared by every provider that streams over HTTP.
 * Only the framing lives here: what a payload means is the vendor module's job
 * (`responses.ts`, `chat-completions.ts`, or the provider package itself).
 */
import { utf8Lines } from '@demicodes/utils'

export interface ServerSentEvent {
  /** The `event:` field of the frame, or null when it carried none. */
  event: string | null
  /**
   * The frame payload: its `data:` fields joined with '\n', as the SSE spec
   * defines a multi-line payload. A JSON event arrives as one payload string.
   */
  data: string
}

/**
 * Reads a response body as a stream of server-sent events.
 *
 * A frame ends at a blank line; a frame without a `data:` field is dropped,
 * and a trailing frame with no final blank line is still yielded. When
 * `signal` aborts, the stream ends without yielding the partial frame in
 * hand. The reader lock is released on every exit, including the consumer
 * abandoning the generator.
 */
export async function* readServerSentEvents(
  body: ReadableStream<Uint8Array> | null,
  signal?: AbortSignal,
): AsyncIterable<ServerSentEvent> {
  if (!body)
    return
  let eventName: string | null = null
  let dataFields: string[] = []

  for await (const line of utf8Lines(readChunks(body, signal))) {
    if (line === '') {
      if (dataFields.length > 0)
        yield { event: eventName, data: dataFields.join('\n') }
      eventName = null
      dataFields = []
      continue
    }
    if (line.startsWith('event:')) {
      eventName = line.slice('event:'.length).trim()
      continue
    }
    if (line.startsWith('data:'))
      dataFields.push(line.slice('data:'.length).trimStart())
  }

  // An aborted stream stops where it is: the frame in hand is incomplete.
  if (signal?.aborted)
    return
  if (dataFields.length > 0)
    yield { event: eventName, data: dataFields.join('\n') }
}

/** The body's chunks until it ends or `signal` aborts, lock always released. */
async function* readChunks(
  body: ReadableStream<Uint8Array>,
  signal: AbortSignal | undefined,
): AsyncIterable<Uint8Array> {
  const reader = body.getReader()
  try {
    while (!signal?.aborted) {
      const { value, done } = await reader.read()
      if (done)
        return
      yield value
    }
  } finally {
    reader.releaseLock()
  }
}
