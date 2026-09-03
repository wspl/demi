import { errorMessage } from '@demicodes/utils'
import { Hono, type Context } from 'hono'
import { z } from 'zod'
import type { AuthEnv, InstanceMode } from '../auth/identity'
import type { ProviderAssembly } from '../llm/assembly'
import { canConfigureProviders, connectionOwner } from '../vault/scope'
import type { Connection, ConnectionVault } from '../vault/connections'
import type { SubscriptionLoginFlows } from '../vault/subscription-login'

const subscriptionLoginBodySchema = z.object({
  type: z.string().min(1),
  label: z.string().min(1).optional(),
})

const createConnectionBodySchema = z.object({
  type: z.string().min(1),
  label: z.string().min(1),
  apiKey: z.string().min(1),
  baseUrl: z.string().url().optional(),
  modelIds: z.array(z.string().min(1)).min(1).optional(),
})

/**
 * `/api/connections` — the vault surface in the caller's connection scope
 * (`vault/scope.ts`): the instance's connections in shared mode, where only
 * admins configure providers; the caller's own in isolated mode. Responses
 * never carry key material: a connection lists as its type, label,
 * endpoint, and model list.
 */
export function connectionRoutes(options: {
  vault: ConnectionVault
  assembly: ProviderAssembly
  logins: SubscriptionLoginFlows
  mode: InstanceMode
}): Hono<AuthEnv> {
  const { vault, assembly, logins, mode } = options
  const app = new Hono<AuthEnv>()

  const ownerOf = (c: Context<AuthEnv>) => connectionOwner(mode, c.get('user').id)
  // A connection outside the caller's scope answers like a missing one.
  const scoped = async (c: Context<AuthEnv>) => {
    const connection = await vault.get(c.req.param('id') ?? '')
    return connection && connection.ownerUserId === ownerOf(c) ? connection : null
  }

  // Configuring providers — creating, logging in, testing, deleting — is
  // the admin's in shared mode and everyone's own in isolated mode.
  app.use('*', async (c, next) => {
    if (c.req.method !== 'GET' && !canConfigureProviders(mode, c.get('user').role)) {
      return c.json({ code: 'forbidden', message: 'Providers are configured by administrators on this instance' }, 403)
    }
    await next()
  })

  // Registered before `/:id` so the literal path wins.
  app.post('/subscription-login', async (c) => {
    const parsed = subscriptionLoginBodySchema.safeParse(await c.req.json().catch(() => null))
    if (!parsed.success) return c.json({ code: 'invalid_body', message: 'Expected { type, label? }' }, 400)
    if (!assembly.hasType(parsed.data.type)) {
      return c.json({ code: 'unknown_provider_type', message: `Unknown provider type "${parsed.data.type}"` }, 400)
    }
    const started = logins.start(parsed.data.type, parsed.data.label ?? `${parsed.data.type} subscription`, ownerOf(c))
    if (!started) {
      return c.json({ code: 'no_login_flow', message: `Provider type "${parsed.data.type}" has no native login flow` }, 400)
    }
    return c.json({ login: { id: started.id, status: 'pending' } }, 202)
  })

  app.get('/subscription-login/:id', async (c) => {
    const state = logins.status(c.req.param('id') ?? '', ownerOf(c))
    if (!state) return c.json({ code: 'login_not_found', message: 'No such login flow' }, 404)
    return c.json({ login: state })
  })

  app.get('/', async (c) => {
    const connections = await vault.list({ ownerUserId: ownerOf(c) })
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
      ownerUserId: ownerOf(c),
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
    const connection = await scoped(c)
    if (!connection) return c.json({ code: 'connection_not_found', message: 'No such connection' }, 404)
    await vault.delete(connection.id)
    await assembly.deleteConnectionState(connection.id)
    return c.body(null, 204)
  })

  app.post('/:id/test', async (c) => {
    const connection = await scoped(c)
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
  const { config } = connection
  return {
    id: connection.id,
    kind: config.kind,
    type: config.provider,
    label: connection.label,
    baseUrl: config.kind === 'api_key' ? (config.baseUrl ?? null) : null,
    modelIds: config.kind === 'api_key' ? (config.modelIds ?? null) : null,
    createdAt: connection.createdAt,
  }
}
