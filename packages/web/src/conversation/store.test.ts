import { afterEach, beforeEach, expect, spyOn, test } from 'bun:test'
import { deferred } from '@demicodes/utils'
import { ConversationRuntime } from '@demicodes/web-ui/agent/conversation-runtime'
import { toasts } from '@demicodes/web-ui/infra/toast'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import { useConversations } from './store'
import { useProduct } from '../state/product'
import { productStateSchema, type BackendConversation } from '../api/contracts'
import { applyConversationEvent, updateLiveStatus } from './activity'

const realFetch = globalThis.fetch

/** The upload transport is XHR; this one answers every POST with a fresh attachment id. */
class FakeXhr {
  static nextId = 1
  upload = { onprogress: null as ((event: { lengthComputable: boolean; loaded: number; total: number }) => void) | null }
  status = 201
  responseText = ''
  timeout = 0
  withCredentials = false
  onload: (() => void) | null = null
  onerror: (() => void) | null = null
  ontimeout: (() => void) | null = null
  onabort: (() => void) | null = null
  open(): void {}
  setRequestHeader(): void {}
  abort(): void {}
  send(): void {
    this.responseText = JSON.stringify({ attachment: { id: `att-${FakeXhr.nextId++}` } })
    queueMicrotask(() => this.onload?.())
  }
}
globalThis.XMLHttpRequest = FakeXhr as unknown as typeof XMLHttpRequest
let records: BackendConversation[]
let rejectCreate: boolean
let rejectFork: boolean
let requests: {
  path: string
  body: unknown
}[]
let pinia: ReturnType<typeof createPinia>

function record(id: string): BackendConversation {
  return {
    id,
    title: id,
    pinned: false,
    archived: false,
    readRevision: 0,
    revision: 0,
    unread: false,
    target: { kind: 'cloud' },
    contextVersion: 0,
    providerId: null,
    modelId: null,
    createdAt: '2026-09-09T00:00:00.000Z',
    updatedAt: '2026-09-09T00:00:00.000Z',
    status: 'idle',
  }
}

function snapshot() {
  return productStateSchema.parse({
    user: {
      id: 'user',
      email: 'test@example.test',
      nickname: 'Test',
      role: 'master',
      createdAt: '2026-09-09T00:00:00.000Z',
    },
    mode: 'shared',
    preferences: {
      appearance: {},
      shortcuts: {},
    },
    devices: [],
    workspaces: [],
    providers: [],
    conversations: records,
    cloud: null,
  })
}

beforeEach(async () => {
  pinia = createPinia()
  setActivePinia(pinia)
  records = [record('first'), record('second')]
  rejectCreate = false
  rejectFork = false
  requests = []
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    const body = typeof init?.body === 'string' ? JSON.parse(init.body) : null
    requests.push({
      path,
      body,
    })
    if (path === '/api/state') {
      return Response.json(snapshot())
    }
    if (path.startsWith('/api/models')) {
      return Response.json({ providers: [] })
    }
    if (path === '/api/conversations/first/fork') {
      if (rejectFork) {
        return Response.json({ code: 'unavailable', message: 'Fork unavailable' }, { status: 503 })
      }
      const created = records.find((item) => item.id === body.id) ?? {
        ...record(body.id), title: 'first (Fork)', providerId: 'stub', modelId: 'model',
        target: { kind: 'cloud' as const, path: '/home/demi/sessions/first' },
      }
      if (!records.includes(created)) records.unshift(created)
      return Response.json({
        conversation: created,
        model: {
          providerId: 'stub', serviceTierId: 'priority',
          thinking: { type: 'effort', effort: 'high', summary: null },
          model: { id: 'model', name: 'Model', contextWindow: 1000,
            outputLimit: null, inputLimit: null, acceptedExtensions: [], thinking: [] },
        },
      }, { status: 201 })
    }
    if (path === '/api/conversations') {
      if (rejectCreate) {
        return Response.json(
          {
            code: 'unavailable',
            message: 'Not saved',
          },
          { status: 503 },
        )
      }
      const created = records.find((item) => item.id === body.id) ?? record(body.id)
      if (!records.includes(created)) {
        records.unshift(created)
      }
      return Response.json({ conversation: created }, { status: 201 })
    }
    if (path === '/api/conversations/batch') {
      const items = body.items as {
        id: string
        patch: Partial<BackendConversation>
      }[]
      return Response.json(
        {
          results: items.map((item) => {
            const current = records.find((record) => record.id === item.id)!
            if (item.id === 'second') {
              return {
                id: item.id,
                conversation: current,
                results: [
                  {
                    field: 'archived',
                    status: 'failed',
                    code: 'busy',
                    message: 'Turn is running',
                  },
                ],
              }
            }
            Object.assign(current, item.patch)
            return {
              id: item.id,
              conversation: current,
              results: Object.keys(item.patch).map((field) => ({
                field,
                status: 'applied',
              })),
            }
          }),
        },
        { status: 207 },
      )
    }
    if (path.endsWith('/read')) {
      return new Response(null, { status: 204 })
    }
    throw new Error(`Unexpected request: ${path}`)
  }) as typeof fetch
  useConversations()
  await useProduct().start()
  await nextTick()
})

afterEach(() => {
  useConversations().stopAll()
  useProduct().stop()
  disposePinia(pinia)
  globalThis.fetch = realFetch
})

test('sidebar stays active until the last running child closes after its parent finishes', () => {
  const conversation = useConversations().items[0]!
  const job = {
    subagentId: 'child-one',
    parentSessionId: conversation.id,
    description: 'First child',
    profile: null,
    phase: 'running' as const,
    startedAt: '2026-09-13T00:00:00.000Z',
    endedAt: null,
    metadata: null,
  }
  conversation.phase = 'idle'
  applyConversationEvent(conversation, { type: 'subagent', event: 'started', job })
  expect(conversation.status).toBe('active')
  const other = { ...job, subagentId: 'child-two' }
  applyConversationEvent(conversation, { type: 'subagent', event: 'started', job: other })
  applyConversationEvent(conversation, {
    type: 'subagent',
    event: 'closed',
    job: { ...job, phase: 'completed', endedAt: '2026-09-13T00:01:00.000Z' },
  })
  expect(conversation.status).toBe('active')
  conversation.lastError = 'The parent failed while its second child was working'
  updateLiveStatus(conversation)
  expect(conversation.status).toBe('active')
  applyConversationEvent(conversation, {
    type: 'subagent',
    event: 'closed',
    job: { ...other, phase: 'aborted', endedAt: '2026-09-13T00:02:00.000Z' },
  })
  expect(conversation.status).toBe('error')
  conversation.phase = 'running'
  updateLiveStatus(conversation)
  expect(conversation.status).toBe('active')
})

test('restored running children keep an idle parent active in the sidebar', () => {
  const conversation = useConversations().items[0]!
  conversation.phase = 'idle'
  conversation.subagents = [{
    id: 'restored-child',
    name: 'Restored child',
    phase: 'running',
    startedAt: '2026-09-13T00:00:00.000Z',
    blocks: [],
  }]
  updateLiveStatus(conversation)
  expect(conversation.status).toBe('active')
  conversation.subagents[0]!.phase = 'completed'
  updateLiveStatus(conversation)
  expect(conversation.status).toBe('idle')
})

test('new conversation is local and does not depend on the server', async () => {
  const store = useConversations()
  rejectCreate = true
  const id = await store.create()
  expect(id).toMatch(/^[0-9a-f-]{36}$/)
  expect(store.items.map((item) => item.id)).toEqual([id!, 'first', 'second'])
  expect(store.items[0]?.persistence).toBe('draft')
  expect(store.items[0]?.load).toBe('ready')
  expect(requests.some((request) => request.path === '/api/conversations')).toBe(false)
})

test('repeated new reuses the active empty draft and polling preserves it', async () => {
  const store = useConversations()
  const id = await store.create()
  await store.activate(id)
  expect(await store.create()).toBe(id)
  await useProduct().refresh()
  await nextTick()
  expect(store.items[0]?.id).toBe(id)
  store.items[0]!.draft = 'Keep this draft'
  expect(await store.create()).not.toBe(id)
})

test('first send failure preserves the conversation and retries its UUID and message', async () => {
  const store = useConversations()
  await store.create()
  const conversation = store.items[0]!
  conversation.draft = 'First message'
  rejectCreate = true
  await store.send(conversation)
  expect(conversation.persistence).toBe('pending')
  expect(conversation.pendingSend?.text).toBe('First message')
  expect(conversation.pendingSend?.error).toBe('Not saved')
  expect(conversation.draft).toBe('')
  const messageId = conversation.pendingSend!.id
  await useProduct().refresh()
  await nextTick()
  expect(store.items[0]?.id).toBe(conversation.id)
  await store.send(conversation)
  expect(conversation.pendingSend?.id).toBe(messageId)
  expect(requests.filter((request) => request.path === '/api/conversations').map((request) => request.body)).toEqual([
    { id: conversation.id },
    { id: conversation.id },
  ])
})

test('a draft keeps its files; the conversation itself is created on first send', async () => {
  const store = useConversations()
  store.create()
  const conversation = store.items[0]!
  store.addFiles(conversation, [new File(['Local notes'], 'notes.txt', { type: 'text/plain' })])
  await new Promise((resolve) => setTimeout(resolve, 0))
  expect(conversation.files[0]).toMatchObject({ kind: 'file', name: 'notes.txt', phase: 'ready', upload: { id: 'att-1' } })
  expect(requests.some((request) => request.path === '/api/conversations')).toBe(false)
  rejectCreate = true
  await store.send(conversation)
  expect(conversation.pendingSend?.fileIds).toEqual([conversation.files[0]!.id])
})

test('leaving an empty draft discards it without removing a draft with input', async () => {
  const store = useConversations()
  const empty = store.create()
  await store.activate(empty)
  await store.activate(null)
  expect(store.items.some((item) => item.id === empty)).toBe(false)
  const retained = store.create()
  await store.activate(retained)
  store.items.find((item) => item.id === retained)!.draft = 'Keep this'
  await store.activate(null)
  expect(store.items.some((item) => item.id === retained)).toBe(true)
})

test('snapshot refresh preserves live transcript and unsent draft', async () => {
  const store = useConversations()
  const current = store.items[0]!
  current.draft = 'Still editing'
  current.phase = 'running'
  current.blocks = [
    {
      type: 'extension_state_snapshot',
      id: 'block',
      createdAt: '2026-09-09T00:00:00.000Z',
      extensionName: 'example',
      state: { value: 1 },
    },
  ]
  records[0]!.title = 'Server title'
  records.reverse()
  await useProduct().refresh()
  await nextTick()
  expect(store.items[1]).toBe(current)
  expect(current.title).toBe('Server title')
  expect(current.draft).toBe('Still editing')
  expect(current.blocks[0]?.id).toBe('block')
  expect(current.phase).toBe('running')
})

test('history remains readable during its own connection after navigation', async () => {
  const store = useConversations()
  const current = store.items[0]!
  current.blocks = [{
    type: 'extension_state_snapshot', id: 'cached', extensionName: 'example',
    state: {}, createdAt: '2026-09-09T00:00:00.000Z',
  }]
  current.load = 'ready'
  current.draft = 'Keep the draft'
  useProduct().snapshot!.providers.push({
    id: 'stub', kind: 'api_key', providerType: 'stub', label: 'Stub',
    wireApi: null, vendorId: null, baseUrl: null, models: null,
    keyConfigured: true, details: null,
  })
  const historyRequested = deferred<void>()
  const history = deferred<Response>()
  const connecting = deferred<void>()
  const connection = deferred<void>()
  const connect = spyOn(ConversationRuntime.prototype, 'connect').mockImplementation(() => {
    connecting.resolve()
    return connection.promise
  })
  const originalFetch = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path.startsWith('/api/models')) {
      return Response.json({ providers: [{
        providerId: 'stub', displayName: 'Stub', sourceFetchedAt: '', stale: false,
        warnings: [], auth: { status: 'ready' }, runtime: { status: 'ready' },
        requiresProcessCapableHost: false,
        availability: { available: true, reason: null, message: null },
        models: [{
          id: 'stub', displayName: 'Stub', contextWindow: 1000, outputLimit: null,
          supportsAttachments: false, supportedThinkingEfforts: [], defaultThinkingEffort: null,
          selection: {
            providerId: 'stub', thinking: null,
            model: { id: 'stub', name: 'Stub', contextWindow: 1000, outputLimit: null,
              inputLimit: null, thinking: [], acceptedExtensions: [] },
          },
        }],
      }] })
    }
    if (path.endsWith('/hosts')) return Response.json({ hosts: [] })
    if (path.endsWith('/transcript')) {
      historyRequested.resolve()
      return history.promise
    }
    return originalFetch(input, init)
  }) as typeof fetch
  await useProduct().loadModels(true)
  const opening = store.activate(current.id)
  try {
    expect(current).toMatchObject({ load: 'loading' })
    await historyRequested.promise
    expect(current).toMatchObject({ load: 'loading' })
    useProduct().activeConversationId = 'second'
    history.resolve(Response.json({ blocks: current.blocks, subagents: [] }))
    await connecting.promise
    expect(current).toMatchObject({ load: 'ready' })
    expect(current.draft).toBe('Keep the draft')
    connection.reject(new Error('Connection failed'))
    await opening
    expect(current).toMatchObject({ load: 'failed' })
    expect(current.lastError).toBe('Connection failed')
  } finally {
    history.resolve(Response.json({ blocks: [], subagents: [] }))
    connection.resolve()
    await opening
    connect.mockRestore()
    globalThis.fetch = originalFetch
  }
})

test('batch partial failure applies only the successful server records', async () => {
  const store = useConversations()
  expect(await store.archive(['first', 'second'])).toBe(false)
  expect(store.items.find((item) => item.id === 'first')?.archived).toBe(true)
  expect(store.items.find((item) => item.id === 'second')?.archived).toBe(false)
  expect(toasts.some((toast) => toast.message?.includes('second: Turn is running'))).toBe(true)
})

test('read acknowledgements use the observed revision and wait for history', async () => {
  const original = globalThis.document
  Object.defineProperty(globalThis, 'document', {
    configurable: true,
    value: { visibilityState: 'visible' },
  })
  try {
    const store = useConversations()
    const current = store.items[0]!
    current.unread = true
    current.revision = 7
    await store.markRead(current.id)
    expect(requests.some((request) => request.path.endsWith('/read'))).toBe(false)
    current.load = 'ready'
    await store.markRead(current.id)
    expect(requests.at(-1)?.body).toEqual({ revision: 7 })
    expect(current.unread).toBe(false)
  } finally {
    Object.defineProperty(globalThis, 'document', {
      configurable: true,
      value: original,
    })
  }
})

test('Fork preserves the source draft and opens an independent empty composer with inherited model settings', async () => {
  const store = useConversations()
  const source = store.items.find((item) => item.id === 'first')!
  source.draft = 'Keep this unsent message'
  source.phase = 'running'
  const request = { id: crypto.randomUUID(), blockId: 'completed-answer' }
  expect(await store.fork(source.id, request)).toBe(request.id)
  const branch = store.items.find((item) => item.id === request.id)!
  expect(branch.title).toBe('first (Fork)')
  expect(branch.draft).toBe('')
  expect(branch.files).toEqual([])
  expect(branch.model).toEqual({ providerId: 'stub', modelId: 'model', thinkingEffort: 'high', serviceTierId: 'priority' })
  expect(branch.target).toEqual({ kind: 'cloud', path: '/home/demi/sessions/first' })
  expect(source.draft).toBe('Keep this unsent message')
  expect(source.phase).toBe('running')
})

test('failed Fork creates no local conversation; retry forwards the same destination ID', async () => {
  const store = useConversations()
  const request = { id: crypto.randomUUID(), blockId: 'completed-answer' }
  rejectFork = true
  await expect(store.fork('first', request)).rejects.toThrow('Fork unavailable')
  expect(store.items.some((item) => item.id === request.id)).toBe(false)
  rejectFork = false
  await store.fork('first', request)
  expect(requests.filter((item) => item.path.endsWith('/fork')).map((item) => item.body))
    .toEqual([request, request])
  expect(store.items.filter((item) => item.id === request.id)).toHaveLength(1)
})

function serveHistory(gate?: ReturnType<typeof deferred<void>>) {
  const requested = deferred<void>()
  const originalFetch = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path.endsWith('/hosts') || path.endsWith('/transcript')) {
      requests.push({ path, body: null })
      if (path.endsWith('/hosts')) {
        return Response.json({ hosts: [] })
      }
      requested.resolve()
      await gate?.promise
      return Response.json({ blocks: [], subagents: [] })
    }
    return originalFetch(input, init)
  }) as typeof fetch
  return requested.promise
}

test('switching between opened sessions performs no reads or load reset', async () => {
  serveHistory()
  const store = useConversations()
  await store.activate('first')
  const first = store.items.find((item) => item.id === 'first')!
  first.draft = 'Keep this input'
  await store.activate('second')
  const count = requests.length
  const returning = store.activate('first')
  expect(first.load).toBe('ready')
  expect(useProduct().activeConversationId).toBe('first')
  await returning
  await store.activate('second')
  await store.activate('first')
  expect(requests).toHaveLength(count)
  expect(first.draft).toBe('Keep this input')
})

test('leaving and returning during history loading shares the pending request', async () => {
  const gate = deferred<void>()
  serveHistory(gate)
  const store = useConversations()
  const first = store.activate('first')
  await store.activate(null)
  const returning = store.activate('first')
  gate.resolve()
  await Promise.all([first, returning])
  expect(requests.filter((item) => item.path.endsWith('/transcript'))).toHaveLength(1)
  expect(store.items[0]!.load).toBe('ready')
})

test('inactive context changes invalidate only that session; retry explicitly reloads', async () => {
  serveHistory()
  const store = useConversations()
  await store.activate('first')
  await store.activate('second')
  records[0]!.contextVersion += 1
  await useProduct().refresh()
  await nextTick()
  expect(requests.filter((item) => item.path.endsWith('/transcript'))).toHaveLength(2)
  await store.activate('first')
  expect(requests.filter((item) => item.path.endsWith('/transcript'))).toHaveLength(3)
  await store.activate('second')
  expect(requests.filter((item) => item.path.endsWith('/transcript'))).toHaveLength(3)
  await store.reloadSession('second')
  expect(requests.filter((item) => item.path.endsWith('/transcript'))).toHaveLength(4)
})

test('archive changes and removal invalidate cached history', async () => {
  serveHistory()
  const store = useConversations()
  await store.activate('first')
  await store.activate(null)
  records[0]!.archived = true
  await useProduct().refresh()
  await nextTick()
  await store.activate('first')
  expect(requests.filter((item) => item.path.endsWith('/transcript'))).toHaveLength(2)
  await store.activate(null)
  const removed = records.shift()!
  await useProduct().refresh()
  await nextTick()
  records.unshift(removed)
  await useProduct().refresh()
  await nextTick()
  await store.activate('first')
  expect(requests.filter((item) => item.path.endsWith('/transcript'))).toHaveLength(3)
})

test('logout cancels pending history and prevents late state restoration', async () => {
  const gate = deferred<void>()
  const requested = serveHistory(gate)
  const store = useConversations()
  const current = store.items[0]!
  const opening = store.activate('first')
  await requested
  store.stopAll()
  gate.resolve()
  await opening
  expect(store.items).toEqual([])
  expect(current.load).toBe('loading')
})


test('slow or failed model discovery does not hold history behind the loading pane', async () => {
  const store = useConversations()
  const current = store.items[0]!
  const modelResponse = deferred<Response>()
  const originalFetch = globalThis.fetch
  const block = {
    type: 'user', id: 'visible-history', turnId: 'history-turn', preamble: null,
    model: {
      providerId: 'fixture', thinking: null,
      model: { id: 'fixture', name: 'Fixture', contextWindow: 1000,
        outputLimit: null, inputLimit: null, thinking: [], acceptedExtensions: [] },
    },
    createdAt: '2026-09-13T00:00:00.000Z', content: [{ type: 'text', text: 'Read this while models load' }],
  }
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path.startsWith('/api/models')) return modelResponse.promise
    if (path.endsWith('/hosts')) return Response.json({ hosts: [] })
    if (path.endsWith('/transcript')) return Response.json({ blocks: [block], subagents: [] })
    return originalFetch(input, init)
  }) as typeof fetch
  const models = useProduct().loadModels(true).catch(error => error)
  const opening = store.activate(current.id)
  await new Promise(resolve => setTimeout(resolve, 0))
  expect(current.lastError).toBeNull()
  expect(current.load).toBe('ready')
  expect(current.blocks[0]?.id).toBe('visible-history')
  modelResponse.reject(new Error('Catalog offline'))
  expect(await models).toBeInstanceOf(Error)
  await opening
  expect(current.load).toBe('ready')
  expect(current.blocks[0]?.id).toBe('visible-history')
})
