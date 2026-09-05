import { errorMessage } from '@demicodes/utils'
import { Hono, type Context } from 'hono'
import { z } from 'zod'
import type { AuthEnv, InstanceMode } from '../auth/identity'
import type { ProviderAssembly } from '../llm/assembly'
import type { VendorCatalog } from '../llm/vendors'
import { canConfigureProviders, providerOwner } from '../vault/scope'
import type { ApiKeyProviderConfig, ProviderEntry, ProviderVault } from '../vault/providers'
import type { SubscriptionLoginFlows } from '../vault/subscription-login'

const subscriptionLoginBodySchema = z.object({
  providerType: z.string().min(1),
  label: z.string().min(1).optional(),
})

const modelIdsSchema = z.array(z.string().min(1)).min(1)

/**
 * An API-key entry comes in two shapes: from the vendor catalog — the
 * vendor's family and endpoint prefilled, its model list live unless the
 * user types one — or a custom endpoint naming the family itself.
 */
const createProviderBodySchema = z.union([
  z.object({
    vendorId: z.string().min(1),
    label: z.string().min(1),
    apiKey: z.string().min(1),
    baseUrl: z.string().url().optional(),
    modelIds: modelIdsSchema.optional(),
  }),
  z.object({
    providerType: z.string().min(1),
    label: z.string().min(1),
    apiKey: z.string().min(1),
    wireApi: z.enum(['responses', 'chat-completions']).optional(),
    baseUrl: z.string().url().optional(),
    modelIds: modelIdsSchema.optional(),
  }),
])

/** Edits: the label of any entry; endpoint, key and model list of an API-key entry (`modelIds: null` returns to the live list). */
const patchProviderBodySchema = z.object({
  label: z.string().min(1).optional(),
  apiKey: z.string().min(1).optional(),
  baseUrl: z.string().url().nullable().optional(),
  modelIds: modelIdsSchema.nullable().optional(),
})

/**
 * `/api/providers` — the vault surface in the caller's provider scope
 * (`vault/scope.ts`): the instance's providers in shared mode, where only
 * admins configure providers; the caller's own in isolated mode. Responses
 * never carry key material: an entry lists as its family, label, endpoint,
 * vendor and model list.
 */
export function providerRoutes(options: {
  vault: ProviderVault
  assembly: ProviderAssembly
  vendors: VendorCatalog
  logins: SubscriptionLoginFlows
  mode: InstanceMode
}): Hono<AuthEnv> {
  const { vault, assembly, vendors, logins, mode } = options
  const app = new Hono<AuthEnv>()

  const ownerOf = (c: Context<AuthEnv>) => providerOwner(mode, c.get('user').id)
  // A provider outside the caller's scope answers like a missing one.
  const scoped = async (c: Context<AuthEnv>) => {
    const provider = await vault.get(c.req.param('id') ?? '')
    return provider && provider.ownerUserId === ownerOf(c) ? provider : null
  }
  const invalidBody = (c: Context<AuthEnv>, error: z.ZodError) => {
    const issue = error.issues[0]
    return c.json(
      { code: 'invalid_body', message: `Invalid request body${issue ? `: ${issue.path.join('.')} ${issue.message}` : ''}` },
      400,
    )
  }

  // Configuring providers — creating, editing, logging in, testing, deleting —
  // is the admin's in shared mode and everyone's own in isolated mode.
  app.use('*', async (c, next) => {
    if (c.req.method !== 'GET' && !canConfigureProviders(mode, c.get('user').role)) {
      return c.json({ code: 'forbidden', message: 'Providers are configured by administrators on this instance' }, 403)
    }
    await next()
  })

  // Literal paths are registered before `/:id` so they win.

  /** What the page can add: the models.dev vendors our runtimes speak to, and each subscription family with whether the scope holds it. */
  app.get('/catalog', async (c) => {
    const entries = await vault.list({ ownerUserId: ownerOf(c) })
    const subscriptions = assembly.typesOf('subscription').map((providerType) => ({
      providerType,
      configured: entries.some((entry) => entry.config.providerType === providerType),
    }))
    try {
      return c.json({ subscriptions, vendors: await vendors.list() })
    } catch (error) {
      return c.json({ code: 'catalog_unavailable', message: errorMessage(error) }, 502)
    }
  })

  app.post('/subscription-login', async (c) => {
    const parsed = subscriptionLoginBodySchema.safeParse(await c.req.json().catch(() => null))
    if (!parsed.success) return c.json({ code: 'invalid_body', message: 'Expected { providerType, label? }' }, 400)
    const { providerType } = parsed.data
    if (!assembly.credentialOf(providerType)) {
      return c.json({ code: 'unknown_provider_type', message: `Unknown provider type "${providerType}"` }, 400)
    }
    const started = await logins.start(providerType, parsed.data.label ?? `${providerType} subscription`, ownerOf(c))
    if ('refused' in started) {
      return started.refused === 'exists'
        ? c.json({ code: 'provider_exists', message: `This scope already has a ${providerType} subscription` }, 409)
        : c.json({ code: 'no_login_flow', message: `Provider type "${providerType}" has no native login flow` }, 400)
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
    if (!parsed.success) return invalidBody(c, parsed.error)
    const body = parsed.data
    let config: ApiKeyProviderConfig
    if ('vendorId' in body) {
      const vendor = await vendors.get(body.vendorId)
      if (!vendor) return c.json({ code: 'unknown_vendor', message: `Unknown vendor "${body.vendorId}"` }, 400)
      const baseUrl = body.baseUrl ?? vendor.baseUrl ?? undefined
      config = {
        kind: 'api_key',
        providerType: vendor.providerType,
        apiKey: body.apiKey,
        vendorId: vendor.id,
        ...(vendor.wireApi ? { wireApi: vendor.wireApi } : {}),
        ...(baseUrl ? { baseUrl } : {}),
        ...(body.modelIds ? { modelIds: body.modelIds } : {}),
      }
    } else {
      const credential = assembly.credentialOf(body.providerType)
      if (!credential) {
        return c.json({ code: 'unknown_provider_type', message: `Unknown provider type "${body.providerType}"` }, 400)
      }
      if (credential !== 'api_key') {
        return c.json({ code: 'subscription_only', message: `Provider type "${body.providerType}" is configured by its login flow` }, 400)
      }
      config = {
        kind: 'api_key',
        providerType: body.providerType,
        apiKey: body.apiKey,
        ...(body.wireApi ? { wireApi: body.wireApi } : {}),
        ...(body.baseUrl ? { baseUrl: body.baseUrl } : {}),
        ...(body.modelIds ? { modelIds: body.modelIds } : {}),
      }
    }
    const provider = await vault.create({ ownerUserId: ownerOf(c), label: body.label, config })
    return c.json({ provider: redact(provider) }, 201)
  })

  app.patch('/:id', async (c) => {
    const provider = await scoped(c)
    if (!provider) return c.json({ code: 'provider_not_found', message: 'No such provider' }, 404)
    const parsed = patchProviderBodySchema.safeParse(await c.req.json().catch(() => null))
    if (!parsed.success) return invalidBody(c, parsed.error)
    const body = parsed.data
    const editsConfig = body.apiKey !== undefined || body.baseUrl !== undefined || body.modelIds !== undefined
    if (editsConfig && provider.config.kind !== 'api_key') {
      return c.json({ code: 'subscription_only', message: 'A subscription entry only takes a new label' }, 400)
    }
    const config: ApiKeyProviderConfig | undefined =
      editsConfig && provider.config.kind === 'api_key'
        ? {
            ...provider.config,
            ...(body.apiKey !== undefined ? { apiKey: body.apiKey } : {}),
            ...(body.baseUrl !== undefined ? { baseUrl: body.baseUrl ?? undefined } : {}),
            ...(body.modelIds !== undefined ? { modelIds: body.modelIds ?? undefined } : {}),
          }
        : undefined
    const updated = await vault.update(provider.id, {
      ...(body.label !== undefined ? { label: body.label } : {}),
      ...(config ? { config } : {}),
    })
    if (!updated) return c.json({ code: 'provider_not_found', message: 'No such provider' }, 404)
    assembly.invalidate(provider.id)
    return c.json({ provider: redact(updated) })
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
  const keyed = config.kind === 'api_key' ? config : null
  return {
    id: provider.id,
    kind: config.kind,
    providerType: config.providerType,
    label: provider.label,
    wireApi: keyed?.wireApi ?? null,
    vendorId: keyed?.vendorId ?? null,
    baseUrl: keyed?.baseUrl ?? null,
    modelIds: keyed?.modelIds ?? null,
    createdAt: provider.createdAt,
  }
}
