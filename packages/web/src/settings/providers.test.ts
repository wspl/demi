import { afterEach, beforeEach, expect, spyOn, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import { productStateSchema, type ProductState } from '../api/contracts'
import { useProduct } from '../state/product'
import { useProviderSettings } from './providers'
import { dismissToast, toasts } from '@demicodes/web-ui/infra/toast'

const realFetch = globalThis.fetch
let pinia: ReturnType<typeof createPinia>
let state: ProductState
let writes: number
let write: (body: Record<string, unknown>) => Promise<Response>
let probe: (body: string, signal: AbortSignal | null | undefined) => Promise<Response>

beforeEach(async () => {
  pinia = createPinia()
  setActivePinia(pinia)
  state = productStateSchema.parse({
    user: {
      id: 'user',
      email: 'test@example.test',
      nickname: 'Test',
      role: 'master',
      createdAt: '2026-09-10T00:00:00.000Z',
    },
    mode: 'shared',
    preferences: { appearance: {}, shortcuts: {} },
    devices: [],
    workspaces: [],
    conversations: [],
    cloud: { device: null, state: 'unallocated', operation: null, error: null, limits: { systemBytes: 0, homeBytes: 0 } },
    exposes: [],
    exposeDomain: null,
    providers: [
      {
        id: 'configured',
        kind: 'api_key',
        providerType: 'openai',
        label: 'API',
        wireApi: 'responses',
        vendorId: null,
        baseUrl: null,
        models: null,
        keyConfigured: true,
        details: null,
      },
    ],
  })
  writes = 0
  probe = async () => Response.json({})
  write = async (body) => {
    if (typeof body.label === 'string') {
      state.providers[0]!.label = body.label
    }
    return Response.json({})
  }
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path === '/api/state') {
      return Response.json(state)
    }
    if (path.startsWith('/api/models')) {
      return Response.json({ providers: [] })
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
      return write(JSON.parse(String(init.body)))
    }
    if (path === '/api/providers/configured/quota' && init?.method === 'POST') {
      return probe(String(init.body), init.signal)
    }
    throw new Error(`Unexpected request: ${path}`)
  }) as typeof fetch
  await useProduct().start()
  await useProduct().loadVendors()
})
afterEach(() => {
  useProduct().stop()
  disposePinia(pinia)
  for (const toast of [...toasts]) {
    dismissToast(toast.id)
  }
  globalThis.fetch = realFetch
})

async function idle(): Promise<void> {
  for (let attempt = 0; attempt < 30; attempt++) {
    if (!Object.keys(useProviderSettings().operations).length) {
      return
    }
    await new Promise<void>((resolve) => setImmediate(resolve))
  }
  throw new Error('Provider operation did not finish')
}

test('reopening settings and revalidating preserve default provider IDs', async () => {
  const settings = useProviderSettings()
  const ids = settings.providers.map((provider) => provider.id)
  expect(ids).toContain('codex')
  expect(ids).toHaveLength(3)
  await useProduct().revalidate()
  await nextTick()
  expect(
    useProviderSettings().providers.map((provider) => provider.id),
  ).toEqual(ids)
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

test('automatic quota failures stay quiet; manual retries explain the error', async () => {
  const settings = useProviderSettings()
  const provider = settings.providers.find((entry) => entry.id === 'configured')!
  let requests = 0
  probe = async () => {
    requests++
    return Response.json({ code: 'quota_unavailable', message: 'Billing unavailable' }, { status: 502 })
  }
  await settings.refreshUsage(provider, 'first', true)
  expect(toasts).toHaveLength(0)
  expect(settings.refreshingUsage).toEqual({})
  await nextTick()
  await settings.refreshUsage(provider, 'first', true)
  expect(requests).toBe(1)
  await settings.refreshUsage(provider, 'first')
  expect(requests).toBe(2)
  expect(toasts.at(-1)).toMatchObject({ title: 'Could not refresh usage', message: 'Billing unavailable' })
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
  deferred.resolve(Response.json({ code: 'quota_unavailable', message: 'Billing unavailable' }, { status: 502 }))
  await automatic
  expect(settings.refreshingUsage).toEqual({})
  expect(toasts.at(-1)).toMatchObject({ title: 'Could not refresh usage', message: 'Billing unavailable' })
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
  expect(settings.operations.configured?.kind).toBe('saving')
  settings.change(provider, { name: 'Duplicate' })
  await nextTick()
  expect(writes).toBe(1)
  state.providers[0]!.label = 'Changed'
  deferred.resolve(Response.json({}))
  await idle()
  expect(
    settings.providers.find((entry) => entry.id === 'configured')?.name,
  ).toBe('Changed')
  expect(settings.operations.configured).toBeUndefined()
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
    title: 'Provider operation failed',
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
