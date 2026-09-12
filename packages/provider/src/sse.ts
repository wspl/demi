import { ProviderDataError } from './validation'

export interface ServerSentEvent {
  event: string | null
  data: string
}

/** Reads complete SSE frames; provider adapters own JSON and terminal events. */
export async function* readServerSentEvents(
  body: ReadableStream<Uint8Array> | null,
  signal?: AbortSignal,
): AsyncIterable<ServerSentEvent> {
  if (!body) {
    throw new ProviderDataError('Provider SSE stream', 'missing response body')
  }
  const reader = body.getReader()
  const decoder = new TextDecoder('utf-8', { fatal: true })
  let line = ''
  let skipLf = false
  let eventName: string | null = null
  let data: string[] = []
  let cancellation: Promise<void> | undefined

  const cancel = (): Promise<void> => {
    cancellation ??= reader.cancel().catch(() => {
      // Cancellation can reject for an already errored body. Reading reports
      // that error; cleanup must still release the reader and abort listener.
    })
    return cancellation
  }
  const onAbort = (): void => {
    void cancel()
  }

  const parseLines = function* (text: string): Iterable<ServerSentEvent> {
    for (const character of text) {
      if (skipLf) {
        skipLf = false
        if (character === '\n')
          continue
      }
      if (character !== '\r' && character !== '\n') {
        line += character
        continue
      }
      skipLf = character === '\r'
      const currentLine = line
      line = ''
      if (currentLine === '') {
        const frame = data.length > 0
          ? { event: eventName, data: data.join('\n') }
          : null
        eventName = null
        data = []
        if (frame)
          yield frame
        continue
      }
      const colon = currentLine.indexOf(':')
      const field = colon === -1 ? currentLine : currentLine.slice(0, colon)
      let value = colon === -1 ? '' : currentLine.slice(colon + 1)
      if (value.startsWith(' '))
        value = value.slice(1)
      if (field === 'event')
        eventName = value || null
      else if (field === 'data')
        data.push(value)
    }
  }

  signal?.addEventListener('abort', onAbort, { once: true })
  try {
    while (true) {
      signal?.throwIfAborted()
      const { value, done } = await reader.read()
      signal?.throwIfAborted()
      let text: string
      try {
        text = done ? decoder.decode() : decoder.decode(value, { stream: true })
      } catch {
        throw new ProviderDataError('Provider SSE stream', 'invalid UTF-8')
      }
      for (const frame of parseLines(text)) {
        signal?.throwIfAborted()
        yield frame
      }
      if (done)
        break
    }
    // SSE dispatch requires a blank line. EOF never commits a partial frame.
  } finally {
    signal?.removeEventListener('abort', onAbort)
    try {
      await cancel()
    } finally {
      reader.releaseLock()
    }
  }
}
