import { createHash } from 'node:crypto'
import { Hono } from 'hono'
import type { AuthEnv } from '../auth/identity'
import type { ProductState } from '../sync/product-state'

/** Conditional snapshot polling synchronizes pages; chat streaming remains on the agent protocol. */
export function stateRoutes(state: ProductState): Hono<AuthEnv> {
  const app = new Hono<AuthEnv>()
  app.get('/', async c => {
    const body = JSON.stringify(await state.read(c.get('user')))
    const etag = `"${createHash('sha256').update(body).digest('hex')}"`
    c.header('Cache-Control', 'private, no-cache')
    c.header('ETag', etag)
    if (c.req.header('If-None-Match') === etag) return c.body(null, 304)
    return c.body(body, 200, { 'Content-Type': 'application/json' })
  })
  return app
}
