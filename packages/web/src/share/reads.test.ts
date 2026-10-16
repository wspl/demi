import { afterEach, expect, test } from 'bun:test'
import { watchEffect } from 'vue'
import type { AttachmentLookup } from '@demicodes/web-ui/markdown/types'
import { lookupAttachment } from '../api/attachments'
import type { SharedItem } from '@demicodes/web-ui/share/shared-items'
import { heldBlocks, type HeldTranscript } from '@demicodes/web-ui/transport/protocol'
import { EMPTY_SHARE, createShareReads, shareBlobUrl, sharePath } from './reads'

// A shared page's attachments (`web-api-conversations.md` § Sharing): each is asked of the
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

test('a share’s page holds each run of steps not shared in its place, and reads the page before from its first item', async () => {
  const model = { providerId: 'p', model: { id: 'm', name: 'M', contextWindow: 1000, outputLimit: null, thinking: [{ type: 'disabled' }], acceptedExtensions: null }, thinking: null, serviceTierId: null }
  const createdAt = '2026-10-15T12:00:00.000Z'
  const asked: string[] = []
  globalThis.fetch = (async (input: RequestInfo | URL) => {
    asked.push(String(input))
    return Response.json({
      start: 2,
      length: 5,
      blocks: [
        { type: 'user', id: 'u-2', turnId: 't-2', createdAt, model, content: [{ type: 'text', text: 'Fix it.' }], preamble: null },
        { type: 'steps_not_shared', count: 12 },
        { type: 'text', id: 'a-2', createdAt, model, text: 'Fixed.' },
      ],
      instructions: [],
      summaries: [],
    })
  }) as typeof fetch
  let history: HeldTranscript<SharedItem> = EMPTY_SHARE
  const reads = createShareReads(TOKEN, () => new AbortController().signal, () => history, (held) => {
    history = held
  })

  await reads.readLatest()
  await reads.readBefore()

  expect(heldBlocks(history).map((item) => (item.type === 'steps_not_shared' ? `steps ${item.count}` : item.id)))
    .toEqual(['u-2', 'steps 12', 'a-2'])
  expect(asked).toEqual([
    `/api/shares/${TOKEN}/transcript?`,
    `/api/shares/${TOKEN}/transcript?before=2&edge=u-2`,
  ])
})
