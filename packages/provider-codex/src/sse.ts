import {
  decodeResponsesFrame,
  readServerSentEvents,
  type ReceivedResponsesEvent
} from '@demicodes/provider'

/**
 * The Responses events of an SSE body, each with the text it arrived as. Event
 * types this adapter does not map are skipped; a mapped type with a malformed
 * payload throws, and the provider reports it as a stream error.
 */
export async function* parseSseResponseStream(
  body: ReadableStream<Uint8Array>
): AsyncIterable<ReceivedResponsesEvent> {
  for await (const frame of readServerSentEvents(body)) {
    const received = decodeResponsesFrame(frame.data)
    if (received)
      yield received
  }
}
