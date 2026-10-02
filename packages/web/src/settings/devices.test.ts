import { afterEach, beforeEach, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { productState } from '../__tests__/product-state'
import { playChannels } from '../__tests__/sync-channel'
import type { ProductState } from '../api/generated/web-api'
import { useDeviceInstallation } from '../devices/pairing'
import { useProduct } from '../state/product'

const realFetch = globalThis.fetch
let pinia: ReturnType<typeof createPinia>
let channels: ReturnType<typeof playChannels>
let state: ProductState

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
        installs: [],
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
  globalThis.fetch = (async (input) => {
    const path = String(input)
    if (path.startsWith('/api/models')) {
      return Response.json({ providers: [] })
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

test('the install command fetches the installers from the backend the product state names, not the page origin', () => {
  expect(useDeviceInstallation().value).toEqual({
    shellInstallerUrl: 'http://192.168.5.2:3271/install.sh',
    powershellInstallerUrl: 'http://192.168.5.2:3271/install.ps1',
  })
})
