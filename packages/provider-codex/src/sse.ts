import { parseProviderJson, readServerSentEvents } from '@demicodes/provider'
import { responsesEventSchema, type ResponsesStreamEvent } from '@demicodes/provider'

export async function* parseSseResponseStream(
  body: ReadableStream<Uint8Array>,
  signal?: AbortSignal,
): AsyncIterable<ResponsesStreamEvent> {
  for await (const { data } of readServerSentEvents(body, signal)) {
    if (data === '[DONE]')
      return
    yield parseProviderJson(responsesEventSchema, data, 'Codex SSE event')
  }
}
