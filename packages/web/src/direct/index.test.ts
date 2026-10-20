import { afterEach, beforeEach, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { conversationSummary, productState } from '../__tests__/product-state'
import { playChannels } from '../__tests__/sync-channel'
import { useConversations } from '../conversation/store'
import { useProduct } from '../state/product'
import { useResources } from '../state/resources'
import { hostDeviceOptions } from '../targets/execution'
import { followDirect, watchDirect } from '.'

// A page that shows a conversation on a paired device whose runner is
// connected opens the device's signaling socket at once, the first step of
// making its peer (`direct-channel.md` § Making the channel), before any
// operation of the conversation needs the device. The backend's channels
// are played; no peer connection is made, since the socket never opens.

const LAPTOP = 'b5c6d7e8-0000-4000-8000-000000000001'
const ON_LAPTOP = '0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b'
const ON_CLOUD = 'e1d2c3b4-0000-4000-8000-000000000002'
const DESK = 'b5c6d7e8-0000-4000-8000-000000000003'
const NEVER = 'b5c6d7e8-0000-4000-8000-000000000004'
const storageDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'localStorage')
/** The browser's local storage, kept across page loads. */
const saved = new Map<string, string>()
let pinia: ReturnType<typeof createPinia>
let channels: ReturnType<typeof playChannels>

beforeEach(() => {
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    value: {
      getItem: (key: string) => saved.get(key) ?? null,
      setItem: (key: string, value: string) => saved.set(key, value),
    },
  })
  pinia = createPinia()
  setActivePinia(pinia)
  channels = playChannels()
  useConversations()
  useProduct().start()
  channels.last().connect(productState({
    devices: [LAPTOP, DESK, NEVER].map((id, index) => ({
      id,
      kind: 'user',
      name: 'laptop',
      platform: 'darwin',
      claimedAt: '2026-09-10T00:00:00.000Z',
      lastSeenAt: null,
      state: 'online',
      home: '/Users/ada',
      installed: [],
      startCommand: null, autostartUnavailable: null,
      os: null,
      runnerVersion: null, downloadPercent: null,
      mark: { logo: 'apple', number: index + 1 },
      route: 'automatic',
    })),
    conversations: [
      conversationSummary(ON_LAPTOP, 'Work', { target: { kind: 'device', deviceId: LAPTOP, path: '/Users/ada/work' } }),
      conversationSummary(ON_CLOUD, 'Cloud work'),
    ],
  }))
  followDirect()
})

afterEach(() => {
  useConversations().stopAll()
  useProduct().stop()
  disposePinia(pinia)
  channels.restore()
  if (storageDescriptor)
    Object.defineProperty(globalThis, 'localStorage', storageDescriptor)
  else
    Reflect.deleteProperty(globalThis, 'localStorage')
})

/** The paths of the device signaling sockets the page opened. */
function signaling(): string[] {
  return channels.opened.map((channel) => new URL(channel.url).pathname).filter((path) => path.endsWith('/direct'))
}

test('showing a conversation on a paired device opens its signaling at once, and one on the Cloud opens none', async () => {
  useProduct().activeConversationId = ON_CLOUD
  await Promise.resolve()
  expect(signaling()).toEqual([])

  useProduct().activeConversationId = ON_LAPTOP
  await Promise.resolve()
  expect(signaling()).toEqual([`/api/devices/${LAPTOP}/direct`])
})

// Run On shows the path this browser found to a device last time, fainter,
// until the page finds a current one. An earlier page found the desk by LAN;
// this one reaches it by the relay, since its signaling never opens here.
test('a device not reached yet shows the path found last time, and the path the page finds replaces it', async () => {
  saved.set(`demi.devicePath.${DESK}`, 'localNetwork')
  expect(runOn(DESK)).toMatchObject({ path: null, lastPath: 'localNetwork' })

  watchDirect(DESK)
  await Promise.resolve()
  expect(runOn(DESK)).toMatchObject({ path: 'relay', lastPath: 'relay' })
})

test('a device this browser never reached shows no path', () => {
  expect(runOn(NEVER)).toMatchObject({ path: null, lastPath: null })
})

/** Device `id`'s row in Run On. */
function runOn(id: string) {
  return hostDeviceOptions(useResources().devices).find((device) => device.id === id)
}
