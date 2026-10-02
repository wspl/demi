import { afterEach, beforeEach, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { productState } from '../__tests__/product-state'
import { playChannels } from '../__tests__/sync-channel'
import type { ExposeState } from '@demicodes/plugin-expose'
import type { ProductState } from '../api/generated/web-api'
import { useDeviceInstallation } from '../devices/pairing'
import { useProduct } from '../state/product'
import { useDeviceSettings } from './devices'

const realFetch = globalThis.fetch
let pinia: ReturnType<typeof createPinia>
let channels: ReturnType<typeof playChannels>
let state: ProductState
/** The `expose` plugin's state the channel brings. */
let expose: ExposeState
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
    publicUrl: 'http://192.168.5.2:3271/',
  })
  expose = {
    available: true,
    exposes: [
      {
        id: 'k7x2maqw4p3s6tavaw2y4z6aab',
        number: 1,
        deviceId: 'laptop',
        address: '127.0.0.1:5173',
        url: 'https://k7x2maqw4p3s6tavaw2y4z6aab.expose.demi.example/',
        expiresAt: '2026-09-17T00:59:00.000Z',
      },
      {
        id: 'm3n5p7rgtxv3w5x7yez4a3c5ek',
        number: 2,
        deviceId: 'cloud',
        address: '127.0.0.1:8080',
        url: 'https://m3n5p7rgtxv3w5x7yez4a3c5ek.expose.demi.example/',
        expiresAt: '2026-09-17T00:30:00.000Z',
      },
    ],
  }
  state.pluginStates = { expose }
  renewals = []
  removals = []
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path.startsWith('/api/models')) {
      return Response.json({ providers: [] })
    }
    if (path === '/api/plugins/expose/calls/renew' && init?.method === 'POST') {
      const id = JSON.parse(String(init.body)).expose
      renewals.push(id)
      const renewed = expose.exposes.find((entry) => entry.id === id)
      if (renewed) {
        renewed.expiresAt = new Date(Date.now() + 60 * 60_000).toISOString()
      }
      channels.last().send({ type: 'plugin', plugin: 'expose', state: expose })
      return Response.json(null)
    }
    if (path === '/api/plugins/expose/calls/remove' && init?.method === 'POST') {
      const id = JSON.parse(String(init.body)).expose
      removals.push(id)
      expose.exposes = expose.exposes.filter((entry) => entry.id !== id)
      channels.last().send({ type: 'plugin', plugin: 'expose', state: expose })
      return Response.json(null)
    }
    throw new Error(`Unexpected request: ${path}`)
  }) as typeof fetch
  channels = playChannels()
  useProduct().start()
  channels.last().connect(state)
})

afterEach(() => {
  globalThis.fetch = realFetch
  useProduct().stop()
  disposePinia(pinia)
  channels.restore()
})

test("the expose plugin's state feeds the expose list", () => {
  const settings = useDeviceSettings()
  expect(settings.exposes.map((expose) => expose.id)).toEqual([
    'k7x2maqw4p3s6tavaw2y4z6aab',
    'm3n5p7rgtxv3w5x7yez4a3c5ek',
  ])
})

test('renew asks the API and shows the moved expiry the channel brings', async () => {
  const settings = useDeviceSettings()
  const id = 'k7x2maqw4p3s6tavaw2y4z6aab'
  await settings.renewExpose(id)
  expect(renewals).toEqual([id])
  expect(removals).toEqual([])
  const renewed = settings.exposes.find((expose) => expose.id === id)
  expect(Date.now() - Date.parse(renewed!.expiresAt)).toBeLessThan(60_000)
})

test('remove drops the row once the channel brings the list without it', async () => {
  const settings = useDeviceSettings()
  const id = 'm3n5p7rgtxv3w5x7yez4a3c5ek'
  await settings.removeExpose(id)
  expect(removals).toEqual([id])
  expect(settings.exposes.map((expose) => expose.id)).toEqual([
    'k7x2maqw4p3s6tavaw2y4z6aab',
  ])
})

test('an expose that expires disappears when the channel brings the list without it, without any request', () => {
  const settings = useDeviceSettings()
  expose.exposes = expose.exposes.filter((entry) => entry.deviceId !== 'laptop')
  channels.last().send({ type: 'plugin', plugin: 'expose', state: expose })
  expect(settings.exposes.map((expose) => expose.deviceId)).toEqual(['cloud'])
  expect(renewals).toEqual([])
  expect(removals).toEqual([])
})

test('an instance without an expose domain lists nothing', () => {
  state.pluginStates = { expose: { available: false, exposes: [] } }
  channels.last().send({ type: 'snapshot', state })
  expect(useDeviceSettings().exposes).toEqual([])
})

test('the install command fetches the installers from the backend the product state names, not the page origin', () => {
  expect(useDeviceInstallation().value).toEqual({
    shellInstallerUrl: 'http://192.168.5.2:3271/install.sh',
    powershellInstallerUrl: 'http://192.168.5.2:3271/install.ps1',
  })
})
