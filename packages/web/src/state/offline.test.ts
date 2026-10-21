import { afterEach, beforeEach, expect, setSystemTime, spyOn, test } from 'bun:test'
import { deferred, waitFor } from '@demicodes/utils'
import { IDBFactory, IDBKeyRange, IDBObjectStore } from 'fake-indexeddb'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import type { Block, HeldEdge } from '@demicodes/protocol'
import { heldBlocks } from '@demicodes/web-ui/transport/protocol'
import { pageReturned } from '@demicodes/web-ui/transport/liveness'
import { NO_COST } from '@demicodes/web-ui/agent/context-cost'
import { TestSocket, playSockets } from '@demicodes/web-ui/transport/__tests__/test-socket'
import { useSession } from '../auth/session'
import { useConversations } from '../conversation/store'
import { conversationMetadata, conversationSummary as summary, productState } from '../__tests__/product-state'
import { playChannels } from '../__tests__/sync-channel'
import { deleteAccountCache, readAccountCopy, readConversationCopy, readNodeCopy, startAccountCache } from './offline'
import { useProduct } from './product'

// The account's cache for opening offline, through the page's stores as the
// page starts them: one page reads conversations while the backend answers, and
// a later page of the same web browser starts without the backend, or with it.
// The backend is the test's fetch and a played conversation socket, the
// database fake-indexeddb's. Each test takes a few tens of milliseconds.

const FIRST = '00000000-0000-4000-8000-000000000001'
const SECOND = '00000000-0000-4000-8000-000000000002'
const THIRD = '00000000-0000-4000-8000-000000000003'
const USER = productState().user
/** The blocks of a transcript's page, as the backend's latest page and a page before a window hold them. */
const PAGE = 100
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
/** Holds the next socket's answer to its open until the test lets it go. */
let holdOpen: { asked: () => void; answer: Promise<void> } | null
/** What each socket's page named of what it holds of each node, as its subscriptions said. */
let subscribed: Map<string, HeldEdge | undefined>[]
let sockets: ReturnType<typeof playSockets>
/** The transcript pages the backend served, as `before=<index>` or `latest`. */
let pagesRead: string[]
/** Whether the backend still knows the page's session. */
let sessionAlive: boolean
/** What the backend answers who is signed in, when a test says otherwise than `sessionAlive`. */
let answerSession: (() => Response) | null

beforeEach(async () => {
  globalThis.indexedDB = new IDBFactory()
  globalThis.IDBKeyRange = IDBKeyRange
  // The page's database stays open across tests: each starts with it empty.
  await deleteAccountCache()
  channels = playChannels()
  reachable = true
  sessionAlive = true
  answerSession = null
  holdOpen = null
  subscribed = []
  pagesRead = []
  // The sync channel's socket is the played channel's.
  const channel: unknown = globalThis.WebSocket
  sockets = playSockets()
  // Each conversation socket the page opens is the backend's while it
  // answers, and ends at once while it does not.
  class ConversationSocket extends TestSocket {
    readonly nodes = new Map<string, HeldEdge | undefined>()
    constructor(url: string) {
      super(url)
      subscribed.push(this.nodes)
      queueMicrotask(() => (reachable ? this.open() : this.end()))
    }
    override send(data: string): void {
      super.send(data)
      const frame = this.sent.at(-1)
      if (frame?.type === 'subscribe') {
        this.nodes.set(frame.node, frame.held ?? undefined)
      } else if (frame?.type === 'open') {
        void this.answerOpen()
      }
    }
    /** The open's answer: the conversation as the backend shows it, and each subscribed node from what the page holds. */
    async answerOpen(): Promise<void> {
      const held = holdOpen
      holdOpen = null
      held?.asked()
      await held?.answer
      const id = decodeURIComponent(/conversations\/([^/]+)\/stream/.exec(this.url)![1]!)
      const frames: unknown[] = [
        { type: 'opened' },
        { type: 'conversation', metadata: conversationMetadata(id) },
        {
          type: 'agents',
          version: { epoch: 'e', revision: 1 },
          agents: [{ id, role: { kind: 'root' }, phase: 'idle', waitedMs: null, pendingCalls: [], context: null, cost: NO_COST, queue: [], pendingSteers: [], starting: null }],
        },
        { type: 'request_output', output: null },
      ]
      for (const [node, edge] of this.nodes) {
        const blocks = transcripts.get(node) ?? []
        const start = edge && blocks[edge.from]?.id === edge.edge ? edge.from : Math.max(0, blocks.length - PAGE)
        frames.push({
          type: 'transcript_reset', node, start, length: blocks.length,
          blocks: blocks.slice(start), version: { epoch: 'e', revision: blocks.length },
        })
      }
      this.dispatchEvent(new MessageEvent('message', { data: JSON.stringify(frames) }))
    }
  }
  Reflect.set(globalThis, 'WebSocket', new Proxy(TestSocket, {
    construct: (_target, [url]: [string]) =>
      /\/conversations\/[^/]+\/stream/.test(url) ? new ConversationSocket(url) : Reflect.construct(channel as typeof TestSocket, [url]),
  }))
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
    const transcript = /\/agents\/([^/]+)\/transcript\?(.*)$/.exec(path)
    if (transcript) {
      const blocks = transcripts.get(decodeURIComponent(transcript[1]!)) ?? []
      const before = new URLSearchParams(transcript[2]).get('before')
      pagesRead.push(before === null ? 'latest' : `before=${before}`)
      const end = before === null ? blocks.length : Number(before)
      const start = Math.max(0, end - PAGE)
      return Response.json({ start, length: blocks.length, blocks: blocks.slice(start, end), instructions: [], summaries: [] })
    }
    if (path.endsWith('/draft')) return Response.json({ draft: { revision: 0, text: '', files: [], comments: [], replaced: null } })
    if (path.endsWith('/saved')) return Response.json({ revision: 0, messages: [] })
    if (path.endsWith('/read') && init?.method === 'POST') return new Response(null, { status: 204 })
    throw new Error(`Unexpected request: ${path}`)
  }) as typeof fetch
})

afterEach(() => {
  closePage()
  channels.restore()
  sockets.restore()
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
  // It shows what the disk keeps, and says it connects.
  await waitFor(() => conversations.items.find((item) => item.id === FIRST)?.load === 'connecting')
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

// A reload shows what was seen at once and catches up by changes alone
// (`docs/decisions/0009-agent-sessions-and-sync.md`): the conversation shows its
// disk copy while the backend has not answered, and its subscription names
// the blocks it holds, after which the backend sends the rest.
test('a page that reaches the backend shows the copy at once, and catches up from the blocks it holds', async () => {
  await readOnline([FIRST])
  closePage()

  transcripts.set(FIRST, [message('first-1', 'Fix the login'), message('first-2', 'And the tests'), message('first-3', 'Since then')])
  openPage()
  await signedIn()
  // The copy, before the channel's first snapshot. The page signs in once
  // the backend answers, and shows the copy once the database answers,
  // whichever comes first.
  await waitFor(() => titles().length > 0, () => 'the page shows no copy')
  expect(titles()).toEqual(['First', 'Second', 'Third'])
  channels.last().connect(backendState({ [FIRST]: 'Renamed' }))
  expect(titles()).toEqual(['Renamed'])

  const asked = deferred<void>()
  const answer = deferred<void>()
  holdOpen = { asked: () => asked.resolve(), answer: answer.promise }
  const conversations = useConversations()
  void conversations.activate(FIRST)
  await asked.promise
  expect(texts(FIRST)).toEqual(['Fix the login', 'And the tests'])
  expect(subscribed.at(-1)!.get(FIRST)).toMatchObject({ from: 1, edge: 'first-2' })
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

test('a copy another build wrote in its own shape is dropped without a word, and only a copy of this build that does not read is reported', async () => {
  const errors = spyOn(console, 'error')
  /** What the page reported of a copy it could not read. */
  const unreadable = () =>
    errors.mock.calls.filter((call) => String(call[0]).includes('Could Not Read the Copy for Offline Use')).length
  // The records are written in a shape this build's schema refuses, as an
  // earlier build's own shape would be.
  const put = IDBObjectStore.prototype.put
  const puts = spyOn(IDBObjectStore.prototype, 'put').mockImplementation(function (this: IDBObjectStore, value: unknown, key?: IDBValidKey) {
    const older = this.name === 'account' ? { state: 'an earlier shape' } : this.name === 'conversations' ? { metadata: 'an earlier shape' } : {}
    return put.call(this, { ...(value as object), ...older }, key)
  })
  process.env.DEMI_WEB_BUILD = 'b1'
  try {
    await readOnline([FIRST])
    closePage()
    process.env.DEMI_WEB_BUILD = 'b2'
    expect(await readConversationCopy(USER.id, FIRST)).toBeNull()
    expect(await readAccountCopy()).toBeNull()
    expect(unreadable()).toBe(0)

    // The same shape from this build is a defect, and says so, once a copy.
    process.env.DEMI_WEB_BUILD = 'b1'
    await readOnline([FIRST])
    closePage()
    expect(await readConversationCopy(USER.id, FIRST)).toBeNull()
    expect(await readAccountCopy()).toBeNull()
    expect(unreadable()).toBe(2)
  } finally {
    delete process.env.DEMI_WEB_BUILD
    puts.mockRestore()
    errors.mockRestore()
  }
})

/** The root's blocks the disk keeps whole of conversation `id`, by id; none when it keeps none. */
async function kept(id: string): Promise<string[] | null> {
  const read = await readNodeCopy(USER.id, id)
  return read ? heldBlocks(read.transcript).map((block) => block.id) : null
}

/**
 * Room for the blocks of two nodes, as a browser's quota allows: a write
 * of a third node's blocks is refused, its whole transaction with it.
 * Returns what gives the room back.
 */
function roomForTwo(): () => void {
  const held = new Map<string, Set<number>>()
  const put = IDBObjectStore.prototype.put
  const remove = IDBObjectStore.prototype.delete
  const puts = spyOn(IDBObjectStore.prototype, 'put').mockImplementation(function (this: IDBObjectStore, value: unknown, key?: IDBValidKey) {
    if (this.name === 'blocks' && Array.isArray(key)) {
      const [node, index] = key as [string, number]
      if (!held.has(node) && held.size >= 2) {
        this.transaction.abort()
        throw new DOMException('The quota has been exceeded.', 'QuotaExceededError')
      }
      held.set(node, (held.get(node) ?? new Set()).add(index))
    }
    return put.call(this, value, key)
  })
  const deletes = spyOn(IDBObjectStore.prototype, 'delete').mockImplementation(function (this: IDBObjectStore, key: IDBValidKey | IDBKeyRange) {
    if (this.name === 'blocks' && Array.isArray(key)) {
      const [node, index] = key as [string, number]
      held.get(node)?.delete(index)
      if (held.get(node)?.size === 0) {
        held.delete(node)
      }
    }
    return remove.call(this, key)
  })
  return () => {
    puts.mockRestore()
    deletes.mockRestore()
  }
}

/** Opens each of `ids` in turn, an hour apart, and keeps its copy. */
async function openInTurn(ids: readonly (readonly [number, string])[]): Promise<void> {
  const conversations = useConversations()
  for (const [at, id] of ids) {
    setSystemTime(new Date(Date.UTC(2026, 9, 16, at)))
    void conversations.activate(id)
    await waitFor(() => conversations.items.find((item) => item.id === id)?.load === 'ready')
    await conversations.keepCopies()
  }
}

test('when the web browser refuses a write for lack of room, the node viewed longest ago goes, its conversation\'s record with it', async () => {
  const giveBack = roomForTwo()
  try {
    openPage()
    await signedIn()
    channels.last().connect(backendState())
    await openInTurn([[1, SECOND], [2, FIRST], [3, THIRD]])
    expect(await kept(SECOND)).toBeNull()
    expect(await readConversationCopy(USER.id, SECOND)).toBeNull()
    expect(await kept(FIRST)).toEqual(['first-1', 'first-2'])
    expect(await kept(THIRD)).toEqual(['third-1'])
  } finally {
    giveBack()
  }
})

// The disk keeps under the bound the browser grants: past it, the node
// viewed longest ago goes while the page is idle.
test('past the disk\'s bound, the node viewed longest ago goes once the page is idle', async () => {
  /** What one of the conversation's blocks takes on disk, about. */
  const bytes = (id: string) => transcripts.get(id)!.reduce((sum, block) => sum + JSON.stringify({ block }).length, 0)
  // Room for the first and the third, which the second's and the first's fit in too.
  const quota = bytes(FIRST) + Math.max(bytes(SECOND), bytes(THIRD))
  Object.defineProperty(navigator, 'storage', { configurable: true, value: { estimate: async () => ({ quota }) } })
  // The eviction's last step deletes the node's record.
  const evicted = deferred<string>()
  const remove = IDBObjectStore.prototype.delete
  const deletes = spyOn(IDBObjectStore.prototype, 'delete').mockImplementation(function (this: IDBObjectStore, key: IDBValidKey | IDBKeyRange) {
    if (this.name === 'nodes') {
      evicted.resolve(String(key))
    }
    return remove.call(this, key)
  })
  try {
    openPage()
    await signedIn()
    channels.last().connect(backendState())
    await openInTurn([[1, SECOND], [2, FIRST], [3, THIRD]])
    expect(await evicted.promise).toBe(SECOND)
    await page!.cache.settled()
    expect(await kept(SECOND)).toBeNull()
    expect(await kept(FIRST)).toEqual(['first-1', 'first-2'])
    expect(await kept(THIRD)).toEqual(['third-1'])
  } finally {
    deletes.mockRestore()
    Reflect.deleteProperty(navigator, 'storage')
  }
})

// A reload that shows a conversation from the disk writes when its node was
// viewed, never its blocks again: the copy writer knows them as written.
test('showing a kept conversation from the disk again writes when its node was viewed, never its blocks', async () => {
  await readOnline([FIRST, SECOND])
  closePage()
  const written: string[] = []
  const put = IDBObjectStore.prototype.put
  const puts = spyOn(IDBObjectStore.prototype, 'put').mockImplementation(function (this: IDBObjectStore, value: unknown, key?: IDBValidKey) {
    written.push(`${this.name} ${String(key)}`)
    return put.call(this, value, key)
  })
  try {
    openPage()
    await signedIn()
    channels.last().connect(backendState())
    const conversations = useConversations()
    void conversations.activate(FIRST)
    await waitFor(() => conversations.items.find((item) => item.id === FIRST)?.load === 'ready')
    await conversations.keepCopies()
    expect(written).toContain(`nodes ${FIRST}`)
    // The edge block the subscription named comes again with the reset, and only it.
    expect(written.filter((write) => write.startsWith('blocks '))).toEqual([`blocks ${FIRST},1`])
    expect(await kept(FIRST)).toEqual(['first-1', 'first-2'])
  } finally {
    puts.mockRestore()
  }
})

/** The page reads back to the start of conversation `id`'s root, a page at a time. */
async function readToStart(id: string): Promise<void> {
  const conversations = useConversations()
  const conversation = conversations.items.find((item) => item.id === id)!
  while (conversation.history.windows[0]?.start !== 0 || conversation.history.windows.length > 1) {
    const before = conversation.history.windows[0]?.start
    await conversations.history.readBefore(conversation, null)
    expect(conversation.history.windows[0]?.start).toBeLessThan(before!)
  }
}

// A reload of a long conversation read its earlier pages from the backend
// again, though the disk kept them: the page asked the backend for the page
// before its window, and each write replaced the disk's runs with the ones
// the page held then, so a visit that read only the latest page made the
// disk forget the earlier ones. Cost: three visits of a 205-block transcript.
test('a reload reads the earlier pages it kept from the disk, and a visit that reads fewer keeps them', async () => {
  transcripts.set(FIRST, Array.from({ length: 205 }, (_, index) => message(`first-${index}`, `Message ${index}`)))
  await readOnline([FIRST])
  await readToStart(FIRST)
  await useConversations().keepCopies()
  expect(pagesRead).toEqual(['before=105', 'before=5'])
  closePage()

  // A visit that reads only what it shows at first.
  await readOnline([FIRST])
  closePage()

  pagesRead = []
  await readOnline([FIRST])
  await readToStart(FIRST)
  expect(pagesRead).toEqual([])
  expect(texts(FIRST)).toEqual(transcripts.get(FIRST)!.map((_, index) => `Message ${index}`))
})

/** Conversation `FIRST`'s transcript of `length` blocks, rewritten from the index `from` on when it is given. */
function longTranscript(length: number, from = length): Block[] {
  return Array.from({ length }, (_, index) => index < from
    ? message(`first-${index}`, `Message ${index}`)
    : message(`first-${index}-edited`, `Edited ${index}`))
}

/** What the page shows of conversation `id`'s root, a stale block marked so. */
const shown = (id: string) =>
  heldBlocks(useConversations().items.find((item) => item.id === id)!.history)
    .map((block) => `${block.type === 'user' && block.content[0]?.type === 'text' ? block.content[0].text : block.type}${block.stale ? ' (stale)' : ''}`)

// The transcript was rewritten while the page was away, here from its 10th
// block on: the reset starts at the latest page and cannot vouch for what
// the page holds before it. That stays on screen, stale, and is read again
// from the backend: the disk holds the old blocks, which do not join the
// new ones. Cost: three visits of a 150-block transcript.
test('blocks a reset cannot vouch for stay on screen and are read again from the backend, and the disk keeps the new ones', async () => {
  transcripts.set(FIRST, longTranscript(150))
  await readOnline([FIRST])
  await readToStart(FIRST)
  await useConversations().keepCopies()
  closePage()

  transcripts.set(FIRST, longTranscript(150, 10))
  pagesRead = []
  openPage()
  await signedIn()
  channels.last().connect(backendState())
  const conversations = useConversations()
  void conversations.activate(FIRST)
  await waitFor(() => conversations.items.find((item) => item.id === FIRST)?.load === 'ready')
  expect(shown(FIRST).slice(48, 51)).toEqual(['Message 48 (stale)', 'Message 49 (stale)', 'Edited 50'])
  await conversations.history.readBefore(conversations.items.find((item) => item.id === FIRST)!, null)
  expect(pagesRead).toEqual(['before=50'])
  expect(shown(FIRST)).toEqual(transcripts.get(FIRST)!.map((_, index) => index < 10 ? `Message ${index}` : `Edited ${index}`))
  await conversations.keepCopies()
  closePage()

  pagesRead = []
  await readOnline([FIRST])
  await readToStart(FIRST)
  expect(pagesRead).toEqual([])
  expect(texts(FIRST)).toEqual(transcripts.get(FIRST)!.map((_, index) => index < 10 ? `Message ${index}` : `Edited ${index}`))
})

// A page that kept the rewritten transcript's stale blocks without reading
// them again wrote its runs over the disk's: the next visit reads the
// earlier ones from the backend, never the old ones the disk held. Cost:
// three visits of a 150-block transcript.
test('a page that leaves before reading its stale blocks again leaves none of them on the disk', async () => {
  transcripts.set(FIRST, longTranscript(150))
  await readOnline([FIRST])
  await readToStart(FIRST)
  await useConversations().keepCopies()
  closePage()

  transcripts.set(FIRST, longTranscript(150, 10))
  await readOnline([FIRST])
  closePage()

  pagesRead = []
  await readOnline([FIRST])
  await readToStart(FIRST)
  expect(pagesRead).toEqual(['before=50'])
  expect(texts(FIRST)).toEqual(transcripts.get(FIRST)!.map((_, index) => index < 10 ? `Message ${index}` : `Edited ${index}`))
})
