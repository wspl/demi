import { afterEach, beforeEach, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { effectScope, nextTick } from 'vue'
import { conversationSummary, productState } from '../__tests__/product-state'
import { playChannels } from '../__tests__/sync-channel'
import { useProduct } from '../state/product'
import { conversationFiles } from './files'
import { useConversations } from './store'

// Cost: no socket, no timer; a few microtask turns per product message.

const realFetch = globalThis.fetch
const CONVERSATION = '6a1d2c3b-4e5f-4a6b-8c7d-9e0f1a2b3c4d'
const LAPTOP = 'b5c6d7e8-0000-4000-8000-000000000002'
let pinia: ReturnType<typeof createPinia>
let channels: ReturnType<typeof playChannels>
/** How often the page listed the working tree's changes. */
let listed: number

/** The laptop as the product state carries it; offline, so no file watch opens and nothing confirms a read. */
function laptop(name: string) {
  return {
    id: LAPTOP,
    kind: 'user' as const,
    name,
    platform: 'linux' as const,
    claimedAt: '2026-09-10T00:00:00.000Z',
    lastSeenAt: null,
    state: 'offline' as const,
    home: '/home/ada',
    installed: [],
    startCommand: null,
    os: null,
    runnerVersion: null,
  }
}

beforeEach(() => {
  pinia = createPinia()
  setActivePinia(pinia)
  listed = 0
  globalThis.fetch = (async (input) => {
    const path = String(input)
    if (path === `/api/conversations/${CONVERSATION}/changes`) {
      listed += 1
      return Response.json({ root: '/home/ada/work', repository: true, head: null, files: [], truncated: false, watched: false })
    }
    if (path.startsWith('/api/models')) {
      return Response.json({ providers: [] })
    }
    throw new Error(`Unexpected request: ${path}`)
  }) as typeof fetch
  channels = playChannels()
  useProduct().start()
  useConversations()
  channels.last().connect(productState({
    devices: [laptop('laptop')],
    conversations: [
      conversationSummary(CONVERSATION, 'Work', {
        cwd: '/home/ada/work',
        target: { kind: 'device', deviceId: LAPTOP, path: '/home/ada/work' },
      }),
    ],
  }))
})

afterEach(async () => {
  globalThis.fetch = realFetch
  useProduct().stop()
  // The files service lives for the page's lifetime: it hears the product stop while the stores still stand.
  await nextTick()
  disposePinia(pinia)
  channels.restore()
})

/** Lets the reads a change set started answer. */
async function settle(): Promise<void> {
  for (let turn = 0; turn < 5; turn += 1) {
    await nextTick()
  }
}

test('the changes list is read when shown, and a product state that keeps its Host and directory reads it no more', async () => {
  const scope = effectScope()
  scope.run(() => conversationFiles(CONVERSATION).showChanges())
  await settle()
  expect(listed).toBe(1)
  // The device changes in a way that leaves where the working tree is: renamed, as any device update.
  channels.last().send({ type: 'devices', devices: [laptop('ada’s laptop')] })
  channels.last().send({ type: 'devices', devices: [laptop('laptop')] })
  await settle()
  expect(listed).toBe(1)
  scope.stop()
})
