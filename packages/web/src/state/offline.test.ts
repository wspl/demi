import { afterEach, beforeEach, expect, setSystemTime, spyOn, test } from 'bun:test'
import { deferred, waitFor } from '@demicodes/utils'
import { IDBFactory, IDBKeyRange, IDBObjectStore } from 'fake-indexeddb'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import type { Block } from '@demicodes/protocol'
import { heldBlocks } from '@demicodes/web-ui/transport/protocol'
import { pageReturned } from '@demicodes/web-ui/transport/liveness'
import { useSession } from '../auth/session'
import { useConversations } from '../conversation/store'
import { conversationSummary as summary, productState } from '../__tests__/product-state'
import { playChannels } from '../__tests__/sync-channel'
import { readAccountCopy, readConversationCopy, startAccountCache } from './offline'
import { useProduct } from './product'

// The account's cache for opening offline (`web-application.md` § Installing
// and opening offline), through the page's stores as the page starts them:
// one page reads conversations while the backend answers, and a later page
// of the same web browser starts without the backend, or with it. The
// backend is the test's fetch, the database fake-indexeddb's. Each test
// takes a few tens of milliseconds.

const FIRST = '00000000-0000-4000-8000-000000000001'
const SECOND = '00000000-0000-4000-8000-000000000002'
const THIRD = '00000000-0000-4000-8000-000000000003'
const USER = productState().user
const MODEL = {
  providerId: 'stub', thinking: null, serviceTierId: null,
  model: { id: 'stub', name: 'Stub', contextWindow: 1000, outputLimit: null, thinking: [], acceptedExtensions: [] },
}

/** A message the user sent, as a block of the transcript. */
function message(id: string, text: string): Block {
  return {
    type: 'user', id, turnId: `turn-${id}`, createdAt: '2026-10-16T00:00:00.000Z', model: MODEL,
    content: [{ type: 'text', text }], preamble: null,
  }
}

const realFetch = globalThis.fetch
const realIndexedDB = globalThis.indexedDB
const realKeyRange = globalThis.IDBKeyRange
let pinia: ReturnType<typeof createPinia>
let channels: ReturnType<typeof playChannels>
/** The page's lifetime: its startup's signal, and the stop of its account cache. */
let page: { startup: AbortController; cache: ReturnType<typeof startAccountCache> } | null
/** Whether the backend can be reached. */
let reachable: boolean
/** What the backend's transcripts hold, by conversation. */
let transcripts: Map<string, Block[]>
/** Holds the next transcript's answer until the test lets it go, and says when it was asked. */
let holdTranscript: { asked: () => void; answer: Promise<void> } | null
/** Whether the backend still knows the page's session. */
let sessionAlive: boolean
/** What the backend answers who is signed in, when a test says otherwise than `sessionAlive`. */
let answerSession: (() => Response) | null

beforeEach(() => {
  globalThis.indexedDB = new IDBFactory()
  globalThis.IDBKeyRange = IDBKeyRange
  channels = playChannels()
  reachable = true
  sessionAlive = true
  answerSession = null
  holdTranscript = null
  transcripts = new Map([
    [FIRST, [message('first-1', 'Fix the login'), message('first-2', 'And the tests')]],
    [SECOND, [message('second-1', 'Second question')]],
    [THIRD, [message('third-1', 'Third question')]],
  ])
  page = null
  globalThis.fetch = (async (input, init) => {
    if (!reachable) {
      throw new TypeError('Failed to fetch')
    }
    const path = String(input)
    if (path === '/api/auth/me' && answerSession) {
      return answerSession()
    }
    if (path === '/api/auth/me') {
      return sessionAlive
        ? Response.json({ user: USER })
        : Response.json({ requestId: 'req_test000000000001', code: 'unauthenticated', message: 'Sign in' }, { status: 401 })
    }
    if (path === '/api/setup') return Response.json({ needed: false, passkeys: null })
    if (path === '/api/auth/logout') return new Response(null, { status: 204 })
    if (path.startsWith('/api/models')) return Response.json({ providers: [], smallModel: null })
    if (path.endsWith('/hosts')) return Response.json({ hosts: [] })
    if (path.endsWith('/subagents')) return Response.json({ subagents: [] })
    if (path.endsWith('/draft')) return Response.json({ draft: { revision: 0, text: '', files: [], replaced: null } })
    if (path.endsWith('/saved')) return Response.json({ revision: 0, messages: [] })
    if (path.endsWith('/read') && init?.method === 'POST') return new Response(null, { status: 204 })
    const transcript = /^\/api\/conversations\/([^/]+)\/transcript$/.exec(path)
    if (transcript) {
      const held = holdTranscript
      holdTranscript = null
      held?.asked()
      await held?.answer
      const blocks = transcripts.get(decodeURIComponent(transcript[1]!)) ?? []
      return Response.json({ start: 0, length: blocks.length, blocks, instructions: [], summaries: [] })
    }
    throw new Error(`Unexpected request: ${path}`)
  }) as typeof fetch
})

afterEach(() => {
  closePage()
  channels.restore()
  globalThis.fetch = realFetch
  globalThis.indexedDB = realIndexedDB
  globalThis.IDBKeyRange = realKeyRange
  setSystemTime()
})

/** The backend's state, with its conversations. */
function backendState(titles: Record<string, string> = { [FIRST]: 'First', [SECOND]: 'Second', [THIRD]: 'Third' }) {
  return productState({ conversations: Object.entries(titles).map(([id, title]) => summary(id, title)) })
}

/** A page starts as `main.ts` starts it: its state, its account cache, and its session's first check. */
function openPage(): void {
  pinia = createPinia()
  setActivePinia(pinia)
  const product = useProduct()
  const session = useSession()
  useConversations()
  const startup = new AbortController()
  product.start()
  const cache = startAccountCache(session, product)
  void session.restore(startup.signal, cache.signInOffline).catch(() => {})
  page = { startup, cache }
}

/** The page goes, as a closed tab does. */
function closePage(): void {
  if (!page) {
    return
  }
  page.startup.abort()
  page.cache.stop()
  useConversations().stopAll()
  useProduct().stop()
  disposePinia(pinia)
  page = null
}

/** The page shows its conversations once signed in, as `main.ts` starts them. */
async function signedIn(): Promise<void> {
  await waitFor(() => useSession().signedIn, () => `the session is ${useSession().current.status}`)
  void useConversations().initialize()
}

/** A page that reaches the backend reads `ids` one after another, and keeps them as it hides. */
async function readOnline(ids: string[]): Promise<void> {
  openPage()
  await signedIn()
  channels.last().connect(backendState())
  const conversations = useConversations()
  for (const id of ids) {
    void conversations.activate(id)
    await waitFor(() => conversations.items.find((item) => item.id === id)?.load === 'ready')
  }
  await conversations.keepCopies()
  await page!.cache.settled()
}

const titles = () => useConversations().listed.map((conversation) => conversation.title)
const texts = (id: string) =>
  heldBlocks(useConversations().items.find((item) => item.id === id)!.history)
    .map((block) => (block.type === 'user' && block.content[0]?.type === 'text' ? block.content[0].text : block.type))

test('a page that starts without the backend is signed in with the cached sidebar, a cached conversation opens, and one not cached opens from the backend once it answers', async () => {
  await readOnline([FIRST])
  closePage()

  reachable = false
  openPage()
  await signedIn()
  expect(useSession().user?.id).toBe(USER.id)
  expect(titles()).toEqual(['First', 'Second', 'Third'])
  const conversations = useConversations()
  void conversations.activate(FIRST)
  await waitFor(() => conversations.items.find((item) => item.id === FIRST)?.load === 'ready')
  expect(texts(FIRST)).toEqual(['Fix the login', 'And the tests'])
  // One the page never opened says so.
  void conversations.activate(SECOND)
  await waitFor(() => conversations.items.find((item) => item.id === SECOND)?.load === 'offline')
  expect(texts(SECOND)).toEqual([])

  // The backend answers again: the conversation the page shows opens from it.
  transcripts.set(SECOND, [message('second-1', 'Second question'), message('second-2', 'Since then')])
  reachable = true
  await pageReturned()
  channels.last().connect(backendState())
  await waitFor(() => conversations.items.find((item) => item.id === SECOND)?.load === 'ready')
  expect(texts(SECOND)).toEqual(['Second question', 'Since then'])
})

test('a page that reaches the backend shows the copy until the snapshot, and reads each conversation from the backend', async () => {
  await readOnline([FIRST])
  closePage()

  transcripts.set(FIRST, [message('first-1', 'Fix the login'), message('first-2', 'And the tests'), message('first-3', 'Since then')])
  openPage()
  await signedIn()
  // The copy, before the channel's first snapshot.
  expect(titles()).toEqual(['First', 'Second', 'Third'])
  channels.last().connect(backendState({ [FIRST]: 'Renamed' }))
  expect(titles()).toEqual(['Renamed'])

  const asked = deferred<void>()
  const answer = deferred<void>()
  holdTranscript = { asked: () => asked.resolve(), answer: answer.promise }
  const conversations = useConversations()
  void conversations.activate(FIRST)
  // Online the conversation loads from the backend, never from the copy:
  // once a read of the database that starts after any the opening made has
  // answered, the conversation still loads. The database answers its reads
  // in order, and the first read here may start before the opening's.
  await asked.promise
  await readConversationCopy(USER.id, FIRST)
  expect(await readConversationCopy(USER.id, FIRST)).not.toBeNull()
  expect(conversations.items[0]!.load).toBe('loading')
  expect(texts(FIRST)).toEqual([])
  answer.resolve()
  await waitFor(() => conversations.items[0]!.load === 'ready')
  expect(texts(FIRST)).toEqual(['Fix the login', 'And the tests', 'Since then'])
})

test('signing out, or a session the backend no longer knows, deletes the cache', async () => {
  await readOnline([FIRST])
  expect(await readAccountCopy()).not.toBeNull()
  await useSession().signOut()
  expect(await readAccountCopy()).toBeNull()
  expect(await readConversationCopy(USER.id, FIRST)).toBeNull()
  closePage()

  await readOnline([FIRST])
  closePage()
  sessionAlive = false
  openPage()
  await waitFor(() => useSession().current.status === 'signedOut')
  expect(await readAccountCopy()).toBeNull()
  expect(await readConversationCopy(USER.id, FIRST)).toBeNull()
})

test('a session that ended takes the cache along at once, though the page cannot learn more, and no later write brings it back', async () => {
  await readOnline([FIRST])
  // The channel says the session ended; asking who is signed in then fails.
  answerSession = () => Response.json({ requestId: 'req_test000000000001', code: 'internal_error', message: 'Failed' }, { status: 500 })
  channels.last().end(4002, 'session_ended')
  // What the page still shows changes, and its copies are written.
  const conversations = useConversations()
  void conversations.activate(SECOND)
  await waitFor(() => conversations.items.find((item) => item.id === SECOND)?.load === 'ready')
  await conversations.keepCopies()
  await page!.cache.settled()
  expect(await readAccountCopy()).toBeNull()
  expect(await readConversationCopy(USER.id, FIRST)).toBeNull()
  expect(await readConversationCopy(USER.id, SECOND)).toBeNull()
})

test('a page of another build drops the cache an earlier build wrote', async () => {
  process.env.DEMI_WEB_BUILD = 'b1'
  try {
    await readOnline([FIRST])
    closePage()
    process.env.DEMI_WEB_BUILD = 'b2'
    reachable = false
    openPage()
    await page!.cache.signInOffline()
    // Not signed in: it waits for the backend, as a page without a cache does.
    expect(useSession().current.status).toBe('checking')
    process.env.DEMI_WEB_BUILD = 'b1'
    expect(await readAccountCopy()).toBeNull()
    expect(await readConversationCopy(USER.id, FIRST)).toBeNull()
  } finally {
    delete process.env.DEMI_WEB_BUILD
  }
})

test('when the web browser refuses a copy for lack of room, the conversations opened longest ago go', async () => {
  // Room for two conversations' copies, as a browser's quota allows.
  const held = new Set<IDBValidKey>()
  const put = IDBObjectStore.prototype.put
  const remove = IDBObjectStore.prototype.delete
  const puts = spyOn(IDBObjectStore.prototype, 'put').mockImplementation(function (this: IDBObjectStore, value: unknown, key?: IDBValidKey) {
    if (this.name === 'conversations' && key !== undefined) {
      if (!held.has(key) && held.size >= 2) {
        throw new DOMException('The quota has been exceeded.', 'QuotaExceededError')
      }
      held.add(key)
    }
    return put.call(this, value, key)
  })
  const deletes = spyOn(IDBObjectStore.prototype, 'delete').mockImplementation(function (this: IDBObjectStore, key: IDBValidKey | IDBKeyRange) {
    if (this.name === 'conversations' && !(key instanceof IDBKeyRange)) {
      held.delete(key)
    }
    return remove.call(this, key)
  })
  try {
    openPage()
    await signedIn()
    channels.last().connect(backendState())
    const conversations = useConversations()
    for (const [at, id] of [[1, SECOND], [2, FIRST], [3, THIRD]] as const) {
      setSystemTime(new Date(Date.UTC(2026, 9, 16, at)))
      void conversations.activate(id)
      await waitFor(() => conversations.items.find((item) => item.id === id)?.load === 'ready')
      await conversations.keepCopies()
    }
    expect(await readConversationCopy(USER.id, SECOND)).toBeNull()
    expect((await readConversationCopy(USER.id, FIRST))?.transcript.blocks.map((block) => block.id)).toEqual(['first-1', 'first-2'])
    expect((await readConversationCopy(USER.id, THIRD))?.transcript.blocks.map((block) => block.id)).toEqual(['third-1'])
  } finally {
    puts.mockRestore()
    deletes.mockRestore()
  }
})
