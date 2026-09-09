import { afterEach, beforeEach, expect, test } from 'bun:test'
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
    cloud: null,
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

test('failed configuration retains input and the same draft can be retried', async () => {
  const settings = useProviderSettings()
  write = async () => {
    throw new Error('offline')
  }
  settings.change(
    settings.providers.find((entry) => entry.id === 'configured')!,
    { name: 'Retained name' },
  )
  await idle()
  expect(settings.saveErrors.configured).toBe('offline')
  expect(
    settings.providers.find((entry) => entry.id === 'configured')?.name,
  ).toBe('Retained name')
  write = async (body) => {
    state.providers[0]!.label = String(body.label)
    return Response.json({})
  }
  settings.retrySave(
    settings.providers.find((entry) => entry.id === 'configured')!,
  )
  await idle()
  expect(writes).toBe(2)
  expect(settings.saveErrors.configured).toBeUndefined()
  expect(
    settings.providers.find((entry) => entry.id === 'configured')?.name,
  ).toBe('Retained name')
})
