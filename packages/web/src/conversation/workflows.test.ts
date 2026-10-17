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
  installed: [], startCommand: null, os: null, runnerVersion: null, downloadPercent: null, mark: { logo: 'tux' as const, number: 1 }, route: 'automatic' as const,
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

/** What the backend answers for the workflows of `directory` at its `answer`th read. */
function workflowsOf(directory: string, answer: number) {
  return {
    directory: `${directory}/.demi/workflows`,
    unavailable: null,
    workflows: [{ id: 'review', name: `Review of ${directory} (${answer})`, path: `${directory}/.demi/workflows/review.yaml`, unavailable: null }],
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
    if (path.split('?')[0] === `/api/conversations/${CONVERSATION}/workflows`) {
      read.push(path)
      // The backend reads the conversation's directory as it is now.
      cwd = useProduct().snapshot!.conversations[0]!.cwd
      return Response.json(workflowsOf(cwd, read.length), { headers: { 'x-demi-read-at': `e.${read.length}` } })
    }
    if (path.startsWith('/api/models')) {
      return Response.json({ providers: [], smallModel: null })
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

const CLOUD = 'c1c2c3c4-0000-4000-8000-000000000001'
const cloud = { ...laptop, id: CLOUD, kind: 'managed' as const, name: 'Cloud', home: '/home/demi', mark: null }

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
  expect(list.value).toEqual({ state: 'read', workflows: workflowsOf('/home/ada/repo-a', 1).workflows })
  await until(() => sockets.length > 0)
  sockets[0]!.dispatchEvent(new Event('open'))
  await until(() => sockets[0]!.sent.length > 0)
  const subscribed = () => sockets.flatMap((socket) => socket.sent.flatMap((message) => message.subscriptions ?? []))
  expect(subscribed().map((subscription) => subscription.path)).toEqual(['/home/ada/repo-a/.demi/workflows'])

  // The user moves the conversation to repo-b on the same laptop.
  channels.last().send({ type: 'conversation', conversation: at('/home/ada/repo-b') })
  await until(() => list.value.state === 'read' && list.value.workflows[0]?.name === 'Review of /home/ada/repo-b (2)')
  expect(read).toHaveLength(2)
  expect(list.value).toEqual({ state: 'read', workflows: workflowsOf('/home/ada/repo-b', 2).workflows })
  await until(() => subscribed().length === 2)
  expect(subscribed().map((subscription) => subscription.path)).toEqual([
    '/home/ada/repo-a/.demi/workflows',
    '/home/ada/repo-b/.demi/workflows',
  ])
  // Each change is numbered after the reads so far, as the Host's are.
  // A change in repo-a, which the conversation left, reads nothing; one in
  // repo-b, told after it, reads the list again. Once that read's answer
  // shows, any read the first had started has reached the backend before
  // it: there is none.
  const key = (path: string) => subscribed().find((subscription) => subscription.path === path)!.key
  sockets[0]!.dispatchEvent(new MessageEvent('message', { data: JSON.stringify({ type: 'live', key: key('/home/ada/repo-b/.demi/workflows'), since: 0, epoch: 'e' }) }))
  sockets[0]!.dispatchEvent(new MessageEvent('message', { data: JSON.stringify({ type: 'live', key: key('/home/ada/repo-a/.demi/workflows'), since: 0, epoch: 'e' }) }))
  sockets[0]!.dispatchEvent(new MessageEvent('message', { data: JSON.stringify({ type: 'changed', changes: [[key('/home/ada/repo-a/.demi/workflows'), read.length + 1]] }) }))
  sockets[0]!.dispatchEvent(new MessageEvent('message', { data: JSON.stringify({ type: 'changed', changes: [[key('/home/ada/repo-b/.demi/workflows'), read.length + 1]] }) }))
  await until(() => list.value.state === 'read' && list.value.workflows[0]?.name === 'Review of /home/ada/repo-b (3)')
  expect(list.value).toEqual({ state: 'read', workflows: workflowsOf('/home/ada/repo-b', 3).workflows })
  expect(read).toHaveLength(3)
  scope.stop()
})

test('a new conversation in a Cloud project reads its project’s workflows and keeps the one the project remembers', async () => {
  const product = useProduct()
  const app = {
    id: 'app', deviceId: CLOUD, path: '/home/demi/projects/app', name: 'App', createdAt: '2026-09-10T00:00:00.000Z',
    lastWorkflow: { type: 'workflow' as const, id: 'review' },
  }
  channels.last().send({ type: 'snapshot', state: productState({ devices: [laptop, cloud], workspaces: [app], conversations: [] }) })
  await nextTick()
  // The device route answers the project's directory, once the test says.
  let answer!: () => void
  const answered = new Promise<void>((resolve) => { answer = resolve })
  const backend = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path.startsWith(`/api/devices/${CLOUD}/workflows?`)) {
      read.push(path)
      await answered
      return Response.json(workflowsOf('/home/demi/projects/app', 1), { headers: { 'x-demi-read-at': 'e.1' } })
    }
    return backend(input, init)
  }) as typeof fetch
  const store = useConversations()
  const id = store.create('app')
  const conversation = store.items.find((item) => item.id === id)!
  expect(conversation.workflow).toEqual({ type: 'workflow', id: 'review' })
  const scope = effectScope()
  const list = scope.run(() => createApp({}).runWithContext(() =>
    useConversationWorkflows(() => store.items.find((item) => item.id === id)!)))!
  await until(() => read.length > 0)
  expect(read).toEqual([`/api/devices/${CLOUD}/workflows?directory=%2Fhome%2Fdemi%2Fprojects%2Fapp`])
  // Not read yet is no reason to give the remembered workflow up.
  expect(list.value.state).toBe('reading')
  expect(conversation.workflow).toEqual({ type: 'workflow', id: 'review' })
  answer()
  await until(() => list.value.state === 'read')
  expect(conversation.workflow).toEqual({ type: 'workflow', id: 'review' })
  expect(product.snapshot!.workspaces[0]!.lastWorkflow).toEqual({ type: 'workflow', id: 'review' })
  scope.stop()
})

test('a watch that went live after the list was read reads it once more, after its number, and then rests', async () => {
  // The backend answers from the search it keeps, with that search's
  // number, unless the page asks for one read at or after a number: then it
  // searches again and answers with the new one.
  // A conversation and directory of its own: the kept files and the
  // watches of the page outlive a test.
  const other = '7b2e3d4c-5f60-4b7c-9d8e-0f1a2b3c4d5e'
  channels.last().send({ type: 'conversation', conversation: conversationSummary(other, 'Other', {
    cwd: '/home/ada/repo-c',
    target: { kind: 'device', deviceId: LAPTOP, path: '/home/ada/repo-c' },
  }) })
  const asked: string[] = []
  globalThis.fetch = (async (input) => {
    const url = new URL(String(input), 'http://backend')
    if (url.pathname !== `/api/conversations/${other}/workflows`)
      throw new Error(`Unexpected request: ${url}`)
    asked.push(url.search)
    // On a later turn, as a network answer comes: a page that asks again
    // without end then still lets the test see it.
    await new Promise((resolve) => setTimeout(resolve, 0))
    const after = url.searchParams.get('readAfter')
    return Response.json(workflowsOf('/home/ada/repo-c', asked.length), { headers: { 'x-demi-read-at': after ?? 'e.10' } })
  }) as typeof fetch
  const store = useConversations()
  const scope = effectScope()
  const list = scope.run(() => createApp({}).runWithContext(() =>
    useConversationWorkflows(() => store.items.find((item) => item.id === other)!)))!
  await until(() => list.value.state === 'read')
  await until(() => sockets.length > 0)
  sockets[0]!.dispatchEvent(new Event('open'))
  await until(() => sockets[0]!.sent.length > 0)
  const key = sockets[0]!.sent.flatMap((message) => message.subscriptions ?? [])[0]!.key
  // The Host watches the folder from 11, after the answer held, read at 10.
  sockets[0]!.dispatchEvent(new MessageEvent('message', { data: JSON.stringify({ type: 'live', key, since: 11, epoch: 'e' }) }))
  await until(() => asked.length > 2)
  expect(asked).toEqual(['', '?readAfter=e.11'])
  expect(list.value).toEqual({ state: 'read', workflows: workflowsOf('/home/ada/repo-c', 2).workflows })
  scope.stop()
})
