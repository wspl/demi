import { parseProviderJson, readServerSentEvents } from '@demicodes/provider'
import { codexResponseEventSchema, type CodexResponseStreamEvent } from './response-schemas'

export async function* parseSseResponseStream(
  body: ReadableStream<Uint8Array>,
  signal?: AbortSignal,
): AsyncIterable<CodexResponseStreamEvent> {
  for await (const { data } of readServerSentEvents(body, signal)) {
    if (data === '[DONE]')
      return
    yield parseProviderJson(codexResponseEventSchema, data, 'Codex SSE event')
  }
}
