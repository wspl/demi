import { afterEach, beforeEach, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { createApp, effectScope, nextTick } from 'vue'
import { conversationSummary, productState } from '../__tests__/product-state'
import { showMetadata } from '../__tests__/shown-metadata'
import { playChannels } from '../__tests__/sync-channel'
import { useProduct } from '../state/product'
import { useConversations } from './store'
import { useConversationWorkflows } from './workflows'

// The Workflow menu's list (`crates/backend-user-shard/src/conversation/workflows.rs`) as the
// page reads it, against a played backend: the product channel, the
// conversation's metadata and the workflows routes. The page never follows
// the Host for them: no file watch opens.
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
  installed: [], startCommand: null, autostartUnavailable: null, os: null, runnerVersion: null, downloadPercent: null, mark: { logo: 'tux' as const, number: 1 }, route: 'automatic' as const,
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

/** Moves the conversation `id` to `directory` of the laptop, as its list entry and its metadata tell the page. */
function moveTo(id: string, title: string, directory: string, connect = false): void {
  const summary = conversationSummary(id, title, { target: { kind: 'device', deviceId: LAPTOP, path: directory } })
  if (connect) {
    channels.last().connect(productState({ devices: [laptop], conversations: [summary] }))
  } else {
    channels.last().send({ type: 'conversation', conversation: summary })
  }
  showMetadata(id, { cwd: directory })
}

/** The workflows of `directory` as the backend read them for its `answer`th answer. */
function workflowsOf(directory: string, answer: number) {
  return {
    directory: `${directory}/.demi/workflows`,
    unavailable: null,
    workflows: [{ id: 'review', name: `Review of ${directory} (${answer})`, path: `${directory}/.demi/workflows/review.yaml`, unavailable: null, profiles: ['explore'] }],
  }
}

/** The backend's answer: the list as it read it, with when. */
function kept(directory: string, answer: number) {
  return { state: 'read', list: workflowsOf(directory, answer), readAt: '2026-09-10T00:00:00.000Z' }
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
      cwd = useConversations().metadataOf(CONVERSATION)!.cwd
      return Response.json(kept(cwd, read.length))
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
  moveTo(CONVERSATION, 'Work', '/home/ada/repo-a', true)
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

/** The Workflow menu's list of conversation `id`, as the composer holds it. */
function listOf(id: string) {
  const store = useConversations()
  const scope = effectScope()
  // Within an app, as the composer is.
  const list = scope.run(() => createApp({}).runWithContext(() =>
    useConversationWorkflows(() => store.items.find((item) => item.id === id)!)))!
  return { list, scope }
}

test('a conversation moved to another directory of the same Host lists that directory’s workflows, and follows no Host', async () => {
  const { list, scope } = listOf(CONVERSATION)
  await until(() => list.value.state === 'read')
  expect(list.value).toEqual({ state: 'read', workflows: workflowsOf('/home/ada/repo-a', 1).workflows })

  // The user moves the conversation to repo-b on the same laptop.
  moveTo(CONVERSATION, 'Work', '/home/ada/repo-b')
  await until(() => list.value.state === 'read' && list.value.workflows[0]?.name === 'Review of /home/ada/repo-b (2)')
  expect(read).toHaveLength(2)
  expect(sockets).toEqual([])
  scope.stop()
})

// The backend reads the list again when the Host reports a change, and
// raises the revision the conversation's metadata carries: the page reads
// the list then, and at no other change of the metadata.
test('a page reads the list again when the metadata names a newer revision, and only then', async () => {
  const { list, scope } = listOf(CONVERSATION)
  await until(() => list.value.state === 'read')
  showMetadata(CONVERSATION, { cwd: '/home/ada/repo-a', workflowsRevision: 1 })
  await until(() => list.value.state === 'read' && list.value.workflows[0]?.name === 'Review of /home/ada/repo-a (2)')
  // Another change of the metadata, at the same revision.
  showMetadata(CONVERSATION, { cwd: '/home/ada/repo-a', workflowsRevision: 1, savedRevision: 4 })
  await nextTick()
  showMetadata(CONVERSATION, { cwd: '/home/ada/repo-a', workflowsRevision: 2 })
  await until(() => list.value.state === 'read' && list.value.workflows[0]?.name === 'Review of /home/ada/repo-a (3)')
  expect(read).toHaveLength(3)
  expect(sockets).toEqual([])
  scope.stop()
})

// The backend answers at once, never waiting on the Host: before its first
// read it says it knows nothing yet, and announces the read when it comes.
test('a list the backend has not read yet reads until the backend announces it', async () => {
  const answers = [{ state: 'unknown' }, kept('/home/ada/repo-a', 2)]
  globalThis.fetch = (async (input) => {
    const path = String(input)
    if (path !== `/api/conversations/${CONVERSATION}/workflows`) {
      throw new Error(`Unexpected request: ${path}`)
    }
    read.push(path)
    return Response.json(answers[read.length - 1])
  }) as typeof fetch
  const { list, scope } = listOf(CONVERSATION)
  await until(() => read.length === 1)
  await nextTick()
  expect(list.value).toEqual({ state: 'reading' })
  showMetadata(CONVERSATION, { cwd: '/home/ada/repo-a', workflowsRevision: 1 })
  await until(() => list.value.state === 'read')
  expect(list.value).toEqual({ state: 'read', workflows: workflowsOf('/home/ada/repo-a', 2).workflows })
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
      return Response.json(kept('/home/demi/projects/app', 1))
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
