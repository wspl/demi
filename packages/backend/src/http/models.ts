import { modelSelectionFromCatalog } from '@demicodes/provider'
import { Hono } from 'hono'
import type { AuthEnv } from '../auth/identity'
import type { ProviderAssembly } from '../llm/assembly'
import type { ProviderVault } from '../vault/providers'
import {
  modelAvailability
} from '../llm/model-availability'
import { booleanQuerySchema } from './query'

/**
 * `/api/models` — the aggregated catalog of the caller's provider scope,
 * grouped by provider, independent of conversation execution targets.
 */
export function modelRoutes(options: {
  assembly: ProviderAssembly;
  vault: ProviderVault
}): Hono<AuthEnv> {
  const app = new Hono<AuthEnv>()
  app.get('/', async (c) => {
    const parsed = booleanQuerySchema.safeParse(c.req.query('refresh'))
    if (!parsed.success)
      return c.json({
        code: 'invalid_query',
        message: 'refresh must be true or false'
      }, 400)
    const providers = await options.assembly.catalog(
      await options.vault.ownerFor(c.get('user').id),
      parsed.data ?? false
    )
    return c.json({ providers: providers.map(provider => ({
        ...provider,
        models: provider.models.map(model => ({
          ...model,
          selection: modelSelectionFromCatalog(provider.providerId, model),
        })),
        availability: modelAvailability(provider)
      })) })
  })
  return app
}
