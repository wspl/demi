import { Hono } from 'hono'
import type { AuthEnv, InstanceMode } from '../auth/identity'
import type { ProviderAssembly } from '../llm/assembly'
import { providerOwner } from '../vault/scope'

/** `/api/models` — the aggregated catalog of the caller's provider scope, grouped by provider (live, never stored). */
export function modelRoutes(options: { assembly: ProviderAssembly; mode: InstanceMode }): Hono<AuthEnv> {
  const app = new Hono<AuthEnv>()
  app.get('/', async (c) => c.json({ providers: await options.assembly.catalog(providerOwner(options.mode, c.get('user').id)) }))
  return app
}
