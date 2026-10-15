import { afterEach, expect, test } from 'bun:test'
import { watchEffect } from 'vue'
import type { AttachmentLookup } from '@demicodes/web-ui/markdown/types'
import { lookupAttachment } from '../api/attachments'
import { shareBlobUrl, sharePath } from './reads'

// A shared page's attachments (`web-api.md` § Sharing): each is asked of the
// share's own route and loads from the share's blob route, and one the
// snapshot lacks shows missing, as in the conversation. The backend is a
// stubbed fetch. A few milliseconds.

const TOKEN = 'Ndu-Yvw4jRKCNbvsZgFrow'
const BLOB = 'c'.repeat(64)
const realFetch = globalThis.fetch

afterEach(() => {
  globalThis.fetch = realFetch
})

/** Resolves with attachment `id`'s lookup once it is no longer loading. */
function answered(id: string, source: Parameters<typeof lookupAttachment>[0]): Promise<AttachmentLookup> {
  return new Promise((resolve) => {
    const stop = watchEffect(() => {
      const lookup = lookupAttachment(source, id)
      if (lookup.state !== 'loading') {
        queueMicrotask(() => stop())
        resolve(lookup)
      }
    })
  })
}

test('a share’s attachment loads from the share’s routes, and one the snapshot lacks is missing', async () => {
  const asked: string[] = []
  globalThis.fetch = (async (input: RequestInfo | URL) => {
    const path = String(input)
    asked.push(path)
    return path.endsWith('/attachments/a1')
      ? Response.json({ id: 'a1', name: 'badge.svg', mediaType: 'image/svg+xml', size: 4, blob: BLOB })
      : Response.json({ requestId: 'req_test000000000002', code: 'share_not_found', message: 'This shared link does not exist' }, { status: 404 })
  }) as typeof fetch
  const source = { path: sharePath(TOKEN), blobUrl: shareBlobUrl(TOKEN) }

  const found = await answered('a1', source)
  const missing = await answered('a2', source)

  expect(found.state === 'found' && found.attachment.url).toBe(`/api/shares/${TOKEN}/blobs/${BLOB}?type=image%2Fsvg%2Bxml`)
  expect(missing.state).toBe('missing')
  expect(asked).toEqual([`/api/shares/${TOKEN}/attachments/a1`, `/api/shares/${TOKEN}/attachments/a2`])
})
