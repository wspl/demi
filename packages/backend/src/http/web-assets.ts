import { Hono } from 'hono'
import { serveStatic } from 'hono/bun'
import { extname, resolve } from 'node:path'

/**
 * Optional built SPA assets. API misses remain JSON errors; deep HTML
 * navigation falls back to the entry page.
 */
export function webAssetRoutes(directory: string | undefined): Hono {
  const app = new Hono()
  if (!directory)
    return app
  const root = resolve(directory)
  const files = serveStatic({ root })
  const index = serveStatic({ root, path: 'index.html' })
  app.get('*', async (c, next) => {
    if (c.req.path === '/api' || c.req.path.startsWith('/api/'))
      return next()
    const file = await files(c, async () => {})
    if (file)
      return file
    if (!extname(c.req.path) && c.req.header('Accept')?.includes('text/html'))
      return index(c, next)
    return next()
  })
  return app
}
