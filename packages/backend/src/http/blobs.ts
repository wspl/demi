import type { BlobStore } from '@demicodes/agent'
import { Hono } from 'hono'
import type { AuthEnv } from '../auth/identity'

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
    return new Response(bytes, {
      status: 200,
      headers: {
        'content-type': c.req.query('type') ?? 'application/octet-stream',
        'cache-control': 'private, max-age=31536000, immutable',
        vary: 'Cookie',
      },
    })
  })

  return app
}
