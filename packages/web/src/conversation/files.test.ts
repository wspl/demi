import { afterEach, beforeEach, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { effectScope, nextTick, shallowRef } from 'vue'
import { conversationSummary, productState } from '../__tests__/product-state'
import { playChannels } from '../__tests__/sync-channel'
import type { FileWatchMessage } from '../api/generated/web-api'
import { useProduct } from '../state/product'
import { conversationFiles, conversationHostSource } from './files'
import { useConversations } from './store'

// Cost: no network, a fake WebSocket; a few microtask turns per product message.

const realFetch = globalThis.fetch
const CONVERSATION = '6a1d2c3b-4e5f-4a6b-8c7d-9e0f1a2b3c4d'
const LAPTOP = 'b5c6d7e8-0000-4000-8000-000000000002'
/** The user's Cloud, attached to the conversation. */
const CLOUD = 'b5c6d7e8-0000-4000-8000-000000000003'
let pinia: ReturnType<typeof createPinia>
let channels: ReturnType<typeof playChannels>
/** How often the page listed the working tree's changes. */
let listed: number
/** The directories the page asked the changes of, `null` for the conversation's own. */
let listedDirectories: (string | null)[]

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
    runnerVersion: null, downloadPercent: null,
    mark: { logo: 'tux' as const, number: 1 },
    route: 'automatic' as const,
  }
}

/** The Cloud as the product state carries it, running or stopped. */
function cloud(state: 'online' | 'offline') {
  return { ...laptop('Cloud'), id: CLOUD, kind: 'managed' as const, state, home: '/home/demi', mark: null }
}

/** A watch socket the page opened, which the test plays as the backend would. */
class WatchSocket extends EventTarget {
  readonly sent: unknown[] = []

  constructor(readonly url: string) {
    super()
    sockets.push(this)
  }

  send(data: string): void {
    this.sent.push(JSON.parse(data))
  }

  close(): void {}

  open(): void {
    this.dispatchEvent(new Event('open'))
  }

  receive(message: FileWatchMessage): void {
    this.dispatchEvent(new MessageEvent('message', { data: JSON.stringify(message) }))
  }

  end(): void {
    this.dispatchEvent(new Event('close'))
  }
}

const sockets: WatchSocket[] = []
const realWindow = Reflect.get(globalThis, 'window')

/** The page's sockets from now on: a file watch is the test's, any other a played channel. */
function routeSockets(): void {
  const played = globalThis.WebSocket
  const route = function (url: string | URL) {
    return String(url).includes('/fs/watch') ? new WatchSocket(String(url)) : new played(url)
  }
  Reflect.set(globalThis, 'WebSocket', Object.assign(route, { CONNECTING: 0, OPEN: 1, CLOSING: 2, CLOSED: 3 }))
}

beforeEach(() => {
  sockets.length = 0
  if (realWindow === undefined) {
    Reflect.set(globalThis, 'window', { location: { href: 'http://127.0.0.1:3271/chat' } })
  }
  pinia = createPinia()
  setActivePinia(pinia)
  listed = 0
  listedDirectories = []
  globalThis.fetch = (async (input) => {
    const path = String(input)
    const changes = new URL(path, 'http://page').pathname === `/api/conversations/${CONVERSATION}/changes`
    if (changes) {
      listed += 1
      const directory = new URL(path, 'http://page').searchParams.get('directory')
      listedDirectories.push(directory)
      return Response.json({ root: directory ?? '/home/ada/work', repository: true, head: null, files: [], truncated: false, watched: false })
    }
    if (path.startsWith(`/api/conversations/${CONVERSATION}/hosts/${CLOUD}/fs?`)) {
      return Response.json(
        { path: '/home/demi/logs', home: '/home/demi', entries: [] },
        { headers: { 'x-demi-read-at': 'e.1' } },
      )
    }
    if (path.startsWith('/api/models')) {
      return Response.json({ providers: [], smallModel: null, imageEntries: {} })
    }
    throw new Error(`Unexpected request: ${path}`)
  }) as typeof fetch
  channels = playChannels()
  routeSockets()
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
  Reflect.set(globalThis, 'window', realWindow)
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

/** Lets the event loop turn until `done`, at most 50 times. */
async function until(done: () => boolean): Promise<void> {
  for (let turn = 0; turn < 50 && !done(); turn += 1) {
    await new Promise((resolve) => setTimeout(resolve, 0))
  }
}

test('the changes list is read when shown, a product state that keeps its Host and directory reads it no more and keeps the workspace, and a view of a child’s directory lists that working tree', async () => {
  const scope = effectScope()
  const files = conversationFiles(CONVERSATION)
  scope.run(() => files.showChanges())
  await settle()
  expect(listed).toBe(1)
  const workspace = files.workspace
  expect(workspace?.root).toBe('/home/ada/work')
  // The device changes in a way that leaves where the working tree is: renamed, as any device update.
  channels.last().send({ type: 'devices', devices: [laptop('ada’s laptop')] })
  channels.last().send({ type: 'devices', devices: [laptop('laptop')] })
  await settle()
  expect(listed).toBe(1)
  // The File view has nothing to render again.
  expect(files.workspace).toBe(workspace)
  scope.stop()

  // A view opened for a child's directory lists that working tree, kept apart from the conversation's.
  const directory = shallowRef<string | undefined>(undefined)
  const child = effectScope()
  child.run(() => files.showChanges(() => directory.value))
  await settle()
  const shown = listedDirectories.length
  directory.value = '/home/ada/other'
  await until(() => listedDirectories.length > shown)
  expect(listedDirectories.slice(shown)).toEqual(['/home/ada/other'])
  expect(files.changes('/home/ada/other')).not.toBe(files.changes())
  // The conversation's own directory named is its working tree, not another one.
  expect(files.changes('/home/ada/work')).toBe(files.changes())
  child.stop()
})

test("the attachment picker watches an attached Cloud's listing on the conversation's watch, naming the Cloud, and again at once when it starts", async () => {
  channels.last().send({ type: 'devices', devices: [laptop('laptop'), cloud('online')] })
  await settle()
  const listing = conversationHostSource(CONVERSATION, CLOUD, false).showListing('/home/demi/logs')
  // The listing subscribes once its read answered, which takes a turn of the event loop.
  await until(() => sockets.length > 0)
  const socket = sockets[0]!
  expect(new URL(socket.url).pathname).toBe(`/api/conversations/${CONVERSATION}/fs/watch`)
  socket.open()
  await settle()
  expect(socket.sent).toEqual([{
    type: 'subscribe',
    subscriptions: [{ key: expect.any(String), kind: 'folder_details', path: '/home/demi/logs', host: CLOUD }],
  }])

  // The Cloud stops, and the watch waits to connect again; once the Cloud
  // runs, it connects at once.
  socket.receive({ type: 'offline' })
  socket.end()
  channels.last().send({ type: 'devices', devices: [laptop('laptop'), cloud('offline')] })
  await settle()
  expect(sockets).toHaveLength(1)
  channels.last().send({ type: 'devices', devices: [laptop('laptop'), cloud('online')] })
  await settle()
  expect(sockets).toHaveLength(2)
  listing.release()
})
