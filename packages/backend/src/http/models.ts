import { Hono } from 'hono'
import type { ProviderAssembly } from '../llm/assembly'

/** `/api/models` — the aggregated catalog, grouped by connection (live, never stored). */
export function modelRoutes(assembly: ProviderAssembly): Hono {
  const app = new Hono()
  app.get('/', async (c) => c.json({ connections: await assembly.catalog() }))
  return app
}
