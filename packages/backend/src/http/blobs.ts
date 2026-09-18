import type { BlobStore } from '@demicodes/agent'
import { Hono } from 'hono'
import type { AuthEnv } from '../auth/identity'
import { contentHeaders } from './file-transfer'

/**
 * `GET /api/blobs/:sha256` — the bytes behind a media reference
 * (`backend.md` § Media by reference). Transcript frames carry
 * `source.ref`; the page fetches the blob here. Content-addressed, so the
 * response is immutable and cacheable for as long as the browser likes.
 */
export function blobRoutes(
  options: { blobsFor: (userId: string) => BlobStore }
): Hono<AuthEnv> {
  const app = new Hono<AuthEnv>()

  app.get('/:sha256', async (c) => {
    const sha256 = c.req.param('sha256')
    if (!/^[0-9a-f]{64}$/.test(sha256))
      return c.json({ code: 'not_found', message: 'No such blob' }, 404)
    const bytes = await options.blobsFor(c.get('user').id).get(sha256)
    if (!bytes)
      return c.json({ code: 'not_found', message: 'No such blob' }, 404)
    // A blob's bytes are whoever uploaded them and its hash is predictable:
    // a type the page does not show in place, text/html above all, leaves as
    // an opaque download, so nothing a blob holds ever runs in this origin.
    return new Response(bytes, {
      status: 200,
      headers: {
        ...contentHeaders(c.req.query('type') ?? null),
        'cache-control': 'private, max-age=31536000, immutable',
        vary: 'Cookie',
      },
    })
  })

  return app
}
