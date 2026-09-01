import { Hono } from 'hono'
import type { Provider, ProviderModel } from '@demicodes/provider'

/** `/api/models` — the aggregated catalog, grouped by connection. */
export function modelRoutes(providers: Provider[]): Hono {
  const app = new Hono()
  app.get('/', async (c) => {
    const connections = await Promise.all(
      providers.map(async (provider) => ({
        connectionId: provider.id,
        displayName: provider.displayName,
        requiresProcessCapableHost: provider.requiresProcessCapableHost ?? false,
        models: ((await provider.listModels?.()) ?? { models: [] as ProviderModel[] }).models,
      })),
    )
    return c.json({ connections })
  })
  return app
}
