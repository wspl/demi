import { afterEach, beforeEach, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { productState } from '../__tests__/product-state'
import type { ProductState } from '../api/unported'
import { useProduct } from '../state/product'
import { useDeviceSettings } from './devices'

const realFetch = globalThis.fetch
let pinia: ReturnType<typeof createPinia>
let state: ProductState
let renewals: string[]
let removals: string[]

beforeEach(async () => {
  pinia = createPinia()
  setActivePinia(pinia)
  state = productState({
    devices: [
      {
        id: 'laptop',
        name: 'laptop',
        kind: 'user',
        platform: 'darwin',
        claimedAt: '2026-01-01T00:00:00.000Z',
        lastSeenAt: null,
        online: true,
        home: null,
      },
    ],
    cloud: {
      device: { id: 'cloud', name: 'Cloud' },
      state: 'running',
      operation: null,
      error: null,
      volumes: null,
      limits: { systemBytes: 0, homeBytes: 0 },
    },
    exposes: [
      {
        id: 'k7x2m9qw4p3s6t8v0w2y4z6a8b',
        deviceId: 'laptop',
        address: '127.0.0.1:5173',
        url: 'https://k7x2m9qw4p3s6t8v0w2y4z6a8b.expose.demi.example/',
        createdAt: '2026-09-17T00:00:00.000Z',
        expiresAt: '2026-09-17T00:59:00.000Z',
      },
      {
        id: 'm3n5p7r9t1v3w5x7y9z1a3c5e',
        deviceId: 'cloud',
        address: '127.0.0.1:8080',
        url: 'https://m3n5p7r9t1v3w5x7y9z1a3c5e.expose.demi.example/',
        createdAt: '2026-09-17T00:00:00.000Z',
        expiresAt: '2026-09-17T00:30:00.000Z',
      },
    ],
    exposeDomain: 'expose.demi.example',
  })
  renewals = []
  removals = []
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path === '/api/state') {
      return Response.json(state)
    }
    if (path.startsWith('/api/models')) {
      return Response.json({ providers: [] })
    }
    if (path.startsWith('/api/exposes/') && path.endsWith('/renew') && init?.method === 'POST') {
      const id = path.split('/')[3]!
      renewals.push(id)
      const expose = state.exposes?.find((entry) => entry.id === id)
      if (expose) {
        expose.expiresAt = new Date(Date.now() + 60 * 60_000).toISOString()
      }
      return Response.json({ expose })
    }
    if (path.startsWith('/api/exposes/') && init?.method === 'DELETE') {
      const id = path.split('/')[3]!
      removals.push(id)
      state.exposes = state.exposes?.filter((entry) => entry.id !== id)
      return new Response(null, { status: 204 })
    }
    throw new Error(`Unexpected request: ${path}`)
  }) as typeof fetch
  await useProduct().start()
})

afterEach(() => {
  globalThis.fetch = realFetch
  useProduct().stop()
  disposePinia(pinia)
})

test('the snapshot feeds the expose list', () => {
  const settings = useDeviceSettings()
  expect(settings.exposes.map((expose) => expose.id)).toEqual([
    'k7x2m9qw4p3s6t8v0w2y4z6a8b',
    'm3n5p7r9t1v3w5x7y9z1a3c5e',
  ])
})

test('renew asks the API and shows the moved expiry from the next snapshot', async () => {
  const settings = useDeviceSettings()
  const id = 'k7x2m9qw4p3s6t8v0w2y4z6a8b'
  await settings.renewExpose(id)
  expect(renewals).toEqual([id])
  expect(removals).toEqual([])
  const renewed = settings.exposes.find((expose) => expose.id === id)
  expect(Date.now() - Date.parse(renewed!.expiresAt)).toBeLessThan(60_000)
})

test('remove drops the row once the snapshot returns without it', async () => {
  const settings = useDeviceSettings()
  const id = 'm3n5p7r9t1v3w5x7y9z1a3c5e'
  await settings.removeExpose(id)
  expect(removals).toEqual([id])
  expect(settings.exposes.map((expose) => expose.id)).toEqual([
    'k7x2m9qw4p3s6t8v0w2y4z6a8b',
  ])
})

test('an expose that expires disappears with the next snapshot, without any request', async () => {
  const settings = useDeviceSettings()
  state.exposes = state.exposes?.filter((entry) => entry.deviceId !== 'laptop')
  await useProduct().refresh()
  expect(settings.exposes.map((expose) => expose.deviceId)).toEqual(['cloud'])
  expect(renewals).toEqual([])
  expect(removals).toEqual([])
})

test('an instance without an expose domain lists nothing', async () => {
  state.exposeDomain = null
  state.exposes = []
  await useProduct().refresh()
  expect(useDeviceSettings().exposes).toEqual([])
})
