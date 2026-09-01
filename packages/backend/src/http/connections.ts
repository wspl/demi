import { errorMessage } from '@demicodes/utils'
import { Hono } from 'hono'
import { z } from 'zod'
import { STUB_USER } from '../auth/identity'
import type { ProviderAssembly } from '../llm/assembly'
import type { Connection, ConnectionVault } from '../vault/connections'

const createConnectionBodySchema = z.object({
  type: z.string().min(1),
  label: z.string().min(1),
  apiKey: z.string().min(1),
  baseUrl: z.string().url().optional(),
  modelIds: z.array(z.string().min(1)).min(1).optional(),
})

/**
 * `/api/connections` — the vault surface. Responses never carry key
 * material: a connection lists as its type, label, endpoint, and model list.
 */
export function connectionRoutes(options: { vault: ConnectionVault; assembly: ProviderAssembly }): Hono {
  const { vault, assembly } = options
  const app = new Hono()

  app.get('/', async (c) => {
    const connections = await vault.list()
    return c.json({ connections: connections.map(redact) })
  })

  app.post('/', async (c) => {
    const parsed = createConnectionBodySchema.safeParse(await c.req.json().catch(() => null))
    if (!parsed.success) {
      const issue = parsed.error.issues[0]
      return c.json(
        { code: 'invalid_body', message: `Invalid request body${issue ? `: ${issue.path.join('.')} ${issue.message}` : ''}` },
        400,
      )
    }
    const body = parsed.data
    if (!assembly.hasType(body.type)) {
      return c.json({ code: 'unknown_provider_type', message: `Unknown provider type "${body.type}"` }, 400)
    }
    const connection = await vault.create({
      ownerUserId: STUB_USER.id,
      label: body.label,
      config: {
        kind: 'api_key',
        provider: body.type,
        apiKey: body.apiKey,
        ...(body.baseUrl ? { baseUrl: body.baseUrl } : {}),
        ...(body.modelIds ? { modelIds: body.modelIds } : {}),
      },
    })
    return c.json({ connection: redact(connection) }, 201)
  })

  app.delete('/:id', async (c) => {
    const connection = await vault.get(c.req.param('id'))
    if (!connection) return c.json({ code: 'connection_not_found', message: 'No such connection' }, 404)
    await vault.delete(connection.id)
    assembly.invalidate(connection.id)
    return c.body(null, 204)
  })

  app.post('/:id/test', async (c) => {
    const connection = await vault.get(c.req.param('id'))
    if (!connection) return c.json({ code: 'connection_not_found', message: 'No such connection' }, 404)
    try {
      const result = await assembly.testConnection(connection.id)
      return c.json(result, result.ok ? 200 : 502)
    } catch (error) {
      return c.json({ ok: false, message: errorMessage(error) }, 502)
    }
  })

  return app
}

function redact(connection: Connection) {
  return {
    id: connection.id,
    type: connection.config.provider,
    label: connection.label,
    baseUrl: connection.config.baseUrl ?? null,
    modelIds: connection.config.modelIds ?? null,
    createdAt: connection.createdAt,
  }
}
