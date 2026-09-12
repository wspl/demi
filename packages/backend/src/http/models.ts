import { z } from 'zod'
import { modelSelectionFromCatalog } from '@demicodes/provider'
import { Hono } from 'hono'
import type { AuthEnv, InstanceMode } from '../auth/identity'
import type { ProviderAssembly } from '../llm/assembly'
import {
  modelAvailability
} from '../llm/model-availability'
import { providerOwner } from '../vault/scope'

/**
 * `/api/models` — the aggregated catalog of the caller's provider scope,
 * grouped by provider, independent of conversation execution targets.
 */
export function modelRoutes(options: {
  assembly: ProviderAssembly;
  mode: InstanceMode
}): Hono<AuthEnv> {
  const app = new Hono<AuthEnv>()
  app.get('/', async (c) => {
    const parsed = z.enum(['true', 'false'])
      .optional()
      .safeParse(c.req.query('refresh'))
    if (!parsed.success)
      return c.json({
        code: 'invalid_query',
        message: 'refresh must be true or false'
      }, 400)
    const providers = await options.assembly.catalog(
      providerOwner(options.mode, c.get('user').id),
      parsed.data === 'true'
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
