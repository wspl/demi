import { Hono } from 'hono'
import type { InstanceMode } from '../auth/identity'

/** `GET /api/settings` — the instance mode, read-only: it is startup configuration (`product.md` § Instance mode). */
export function settingsRoutes(options: { mode: InstanceMode }): Hono {
  const app = new Hono()
  app.get('/', (c) => c.json({ mode: options.mode }))
  return app
}
