import {
  decodeResponsesFrame,
  readServerSentEvents,
  type ResponsesEvent
} from '@demicodes/provider'

/**
 * The Responses events of an SSE body. Event types this adapter does not map
 * are skipped; a mapped type with a malformed payload throws, and the provider
 * reports it as a stream error.
 */
export async function* parseSseResponseStream(
  body: ReadableStream<Uint8Array>
): AsyncIterable<ResponsesEvent> {
  for await (const frame of readServerSentEvents(body)) {
    const event = decodeResponsesFrame(frame.data)
    if (event)
      yield event
  }
}
