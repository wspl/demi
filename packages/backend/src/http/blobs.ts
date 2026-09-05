import type { BlobStore } from '@demicodes/agent'
import { Hono } from 'hono'
import type { AuthEnv } from '../auth/identity'

/**
 * The media types a blob is served under for the page to render in place.
 * A blob's bytes are whoever uploaded them and its hash is predictable, so
 * anything else — text/html above all — leaves as an opaque download and
 * nothing a blob holds ever runs in this origin.
 */
const INLINE_MEDIA_TYPES = new Set([
  'image/png', 'image/jpeg', 'image/gif', 'image/webp', 'image/avif',
  'video/mp4', 'video/webm', 'video/quicktime',
  'audio/mpeg', 'audio/mp4', 'audio/ogg', 'audio/wav', 'audio/webm',
  'application/pdf',
])

/**
 * `GET /api/blobs/:sha256` — the bytes behind a media reference
 * (`backend.md` § Media by reference). Transcript frames carry
 * `source.ref`; the page fetches the blob here. Content-addressed, so the
 * response is immutable and cacheable for as long as the browser likes.
 */
export function blobRoutes(options: { blobsFor: (userId: string) => BlobStore }): Hono<AuthEnv> {
  const app = new Hono<AuthEnv>()

  app.get('/:sha256', async (c) => {
    const sha256 = c.req.param('sha256')
    if (!/^[0-9a-f]{64}$/.test(sha256)) return c.json({ code: 'not_found', message: 'No such blob' }, 404)
    const bytes = await options.blobsFor(c.get('user').id).get(sha256)
    if (!bytes) return c.json({ code: 'not_found', message: 'No such blob' }, 404)
    const requested = c.req.query('type')?.toLowerCase()
    const inline = requested !== undefined && INLINE_MEDIA_TYPES.has(requested) ? requested : null
    return new Response(bytes, {
      status: 200,
      headers: {
        'content-type': inline ?? 'application/octet-stream',
        ...(inline ? {} : { 'content-disposition': 'attachment' }),
        'x-content-type-options': 'nosniff',
        'cache-control': 'private, max-age=31536000, immutable',
        vary: 'Cookie',
      },
    })
  })

  return app
}
