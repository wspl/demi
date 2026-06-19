import type { CodexResponseStreamEvent } from './responses'

export async function* parseSseResponseStream(body: ReadableStream<Uint8Array>): AsyncIterable<CodexResponseStreamEvent> {
  const reader = body.getReader()
  const decoder = new TextDecoder()
  let buffer = ''
  try {
    while (true) {
      const { done, value } = await reader.read()
      if (done) break
      buffer += decoder.decode(value, { stream: true })
      const chunks = buffer.split(/\r?\n\r?\n/)
      buffer = chunks.pop() ?? ''
      for (const chunk of chunks) {
        const event = parseSseChunk(chunk)
        if (event) yield event
      }
    }
    buffer += decoder.decode()
    const final = parseSseChunk(buffer)
    if (final) yield final
  } finally {
    reader.releaseLock()
  }
}

export function parseSseChunk(chunk: string): CodexResponseStreamEvent | null {
  const data = chunk
    .split(/\r?\n/)
    .filter((line) => line.startsWith('data:'))
    .map((line) => line.slice(5).trimStart())
    .join('\n')
    .trim()
  if (!data || data === '[DONE]') return null
  return JSON.parse(data) as CodexResponseStreamEvent
}
