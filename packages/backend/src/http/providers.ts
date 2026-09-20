import { publicProvider } from '../vault/public-provider'
import {
  AccountRefused,
  type ProviderAccounts
} from '../vault/provider-accounts'
import type { ProviderOperations } from '../vault/provider-operations'
import { errorMessage } from '@demicodes/utils'
import { Hono, type Context } from 'hono'
import { z } from 'zod'
import { publicQuota } from '../llm/provider-details'
import { configuredModelsSchema } from '../llm/model-config'
import type { AuthEnv, InstanceMode } from '../auth/identity'
import type { ProviderAssembly, SessionProviderContext } from '../llm/assembly'
import type { CliInstalls } from '../llm/cli-installs'
import { booleanQuerySchema } from './query'
import type { VendorCatalog } from '../llm/vendors'
import { canConfigureProviders } from '../vault/scope'
import type {
  ApiKeyProviderConfig,
  ProviderEntry,
  ProviderVault
} from '../vault/providers'
import type { SubscriptionLoginFlows } from '../vault/subscription-login'

const subscriptionLoginBodySchema = z.strictObject({
  providerType: z.string().min(1),
  label: z.string().min(1).optional(),
})


/**
 * An API-key entry comes in two shapes: from the vendor catalog — the
 * vendor's family and endpoint prefilled, its model list live unless the
 * user types one — or a custom endpoint naming the family itself.
 */
const createProviderBodySchema = z.union([
  z.strictObject({
    vendorId: z.string().min(1),
    label: z.string().min(1),
    apiKey: z.string().min(1),
    baseUrl: z.url().optional(),
    models: configuredModelsSchema.optional(),
  }),
  z.strictObject({
    providerType: z.string().min(1),
    label: z.string().min(1),
    apiKey: z.string().min(1),
    wireApi: z.enum(['responses', 'chat-completions']).optional(),
    baseUrl: z.url().optional(),
    models: configuredModelsSchema.optional(),
  }),
])

/**
 * Edits: the label of any entry; endpoint, key and model list of an API-key
 * entry (`models: null` returns to the live list).
 */
const patchProviderBodySchema = z.strictObject({
  label: z.string().min(1).optional(),
  apiKey: z.string().min(1).optional(),
  baseUrl: z.url().nullable().optional(),
  models: configuredModelsSchema.nullable().optional(),
})

/**
 * `/api/providers` — the vault surface in the caller's provider scope
 * (`vault/scope.ts`): the instance's providers in shared mode, where only
 * admins configure providers; the caller's own in isolated mode. Responses
 * never carry key material: an entry lists as its family, label, endpoint,
 * vendor and model list.
 */
export function providerRoutes(options: {
  accounts: ProviderAccounts
  operations: ProviderOperations
  vault: ProviderVault
  assembly: ProviderAssembly
  vendors: VendorCatalog
  logins: SubscriptionLoginFlows
  mode: InstanceMode
  /** A process provider's CLI: its versions and where it is installed (`claude-cli.md`). */
  cli: {
    installs: CliInstalls
    /** The vendor's newest version; `refresh` asks the vendor at once. */
    newest(refresh: boolean): Promise<string>
    /** Refuses a version the vendor does not publish. */
    verify(version: string): Promise<void>
    /** What each machine that can be asked now has installed. */
    machines(userId: string, signal: AbortSignal): Promise<Array<{
      deviceId: string
      name: string
      versions: string[] | null
    }>>
  }
  /** The machine for a process provider's work that belongs to no conversation. */
  placeAccountProcess: (
    userId: string,
    entry: ProviderEntry,
    signal: AbortSignal
  ) => Promise<{
    session: SessionProviderContext
    cwd: string
    release(): void
  }>
}): Hono<AuthEnv> {
  const { vault, assembly, vendors, logins, mode } = options
  const app = new Hono<AuthEnv>()
  app.onError((error, c) => {
    if (error instanceof AccountRefused)
      return c.json({ code: error.code, message: error.message }, error.status)
    throw error
  })

  const ownerOf = (c: Context<AuthEnv>) => vault.ownerFor(c.get('user').id)
  // A provider outside the caller's scope answers like a missing one.
  const scoped = async (c: Context<AuthEnv>) => {
    const provider = await vault.get(c.req.param('id') ?? '')
    return provider && provider.ownerUserId === await ownerOf(c) ? provider : null
  }
  // An account of a process provider is no use without its CLI, so adding
  // one starts the install where account work runs; nobody waits for it.
  const installCli = async (c: Context<AuthEnv>, entry: ProviderEntry) => {
    const provider = (await assembly.providerFor(entry.id))?.provider
    if (provider?.requiresProcessCapableHost)
      options.cli.installs.start(c.get('user').id, entry)
  }
  const accountBodySchema = z.object({
    credentialId: z.string().min(1).optional()
  })
  const invalidBody = (c: Context<AuthEnv>, error: z.ZodError) => {
    const issue = error.issues[0]
    return c.json(
      {
        code: 'invalid_body',
        message: `Invalid request body${issue ? `: ${issue.path.join('.')} ${issue.message}` : ''}`
      },
      400,
    )
  }

  // Configuring providers — creating, editing, logging in, testing, deleting —
  // is the master's in shared mode and everyone's own in isolated mode.
  app.use('*', async (c, next) => {
    const refreshingQuota = c.req.method === 'POST' &&
      /^\/api\/providers\/[^/]+\/quota$/.test(c.req.path)
    if (c.req.method !== 'GET' &&
      !canConfigureProviders(mode, c.get('user').role)) {
      return c.json({
        code: 'forbidden',
        message: 'Providers are configured by the instance owner'
      }, 403)
    }
    const id = c.req.path.split('/')[3]
    const providerMutation = id &&
      id !== 'subscription-login' &&
      id !== 'setup-token' &&
      c.req.method !== 'GET' &&
      !refreshingQuota &&
      !c.req.path.endsWith('/accounts/login')
    const release = providerMutation ? options.operations.reserve(id) : () => {}
    if (!release)
      return c.json({
        code: 'provider_busy',
        message: 'Another provider operation is still running'
      }, 409)
    try {
      await next()
    } finally {
      release()
    }
  })

  // Literal paths are registered before `/:id` so they win.

  /**
   * What the page can add: the models.dev vendors our runtimes speak to, and
   * each subscription family with whether the scope holds it.
   */
  app.get('/catalog', async (c) => {
    const entries = await vault.list({ ownerUserId: await ownerOf(c) })
    const subscriptions = assembly.typesOf('subscription').map(
      (providerType) => ({
        providerType,
        configured: entries.some(
          (entry) => entry.config.providerType === providerType
        ),
      })
    )
    try {
      return c.json({ subscriptions, vendors: await vendors.list() })
    } catch (error) {
      return c.json({
        code: 'catalog_unavailable',
        message: errorMessage(error)
      }, 502)
    }
  })

  app.post('/subscription-login', async (c) => {
    const parsed = subscriptionLoginBodySchema.safeParse(
      await c.req.json().catch(() => null)
    )
    if (!parsed.success)
      return c.json({
        code: 'invalid_body',
        message: 'Expected { providerType, label? }'
      }, 400)
    const { providerType } = parsed.data
    if (!assembly.credentialOf(providerType)) {
      return c.json({
        code: 'unknown_provider_type',
        message: `Unknown provider type "${providerType}"`
      }, 400)
    }
    const started = await logins.start(
      providerType,
      parsed.data.label ?? `${providerType} subscription`,
      await ownerOf(c)
    )
    if ('refused' in started) {
      if (started.refused === 'busy')
        return c.json({
          code: 'provider_busy',
          message: 'Provider login is already running'
        }, 409)
      return started.refused === 'exists'
        ? c.json({
          code: 'provider_exists',
          message: `This scope already has a ${providerType} subscription`
        }, 409)
        : c.json({
          code: 'no_login_flow',
          message: `Provider type "${providerType}" has no native login flow`
        }, 400)
    }
    return c.json({ login: { id: started.id, status: 'pending' } }, 202)
  })

  app.get('/subscription-login/:id', async (c) => {
    const state = logins.status(c.req.param('id') ?? '', await ownerOf(c))
    if (!state)
      return c.json({
        code: 'login_not_found',
        message: 'No such login flow'
      }, 404)
    return c.json({ login: state })
  })

  app.delete('/subscription-login/:id', async (c) => {
    if (!(await logins.cancel(c.req.param('id'), await ownerOf(c))))
      return c.json({
        code: 'login_not_found',
        message: 'No such login flow'
      }, 404)
    return c.body(null, 204)
  })

  app.post('/setup-token', async (c) => {
    const parsed = z.strictObject({
      token: z.string().trim().min(1).max(16384),
      label: z.string().trim().min(1).max(80)
    }).safeParse(await c.req.json().catch(() => null))
    if (!parsed.success)
      return c.json({
        code: 'invalid_body',
        message: 'Expected { token, label }'
      }, 400)
    const entry = await options.accounts.importClaude(
      await ownerOf(c),
      parsed.data.label,
      parsed.data.token
    )
    await installCli(c, entry)
    return c.json({ provider: publicProvider(entry) }, 201)
  })

  app.post('/:id/accounts/login', async (c) => {
    const entry = await scoped(c)
    if (!entry)
      return c.json({
        code: 'provider_not_found',
        message: 'No such provider'
      }, 404)
    if (entry.config.kind !== 'subscription')
      return c.json({
        code: 'unsupported',
        message: 'This provider does not use subscription accounts'
      }, 400)
    const started = await logins.start(
      entry.config.providerType,
      entry.label,
      await ownerOf(c),
      entry
    )
    if ('refused' in started)
      return c.json({
        code: started.refused,
        message: 'Device login is unavailable or already running; Claude uses setup-token import'
      }, 409)
    return c.json({ login: { id: started.id, status: 'pending' } }, 202)
  })

  app.get('/:id/accounts', async (c) => {
    const entry = await scoped(c)
    if (!entry)
      return c.json({
        code: 'provider_not_found',
        message: 'No such provider'
      }, 404)
    return c.json(await options.accounts.list(entry))
  })

  app.post('/:id/accounts', async (c) => {
    const entry = await scoped(c)
    if (!entry)
      return c.json({
        code: 'provider_not_found',
        message: 'No such provider'
      }, 404)
    const parsed = z.strictObject({ token: z.string().trim().min(1).max(16384) })
      .safeParse(await c.req.json().catch(() => null))
    if (!parsed.success)
      return c.json({
        code: 'invalid_body',
        message: 'Expected { token }'
      }, 400)
    const added = await options.accounts.addToken(entry, parsed.data.token)
    await installCli(c, entry)
    return c.json(added, 201)
  })

  app.put('/:id/accounts/active', async (c) => {
    const entry = await scoped(c)
    if (!entry)
      return c.json({
        code: 'provider_not_found',
        message: 'No such provider'
      }, 404)
    const parsed = z.strictObject({ credentialId: z.string().min(1) })
      .safeParse(await c.req.json().catch(() => null))
    if (!parsed.success)
      return c.json({
        code: 'invalid_body',
        message: 'Expected { credentialId }'
      }, 400)
    return c.json(
      await options.accounts.activate(entry, parsed.data.credentialId)
    )
  })

  app.delete('/:id/accounts/:credentialId', async (c) => {
    const entry = await scoped(c)
    if (!entry)
      return c.json({
        code: 'provider_not_found',
        message: 'No such provider'
      }, 404)
    await options.accounts.remove(entry, c.req.param('credentialId'))
    return c.body(null, 204)
  })

  app.get('/', async (c) => {
    const providers = await vault.list({ ownerUserId: await ownerOf(c) })
    return c.json({ providers: providers.map(publicProvider) })
  })

  app.post('/', async (c) => {
    const parsed = createProviderBodySchema.safeParse(
      await c.req.json().catch(() => null)
    )
    if (!parsed.success)
      return invalidBody(c, parsed.error)
    const body = parsed.data
    let config: ApiKeyProviderConfig
    if ('vendorId' in body) {
      const vendor = await vendors.get(body.vendorId)
      if (!vendor)
        return c.json({
          code: 'unknown_vendor',
          message: `Unknown vendor "${body.vendorId}"`
        }, 400)
      const baseUrl = body.baseUrl ?? vendor.baseUrl ?? undefined
      config = {
        kind: 'api_key',
        providerType: vendor.providerType,
        apiKey: body.apiKey,
        vendorId: vendor.id,
        ...(vendor.wireApi ? { wireApi: vendor.wireApi } : {}),
        ...(baseUrl ? { baseUrl } : {}),
        ...(body.models ? { models: body.models } : {}),
      }
    } else {
      const credential = assembly.credentialOf(body.providerType)
      if (!credential) {
        return c.json({
          code: 'unknown_provider_type',
          message: `Unknown provider type "${body.providerType}"`
        }, 400)
      }
      if (credential !== 'api_key') {
        return c.json({
          code: 'subscription_only',
          message: `Provider type "${body.providerType}" is configured by its login flow`
        }, 400)
      }
      config = {
        kind: 'api_key',
        providerType: body.providerType,
        apiKey: body.apiKey,
        ...(body.wireApi ? { wireApi: body.wireApi } : {}),
        ...(body.baseUrl ? { baseUrl: body.baseUrl } : {}),
        ...(body.models ? { models: body.models } : {}),
      }
    }
    const provider = await vault.create({
      ownerUserId: await ownerOf(c),
      label: body.label,
      config
    })
    return c.json({ provider: publicProvider(provider) }, 201)
  })

  app.patch('/:id', async (c) => {
    const provider = await scoped(c)
    if (!provider)
      return c.json({
        code: 'provider_not_found',
        message: 'No such provider'
      }, 404)
    const parsed = patchProviderBodySchema.safeParse(
      await c.req.json().catch(() => null)
    )
    if (!parsed.success)
      return invalidBody(c, parsed.error)
    const body = parsed.data
    const editsConfig = body.apiKey !== undefined ||
      body.baseUrl !== undefined ||
      body.models !== undefined
    if (editsConfig && provider.config.kind !== 'api_key') {
      return c.json({
        code: 'subscription_only',
        message: 'A subscription entry only takes a new label'
      }, 400)
    }
    const config: ApiKeyProviderConfig | undefined =
      editsConfig && provider.config.kind === 'api_key'
        ? {
          ...provider.config,
          ...(body.apiKey !== undefined ? { apiKey: body.apiKey } : {}),
          ...(body.baseUrl !== undefined
            ? { baseUrl: body.baseUrl ??
                undefined }
            : {}),
          ...(body.models !== undefined
            ? { models: body.models ??
                undefined }
            : {}),
        }
        : undefined
    const updated = await vault.update(provider.id, {
      ...(body.label !== undefined ? { label: body.label } : {}),
      ...(config ? { config } : {}),
    })
    if (!updated)
      return c.json({
        code: 'provider_not_found',
        message: 'No such provider'
      }, 404)
    await assembly.invalidate(provider.id)
    return c.json({ provider: publicProvider(updated) })
  })

  app.delete('/:id', async (c) => {
    const provider = await scoped(c)
    if (!provider)
      return c.json({
        code: 'provider_not_found',
        message: 'No such provider'
      }, 404)
    await vault.delete(provider.id)
    await assembly.deleteProviderState(provider.id)
    options.cli.installs.forget(provider.id)
    return c.body(null, 204)
  })

  app.get('/:id/status', async (c) => {
    const entry = await scoped(c)
    if (!entry)
      return c.json({
        code: 'provider_not_found',
        message: 'No such provider'
      }, 404)
    const resolved = await assembly.providerFor(entry.id)
    if (!resolved)
      return c.json({
        code: 'provider_not_found',
        message: 'No such provider'
      }, 404)
    try {
      return c.json({
        providerId: entry.id,
        ...await assembly.details(entry, resolved.provider)
      })
    } catch (error) {
      return c.json({
        code: 'provider_status_failed',
        message: errorMessage(error)
      }, 502)
    }
  })

  app.post('/:id/quota', async (c) => {
    const entry = await scoped(c)
    if (!entry)
      return c.json({
        code: 'provider_not_found',
        message: 'No such provider'
      }, 404)
    const parsed = accountBodySchema.safeParse(
      await c.req.json().catch(() => ({}))
    )
    if (!parsed.success)
      return invalidBody(c, parsed.error)
    // The account the user names, or the one the entry infers with.
    const provider = parsed.data.credentialId
      ? await assembly.forAccount(entry, parsed.data.credentialId)
      : (await assembly.providerFor(entry.id))?.provider
    if (!provider)
      return c.json({
        code: 'account_not_found',
        message: 'No such account'
      }, 404)
    const quota = provider.quota
    const capability = quota?.capability()
    if (!quota || capability?.mode !== 'supported' || !capability.canProbe)
      return c.json({ quota: publicQuota(quota?.latest() ?? null) })
    if (capability.probeCost !== 'free')
      return c.json({
        code: 'quota_requires_inference',
        message: 'This provider cannot refresh quota without an inference request'
      }, 409)
    try {
      return c.json(
        { quota: publicQuota(
            await quota.probe({ force: true, signal: c.req.raw.signal })
          ) }
      )
    } catch (error) {
      return c.json({
        code: 'quota_unavailable',
        message: errorMessage(error)
      }, 502)
    }
  })

  app.get('/:id/cli', async (c) => {
    const entry = await scoped(c)
    if (!entry || entry.config.kind !== 'subscription')
      return c.json({
        code: 'provider_not_found',
        message: 'No such provider'
      }, 404)
    const refresh = booleanQuerySchema.safeParse(c.req.query('refresh'))
    if (!refresh.success)
      return c.json({
        code: 'invalid_query',
        message: 'refresh must be true or false'
      }, 400)
    const [newest, machines] = await Promise.all([
      options.cli.newest(refresh.data ?? false).then(
        version => ({ version }),
        error => ({ error: errorMessage(error) })
      ),
      options.cli.machines(c.get('user').id, c.req.raw.signal),
    ])
    return c.json({
      newest,
      held: entry.config.cliVersion ?? null,
      install: options.cli.installs.state(c.get('user').id, entry.id),
      machines,
    })
  })

  app.put('/:id/cli', async (c) => {
    const entry = await scoped(c)
    if (!entry || entry.config.kind !== 'subscription')
      return c.json({
        code: 'provider_not_found',
        message: 'No such provider'
      }, 404)
    const parsed = z.strictObject({ held: z.string().min(1).max(64).nullable() })
      .safeParse(await c.req.json().catch(() => null))
    if (!parsed.success)
      return invalidBody(c, parsed.error)
    if (parsed.data.held) {
      try {
        await options.cli.verify(parsed.data.held)
      } catch (error) {
        return c.json({ code: 'unknown_version', message: errorMessage(error) }, 400)
      }
    }
    const { cliVersion: _held, ...config } = entry.config
    await vault.update(entry.id, {
      config: parsed.data.held ? { ...config, cliVersion: parsed.data.held } : config
    })
    await assembly.invalidate(entry.id)
    return c.json({ held: parsed.data.held })
  })

  app.post('/:id/cli/install', async (c) => {
    const entry = await scoped(c)
    if (!entry || entry.config.kind !== 'subscription')
      return c.json({
        code: 'provider_not_found',
        message: 'No such provider'
      }, 404)
    options.cli.installs.start(c.get('user').id, entry)
    return c.json({ install: options.cli.installs.state(c.get('user').id, entry.id) }, 202)
  })

  app.post('/:id/test', async (c) => {
    const provider = await scoped(c)
    if (!provider)
      return c.json({
        code: 'provider_not_found',
        message: 'No such provider'
      }, 404)
    const parsed = z.object({
      modelId: z.string().min(1),
      credentialId: z.string().min(1).optional()
    }).safeParse(
      await c.req.json().catch(() => null)
    )
    if (!parsed.success)
      return c.json({
        code: 'invalid_body',
        message: 'Expected { modelId: string, credentialId?: string }: the model to test with'
      }, 400)
    // The test ran either way: a provider that refused is the answer, with
    // its reason, not a failure of this request.
    try {
      return c.json(await assembly.testProvider(provider.id, parsed.data.modelId, {
        ...(parsed.data.credentialId ? { credentialId: parsed.data.credentialId } : {}),
        placeProcess: entry => options.placeAccountProcess(
          c.get('user').id,
          entry,
          c.req.raw.signal
        ),
      }))
    } catch (error) {
      return c.json({ ok: false, message: errorMessage(error) })
    }
  })

  return app
}
