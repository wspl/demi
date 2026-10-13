import { afterEach, beforeEach, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { createApp, effectScope, nextTick } from 'vue'
import { conversationSummary, productState } from '../__tests__/product-state'
import { playChannels } from '../__tests__/sync-channel'
import { useProduct } from '../state/product'
import { useConversations } from './store'
import { useConversationWorkflows } from './workflows'

// The Workflow menu's list (`subagents.md` § Workflows of a project) as the
// page reads and follows it, against a played backend: the product channel,
// the workflows routes and the file watch.
// Cost: no network, a fake WebSocket; a few event-loop turns each.

const realFetch = globalThis.fetch
const realWindow = Reflect.get(globalThis, 'window')
const CONVERSATION = '6a1d2c3b-4e5f-4a6b-8c7d-9e0f1a2b3c4d'
const LAPTOP = 'b5c6d7e8-0000-4000-8000-000000000002'
let pinia: ReturnType<typeof createPinia>
let channels: ReturnType<typeof playChannels>
/** The workflows routes the page read, in order. */
let read: string[]
const sockets: WatchSocket[] = []

const laptop = {
  id: LAPTOP, kind: 'user' as const, name: 'laptop', platform: 'linux' as const,
  claimedAt: '2026-09-10T00:00:00.000Z', lastSeenAt: null, state: 'online' as const, home: '/home/ada',
  installed: [], startCommand: null, os: null, runnerVersion: null, route: 'automatic' as const,
}

/** A watch socket the page opened, which the test plays as the backend would. */
class WatchSocket extends EventTarget {
  readonly sent: { type: string; subscriptions?: { key: string; path: string }[]; keys?: string[] }[] = []
  constructor(readonly url: string) {
    super()
    sockets.push(this)
  }
  send(data: string): void {
    this.sent.push(JSON.parse(data))
  }
  close(): void {}
}

/** The conversation's summary when it works in `directory` of the laptop. */
function at(directory: string) {
  return conversationSummary(CONVERSATION, 'Work', {
    cwd: directory,
    target: { kind: 'device', deviceId: LAPTOP, path: directory },
  })
}

/** What the backend answers for the workflows of `directory`. */
function workflowsOf(directory: string) {
  return {
    directory: `${directory}/.demi/workflows`,
    unavailable: null,
    workflows: [{ id: 'review', name: `Review of ${directory}`, error: null }],
    watch: [{ kind: 'folder_details', path: `${directory}/.demi/workflows` }],
  }
}

beforeEach(() => {
  sockets.length = 0
  read = []
  if (realWindow === undefined) {
    Reflect.set(globalThis, 'window', { location: { href: 'http://127.0.0.1:3271/chat' } })
  }
  pinia = createPinia()
  setActivePinia(pinia)
  let cwd = '/home/ada/repo-a'
  globalThis.fetch = (async (input) => {
    const path = String(input)
    if (path === `/api/conversations/${CONVERSATION}/workflows`) {
      read.push(path)
      // The backend reads the conversation's directory as it is now.
      cwd = useProduct().snapshot!.conversations[0]!.cwd
      return Response.json(workflowsOf(cwd), { headers: { 'x-demi-read-at': `e.${read.length}` } })
    }
    if (path.startsWith('/api/models')) {
      return Response.json({ providers: [] })
    }
    throw new Error(`Unexpected request: ${path}`)
  }) as typeof fetch
  channels = playChannels()
  const played = globalThis.WebSocket
  const route = function (url: string | URL) {
    return String(url).includes('/fs/watch') ? new WatchSocket(String(url)) : new played(url)
  }
  Reflect.set(globalThis, 'WebSocket', Object.assign(route, { CONNECTING: 0, OPEN: 1, CLOSING: 2, CLOSED: 3 }))
  useProduct().start()
  useConversations()
  channels.last().connect(productState({ devices: [laptop], conversations: [at('/home/ada/repo-a')] }))
})

afterEach(async () => {
  Reflect.set(globalThis, 'window', realWindow)
  globalThis.fetch = realFetch
  useProduct().stop()
  await nextTick()
  disposePinia(pinia)
  channels.restore()
})

/** Lets the event loop turn until `done`, at most 50 times. */
async function until(done: () => boolean): Promise<void> {
  for (let turn = 0; turn < 50 && !done(); turn += 1) {
    await new Promise((resolve) => setTimeout(resolve, 0))
  }
}

test('a conversation moved to another directory of the same Host lists and watches that directory’s workflows', async () => {
  const store = useConversations()
  const scope = effectScope()
  // Within an app, as the composer is: its views are on screen.
  const app = createApp({})
  const list = scope.run(() => app.runWithContext(() =>
    useConversationWorkflows(() => store.items.find((item) => item.id === CONVERSATION)!)))!
  await until(() => list.value.state === 'read')
  expect(list.value).toEqual({ state: 'read', workflows: workflowsOf('/home/ada/repo-a').workflows })
  await until(() => sockets.length > 0)
  sockets[0]!.dispatchEvent(new Event('open'))
  await until(() => sockets[0]!.sent.length > 0)
  const subscribed = () => sockets.flatMap((socket) => socket.sent.flatMap((message) => message.subscriptions ?? []))
  expect(subscribed().map((subscription) => subscription.path)).toEqual(['/home/ada/repo-a/.demi/workflows'])

  // The user moves the conversation to repo-b on the same laptop.
  channels.last().send({ type: 'conversation', conversation: at('/home/ada/repo-b') })
  await until(() => list.value.state === 'read' && list.value.workflows[0]?.name === 'Review of /home/ada/repo-b')
  expect(read).toHaveLength(2)
  expect(list.value).toEqual({ state: 'read', workflows: workflowsOf('/home/ada/repo-b').workflows })
  await until(() => subscribed().length === 2)
  expect(subscribed().map((subscription) => subscription.path)).toEqual([
    '/home/ada/repo-a/.demi/workflows',
    '/home/ada/repo-b/.demi/workflows',
  ])
  // A change in repo-b reads the list again; one in repo-a, which the
  // conversation left, does not.
  const key = (path: string) => subscribed().find((subscription) => subscription.path === path)!.key
  sockets[0]!.dispatchEvent(new MessageEvent('message', { data: JSON.stringify({ type: 'live', key: key('/home/ada/repo-b/.demi/workflows'), since: 0, epoch: 'e' }) }))
  sockets[0]!.dispatchEvent(new MessageEvent('message', { data: JSON.stringify({ type: 'live', key: key('/home/ada/repo-a/.demi/workflows'), since: 0, epoch: 'e' }) }))
  sockets[0]!.dispatchEvent(new MessageEvent('message', { data: JSON.stringify({ type: 'changed', changes: [[key('/home/ada/repo-a/.demi/workflows'), 9]] }) }))
  await until(() => false)
  expect(read).toHaveLength(2)
  sockets[0]!.dispatchEvent(new MessageEvent('message', { data: JSON.stringify({ type: 'changed', changes: [[key('/home/ada/repo-b/.demi/workflows'), 9]] }) }))
  await until(() => read.length === 3)
  expect(read).toHaveLength(3)
  scope.stop()
})
