import { FileBrowserError, type FileSearchEvent } from '@demicodes/web-ui/files/types'
import { invalidResponse } from './client'
import { searchLineSchema } from './generated/web-api'
import { t } from './locales/text'

/**
 * A search's answer as the file views read it, over the relay or a direct
 * channel alike (`crates/runner-protocol/src/search.rs`): `bytes` carry one
 * JSON object per line, and an empty line only says the search still runs.
 * Output that ends before the search's end line was cut short, which rejects.
 * Leaving the loop before the end cancels `bytes`, which stops the search.
 */
export async function* searchEvents(bytes: ReadableStream<Uint8Array>): AsyncGenerator<FileSearchEvent> {
  const reader = bytes.getReader()
  const decoder = new TextDecoder()
  let rest = ''
  let ended = false
  try {
    for (;;) {
      const { done, value } = await reader.read()
      if (done)
        break
      rest += decoder.decode(value, { stream: true })
      for (let end = rest.indexOf('\n'); end >= 0; end = rest.indexOf('\n')) {
        const line = rest.slice(0, end)
        rest = rest.slice(end + 1)
        if (line === '')
          continue
        const parsed = searchLineSchema.safeParse(JSON.parse(line))
        if (!parsed.success)
          throw invalidResponse(parsed.error)
        // A file the text was found in by its name alone may leave its line out.
        yield parsed.data.type === 'file' ? { ...parsed.data, line: parsed.data.line ?? null } : parsed.data
        if (parsed.data.type === 'end') {
          // The answer ends with it: read to its end, so the browser counts it whole, not cut.
          while (!(await reader.read()).done) {}
          ended = true
          return
        }
      }
    }
  } finally {
    if (!ended)
      void reader.cancel().catch(() => {})
  }
  if (!ended)
    throw new FileBrowserError('other', t('files.searchCut'))
}
