import { afterEach, beforeEach, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { productStateSchema } from '../api/contracts'
import { useProduct } from './product'

const realFetch = globalThis.fetch
let pinia: ReturnType<typeof createPinia>
const state = productStateSchema.parse({
  user: {
    id: 'test-user',
    email: 'test@example.test',
    nickname: 'Test',
    role: 'master',
    createdAt: '2026-09-10T00:00:00.000Z',
  },
  mode: 'shared',
  preferences: { appearance: {}, shortcuts: {} },
  devices: [],
  workspaces: [],
  providers: [],
  conversations: [],
  cloud: null,
})
let modelResponse: () => Promise<Response>
let vendorResponse: () => Promise<Response>
let vendorReads: number

beforeEach(() => {
  pinia = createPinia()
  setActivePinia(pinia)
  modelResponse = async () => Response.json({ providers: [] })
  vendorResponse = async () => Response.json({ subscriptions: [], vendors: [] })
  vendorReads = 0
  globalThis.fetch = (async (input) => {
    const path = String(input)
    if (path === '/api/state') {
      return Response.json(state)
    }
    if (path.startsWith('/api/models')) {
      return modelResponse()
    }
    if (path === '/api/providers/catalog') {
      vendorReads++
      return vendorResponse()
    }
    throw new Error(`Unexpected request: ${path}`)
  }) as typeof fetch
})
afterEach(() => {
  useProduct().stop()
  disposePinia(pinia)
  globalThis.fetch = realFetch
})

test('vendor readers share one request and reopening uses cached data', async () => {
  const product = useProduct()
  await product.start()
  const deferred = Promise.withResolvers<Response>()
  vendorResponse = () => deferred.promise
  const first = product.loadVendors()
  const second = product.loadVendors()
  expect(product.vendorLoad).toBe('loading')
  expect(vendorReads).toBe(1)
  deferred.resolve(Response.json({ subscriptions: [], vendors: [] }))
  await Promise.all([first, second])
  expect(product.vendorLoad).toBe('ready')
  await product.loadVendors()
  expect(vendorReads).toBe(1)
})

test('failed initial reads can retry and cached model data survives refresh failure', async () => {
  const product = useProduct()
  modelResponse = async () => {
    throw new Error('offline')
  }
  await product.start()
  expect(product.load).toBe('ready')
  expect(product.catalogLoad).toBe('failed')
  const deferred = Promise.withResolvers<Response>()
  modelResponse = () => deferred.promise
  const retry = product.loadModels()
  expect(product.catalogLoad).toBe('loading')
  deferred.resolve(Response.json({ providers: [] }))
  await retry
  expect(product.catalogLoad).toBe('ready')
  modelResponse = async () => {
    throw new Error('offline')
  }
  await expect(product.loadModels()).rejects.toThrow('offline')
  expect(product.catalogLoad).toBe('ready')
  vendorResponse = async () => {
    throw new Error('offline')
  }
  await expect(product.loadVendors()).rejects.toThrow('offline')
  expect(product.vendorLoad).toBe('failed')
  vendorResponse = async () => Response.json({ subscriptions: [], vendors: [] })
  await product.loadVendors()
  expect(product.vendorLoad).toBe('ready')
})

test('an account change cannot populate the next account with a late vendor response', async () => {
  const product = useProduct()
  await product.start()
  const deferred = Promise.withResolvers<Response>()
  vendorResponse = () => deferred.promise
  const previous = product.loadVendors()
  product.stop()
  await product.start()
  vendorResponse = async () => Response.json({ subscriptions: [], vendors: [] })
  await product.loadVendors()
  deferred.resolve(
    Response.json({
      subscriptions: [{ providerType: 'old-account', configured: true }],
      vendors: [],
    }),
  )
  await expect(previous).rejects.toThrow()
  expect(product.vendors?.subscriptions).toEqual([])
})

test('retrying a failed initial state read shows loading until the result arrives', async () => {
  const normalFetch = globalThis.fetch
  let readState: () => Promise<Response> = async () => {
    throw new Error('offline')
  }
  globalThis.fetch = (async (input, init) =>
    String(input) === '/api/state'
      ? readState()
      : normalFetch(input, init)) as typeof fetch
  const product = useProduct()
  await product.start()
  expect(product.load).toBe('failed')
  const deferred = Promise.withResolvers<Response>()
  readState = () => deferred.promise
  const retry = product.refresh()
  expect(product.load).toBe('loading')
  deferred.resolve(Response.json(state))
  await retry
  expect(product.load).toBe('ready')
})
