import { afterEach, beforeEach, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { conversationSummary, productState } from '../__tests__/product-state'
import { playChannels } from '../__tests__/sync-channel'
import { useConversations } from '../conversation/store'
import { useProduct } from '../state/product'
import { followDirect } from '.'

// A page that shows a conversation on a paired device whose runner is
// connected opens the device's signaling socket at once, the first step of
// making its peer (`direct-channel.md` § Making the channel), before any
// operation of the conversation needs the device. The backend's channels
// are played; no peer connection is made, since the socket never opens.

const LAPTOP = 'b5c6d7e8-0000-4000-8000-000000000001'
const ON_LAPTOP = '0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b'
const ON_CLOUD = 'e1d2c3b4-0000-4000-8000-000000000002'
let pinia: ReturnType<typeof createPinia>
let channels: ReturnType<typeof playChannels>

beforeEach(() => {
  pinia = createPinia()
  setActivePinia(pinia)
  channels = playChannels()
  useConversations()
  useProduct().start()
  channels.last().connect(productState({
    devices: [{
      id: LAPTOP,
      kind: 'user',
      name: 'laptop',
      platform: 'darwin',
      claimedAt: '2026-09-10T00:00:00.000Z',
      lastSeenAt: null,
      state: 'online',
      home: '/Users/ada',
      installed: [],
      startCommand: null,
      os: null,
      runnerVersion: null,
      route: 'automatic',
    }],
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
