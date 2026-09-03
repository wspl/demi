import { Hono } from 'hono'
import type { AuthEnv, InstanceMode } from '../auth/identity'
import type { ProviderAssembly } from '../llm/assembly'
import { connectionOwner } from '../vault/scope'

/** `/api/models` — the aggregated catalog of the caller's connection scope, grouped by connection (live, never stored). */
export function modelRoutes(options: { assembly: ProviderAssembly; mode: InstanceMode }): Hono<AuthEnv> {
  const app = new Hono<AuthEnv>()
  app.get('/', async (c) => c.json({ connections: await options.assembly.catalog(connectionOwner(options.mode, c.get('user').id)) }))
  return app
}
