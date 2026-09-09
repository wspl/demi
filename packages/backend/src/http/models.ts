import { z } from 'zod'
import { Hono } from 'hono'
import type { AuthEnv, InstanceMode } from '../auth/identity'
import type { ProviderAssembly } from '../llm/assembly'
import { providerOwner } from '../vault/scope'

/** `/api/models` — the aggregated catalog of the caller's provider scope, grouped by provider (live, never stored). */
export function modelRoutes(options: { assembly: ProviderAssembly; mode: InstanceMode }): Hono<AuthEnv> {
  const app = new Hono<AuthEnv>()
  app.get('/', async (c) => {
    const parsed = z.enum(['true', 'false']).optional().safeParse(c.req.query('refresh'))
    if (!parsed.success) return c.json({ code: 'invalid_query', message: 'refresh must be true or false' }, 400)
    return c.json({ providers: await options.assembly.catalog(providerOwner(options.mode, c.get('user').id), parsed.data === 'true') })
  })
  return app
}
