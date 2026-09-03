import { errorMessage } from '@demicodes/utils'
import { Hono, type Context } from 'hono'
import { z } from 'zod'
import type { AuthEnv, InstanceMode } from '../auth/identity'
import type { ProviderAssembly } from '../llm/assembly'
import { canConfigureProviders, providerOwner } from '../vault/scope'
import type { ProviderEntry, ProviderVault } from '../vault/providers'
import type { SubscriptionLoginFlows } from '../vault/subscription-login'

const subscriptionLoginBodySchema = z.object({
  providerType: z.string().min(1),
  label: z.string().min(1).optional(),
})

const createProviderBodySchema = z.object({
  providerType: z.string().min(1),
  label: z.string().min(1),
  apiKey: z.string().min(1),
  baseUrl: z.string().url().optional(),
  modelIds: z.array(z.string().min(1)).min(1).optional(),
})

/**
 * `/api/providers` — the vault surface in the caller's provider scope
 * (`vault/scope.ts`): the instance's providers in shared mode, where only
 * admins configure providers; the caller's own in isolated mode. Responses
 * never carry key material: a provider lists as its type, label,
 * endpoint, and model list.
 */
export function providerRoutes(options: {
  vault: ProviderVault
  assembly: ProviderAssembly
  logins: SubscriptionLoginFlows
  mode: InstanceMode
}): Hono<AuthEnv> {
  const { vault, assembly, logins, mode } = options
  const app = new Hono<AuthEnv>()

  const ownerOf = (c: Context<AuthEnv>) => providerOwner(mode, c.get('user').id)
  // A provider outside the caller's scope answers like a missing one.
  const scoped = async (c: Context<AuthEnv>) => {
    const provider = await vault.get(c.req.param('id') ?? '')
    return provider && provider.ownerUserId === ownerOf(c) ? provider : null
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
    if (!parsed.success) return c.json({ code: 'invalid_body', message: 'Expected { providerType, label? }' }, 400)
    if (!assembly.hasType(parsed.data.providerType)) {
      return c.json({ code: 'unknown_provider_type', message: `Unknown provider type "${parsed.data.providerType}"` }, 400)
    }
    const started = logins.start(parsed.data.providerType, parsed.data.label ?? `${parsed.data.providerType} subscription`, ownerOf(c))
    if (!started) {
      return c.json({ code: 'no_login_flow', message: `Provider type "${parsed.data.providerType}" has no native login flow` }, 400)
    }
    return c.json({ login: { id: started.id, status: 'pending' } }, 202)
  })

  app.get('/subscription-login/:id', async (c) => {
    const state = logins.status(c.req.param('id') ?? '', ownerOf(c))
    if (!state) return c.json({ code: 'login_not_found', message: 'No such login flow' }, 404)
    return c.json({ login: state })
  })

  app.get('/', async (c) => {
    const providers = await vault.list({ ownerUserId: ownerOf(c) })
    return c.json({ providers: providers.map(redact) })
  })

  app.post('/', async (c) => {
    const parsed = createProviderBodySchema.safeParse(await c.req.json().catch(() => null))
    if (!parsed.success) {
      const issue = parsed.error.issues[0]
      return c.json(
        { code: 'invalid_body', message: `Invalid request body${issue ? `: ${issue.path.join('.')} ${issue.message}` : ''}` },
        400,
      )
    }
    const body = parsed.data
    if (!assembly.hasType(body.providerType)) {
      return c.json({ code: 'unknown_provider_type', message: `Unknown provider type "${body.providerType}"` }, 400)
    }
    const provider = await vault.create({
      ownerUserId: ownerOf(c),
      label: body.label,
      config: {
        kind: 'api_key',
        providerType: body.providerType,
        apiKey: body.apiKey,
        ...(body.baseUrl ? { baseUrl: body.baseUrl } : {}),
        ...(body.modelIds ? { modelIds: body.modelIds } : {}),
      },
    })
    return c.json({ provider: redact(provider) }, 201)
  })

  app.delete('/:id', async (c) => {
    const provider = await scoped(c)
    if (!provider) return c.json({ code: 'provider_not_found', message: 'No such provider' }, 404)
    await vault.delete(provider.id)
    await assembly.deleteProviderState(provider.id)
    return c.body(null, 204)
  })

  app.post('/:id/test', async (c) => {
    const provider = await scoped(c)
    if (!provider) return c.json({ code: 'provider_not_found', message: 'No such provider' }, 404)
    try {
      const result = await assembly.testProvider(provider.id)
      return c.json(result, result.ok ? 200 : 502)
    } catch (error) {
      return c.json({ ok: false, message: errorMessage(error) }, 502)
    }
  })

  return app
}

function redact(provider: ProviderEntry) {
  const { config } = provider
  return {
    id: provider.id,
    kind: config.kind,
    providerType: config.providerType,
    label: provider.label,
    baseUrl: config.kind === 'api_key' ? (config.baseUrl ?? null) : null,
    modelIds: config.kind === 'api_key' ? (config.modelIds ?? null) : null,
    createdAt: provider.createdAt,
  }
}
