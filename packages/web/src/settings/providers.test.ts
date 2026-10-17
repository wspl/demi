import { afterEach, beforeEach, expect, spyOn, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import { productState } from '../__tests__/product-state'
import { playChannels } from '../__tests__/sync-channel'
import type { ListedLogin, ProductState } from '../api/generated/web-api'
import type { LoginStage } from '@demicodes/protocol'
import { useProduct } from '../state/product'
import { useResources } from '../state/resources'
import { useProviderSettings } from './providers'
import { dismissToast, toasts } from '@demicodes/web-ui/infra/toast'
import { waitFor } from '@demicodes/utils'
import { operations } from '@demicodes/web-ui/ui/operations'
import { providerOperation, providerTarget } from '@demicodes/web-ui/settings/provider-operations'

const realFetch = globalThis.fetch
let pinia: ReturnType<typeof createPinia>
let channels: ReturnType<typeof playChannels>
let state: ProductState
let writes: number
/** Each read of the model catalog, by its path. */
let catalogReads: string[]
/** What the model catalog answers: the providers it lists. */
let catalogProviders: unknown[]
/** Each entry the page created, by the body it sent. */
let created: Record<string, unknown>[]
/** The id the next created entry gets. */
let nextId: string
let write: (body: Record<string, unknown>) => Promise<Response>
let probe: (body: string, signal: AbortSignal | null | undefined) => Promise<Response>
/** What the backend answers to making another account the active one. */
let activate: () => Promise<Response>
/** What the backend answers to a sign-in's start, by the path it was sent to. */
let startLogin: (path: string) => Promise<Response>
/** The sign-ins the page cancelled, by their ids. */
let cancelled: string[]
/** What the backend answers to a pasted code. */
let pasteCode: (code: string) => Promise<Response>

beforeEach(async () => {
  pinia = createPinia()
  setActivePinia(pinia)
  state = productState({
    providers: [
      {
        id: 'configured',
        kind: 'api_key',
        providerType: 'openai',
        label: 'API',
        wireApi: 'responses',
        vendorId: null,
        compatible: false,
        endpoint: null,
        hasApiKey: true,
        rotation: 'manual' as const, imageGeneration: true,
        models: null,
        createdAt: '2026-09-10T00:00:00.000Z',
        details: { type: 'failed', message: 'Not read in this test' },
      },
    ],
  })
  writes = 0
  probe = async () => Response.json({})
  activate = async () => new Response(null, { status: 204 })
  startLogin = async () => {
    throw new Error('No sign-in starts in this test')
  }
  cancelled = []
  pasteCode = async () => {
    throw new Error('No code is pasted in this test')
  }
  write = async (body) => {
    if (typeof body.label === 'string') {
      state.providers[0]!.label = body.label
    }
    return Response.json({})
  }
  catalogReads = []
  catalogProviders = []
  created = []
  nextId = 'created'
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path.startsWith('/api/models/search')) {
      return Response.json({
        results: [{
          ref: 'google/gemini-3.8-flash', displayName: 'Gemini 3.8 Flash', id: 'gemini-3.8-flash', vendor: 'google',
          contextWindow: 1_000_000, outputLimit: 65_536, thinkingEfforts: ['low', 'high'],
          acceptedExtensions: ['png', 'pdf'], releaseDate: '2026-06-17',
        }],
      })
    }
    if (path.startsWith('/api/models')) {
      catalogReads.push(path)
      return Response.json({ providers: catalogProviders, smallModel: null })
    }
    if (path === '/api/providers' && init?.method === 'POST') {
      const body = JSON.parse(String(init.body)) as Record<string, unknown>
      created.push(body)
      const entry = {
        id: nextId,
        kind: 'api_key' as const,
        providerType: body.source === 'compatible' ? 'anthropic' : 'google',
        label: body.source === 'vendor' ? 'Google' : 'Compatible API',
        wireApi: null,
        vendorId: body.source === 'vendor' ? String(body.vendorId) : null,
        compatible: body.source === 'compatible',
        endpoint: null,
        hasApiKey: false,
        rotation: 'manual' as const, imageGeneration: true,
        models: null,
        createdAt: '2026-09-11T00:00:00.000Z',
      }
      // The entry joins the product state as the channel brings it.
      state.providers.push({ ...entry, details: { type: 'failed', message: 'Not read in this test' } })
      channels.last().send({ type: 'providers', providers: state.providers })
      return Response.json({ provider: entry })
    }
    if (path === '/api/providers/catalog') {
      return Response.json({
        subscriptions: [{ providerType: 'codex', configured: false }],
        vendors: [
          {
            id: 'anthropic',
            name: 'Anthropic',
            providerType: 'anthropic',
            baseUrl: 'https://example.test',
            doc: null,
          },
        ],
      })
    }
    if (path === '/api/providers/configured' && init?.method === 'PATCH') {
      writes++
      const answer = await write(JSON.parse(String(init.body)))
      // The page's channel brings the entry as the write left it.
      channels.last().send({ type: 'providers', providers: state.providers })
      return answer
    }
    if (path === '/api/providers/codex/accounts/active' && init?.method === 'PUT') {
      return activate()
    }
    if (path.endsWith('/login') && init?.method === 'POST') {
      return startLogin(path)
    }
    if (path.endsWith('/code') && init?.method === 'POST') {
      return pasteCode(JSON.parse(String(init.body)).code as string)
    }
    if (path.startsWith('/api/providers/subscription-login/') && init?.method === 'DELETE') {
      cancelled.push(path.slice('/api/providers/subscription-login/'.length))
      return new Response(null, { status: 204 })
    }
    if (path === '/api/providers/configured/quota' && init?.method === 'POST') {
      return probe(String(init.body), init.signal)
    }
    throw new Error(`Unexpected request: ${path}`)
  }) as typeof fetch
  channels = playChannels()
  useProduct().start()
  channels.last().connect(state)
  await useProduct().loadVendors()
})
afterEach(() => {
  useProduct().stop()
  disposePinia(pinia)
  channels.restore()
  for (const toast of [...toasts]) {
    dismissToast(toast.id)
  }
  globalThis.fetch = realFetch
})

async function idle(): Promise<void> {
  for (let attempt = 0; attempt < 30; attempt++) {
    if (!useProviderSettings().providers.some((entry) => operations.onTarget(providerTarget(entry.id))) && !operations.onTarget(providerTarget('new'))) {
      return
    }
    await new Promise<void>((resolve) => setImmediate(resolve))
  }
  throw new Error('Provider operation did not finish')
}

test('adding a vendor asks only its kind: the entry joins the list at once, selected, with nothing else sent', async () => {
  const settings = useProviderSettings()
  const resources = useResources()
  const listed = () => settings.providers.map((provider) => provider.id)
  // The subscriptions and the saved key; no vendor the user never added.
  expect(listed()).toEqual(['codex', 'configured'])

  settings.addProvider({ ...resources.vendors[0]!, id: 'google' })
  await waitFor(() => listed().includes('created'))
  expect(created).toEqual([{ source: 'vendor', vendorId: 'google' }])
  expect(resources.selectedProviderId).toBe('created')
  expect(resources.providerDetailOpen).toBe(true)
  expect(listed()).toEqual(['codex', 'configured', 'created'])
})

test('adding a compatible API sends only its kind, and its protocol changes by family and wire', async () => {
  const settings = useProviderSettings()
  settings.addCompatible()
  await waitFor(() => created.length === 1)
  expect(created).toEqual([{ source: 'compatible' }])

  const sent: Record<string, unknown>[] = []
  write = async (body) => {
    sent.push(body)
    return Response.json({})
  }
  const entry = () => settings.providers.find((provider) => provider.id === 'configured')!
  settings.change(entry(), { wireApi: 'openai-chat' })
  await idle()
  settings.change(entry(), { wireApi: 'anthropic-messages' })
  await idle()
  expect(sent).toEqual([{ providerType: 'openai', wireApi: 'chat-completions' }, { providerType: 'anthropic' }])
})

test('a model Custom adds is saved at once without a context window, and a reload keeps it', async () => {
  const sent: Record<string, unknown>[] = []
  write = async (body) => {
    sent.push(body)
    // The backend keeps the list, and its catalog lists it as configured.
    const models = body.models as { id: string; displayName: string; contextWindow: number | null }[]
    state.providers[0]!.models = models as never
    catalogProviders = [{
      providerId: 'configured', displayName: 'API', sourceFetchedAt: '1970-01-01T00:00:00.000Z', stale: false,
      warnings: [], auth: { status: 'unknown' }, runtime: { status: 'ready' }, cliPackage: null,
      availability: { type: 'available' },
      models: models.map((model) => ({
        id: model.id, displayName: model.displayName, description: null, contextWindow: model.contextWindow,
        outputLimit: null, supportsTools: true, acceptedExtensions: null, supportsReasoning: false,
        supportedThinkingEfforts: [], canDisableThinking: null, serviceTiers: [], defaultServiceTierId: null, cost: null,
        selection: {
          providerId: 'configured', thinking: null, serviceTierId: null,
          model: { id: model.id, name: model.displayName, contextWindow: 0, outputLimit: null, thinking: [], acceptedExtensions: null },
        },
      })),
    }]
    return Response.json({})
  }
  const settings = useProviderSettings()
  const entry = () => settings.providers.find((provider) => provider.id === 'configured')!
  await settings.saveModel(entry(), {
    id: 'my-model', name: 'my-model', contextWindow: null, outputLimit: null, efforts: [], extensions: null, fastTier: null,
  }, null)
  await idle()
  expect(sent).toEqual([{
    models: [{
      id: 'my-model', displayName: 'my-model', contextWindow: null, outputLimit: null,
      thinkingEfforts: [], acceptedExtensions: null, fastTier: null,
    }],
  }])

  // A new page reads it back from the backend, its window still unknown.
  useProduct().stop()
  disposePinia(pinia)
  channels.restore()
  pinia = createPinia()
  setActivePinia(pinia)
  channels = playChannels()
  useProduct().start()
  channels.last().connect(state)
  const reloaded = useProviderSettings()
  await waitFor(
    () => reloaded.providers.find((provider) => provider.id === 'configured')?.models.length === 1,
    () => 'the saved model was not listed after the reload',
  )
  expect(reloaded.providers.find((provider) => provider.id === 'configured')!.models.map((model) => [model.id, model.contextWindow]))
    .toEqual([['my-model', null]])
})

test('emptying a saved key or endpoint and leaving it removes them', async () => {
  const settings = useProviderSettings()
  const sent: Record<string, unknown>[] = []
  write = async (body) => {
    sent.push(body)
    return Response.json({})
  }
  const entry = () => settings.providers.find((provider) => provider.id === 'configured')!
  settings.change(entry(), { apiKey: '' })
  await idle()
  settings.change(entry(), { baseUrl: '' })
  await idle()
  expect(sent).toEqual([{ apiKey: null }, { baseUrl: null }])
})

test('the page shows an entry’s kind and effective endpoint as the read model says them, and a compatible API’s list is its own', async () => {
  const settings = useProviderSettings()
  const entry = (id: string, vendorId: string | null, compatible: boolean, endpoint: string) => ({
    id,
    kind: 'api_key' as const,
    providerType: 'anthropic',
    label: id,
    wireApi: null,
    vendorId,
    compatible,
    endpoint,
    hasApiKey: true,
    rotation: 'manual' as const, imageGeneration: true,
    models: null,
    createdAt: '2026-09-10T00:00:00.000Z',
    details: { type: 'failed' as const, message: 'Not read in this test' },
  })
  state.providers.push(
    entry('proxy', null, true, 'https://proxy.example/v1'),
    entry('anthropic', 'anthropic', false, 'https://api.anthropic.com/v1'),
  )
  channels.last().send({ type: 'providers', providers: state.providers })
  await waitFor(() => settings.providers.some((provider) => provider.id === 'anthropic'))
  const shown = (id: string) => {
    const provider = settings.providers.find((candidate) => candidate.id === id)!
    return [provider.compatible, provider.modelSource, provider.baseUrl]
  }
  expect(shown('proxy')).toEqual([true, 'manual', 'https://proxy.example/v1'])
  expect(shown('anthropic')).toEqual([false, 'catalog', 'https://api.anthropic.com/v1'])
})

test('an incomplete entry is in the setup state, from its availability; a complete one is not', async () => {
  const settings = useProviderSettings()
  catalogProviders = [{
    providerId: 'configured', displayName: 'API', sourceFetchedAt: '1970-01-01T00:00:00.000Z', stale: false,
    warnings: [], auth: { status: 'unknown' }, runtime: { status: 'ready' }, cliPackage: null,
    availability: { type: 'unavailable', reason: 'setup', message: 'Finish setting up this provider in Settings', missing: ['apiKey', 'models'] },
    models: [],
  }]
  await useProduct().loadModels(true)
  await waitFor(() => settings.providers.find((entry) => entry.id === 'configured')?.state === 'setup')
  const entry = settings.providers.find((candidate) => candidate.id === 'configured')!
  expect(entry.state).toBe('setup')
  // The model menu lists its models dimmed, with the backend's sentence as the reason.
  expect(useResources().providerInfos).toEqual([
    { id: 'configured', label: 'API', isAvailable: false, setupReason: 'Finish setting up this provider in Settings' },
  ])

  catalogProviders = [{ ...(catalogProviders[0] as object), availability: { type: 'available' } }]
  await useProduct().loadModels(true)
  await waitFor(() => settings.providers.find((candidate) => candidate.id === 'configured')?.state !== 'setup')
})

test('the Add Model search asks for the entry and fills a draft the way the dialog shows it', async () => {
  const settings = useProviderSettings()
  const found = await settings.searchModels('configured', 'gemini fl', new AbortController().signal)
  expect(found).toEqual([{
    ref: 'google/gemini-3.8-flash',
    vendor: 'google',
    model: {
      id: 'gemini-3.8-flash', name: 'Gemini 3.8 Flash', contextWindow: 1_000_000, outputLimit: 65_536,
      efforts: ['low', 'high'], extensions: ['.png', '.pdf'], fastTier: null,
    },
  }])
})

test('a provider wears its vendor\'s mark: a subscription by its type, a key by its vendor', async () => {
  const settings = useProviderSettings()
  const resources = useResources()
  const logo = (id: string) => settings.providers.find((provider) => provider.id === id)?.logo
  // A subscription not yet signed in to, and a bare endpoint, which has no vendor.
  expect(logo('codex')).toEndWith('/openai.svg')
  expect(logo('configured')).toBeNull()

  const claude = { ...state.providers[0]!, id: 'claude', kind: 'subscription' as const, providerType: 'claude-code', label: 'Claude Code' }
  channels.last().send({ type: 'providers', providers: [...state.providers, claude] })
  await nextTick()
  expect(logo('claude')).toEndWith('/claude.svg')

  expect(resources.vendors.find((vendor) => vendor.id === 'anthropic')!.logo).toEndWith('/anthropic.svg')
})

test('an edit or a model made while a test runs shows at once and is saved when the test ends', async () => {
  const settings = useProviderSettings()
  let answerTest!: () => void
  const testing = new Promise<void>((resolve) => {
    answerTest = resolve
  })
  const plainFetch = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    if (String(input) === '/api/providers/configured/test') {
      await testing
      return Response.json({ type: 'passed', model: 'gpt' })
    }
    return plainFetch(input, init)
  }) as typeof fetch
  const provider = () => settings.providers.find((entry) => entry.id === 'configured')!
  const gpt = {
    id: 'gpt',
    name: 'GPT',
    contextWindow: 200_000,
    outputLimit: null,
    efforts: [],
    extensions: null,
    fastTier: null,
    enabled: true,
  }
  settings.test({ ...provider(), models: [gpt] })
  await nextTick()
  expect(operations.runs(providerOperation('configured', { kind: 'testing' }))).toBe(true)

  settings.change(provider(), { name: 'Renamed' })
  expect(provider().name).toBe('Renamed')
  // A model saved meanwhile waits for the test too, rather than being refused.
  const savedModel = settings.saveModel({ ...provider(), models: [gpt] }, { ...gpt, id: 'gpt-mini', name: 'GPT Mini' }, null)
  expect(writes).toBe(0)

  answerTest()
  await savedModel
  await idle()
  expect(writes).toBe(2)
  expect(state.providers[0]!.label).toBe('Renamed')
})

test('quota refreshes coalesce per account and do not block other accounts or edits', async () => {
  const settings = useProviderSettings()
  const provider = settings.providers.find((entry) => entry.id === 'configured')!
  const deferred = Promise.withResolvers<Response>()
  const accounts: string[] = []
  probe = (body) => {
    accounts.push(body)
    return deferred.promise
  }
  const first = settings.refreshUsage(provider, 'first', true)
  await settings.refreshUsage(provider, 'first', true)
  const second = settings.refreshUsage(provider, 'second', true)
  expect(accounts).toEqual([
    JSON.stringify({ credentialId: 'first' }),
    JSON.stringify({ credentialId: 'second' }),
  ])
  expect(settings.refreshingUsage.configured).toEqual({ first: 'automatic', second: 'automatic' })
  settings.change(provider, { name: 'While refreshing' })
  await idle()
  expect(writes).toBe(1)
  deferred.resolve(Response.json({}))
  await Promise.all([first, second])
  expect(settings.refreshingUsage).toEqual({})
  await settings.refreshUsage(provider, 'first', true)
  expect(accounts).toHaveLength(2)
})

test('automatic quota refresh uses a one-minute TTL; manual refresh bypasses and renews it', async () => {
  const clock = spyOn(Date, 'now').mockReturnValue(1_000)
  try {
    const settings = useProviderSettings()
    const provider = settings.providers.find((entry) => entry.id === 'configured')!
    let requests = 0
    const deferred = Promise.withResolvers<Response>()
    probe = async () => {
      requests++
      return requests === 1 ? deferred.promise : Response.json({})
    }
    const initial = settings.refreshUsage(provider, 'first', true)
    clock.mockReturnValue(11_000)
    deferred.resolve(Response.json({}))
    await initial
    clock.mockReturnValue(70_999)
    await settings.refreshUsage(provider, 'first', true)
    expect(requests).toBe(1)
    clock.mockReturnValue(71_000)
    await settings.refreshUsage(provider, 'first', true)
    expect(requests).toBe(2)
    clock.mockReturnValue(72_000)
    await settings.refreshUsage(provider, 'first')
    expect(requests).toBe(3)
    clock.mockReturnValue(131_999)
    await settings.refreshUsage(provider, 'first', true)
    expect(requests).toBe(3)
    clock.mockReturnValue(132_000)
    await settings.refreshUsage(provider, 'first', true)
    expect(requests).toBe(4)
  } finally {
    clock.mockRestore()
  }
})

test('automatic quota failures stay quiet; a manual retry records the vendor\'s words for the account\'s line, with no toast', async () => {
  const settings = useProviderSettings()
  const provider = settings.providers.find((entry) => entry.id === 'configured')!
  let requests = 0
  probe = async () => {
    requests++
    return Response.json({ requestId: 'req_test000000000001', code: 'quota_unavailable', message: 'Billing unavailable' }, { status: 503 })
  }
  await settings.refreshUsage(provider, 'first', true)
  expect(toasts).toHaveLength(0)
  expect(settings.refreshingUsage).toEqual({})
  await nextTick()
  await settings.refreshUsage(provider, 'first', true)
  expect(requests).toBe(1)
  await settings.refreshUsage(provider, 'first')
  expect(requests).toBe(2)
  expect(toasts).toHaveLength(0)
  expect(settings.usageFailures).toEqual({ configured: { first: { message: 'Billing unavailable', requestId: 'req_test000000000001' } } })
})

test('a manual refresh joins an automatic request and shows its pending state and failure', async () => {
  const settings = useProviderSettings()
  const provider = settings.providers.find((entry) => entry.id === 'configured')!
  const deferred = Promise.withResolvers<Response>()
  let requests = 0
  probe = () => {
    requests++
    return deferred.promise
  }
  const automatic = settings.refreshUsage(provider, 'first', true)
  expect(settings.refreshingUsage.configured?.first).toBe('automatic')
  await settings.refreshUsage(provider, 'first')
  expect(settings.refreshingUsage.configured?.first).toBe('manual')
  await settings.refreshUsage(provider, 'first', true)
  expect(settings.refreshingUsage.configured?.first).toBe('manual')
  expect(requests).toBe(1)
  deferred.resolve(Response.json({ requestId: 'req_test000000000001', code: 'quota_unavailable', message: 'Billing unavailable' }, { status: 503 }))
  await automatic
  expect(settings.refreshingUsage).toEqual({})
  expect(toasts).toHaveLength(0)
  expect(settings.usageFailures).toEqual({ configured: { first: { message: 'Billing unavailable', requestId: 'req_test000000000001' } } })
})

test('a manual usage refresh that fails keeps the vendor\'s words for that account until the next answer', async () => {
  const settings = useProviderSettings()
  const provider = settings.providers.find((entry) => entry.id === 'configured')!
  let fail = true
  probe = async () =>
    fail
      ? Response.json({ requestId: 'req_test000000000001', code: 'quota_unavailable', message: 'Billing unavailable' }, { status: 503 })
      : Response.json({})
  await settings.refreshUsage(provider, 'first', true)
  expect(settings.usageFailures).toEqual({})
  await settings.refreshUsage(provider, 'first')
  expect(settings.usageFailures).toEqual({ configured: { first: { message: 'Billing unavailable', requestId: 'req_test000000000001' } } })
  expect(toasts).toHaveLength(0)
  fail = false
  await settings.refreshUsage(provider, 'second')
  expect(settings.usageFailures).toEqual({ configured: { first: { message: 'Billing unavailable', requestId: 'req_test000000000001' } } })
  await settings.refreshUsage(provider, 'first')
  expect(settings.usageFailures).toEqual({})
})

test('disposing provider settings cancels quota requests without a toast', async () => {
  const settings = useProviderSettings()
  const provider = settings.providers.find((entry) => entry.id === 'configured')!
  let requestSignal: AbortSignal | null | undefined
  probe = (_body, signal) => {
    requestSignal = signal
    return new Promise((_resolve, reject) => {
      signal?.addEventListener('abort', () => reject(signal.reason), { once: true })
    })
  }
  const pending = settings.refreshUsage(provider, 'first')
  settings.$dispose()
  await pending
  expect(requestSignal?.aborted).toBe(true)
  expect(toasts).toHaveLength(0)
  expect(settings.refreshingUsage).toEqual({})
})

test('provider writes become busy before dispatch and reject duplicate clicks', async () => {
  const settings = useProviderSettings()
  const provider = settings.providers.find(
    (entry) => entry.id === 'configured',
  )!
  const deferred = Promise.withResolvers<Response>()
  write = () => deferred.promise
  settings.change(provider, { name: 'Changed' })
  expect(operations.runs(providerOperation('configured', { kind: 'saving' }))).toBe(true)
  settings.change(provider, { name: 'Duplicate' })
  await nextTick()
  expect(writes).toBe(1)
  state.providers[0]!.label = 'Changed'
  deferred.resolve(Response.json({}))
  await idle()
  expect(
    settings.providers.find((entry) => entry.id === 'configured')?.name,
  ).toBe('Changed')
  expect(operations.onTarget(providerTarget('configured'))).toBe(false)
})

test('a failed save is a toast; the input stays and the next change resends it', async () => {
  const settings = useProviderSettings()
  write = async () => {
    throw new Error('offline')
  }
  settings.change(
    settings.providers.find((entry) => entry.id === 'configured')!,
    { name: 'Retained name' },
  )
  await idle()
  expect(toasts.at(-1)).toMatchObject({
    title: 'Could Not Save the Provider',
    message: 'offline',
    tone: 'danger',
  })
  expect(
    settings.providers.find((entry) => entry.id === 'configured')?.name,
  ).toBe('Retained name')
  write = async (body) => {
    state.providers[0]!.label = String(body.label)
    return Response.json({})
  }
  settings.change(
    settings.providers.find((entry) => entry.id === 'configured')!,
    { name: 'Retained name' },
  )
  await idle()
  expect(writes).toBe(2)
  expect(
    settings.providers.find((entry) => entry.id === 'configured')?.name,
  ).toBe('Retained name')
})

// The Image Generation switch shows its new value while it saves; a save
// that fails puts it back to the backend's with the toast, and the next
// edit does not send the failed value again. Cost: a stubbed backend, a
// few milliseconds.
test('a switch that fails to save returns to the backend\'s value with a toast, and later edits leave it out', async () => {
  state.providers[0]!.details = {
    type: 'read',
    auth: { status: 'authenticated' },
    runtime: { status: 'ready' },
    accounts: [],
    active: null,
    quota: null,
    quotaCapability: { type: 'none' },
    cliPackage: null,
    makesImages: true,
  }
  channels.last().send({ type: 'providers', providers: state.providers })
  const settings = useProviderSettings()
  const entry = () => settings.providers.find((candidate) => candidate.id === 'configured')!
  const answer = Promise.withResolvers<Response>()
  const bodies: Record<string, unknown>[] = []
  write = (body) => {
    bodies.push(body)
    return answer.promise
  }
  settings.change(entry(), { imageGeneration: false })
  expect(entry().imageGeneration).toBe(false)

  answer.resolve(
    Response.json({ requestId: 'req_test000000000001', code: 'invalid_body', message: 'The network is offline.' }, { status: 400 }),
  )
  await idle()
  expect(entry().imageGeneration).toBe(true)
  expect(toasts.at(-1)).toMatchObject({ title: 'Could Not Save the Provider', tone: 'danger' })

  write = async (body) => {
    bodies.push(body)
    state.providers[0]!.label = String(body.label)
    return Response.json({})
  }
  settings.change(entry(), { name: 'Renamed' })
  await idle()
  expect(bodies).toEqual([{ imageGeneration: false }, { label: 'Renamed' }])
  expect(entry().imageGeneration).toBe(true)
})

test('a saved change is not read back: the catalog follows the channel\'s entry, read once', async () => {
  const settings = useProviderSettings()
  await useProduct().loadModels()
  catalogReads = []
  settings.change(settings.providers.find((entry) => entry.id === 'configured')!, { name: 'Renamed' })
  await idle()
  expect(settings.providers.find((entry) => entry.id === 'configured')?.name).toBe('Renamed')
  expect(catalogReads).toEqual(['/api/models'])
})

test('an API key change, which no view shows, still loads the catalog once the channel brings the providers', async () => {
  const settings = useProviderSettings()
  await useProduct().loadModels()
  catalogReads = []
  write = async () => Response.json({})
  settings.change(settings.providers.find((entry) => entry.id === 'configured')!, { apiKey: 'sk-new-test-key' })
  await idle()
  await new Promise<void>((resolve) => setImmediate(resolve))
  expect(catalogReads).toEqual(['/api/models'])
})

test('Activate moves the Active mark at once, and a refused switch moves it back with a toast', async () => {
  // Cost: the product state and a stubbed backend; a few milliseconds.
  const account = (id: string) => ({ id, label: `${id}@example.com`, detail: null, updatedAt: null, quota: null })
  state.providers.push({
    id: 'codex',
    kind: 'subscription',
    providerType: 'codex',
    label: 'Codex',
    wireApi: null,
    vendorId: null,
    compatible: false,
    endpoint: null,
    hasApiKey: false,
    rotation: 'manual' as const, imageGeneration: true,
    models: null,
    createdAt: '2026-09-10T00:00:00.000Z',
    details: {
      type: 'read',
      auth: { status: 'authenticated' },
      runtime: { status: 'ready' },
      accounts: [account('a'), account('b')],
      active: 'a',
      quota: null,
      quotaCapability: { type: 'none' },
      cliPackage: null, makesImages: false,
    },
  })
  channels.last().send({ type: 'providers', providers: state.providers })
  const settings = useProviderSettings()
  const entry = () => settings.providers.find((candidate) => candidate.id === 'codex')!
  const activeAccount = () => entry().accounts.find((candidate) => candidate.active)?.id
  expect(activeAccount()).toBe('a')

  const answer = Promise.withResolvers<Response>()
  activate = () => answer.promise
  const switching = settings.activateAccount(entry(), 'b')
  expect(activeAccount()).toBe('b')

  answer.resolve(
    Response.json({ requestId: 'req_test000000000001', code: 'account_not_found', message: 'That account is gone.' }, { status: 404 }),
  )
  await switching
  expect(activeAccount()).toBe('a')
  expect(toasts.map((toast) => toast.title)).toEqual(['Could Not Switch Accounts'])
})

/** Sign-in `id` into `providerId`'s entry, or a new Codex entry, as the channel lists it while it runs at `stage`. */
function runningLogin(id: string, stage: LoginStage, providerId: string | null = null): ListedLogin {
  return {
    id,
    providerId,
    providerType: 'codex',
    login: {
      status: 'pending',
      stage,
      verificationUrl: stage === 'waiting_for_user' ? 'https://auth.openai.com/codex/device' : null,
      userCode: stage === 'waiting_for_user' ? 'HXRV-7K2M' : null,
      needsCode: false,
      codesTaken: 0,
      codeError: null,
      expiresAt: '2026-09-10T00:10:00.000Z',
    },
  }
}

test('a page loaded while a sign-in runs shows it again, follows it, and Cancel cancels it for good', async () => {
  // A reload: a new page whose snapshot lists the running sign-in.
  useProduct().stop()
  disposePinia(pinia)
  pinia = createPinia()
  setActivePinia(pinia)
  state.logins = [runningLogin('running', 'waiting_for_machine')]
  useProduct().start()
  channels.last().connect(state)
  await useProduct().loadVendors()
  const settings = useProviderSettings()
  await waitFor(() => settings.login !== null)
  expect(settings.login?.provider.id).toBe('codex')
  expect(settings.login?.phase).toEqual({ kind: 'starting', stage: 'waiting_for_machine' })

  // The dialog follows what the vault announces.
  state.logins = [runningLogin('running', 'waiting_for_user')]
  channels.last().send({ type: 'logins', logins: state.logins })
  await waitFor(() => settings.login?.phase.kind === 'device')
  expect(settings.login?.phase).toEqual({ kind: 'device', url: 'https://auth.openai.com/codex/device', code: 'HXRV-7K2M', expiresAt: Date.parse('2026-09-10T00:10:00.000Z') })

  // Cancel cancels it, and the dialog stays closed when its end is announced.
  await settings.closeLogin()
  expect(cancelled).toEqual(['running'])
  expect(settings.login).toBeNull()
  state.logins = [{ ...runningLogin('running', 'waiting_for_user'), login: { status: 'cancelled' } }]
  channels.last().send({ type: 'logins', logins: state.logins })
  await nextTick()
  expect(settings.login).toBeNull()
})

test('a start the entry’s running sign-in holds opens that sign-in, whose failure names the request that started it', async () => {
  const entry = {
    ...state.providers[0]!,
    id: 'subscription',
    kind: 'subscription' as const,
    providerType: 'codex',
    label: 'Codex',
    hasApiKey: false,
    rotation: 'manual' as const, imageGeneration: true,
  }
  state.providers.push(entry)
  // Another page of the user's started it.
  state.logins = [runningLogin('running', 'waiting_for_user', 'subscription')]
  channels.last().send({ type: 'providers', providers: state.providers })
  const settings = useProviderSettings()
  await waitFor(() => settings.providers.some((provider) => provider.id === 'subscription'))
  const starts: string[] = []
  startLogin = async (path) => {
    starts.push(path)
    // The channel lists it by the time the start is refused.
    channels.last().send({ type: 'logins', logins: state.logins })
    return Response.json(
      { code: 'provider_busy', message: 'Another provider operation is still running', requestId: 'req_refused', loginId: 'running' },
      { status: 409 },
    )
  }
  await settings.beginLogin(settings.providers.find((provider) => provider.id === 'subscription')!)
  expect(starts).toEqual(['/api/providers/subscription/accounts/login'])
  expect(settings.login?.phase).toEqual({ kind: 'device', url: 'https://auth.openai.com/codex/device', code: 'HXRV-7K2M', expiresAt: Date.parse('2026-09-10T00:10:00.000Z') })

  state.logins = [{
    ...state.logins[0]!,
    login: { status: 'failed', message: 'Claude Code’s machine didn’t start. Its runner didn’t connect in 60 seconds.', requestId: 'req_started' },
  }]
  channels.last().send({ type: 'logins', logins: state.logins })
  await waitFor(() => settings.login?.phase.kind === 'failed')
  expect(settings.login?.phase).toEqual({
    kind: 'failed',
    message: 'Claude Code’s machine didn’t start. Its runner didn’t connect in 60 seconds.',
    requestId: 'req_started',
  })
  // Nothing to cancel: it ended.
  await settings.closeLogin()
  expect(cancelled).toEqual([])
})

test('of two tabs that paste a code each, each shows only its own code’s refusal, and Signing in until its paste is answered', async () => {
  const listed = (codesTaken: number, codeError: string | null): ListedLogin => ({
    id: 'claude',
    providerId: null,
    // The page takes a code the same way whatever the family.
    providerType: 'codex',
    login: {
      status: 'pending',
      stage: 'waiting_for_user',
      verificationUrl: 'https://claude.com/cai/oauth/authorize',
      userCode: null,
      needsCode: true,
      codesTaken,
      codeError,
      expiresAt: '2026-09-10T00:15:00.000Z',
    },
  })
  const settings = useProviderSettings()
  // The sign-in another tab started waits for a code; this tab shows it.
  channels.last().send({ type: 'logins', logins: [listed(0, null)] })
  await waitFor(() => settings.login?.phase.kind === 'code')
  const phase = () => {
    const shown = settings.login?.phase
    if (shown?.kind !== 'code') {
      throw new Error(`Not waiting for a code: ${JSON.stringify(shown)}`)
    }
    return [shown.submitted, shown.error ?? null]
  }
  expect(phase()).toEqual([false, null])
  expect(settings.login?.phase).toMatchObject({ expiresAt: Date.parse('2026-09-10T00:15:00.000Z') })

  // This tab pastes a code, whose answer is slow; meanwhile the other tab's
  // code, taken first as code 1, is refused.
  let pasted = ''
  const answer = Promise.withResolvers<Response>()
  pasteCode = (code) => {
    pasted = code
    return answer.promise
  }
  const pasting = settings.submitCode('mine#state')
  expect(pasted).toBe('mine#state')
  expect(phase()).toEqual([true, null])
  channels.last().send({ type: 'logins', logins: [listed(1, 'Invalid code 1')] })
  await nextTick()
  expect(phase()).toEqual([true, null])

  // The answer names this tab's code 2: code 1's refusal is still not its own.
  answer.resolve(Response.json({ code: 2 }))
  await pasting
  expect(phase()).toEqual([true, null])
  channels.last().send({ type: 'logins', logins: [listed(2, null)] })
  await nextTick()
  expect(phase()).toEqual([true, null])
  // Its own refusal shows, and it takes another.
  channels.last().send({ type: 'logins', logins: [listed(2, 'Invalid code 2')] })
  await waitFor(() => phase()[1] !== null)
  expect(phase()).toEqual([false, 'Invalid code 2'])
})

test('an account whose sign-in the vendor refused reads as signed out, and Sign In Again starts the account login', async () => {
  const account = (id: string, signedOut?: boolean) => ({ id, label: `${id}@example.com`, detail: null, updatedAt: null, quota: null, signedOut })
  state.providers.push({
    ...state.providers[0]!,
    id: 'claude',
    kind: 'subscription' as const,
    providerType: 'claude-code',
    label: 'Claude',
    hasApiKey: false,
    rotation: 'manual' as const, imageGeneration: true,
    details: {
      type: 'read' as const,
      auth: { status: 'authenticated' as const },
      runtime: { status: 'ready' as const },
      accounts: [account('first'), account('second', true)],
      active: 'first',
      quota: null,
      quotaCapability: { type: 'none' as const },
      cliPackage: null, makesImages: false,
    },
  })
  channels.last().send({ type: 'providers', providers: state.providers })
  const settings = useProviderSettings()
  await waitFor(() => settings.providers.some((provider) => provider.id === 'claude'))
  const claude = settings.providers.find((provider) => provider.id === 'claude')!
  expect(claude.accounts.map((row) => [row.id, row.signedOut])).toEqual([['first', false], ['second', true]])
  const starts: string[] = []
  startLogin = async (path) => {
    starts.push(path)
    return Response.json({ code: 'provider_busy', message: 'Another provider operation is still running', requestId: 'req_refused' }, { status: 409 })
  }
  await settings.beginLogin(claude)
  expect(starts).toEqual(['/api/providers/claude/accounts/login'])
})

test('a completed sign-in says done the moment the page learns it, and an account removed later leaves it done', async () => {
  const account = (id: string) => ({ id, label: `${id}@example.com`, detail: null, updatedAt: null, quota: null })
  const entry = (accounts: string[]) => ({
    ...state.providers[0]!,
    id: 'codex',
    kind: 'subscription' as const,
    providerType: 'codex',
    label: 'Codex',
    hasApiKey: false,
    rotation: 'manual' as const, imageGeneration: true,
    details: {
      type: 'read' as const,
      auth: { status: 'authenticated' as const },
      runtime: { status: 'ready' as const },
      accounts: accounts.map(account),
      active: 'first',
      quota: null,
      quotaCapability: { type: 'none' as const },
      cliPackage: null, makesImages: false,
    },
  })
  state.providers.push(entry(['first']))
  channels.last().send({ type: 'providers', providers: state.providers })
  const settings = useProviderSettings()
  channels.last().send({ type: 'logins', logins: [runningLogin('adding', 'waiting_for_user', 'codex')] })
  await waitFor(() => settings.login?.phase.kind === 'device')

  // The channel brings the account, then the end.
  state.providers[1] = entry(['first', 'second'])
  channels.last().send({ type: 'providers', providers: state.providers })
  channels.last().send({
    type: 'logins',
    logins: [{ ...runningLogin('adding', 'waiting_for_user', 'codex'), login: { status: 'completed', providerId: 'codex', credentialId: 'second' } }],
  })
  await waitFor(() => settings.login?.phase.kind === 'done')
  // Added beside the active one, which stays.
  expect(settings.login?.phase).toEqual({ kind: 'done', account: 'second@example.com', active: false })
  expect(useResources().selectedProviderId).toBe('codex')

  // Another page removes the account; this one still says it signed in.
  state.providers[1] = entry(['first'])
  channels.last().send({ type: 'providers', providers: state.providers })
  await waitFor(() => useResources().providers.find((provider) => provider.id === 'codex')?.accounts.length === 1)
  expect(settings.login?.phase).toEqual({ kind: 'done', account: 'second@example.com', active: false })
})

test('a sign-in cancelled on another page closes the dialog here, with nothing sent', async () => {
  const settings = useProviderSettings()
  channels.last().send({ type: 'logins', logins: [runningLogin('running', 'waiting_for_user')] })
  await waitFor(() => settings.login !== null)
  channels.last().send({
    type: 'logins',
    logins: [{ ...runningLogin('running', 'waiting_for_user'), login: { status: 'cancelled' } }],
  })
  await waitFor(() => settings.login === null)
  expect(cancelled).toEqual([])
})
